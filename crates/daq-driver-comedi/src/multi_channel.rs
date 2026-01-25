//! Multi-channel analog input streaming for high-speed data acquisition.
//!
//! This module provides synchronized acquisition from multiple analog input channels
//! with hardware timing and background buffering.
//!
//! # Architecture
//!
//! The multi-channel acquisition uses Comedi's command interface for hardware-timed
//! sampling. A background task reads data from the Comedi buffer and writes it to
//! an internal ring buffer for lock-free consumer access.
//!
//! # Example
//!
//! ```no_run
//! use daq_driver_comedi::{ComediDevice, multi_channel::ComediMultiChannelAcquisition};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let device_path = "/dev/comedi0";
//! let channels = vec![0, 1, 2, 3];
//! let sample_rate = 10000.0; // 10 kS/s per channel
//!
//! let mut acq = ComediMultiChannelAcquisition::new_async(
//!     device_path,
//!     channels,
//!     sample_rate
//! ).await?;
//!
//! acq.start_acquisition().await?;
//!
//! // Read latest samples
//! let samples = acq.get_latest_samples(1000)?;
//! println!("Read {} samples per channel", samples[0].len());
//!
//! acq.stop_acquisition().await?;
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use tokio::task::{self, JoinHandle};
use tracing::{debug, error, info, warn};

use crate::device::{ComediDevice, SubdeviceType};
use crate::error::ComediError;
use crate::streaming::{StreamAcquisition, StreamConfig};

/// Internal circular buffer for storing samples.
///
/// This is a simple ring buffer implementation for lock-free reads
/// with a single writer (background task).
struct SimpleRingBuffer {
    /// Buffer storage (interleaved samples: [ch0, ch1, ..., chN, ch0, ch1, ...])
    data: Vec<f64>,
    /// Write position (samples written, wraps around capacity)
    write_pos: AtomicU64,
    /// Number of channels
    n_channels: usize,
    /// Capacity in total samples (not per channel)
    capacity: usize,
}

impl SimpleRingBuffer {
    /// Create a new ring buffer.
    ///
    /// # Arguments
    ///
    /// * `capacity_samples` - Total capacity in samples (will be rounded up to multiple of n_channels)
    /// * `n_channels` - Number of channels
    fn new(capacity_samples: usize, n_channels: usize) -> Self {
        // Round up to multiple of n_channels for complete scans
        let capacity = capacity_samples.div_ceil(n_channels) * n_channels;

        Self {
            data: vec![0.0; capacity],
            write_pos: AtomicU64::new(0),
            n_channels,
            capacity,
        }
    }

    /// Write a scan (one sample per channel).
    ///
    /// # Arguments
    ///
    /// * `scan` - Samples for all channels [ch0, ch1, ..., chN]
    ///
    /// # Panics
    ///
    /// Panics if scan length doesn't match n_channels.
    fn write_scan(&self, scan: &[f64]) {
        assert_eq!(scan.len(), self.n_channels, "Scan length mismatch");

        let pos = self.write_pos.load(Ordering::Relaxed) as usize;
        let start_idx = pos % self.capacity;

        // Write samples (may wrap around)
        for (i, sample) in scan.iter().enumerate() {
            let idx = (start_idx + i) % self.capacity;
            // SAFETY: Single writer, idx is always in bounds
            unsafe {
                let ptr = self.data.as_ptr().add(idx) as *mut f64;
                ptr.write(*sample);
            }
        }

        // Update write position
        self.write_pos
            .fetch_add(self.n_channels as u64, Ordering::Release);
    }

