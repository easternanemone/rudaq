//! Streaming acquisition module for high-performance multi-channel data acquisition.
//!
//! This module provides command-based acquisition using Comedi's asynchronous I/O
//! interface. It supports:
//!
//! - Multi-channel synchronized acquisition
//! - Hardware timing (internal timer or external clock)
//! - DMA buffer management
//! - Continuous and finite acquisitions
//! - Overflow detection and recovery
//!
//! # Architecture
//!
//! The streaming system uses Comedi's command interface which provides:
//! - Hardware-timed sampling for accurate inter-sample timing
//! - DMA transfers for minimal CPU overhead
//! - Double-buffered ring buffer for continuous acquisition
//!
//! # Example
//!
//! ```no_run
//! use daq_driver_comedi::{ComediDevice, StreamConfig, StreamAcquisition};
//!
//! # fn example() -> anyhow::Result<()> {
//! let device = ComediDevice::open("/dev/comedi0")?;
//!
//! let config = StreamConfig::builder()
//!     .channels(&[0, 1, 2, 3])
//!     .sample_rate(10000.0)  // 10 kS/s per channel
//!     .build()?;
//!
//! let mut stream = StreamAcquisition::new(&device, config)?;
//! stream.start()?;
//!
//! while let Some(samples) = stream.read_available()? {
//!     println!("Got {} samples", samples.len());
//! }
//!
//! stream.stop()?;
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::{debug, error, info, trace, warn};

use comedi_sys::{
    comedi_cmd, lsampl_t, AREF_GROUND, CMDF_PRIORITY, CR_PACK, SDF_LSAMPL, TRIG_COUNT, TRIG_EXT,
    TRIG_FOLLOW, TRIG_NOW, TRIG_TIMER,
};

use crate::device::{ComediDevice, SubdeviceType};
use crate::error::{ComediError, Result};
use crate::subsystem::Range;

/// Trigger source for acquisition timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerSource {
    /// Internal hardware timer
    #[default]
    Internal,
    /// External trigger input
    External {
        /// Trigger input line (PFI pin number for NI cards)
        input: u32,
    },
    /// Software trigger (immediate start)
    Software,
    /// Follow previous stage (for chained triggers)
    Follow,
}

impl TriggerSource {
    fn to_raw(self) -> u32 {
        match self {
            Self::Internal => TRIG_TIMER,
            Self::External { .. } => TRIG_EXT,
            Self::Software => TRIG_NOW,
            Self::Follow => TRIG_FOLLOW,
        }
    }

    fn to_arg(self) -> u32 {
        match self {
            Self::External { input } => input,
            _ => 0,
        }
    }
}

/// Stop condition for acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopCondition {
    /// Acquire indefinitely until stopped
    #[default]
    Continuous,
    /// Acquire fixed number of samples per channel
    Count(u64),
    /// Acquire for specified duration
    Duration(Duration),
}

/// Channel configuration for streaming acquisition.
#[derive(Debug, Clone)]
pub struct ChannelSpec {
    /// Channel number
    pub channel: u32,
    /// Range index to use
    pub range: u32,
    /// Analog reference (default: ground)
    pub aref: u32,
}

impl ChannelSpec {
    /// Create a new channel specification.
    pub fn new(channel: u32) -> Self {
        Self {
            channel,
            range: 0,
            aref: AREF_GROUND,
        }
    }

    /// Set the voltage range.
    pub fn with_range(mut self, range: u32) -> Self {
        self.range = range;
        self
    }

    /// Set the analog reference.
    pub fn with_aref(mut self, aref: u32) -> Self {
        self.aref = aref;
        self
    }

    /// Pack into Comedi chanlist format.
    fn pack(&self) -> u32 {
        CR_PACK(self.channel, self.range, self.aref)
    }
}

/// Configuration for streaming acquisition.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// Channels to sample (in order)
    pub channels: Vec<ChannelSpec>,
    /// Sample rate per channel in Hz
    pub sample_rate: f64,
    /// Scan rate (if different from sample_rate * n_channels)
    pub scan_rate: Option<f64>,
    /// Convert interval in nanoseconds (inter-channel delay)
    pub convert_interval_ns: Option<u32>,
    /// Start trigger source
    pub start_trigger: TriggerSource,
    /// Scan begin trigger source
    pub scan_trigger: TriggerSource,
    /// Stop condition
    pub stop: StopCondition,
    /// Buffer size in samples (per channel)
    pub buffer_size: usize,
    /// Enable priority mode (requires root)
    pub priority: bool,
    /// Subdevice to use (auto-detected if None)
    pub subdevice: Option<u32>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            channels: vec![ChannelSpec::new(0)],
            sample_rate: 1000.0,
            scan_rate: None,
            convert_interval_ns: None,
            start_trigger: TriggerSource::Software,
            scan_trigger: TriggerSource::Internal,
            stop: StopCondition::Continuous,
            buffer_size: 16384,
            priority: false,
            subdevice: None,
        }
    }
}

impl StreamConfig {
    /// Create a new builder for stream configuration.
    pub fn builder() -> StreamConfigBuilder {
        StreamConfigBuilder::default()
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<()> {
        if self.channels.is_empty() {
            return Err(ComediError::InvalidConfig {
                message: "At least one channel is required".to_string(),
            });
        }

        if self.sample_rate <= 0.0 {
            return Err(ComediError::InvalidConfig {
                message: format!("Invalid sample rate: {}", self.sample_rate),
            });
        }

        if self.buffer_size == 0 {
            return Err(ComediError::InvalidConfig {
                message: "Buffer size must be greater than 0".to_string(),
            });
        }

        Ok(())
    }

    /// Calculate the scan interval in nanoseconds.
    pub fn scan_interval_ns(&self) -> u32 {
        let scan_rate = self.scan_rate.unwrap_or(self.sample_rate);
        (1e9 / scan_rate) as u32
    }

    /// Calculate the convert interval in nanoseconds.
    pub fn convert_interval_ns(&self) -> u32 {
        self.convert_interval_ns.unwrap_or_else(|| {
            // Default: evenly space conversions within scan
            let scan_ns = self.scan_interval_ns();
            let n_channels = self.channels.len() as u32;
            if n_channels > 1 {
                scan_ns / n_channels
            } else {
                0 // Single channel doesn't need convert timing
            }
        })
    }
}

/// Builder for StreamConfig.
#[derive(Debug, Default)]
pub struct StreamConfigBuilder {
    config: StreamConfig,
}

impl StreamConfigBuilder {
    /// Set the channels to sample.
    pub fn channels(mut self, channels: &[u32]) -> Self {
        self.config.channels = channels.iter().map(|&ch| ChannelSpec::new(ch)).collect();
        self
    }

    /// Set the channels with full specifications.
    pub fn channel_specs(mut self, specs: Vec<ChannelSpec>) -> Self {
        self.config.channels = specs;
        self
    }

    /// Set the sample rate in Hz.
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.config.sample_rate = rate;
        self
    }

    /// Set the scan rate (defaults to sample_rate if not set).
    pub fn scan_rate(mut self, rate: f64) -> Self {
        self.config.scan_rate = Some(rate);
        self
    }

    /// Set the inter-channel convert interval.
    pub fn convert_interval_ns(mut self, ns: u32) -> Self {
        self.config.convert_interval_ns = Some(ns);
        self
    }

    /// Set the start trigger source.
    pub fn start_trigger(mut self, trigger: TriggerSource) -> Self {
        self.config.start_trigger = trigger;
        self
    }

    /// Set the scan trigger source.
    pub fn scan_trigger(mut self, trigger: TriggerSource) -> Self {
        self.config.scan_trigger = trigger;
        self
    }

    /// Set the stop condition.
    pub fn stop(mut self, stop: StopCondition) -> Self {
        self.config.stop = stop;
        self
    }