    /// Read the most recent N samples per channel.
    ///
    /// Returns a Vec<Vec<f64>> where each inner Vec contains samples for one channel.
    ///
    /// # Arguments
    ///
    /// * `num_samples` - Number of samples to read per channel
    fn read_latest(&self, num_samples: usize) -> Vec<Vec<f64>> {
        let total_written = self.write_pos.load(Ordering::Acquire);
        let scans_written = (total_written as usize) / self.n_channels;

        // Clamp to available data
        let scans_to_read = num_samples
            .min(scans_written)
            .min(self.capacity / self.n_channels);

        if scans_to_read == 0 {
            return vec![Vec::new(); self.n_channels];
        }

        // Calculate start position (read backwards from write position)
        let read_start = total_written.saturating_sub((scans_to_read * self.n_channels) as u64);
        let start_idx = (read_start as usize) % self.capacity;

        // Pre-allocate channel vectors
        let mut channels: Vec<Vec<f64>> = vec![Vec::with_capacity(scans_to_read); self.n_channels];

        // Read samples
        for scan_idx in 0..scans_to_read {
            for (ch_idx, channel) in channels.iter_mut().enumerate() {
                let buf_idx = (start_idx + scan_idx * self.n_channels + ch_idx) % self.capacity;
                // SAFETY: buf_idx is always in bounds, Acquire ordering ensures visibility
                let sample = unsafe { *self.data.as_ptr().add(buf_idx) };
                channel.push(sample);
            }
        }

        channels
    }

    /// Get the total number of scans written (may exceed capacity).
    fn scans_written(&self) -> usize {
        (self.write_pos.load(Ordering::Acquire) as usize) / self.n_channels
    }
}

/// Multi-channel analog input acquisition with background streaming.
///
/// This struct manages hardware-timed acquisition from multiple channels,
/// buffering data in a ring buffer for lock-free consumer access.
pub struct ComediMultiChannelAcquisition {
    /// Reference to the Comedi device
    device: Arc<ComediDevice>,
    /// Channels being acquired
    channels: Vec<u32>,
    /// Sample rate in Hz (per channel)
    sample_rate_hz: f64,
    /// Internal ring buffer for samples
    buffer: Arc<SimpleRingBuffer>,
    /// Flag indicating acquisition is running
    running: Arc<AtomicBool>,
    /// Background acquisition task handle
    task_handle: Option<JoinHandle<()>>,
    /// Statistics: total samples acquired
    samples_acquired: Arc<AtomicU64>,
    /// Statistics: buffer overflow count
    overflow_count: Arc<AtomicU64>,
}