    /// Set the buffer size in samples per channel.
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.config.buffer_size = size;
        self
    }

    /// Enable priority mode (requires root).
    pub fn priority(mut self, enable: bool) -> Self {
        self.config.priority = enable;
        self
    }

    /// Set the subdevice explicitly.
    pub fn subdevice(mut self, subdev: u32) -> Self {
        self.config.subdevice = Some(subdev);
        self
    }

    /// Build the configuration.
    pub fn build(self) -> Result<StreamConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

/// Statistics for streaming acquisition.
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    /// Total samples acquired per channel
    pub samples_acquired: u64,
    /// Total scans acquired
    pub scans_acquired: u64,
    /// Number of buffer overflows detected
    pub overflows: u64,
    /// Actual sample rate achieved
    pub actual_sample_rate: f64,
    /// Time acquisition has been running
    pub elapsed: Duration,
    /// Current buffer fill level (0.0 - 1.0)
    pub buffer_fill: f64,
}

/// Internal state for the acquisition.
#[allow(dead_code)]
struct StreamState {
    /// Comedi command structure
    cmd: comedi_cmd,
    /// Channel list for the command
    chanlist: Vec<u32>,
    /// Read buffer
    read_buffer: Vec<u8>,
    /// Subdevice being used
    subdevice: u32,
    /// Whether using lsampl_t (32-bit) or sampl_t (16-bit)
    use_lsampl: bool,
    /// Maximum data value
    maxdata: lsampl_t,
    /// Voltage ranges for each channel
    ranges: Vec<Range>,
    /// Start time of acquisition
    start_time: Option<Instant>,
    /// Total bytes read
    bytes_read: u64,
}

// SAFETY: StreamState contains raw pointers from comedi_cmd (chanlist, data)
// but these are only accessed through the Mutex which provides synchronization.
// The pointers point to memory owned by the state itself (chanlist Vec).
unsafe impl Send for StreamState {}

/// High-performance streaming acquisition.
///
/// This provides hardware-timed multi-channel acquisition using Comedi's
/// asynchronous command interface.
pub struct StreamAcquisition {
    device: ComediDevice,
    config: StreamConfig,
    state: Mutex<StreamState>,
    running: AtomicBool,
    samples_acquired: AtomicU64,
    overflows: AtomicU64,
}

impl StreamAcquisition {
    /// Create a new streaming acquisition.
    pub fn new(device: &ComediDevice, config: StreamConfig) -> Result<Self> {
        config.validate()?;

        // Find AI subdevice that supports commands
        let subdevice = config.subdevice.unwrap_or_else(|| {
            device
                .find_subdevice(SubdeviceType::AnalogInput)
                .unwrap_or(0)
        });

        // Verify subdevice supports commands
        let info = device.subdevice_info(subdevice)?;
        if !info.supports_commands() {
            return Err(ComediError::NotSupported {
                message: format!(
                    "Subdevice {} does not support command-based acquisition",
                    subdevice
                ),
            });
        }

        // Check if using lsampl (32-bit) or sampl (16-bit)
        let use_lsampl = info.flags & SDF_LSAMPL != 0;
        let maxdata = info.maxdata;

        // Build channel list
        let chanlist: Vec<u32> = config.channels.iter().map(|ch| ch.pack()).collect();

        // Get range info for each channel
        let ranges: Vec<Range> = config
            .channels
            .iter()
            .map(|ch| {
                let ptr = unsafe {
                    comedi_sys::comedi_get_range(device.handle(), subdevice, ch.channel, ch.range)
                };
                unsafe { Range::from_ptr(ch.range, ptr) }
                    .unwrap_or_else(|| Range::new(ch.range, -10.0, 10.0))
            })
            .collect();

        // Calculate read buffer size
        let sample_size = if use_lsampl { 4 } else { 2 };
        let buffer_bytes = config.buffer_size * config.channels.len() * sample_size;

        // Initialize state
        let state = StreamState {
            cmd: comedi_cmd::default(),
            chanlist,
            read_buffer: vec![0u8; buffer_bytes],
            subdevice,
            use_lsampl,
            maxdata,
            ranges,
            start_time: None,
            bytes_read: 0,
        };

        info!(
            subdevice = subdevice,
            n_channels = config.channels.len(),
            sample_rate = config.sample_rate,
            buffer_size = config.buffer_size,
            use_lsampl = use_lsampl,
            "Created streaming acquisition"
        );

        Ok(Self {
            device: device.clone(),
            config,
            state: Mutex::new(state),
            running: AtomicBool::new(false),
            samples_acquired: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
        })
    }

    /// Start the acquisition.
    pub fn start(&self) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Err(ComediError::DeviceBusy {
                path: self.device.path().to_string(),
            });
        }

        let mut state = self.state.lock();

        // Build the command structure
        self.build_command(&mut state)?;

        // Test the command (multiple passes may be needed)
        self.test_command(&mut state)?;

        // Execute the command
        let result = self
            .device
            .with_handle(|handle| unsafe { comedi_sys::comedi_command(handle, &mut state.cmd) });

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        state.start_time = Some(Instant::now());
        self.running.store(true, Ordering::SeqCst);
        self.samples_acquired.store(0, Ordering::SeqCst);

        info!(
            subdevice = state.subdevice,
            sample_rate = self.config.sample_rate,
            "Started streaming acquisition"
        );

        Ok(())
    }

    /// Stop the acquisition.
    pub fn stop(&self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }

        let state = self.state.lock();

        let result = self
            .device
            .with_handle(|handle| unsafe { comedi_sys::comedi_cancel(handle, state.subdevice) });

        if result < 0 {
            warn!("Error cancelling acquisition: {}", unsafe {
                ComediError::from_errno()
            });
        }

        self.running.store(false, Ordering::SeqCst);

        info!(
            samples = self.samples_acquired.load(Ordering::SeqCst),
            "Stopped streaming acquisition"
        );

        Ok(())
    }

    /// Check if acquisition is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Read available samples without blocking.
    ///
    /// Returns a vector of voltage readings organized as [scan0_ch0, scan0_ch1, ...].
    /// Returns None if acquisition is not running.
    pub fn read_available(&self) -> Result<Option<Vec<f64>>> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(None);
        }

        let mut state = self.state.lock();

        // Check how many bytes are available
        let available = self.device.with_handle(|handle| unsafe {
            comedi_sys::comedi_get_buffer_contents(handle, state.subdevice)
        });

        if available <= 0 {
            return Ok(Some(Vec::new()));
        }

        let available = available as usize;
        let sample_size = if state.use_lsampl { 4 } else { 2 };
        let scan_bytes = self.config.channels.len() * sample_size;

        // Round down to complete scans
        let scans_available = available / scan_bytes;
        if scans_available == 0 {
            return Ok(Some(Vec::new()));
        }

        let bytes_to_read = scans_available * scan_bytes;

        // Read from the device file descriptor
        let fd = self.device.fileno();
        let read_result = unsafe {
            libc::read(
                fd,
                state.read_buffer.as_mut_ptr() as *mut libc::c_void,
                bytes_to_read,
            )
        };

        if read_result < 0 {
            let errno = std::io::Error::last_os_error();
            if errno.raw_os_error() == Some(libc::EAGAIN) {
                return Ok(Some(Vec::new()));
            }
            return Err(ComediError::IoError {
                message: format!("Read error: {}", errno),
            });
        }

        let bytes_read = read_result as usize;
        state.bytes_read += bytes_read as u64;

        // Mark buffer as read
        self.device.with_handle(|handle| unsafe {
            comedi_sys::comedi_mark_buffer_read(handle, state.subdevice, bytes_read as u32)
        });

        // Convert to voltages
        let samples_read = bytes_read / sample_size;
        let mut voltages = Vec::with_capacity(samples_read);

        for i in 0..samples_read {
            let ch_idx = i % self.config.channels.len();
            let raw = if state.use_lsampl {
                let offset = i * 4;
                u32::from_ne_bytes([
                    state.read_buffer[offset],
                    state.read_buffer[offset + 1],
                    state.read_buffer[offset + 2],
                    state.read_buffer[offset + 3],
                ])
            } else {
                let offset = i * 2;
                u16::from_ne_bytes([state.read_buffer[offset], state.read_buffer[offset + 1]])
                    as u32
            };

            let range = &state.ranges[ch_idx];
            let voltage = self.raw_to_voltage(raw, range, state.maxdata);
            voltages.push(voltage);
        }

        // Update statistics
        let scans_read = samples_read / self.config.channels.len();
        self.samples_acquired
            .fetch_add(scans_read as u64, Ordering::SeqCst);

        trace!(
            bytes = bytes_read,
            scans = scans_read,
            "Read streaming data"
        );

        Ok(Some(voltages))
    }

    /// Read a specific number of scans, blocking if necessary.
    ///
    /// Returns voltage readings organized as [scan0_ch0, scan0_ch1, ..., scan1_ch0, ...].
    pub fn read_n_scans(&self, n_scans: usize) -> Result<Vec<f64>> {
        let n_samples = n_scans * self.config.channels.len();
        let mut result = Vec::with_capacity(n_samples);

        while result.len() < n_samples {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }

            if let Some(mut samples) = self.read_available()? {
                let remaining = n_samples - result.len();
                if samples.len() > remaining {
                    samples.truncate(remaining);
                }
                result.extend(samples);
            }

            if result.len() < n_samples {
                // Wait for more data
                std::thread::sleep(Duration::from_micros(100));
            }
        }

        Ok(result)
    }

    /// Get current statistics.
    pub fn stats(&self) -> StreamStats {
        let state = self.state.lock();
        let samples = self.samples_acquired.load(Ordering::SeqCst);

        let elapsed = state
            .start_time
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);

        let actual_rate = if elapsed.as_secs_f64() > 0.0 {
            samples as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        // Get buffer fill level
        let available = self.device.with_handle(|handle| unsafe {
            comedi_sys::comedi_get_buffer_contents(handle, state.subdevice)
        });
        let buffer_size = self.device.with_handle(|handle| unsafe {
            comedi_sys::comedi_get_buffer_size(handle, state.subdevice)
        });

        let buffer_fill = if buffer_size > 0 {
            available.max(0) as f64 / buffer_size as f64
        } else {
            0.0
        };

        StreamStats {
            samples_acquired: samples,
            scans_acquired: samples,
            overflows: self.overflows.load(Ordering::SeqCst),
            actual_sample_rate: actual_rate,
            elapsed,
            buffer_fill,
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &StreamConfig {
        &self.config
    }

    /// Build the comedi command structure.
    fn build_command(&self, state: &mut StreamState) -> Result<()> {
        let n_channels = self.config.channels.len() as u32;

        // Calculate timing
        let scan_interval_ns = self.config.scan_interval_ns();
        let convert_interval_ns = self.config.convert_interval_ns();

        // Determine stop source and argument
        let (stop_src, stop_arg) = match self.config.stop {
            StopCondition::Continuous => (TRIG_COUNT, 0u32), // 0 = infinite for NI
            StopCondition::Count(n) => (TRIG_COUNT, n as u32),
            StopCondition::Duration(d) => {
                let scans = (d.as_secs_f64() * self.config.sample_rate) as u32;
                (TRIG_COUNT, scans)
            }
        };

        state.cmd = comedi_cmd {
            subdev: state.subdevice,
            flags: if self.config.priority {
                CMDF_PRIORITY
            } else {
                0
            },

            // Start: software trigger by default
            start_src: self.config.start_trigger.to_raw(),
            start_arg: self.config.start_trigger.to_arg(),

            // Scan begin: timer trigger
            scan_begin_src: self.config.scan_trigger.to_raw(),
            scan_begin_arg: scan_interval_ns,

            // Convert: follow scan or timer
            convert_src: if n_channels > 1 { TRIG_TIMER } else { TRIG_NOW },
            convert_arg: convert_interval_ns,

            // Scan end: count channels
            scan_end_src: TRIG_COUNT,
            scan_end_arg: n_channels,

            // Stop
            stop_src,
            stop_arg,

            // Channel list
            chanlist: state.chanlist.as_mut_ptr(),
            chanlist_len: n_channels,

            // Data (not used for streaming)
            data: std::ptr::null_mut(),
            data_len: 0,
        };

        debug!(
            scan_interval_ns = scan_interval_ns,
            convert_interval_ns = convert_interval_ns,
            n_channels = n_channels,
            stop_arg = stop_arg,
            "Built command structure"
        );

        Ok(())
    }

    /// Test and adjust the command.
    fn test_command(&self, state: &mut StreamState) -> Result<()> {
        // Comedi command test may need multiple passes
        for pass in 0..3 {
            let result = self.device.with_handle(|handle| unsafe {
                comedi_sys::comedi_command_test(handle, &mut state.cmd)
            });

            match result {
                0 => {
                    debug!(pass = pass, "Command test passed");
                    return Ok(());
                }
                1..=5 => {
                    // Command was modified, try again
                    debug!(
                        pass = pass,
                        result = result,
                        "Command test modified parameters, retrying"
                    );
                }
                _ => {
                    return Err(ComediError::CommandError {
                        code: result,
                        message: format!("Command test failed with code {}", result),
                    });
                }
            }
        }

        // Accept after 3 passes (parameters were adjusted)
        warn!("Command test needed multiple passes, using adjusted parameters");
        Ok(())
    }

    /// Convert raw ADC value to voltage.
    fn raw_to_voltage(&self, raw: u32, range: &Range, maxdata: lsampl_t) -> f64 {
        let fraction = raw as f64 / maxdata as f64;
        range.min + fraction * range.span()
    }
}

impl Drop for StreamAcquisition {
    fn drop(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            if let Err(e) = self.stop() {
                error!("Error stopping acquisition on drop: {}", e);
            }
        }
    }
}