impl ComediMultiChannelAcquisition {
    /// Create a new multi-channel acquisition instance.
    ///
    /// This performs device validation and allocates the internal buffer.
    ///
    /// # Arguments
    ///
    /// * `device_path` - Path to Comedi device (e.g., "/dev/comedi0")
    /// * `channels` - Vector of channel indices to acquire
    /// * `sample_rate` - Sample rate in Hz (per channel)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Device cannot be opened
    /// - No analog input subdevice found
    /// - Invalid channel indices
    /// - Sample rate is invalid (≤ 0 or too high)
    pub async fn new_async(
        device_path: &str,
        channels: Vec<u32>,
        sample_rate: f64,
    ) -> Result<Self> {
        // Validate parameters
        if channels.is_empty() {
            return Err(anyhow!("At least one channel is required"));
        }

        if sample_rate <= 0.0 {
            return Err(anyhow!("Sample rate must be positive"));
        }

        // Open device (use spawn_blocking for FFI call)
        let device_path = device_path.to_string();
        let device = task::spawn_blocking(move || ComediDevice::open(&device_path))
            .await
            .context("Failed to spawn blocking task")??;

        let device = Arc::new(device);

        // Find analog input subdevice
        let subdevice = device
            .find_subdevice(SubdeviceType::AnalogInput)
            .ok_or_else(|| anyhow!("No analog input subdevice found"))?;

        // Validate channels
        let ai = device.analog_input_subdevice(subdevice)?;
        let max_channel = ai.n_channels();

        for &ch in &channels {
            if ch >= max_channel {
                return Err(anyhow!(
                    "Invalid channel {}: device has {} channels",
                    ch,
                    max_channel
                ));
            }
        }

        // Allocate ring buffer (default: 10 seconds of data)
        let buffer_duration_secs = 10.0;
        let buffer_capacity = (sample_rate * buffer_duration_secs * channels.len() as f64) as usize;

        let buffer = Arc::new(SimpleRingBuffer::new(buffer_capacity, channels.len()));

        info!(
            device = %device.path(),
            subdevice = subdevice,
            channels = ?channels,
            sample_rate = sample_rate,
            buffer_capacity = buffer_capacity,
            "Created multi-channel acquisition"
        );

        Ok(Self {
            device,
            channels,
            sample_rate_hz: sample_rate,
            buffer,
            running: Arc::new(AtomicBool::new(false)),
            task_handle: None,
            samples_acquired: Arc::new(AtomicU64::new(0)),
            overflow_count: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Start the acquisition.
    ///
    /// Spawns a background task that continuously reads from the hardware
    /// and writes to the ring buffer.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Already running
    /// - Failed to configure hardware
    /// - Failed to start acquisition
    pub async fn start_acquisition(&mut self) -> Result<()> {
        if self.running.load(Ordering::Acquire) {
            return Err(anyhow!("Acquisition already running"));
        }

        // Build stream configuration
        let config = StreamConfig::builder()
            .channels(&self.channels)
            .sample_rate(self.sample_rate_hz)
            .buffer_size(16384) // 16K samples
            .build()?;

        // Create stream acquisition
        let device_clone = Arc::clone(&self.device);
        let stream =
            task::spawn_blocking(move || StreamAcquisition::new(&device_clone, config)).await??;

        let stream = Arc::new(Mutex::new(stream));

        // Start hardware acquisition
        {
            let stream = Arc::clone(&stream);
            task::spawn_blocking(move || stream.lock().start()).await??;
        }

        self.running.store(true, Ordering::Release);

        // Spawn background task
        let task_handle = self.spawn_reader_task(stream);
        self.task_handle = Some(task_handle);

        info!("Multi-channel acquisition started");

        Ok(())
    }

    /// Spawn the background reader task.
    fn spawn_reader_task(&self, stream: Arc<Mutex<StreamAcquisition>>) -> JoinHandle<()> {
        let running = Arc::clone(&self.running);
        let buffer = Arc::clone(&self.buffer);
        let samples_acquired = Arc::clone(&self.samples_acquired);
        let overflow_count = Arc::clone(&self.overflow_count);
        let n_channels = self.channels.len();

        task::spawn_blocking(move || {
            debug!("Reader task started");

            let mut last_log = Instant::now();
            let log_interval = Duration::from_secs(5);

            while running.load(Ordering::Acquire) {
                // Read available samples from stream (already converted to voltages)
                let samples_result = stream.lock().read_available();

                match samples_result {
                    Ok(Some(voltage_samples)) => {
                        if voltage_samples.is_empty() {
                            // No data available, yield
                            std::thread::sleep(Duration::from_millis(1));
                            continue;
                        }

                        // Samples are already voltages, organized as [ch0, ch1, ..., chN, ch0, ch1, ...]
                        let n_scans = voltage_samples.len() / n_channels;

                        for scan_idx in 0..n_scans {
                            let scan_start = scan_idx * n_channels;
                            let scan_end = scan_start + n_channels;
                            let scan = &voltage_samples[scan_start..scan_end];

                            // Write to ring buffer
                            buffer.write_scan(scan);
                        }

                        samples_acquired.fetch_add(n_scans as u64, Ordering::Relaxed);

                        // Periodic logging
                        if last_log.elapsed() >= log_interval {
                            let total_scans = buffer.scans_written();
                            debug!(
                                scans_acquired = total_scans,
                                overflow_count = overflow_count.load(Ordering::Relaxed),
                                "Acquisition progress"
                            );
                            last_log = Instant::now();
                        }
                    }
                    Ok(None) => {
                        // No data available
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(ComediError::BufferOverflow) => {
                        overflow_count.fetch_add(1, Ordering::Relaxed);
                        warn!("Buffer overflow detected");
                        // Continue reading; StreamAcquisition handles buffer management
                    }
                    Err(e) => {
                        error!("Read error: {}", e);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }

            debug!("Reader task stopped");
        })
    }

    /// Stop the acquisition.
    ///
    /// Gracefully stops the background task and hardware acquisition.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Not currently running
    /// - Failed to stop hardware
    /// - Background task panicked
    pub async fn stop_acquisition(&mut self) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Err(anyhow!("Acquisition not running"));
        }

        info!("Stopping multi-channel acquisition");

        // Signal task to stop
        self.running.store(false, Ordering::Release);

        // Wait for background task to finish (with timeout)
        if let Some(handle) = self.task_handle.take() {
            let timeout_result = tokio::time::timeout(Duration::from_secs(5), handle).await;

            match timeout_result {
                Ok(join_result) => {
                    if let Err(e) = join_result {
                        error!("Reader task panicked: {:?}", e);
                        return Err(anyhow!("Reader task panicked"));
                    }
                }
                Err(_) => {
                    warn!("Reader task did not stop within timeout, aborting");
                    return Err(anyhow!("Stop timeout"));
                }
            }
        }

        info!(
            total_scans = self.buffer.scans_written(),
            overflow_count = self.overflow_count.load(Ordering::Relaxed),
            "Multi-channel acquisition stopped"
        );

        Ok(())
    }

    /// Get the latest N samples from each channel.
    ///
    /// Returns a Vec<Vec<f64>> where each inner Vec contains samples for one channel
    /// in temporal order (oldest to newest).
    ///
    /// # Arguments
    ///
    /// * `num_samples` - Number of samples to retrieve per channel
    ///
    /// # Errors
    ///
    /// Returns error if num_samples is 0.
    pub fn get_latest_samples(&self, num_samples: usize) -> Result<Vec<Vec<f64>>> {
        if num_samples == 0 {
            return Err(anyhow!("num_samples must be > 0"));
        }

        Ok(self.buffer.read_latest(num_samples))
    }

    /// Check if acquisition is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Get the total number of scans acquired (across all channels).
    pub fn scans_acquired(&self) -> u64 {
        self.samples_acquired.load(Ordering::Relaxed)
    }

    /// Get the number of buffer overflow events.
    pub fn overflow_count(&self) -> u64 {
        self.overflow_count.load(Ordering::Relaxed)
    }

    /// Get the configured channels.
    pub fn channels(&self) -> &[u32] {
        &self.channels
    }

    /// Get the configured sample rate.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate_hz
    }
}

impl Drop for ComediMultiChannelAcquisition {
    fn drop(&mut self) {
        if self.running.load(Ordering::Acquire) {
            warn!("ComediMultiChannelAcquisition dropped while running, forcing stop");

            // Signal stop
            self.running.store(false, Ordering::Release);

            // Wait for task (blocking drop is acceptable here)
            if let Some(handle) = self.task_handle.take() {
                // Use a runtime handle if available, otherwise just abort
                if let Ok(rt_handle) = tokio::runtime::Handle::try_current() {
                    let _ = rt_handle.block_on(async {
                        tokio::time::timeout(Duration::from_secs(2), handle).await
                    });
                } else {
                    handle.abort();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_ring_buffer() {
        let buffer = SimpleRingBuffer::new(100, 4);

        // Write some scans
        buffer.write_scan(&[1.0, 2.0, 3.0, 4.0]);
        buffer.write_scan(&[5.0, 6.0, 7.0, 8.0]);

        assert_eq!(buffer.scans_written(), 2);

        // Read back
        let data = buffer.read_latest(2);
        assert_eq!(data.len(), 4); // 4 channels
        assert_eq!(data[0], vec![1.0, 5.0]); // Channel 0
        assert_eq!(data[1], vec![2.0, 6.0]); // Channel 1
    }

    #[test]
    fn test_ring_buffer_wrap() {
        // Small buffer that will wrap
        let buffer = SimpleRingBuffer::new(8, 2); // 4 scans max

        // Write 6 scans (more than capacity)
        for i in 0..6 {
            buffer.write_scan(&[(i * 2) as f64, (i * 2 + 1) as f64]);
        }

        assert_eq!(buffer.scans_written(), 6);

        // Read latest 2 scans (should be scans 4 and 5)
        let data = buffer.read_latest(2);
        assert_eq!(data[0], vec![8.0, 10.0]);
        assert_eq!(data[1], vec![9.0, 11.0]);
    }

    #[test]
    fn test_ring_buffer_read_more_than_available() {
        let buffer = SimpleRingBuffer::new(100, 3);

        buffer.write_scan(&[1.0, 2.0, 3.0]);
        buffer.write_scan(&[4.0, 5.0, 6.0]);

        // Request more than available
        let data = buffer.read_latest(10);

        // Should only get 2 scans
        assert_eq!(data[0].len(), 2);
        assert_eq!(data[0], vec![1.0, 4.0]);
    }
}