/// Shared streaming acquisition wrapped in Arc for multi-threaded access.
pub type SharedStreamAcquisition = Arc<StreamAcquisition>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = StreamConfig::builder()
            .channels(&[0, 1, 2, 3])
            .sample_rate(10000.0)
            .buffer_size(8192)
            .build()
            .unwrap();

        assert_eq!(config.channels.len(), 4);
        assert_eq!(config.sample_rate, 10000.0);
        assert_eq!(config.buffer_size, 8192);
    }

    #[test]
    fn test_config_validation() {
        // Empty channels should fail
        let result = StreamConfig::builder().channels(&[]).build();
        assert!(result.is_err());

        // Zero sample rate should fail
        let result = StreamConfig::builder()
            .channels(&[0])
            .sample_rate(0.0)
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_channel_spec_pack() {
        let spec = ChannelSpec::new(5).with_range(2).with_aref(AREF_GROUND);
        let packed = spec.pack();

        // Verify unpacking
        assert_eq!(comedi_sys::CR_CHAN(packed), 5);
        assert_eq!(comedi_sys::CR_RANGE(packed), 2);
        assert_eq!(comedi_sys::CR_AREF(packed), AREF_GROUND);
    }

    #[test]
    fn test_scan_interval_calculation() {
        let config = StreamConfig::builder()
            .channels(&[0, 1])
            .sample_rate(10000.0)
            .build()
            .unwrap();

        // 10kHz = 100us = 100000ns
        assert_eq!(config.scan_interval_ns(), 100000);
    }
}
