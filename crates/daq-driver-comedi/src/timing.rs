//! Hardware timing configuration for NI DAQ devices.
//!
//! This module provides comprehensive timing configuration for data acquisition:
//!
//! - Internal clock rate selection
//! - External clock input support
//! - Clock output configuration
//! - Scan and convert interval configuration
//! - Timing validation and limits
//!
//! # NI PCI-MIO-16XE-10 Timing Specifications
//!
//! The NI PCI-MIO-16XE-10 uses the following timing subsystems:
//!
//! - **Base clock**: 20 MHz onboard oscillator
//! - **AI timing**: Hardware supports up to 100 kS/s aggregate
//! - **AO timing**: Hardware supports up to 100 kS/s aggregate
//! - **External clock**: PFI0/AI_START_TRIGGER or PFI1/AI_EXTMUX_CLK
//!
//! # Timing Model
//!
//! ```text
//!           ┌─────────────────────────────────────┐
//!           │          SCAN INTERVAL              │
//!           │  ┌─────┬─────┬─────┬─────┬─────┐    │
//!           │  │ CH0 │ CH1 │ CH2 │ CH3 │WAIT │    │
//!           │  └─────┴─────┴─────┴─────┴─────┘    │
//!           │   ←────CONVERT──────────→│          │
//!           └─────────────────────────────────────┘
//!
//! scan_interval = time between scan starts
//! convert_interval = time between channel samples within a scan
//! ```
//!
//! # Example
//!
//! ```no_run
//! use daq_driver_comedi::{ComediDevice, TimingConfig, TimingCapabilities, ClockSource};
//!
//! # fn example() -> anyhow::Result<()> {
//! let device = ComediDevice::open("/dev/comedi0")?;
//!
//! // Query timing capabilities
//! let caps = TimingCapabilities::query(&device)?;
//! println!("Max sample rate: {} Hz", caps.max_sample_rate);
//! println!("Min convert time: {} ns", caps.min_convert_ns);
//!
//! // Configure for 50 kHz sampling with internal clock
//! let config = TimingConfig::builder()
//!     .sample_rate(50000.0)
//!     .clock_source(ClockSource::Internal)
//!     .build()?;
//!
//! println!("Scan interval: {} ns", config.scan_interval_ns());
//! println!("Convert interval: {} ns", config.convert_interval_ns());
//! # Ok(())
//! # }
//! ```

use tracing::{debug, warn};

use crate::device::{ComediDevice, SubdeviceType};
use crate::error::{ComediError, Result};

/// Clock source for timing generation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ClockSource {
    /// Internal 20 MHz oscillator
    #[default]
    Internal,
    /// External clock input
    External {
        /// PFI pin number (0-9 for NI E-series)
        pin: u32,
        /// Expected external clock frequency in Hz (for validation)
        frequency: Option<f64>,
    },
    /// Internal with clock output enabled
    InternalWithOutput {
        /// PFI pin for clock output
        output_pin: u32,
    },
}

/// Clock polarity for external clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockPolarity {
    /// Sample on rising edge
    #[default]
    RisingEdge,
    /// Sample on falling edge
    FallingEdge,
}

/// Hardware timing capabilities.
#[derive(Debug, Clone)]
pub struct TimingCapabilities {
    /// Maximum aggregate sample rate in Hz
    pub max_sample_rate: f64,
    /// Minimum aggregate sample rate in Hz
    pub min_sample_rate: f64,
    /// Minimum convert interval in nanoseconds
    pub min_convert_ns: u32,
    /// Maximum convert interval in nanoseconds
    pub max_convert_ns: u32,
    /// Minimum scan interval in nanoseconds
    pub min_scan_ns: u32,
    /// Maximum scan interval in nanoseconds
    pub max_scan_ns: u32,
    /// Base clock frequency in Hz
    pub base_clock_hz: f64,
    /// Clock divisor range
    pub divisor_range: (u32, u32),
    /// Supports external clock input
    pub external_clock: bool,
    /// Supports clock output
    pub clock_output: bool,
    /// Available PFI pins for clock I/O
    pub pfi_pins: Vec<u32>,
}

impl Default for TimingCapabilities {
    fn default() -> Self {
        // Default values for NI PCI-MIO-16XE-10
        Self {
            max_sample_rate: 100_000.0,      // 100 kS/s
            min_sample_rate: 0.0001,         // Essentially unlimited minimum
            min_convert_ns: 1_000,           // 1 µs minimum convert
            max_convert_ns: 1_000_000_000,   // 1 second max
            min_scan_ns: 10_000,             // 10 µs minimum scan
            max_scan_ns: 4_000_000_000,      // ~4 seconds max
            base_clock_hz: 20_000_000.0,     // 20 MHz
            divisor_range: (1, 0x00FF_FFFF), // 24-bit counter
            external_clock: true,
            clock_output: true,
            pfi_pins: vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        }
    }
}

impl TimingCapabilities {
    /// Query timing capabilities from the device.
    pub fn query(device: &ComediDevice) -> Result<Self> {
        // Find AI subdevice
        let _subdevice = device
            .find_subdevice(SubdeviceType::AnalogInput)
            .ok_or_else(|| ComediError::NotSupported {
                message: "No analog input subdevice".to_string(),
            })?;

        // For NI PCI-MIO-16XE-10, use known specifications
        // In a full implementation, we'd query the driver for cmd_mask support
        let caps = Self::default();

        debug!(
            max_rate = caps.max_sample_rate,
            min_convert_ns = caps.min_convert_ns,
            "Queried timing capabilities"
        );

        Ok(caps)
    }

    /// Check if a sample rate is achievable.
    pub fn validate_sample_rate(&self, rate: f64) -> Result<()> {
        if rate < self.min_sample_rate {
            return Err(ComediError::InvalidConfig {
                message: format!(
                    "Sample rate {} Hz below minimum {} Hz",
                    rate, self.min_sample_rate
                ),
            });
        }
        if rate > self.max_sample_rate {
            return Err(ComediError::InvalidConfig {
                message: format!(
                    "Sample rate {} Hz exceeds maximum {} Hz",
                    rate, self.max_sample_rate
                ),
            });
        }
        Ok(())
    }

    /// Calculate the closest achievable sample rate.
    pub fn nearest_sample_rate(&self, requested: f64) -> f64 {
        // Calculate the divisor needed
        let ideal_divisor = self.base_clock_hz / requested;
        let actual_divisor = ideal_divisor
            .round()
            .clamp(self.divisor_range.0 as f64, self.divisor_range.1 as f64);

        self.base_clock_hz / actual_divisor
    }

    /// Check if a convert interval is valid.
    pub fn validate_convert_ns(&self, ns: u32) -> Result<()> {
        if ns < self.min_convert_ns {
            return Err(ComediError::InvalidConfig {
                message: format!(
                    "Convert interval {} ns below minimum {} ns",
                    ns, self.min_convert_ns
                ),
            });
        }
        if ns > self.max_convert_ns {
            return Err(ComediError::InvalidConfig {
                message: format!(
                    "Convert interval {} ns exceeds maximum {} ns",
                    ns, self.max_convert_ns
                ),
            });
        }
        Ok(())
    }
}

/// Complete timing configuration.
#[derive(Debug, Clone)]
pub struct TimingConfig {
    /// Clock source
    pub clock_source: ClockSource,
    /// Clock polarity (for external clock)
    pub clock_polarity: ClockPolarity,
    /// Target sample rate per channel in Hz
    pub sample_rate: f64,
    /// Number of channels in scan
    pub n_channels: u32,
    /// Explicit scan interval in nanoseconds (overrides sample_rate if set)
    pub scan_interval_ns: Option<u32>,
    /// Explicit convert interval in nanoseconds (auto-calculated if None)
    pub convert_interval_ns: Option<u32>,
    /// Enable dithering (some boards support this for noise reduction)
    pub dithering: bool,
    /// Settling time multiplier (1.0 = default, higher = more settling)
    pub settling_multiplier: f64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            clock_source: ClockSource::Internal,
            clock_polarity: ClockPolarity::RisingEdge,
            sample_rate: 1000.0,
            n_channels: 1,
            scan_interval_ns: None,
            convert_interval_ns: None,
            dithering: false,
            settling_multiplier: 1.0,
        }
    }
}

impl TimingConfig {
    /// Create a new builder.
    pub fn builder() -> TimingConfigBuilder {
        TimingConfigBuilder::default()
    }

    /// Calculate the scan interval in nanoseconds.
    pub fn scan_interval_ns(&self) -> u32 {
        self.scan_interval_ns
            .unwrap_or_else(|| (1e9 / self.sample_rate) as u32)
    }

    /// Calculate the convert interval in nanoseconds.
    pub fn convert_interval_ns(&self) -> u32 {
        self.convert_interval_ns.unwrap_or_else(|| {
            if self.n_channels <= 1 {
                0 // Single channel doesn't need convert timing
            } else {
                // Divide scan interval among channels with settling time
                let scan_ns = self.scan_interval_ns();
                let base_convert = scan_ns / self.n_channels;
                (base_convert as f64 * self.settling_multiplier) as u32
            }
        })
    }

    /// Get the effective sample rate.
    pub fn effective_sample_rate(&self) -> f64 {
        1e9 / self.scan_interval_ns() as f64
    }

    /// Validate the configuration against capabilities.
    pub fn validate(&self, caps: &TimingCapabilities) -> Result<()> {
        // Validate sample rate
        caps.validate_sample_rate(self.sample_rate)?;

        // Validate scan interval
        let scan_ns = self.scan_interval_ns();
        if scan_ns < caps.min_scan_ns {
            return Err(ComediError::InvalidConfig {
                message: format!(
                    "Scan interval {} ns below minimum {} ns",
                    scan_ns, caps.min_scan_ns
                ),
            });
        }

        // Validate convert interval
        let convert_ns = self.convert_interval_ns();
        if convert_ns > 0 {
            caps.validate_convert_ns(convert_ns)?;

            // Check that converts fit in scan
            let total_convert = convert_ns * self.n_channels;
            if total_convert > scan_ns {
                return Err(ComediError::InvalidConfig {
                    message: format!(
                        "Total convert time {} ns exceeds scan interval {} ns",
                        total_convert, scan_ns
                    ),
                });
            }
        }

        // Validate external clock config
        if let ClockSource::External { pin, .. } = self.clock_source {
            if !caps.pfi_pins.contains(&pin) {
                return Err(ComediError::InvalidConfig {
                    message: format!("Invalid PFI pin {} for external clock", pin),
                });
            }
        }

        Ok(())
    }

    /// Apply timing adjustments from Comedi command test.
    ///
    /// Comedi's command_test may adjust timing parameters. This method
    /// updates the config to reflect actual hardware values.
    pub fn apply_adjustments(&mut self, actual_scan_ns: u32, actual_convert_ns: u32) {
        let requested_scan = self.scan_interval_ns();
        let requested_convert = self.convert_interval_ns();

        if actual_scan_ns != requested_scan {
            warn!(
                requested = requested_scan,
                actual = actual_scan_ns,
                "Scan interval adjusted by hardware"
            );
            self.scan_interval_ns = Some(actual_scan_ns);
            // Update sample rate to match
            self.sample_rate = 1e9 / actual_scan_ns as f64;
        }

        if actual_convert_ns != requested_convert {
            warn!(
                requested = requested_convert,
                actual = actual_convert_ns,
                "Convert interval adjusted by hardware"
            );
            self.convert_interval_ns = Some(actual_convert_ns);
        }
    }
}

/// Builder for TimingConfig.
#[derive(Debug, Default)]
pub struct TimingConfigBuilder {
    config: TimingConfig,
}

impl TimingConfigBuilder {
    /// Set the clock source.
    pub fn clock_source(mut self, source: ClockSource) -> Self {
        self.config.clock_source = source;
        self
    }

    /// Set the clock polarity.
    pub fn clock_polarity(mut self, polarity: ClockPolarity) -> Self {
        self.config.clock_polarity = polarity;
        self
    }

    /// Set the target sample rate in Hz.
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.config.sample_rate = rate;
        self
    }

    /// Set the number of channels.
    pub fn n_channels(mut self, n: u32) -> Self {
        self.config.n_channels = n;
        self
    }

    /// Set explicit scan interval in nanoseconds.
    pub fn scan_interval_ns(mut self, ns: u32) -> Self {
        self.config.scan_interval_ns = Some(ns);
        self
    }

    /// Set explicit convert interval in nanoseconds.
    pub fn convert_interval_ns(mut self, ns: u32) -> Self {
        self.config.convert_interval_ns = Some(ns);
        self
    }

    /// Enable or disable dithering.
    pub fn dithering(mut self, enable: bool) -> Self {
        self.config.dithering = enable;
        self
    }

    /// Set the settling time multiplier.
    pub fn settling_multiplier(mut self, mult: f64) -> Self {
        self.config.settling_multiplier = mult;
        self
    }

    /// Build with default capabilities (no validation).
    pub fn build(self) -> Result<TimingConfig> {
        let caps = TimingCapabilities::default();
        self.build_with_caps(&caps)
    }

    /// Build with validation against specific capabilities.
    pub fn build_with_caps(self, caps: &TimingCapabilities) -> Result<TimingConfig> {
        self.config.validate(caps)?;
        Ok(self.config)
    }
}

/// Helper functions for common timing calculations.
pub mod timing_utils {
    /// Convert sample rate to nanosecond interval.
    pub fn rate_to_ns(rate_hz: f64) -> u32 {
        (1e9 / rate_hz) as u32
    }

    /// Convert nanosecond interval to sample rate.
    pub fn ns_to_rate(ns: u32) -> f64 {
        1e9 / ns as f64
    }

    /// Calculate minimum scan interval for given channels and convert time.
    pub fn min_scan_for_channels(n_channels: u32, convert_ns: u32) -> u32 {
        n_channels * convert_ns
    }

    /// Calculate aggregate sample rate.
    pub fn aggregate_rate(per_channel_rate: f64, n_channels: u32) -> f64 {
        per_channel_rate * n_channels as f64
    }

    /// Calculate per-channel rate from aggregate.
    pub fn per_channel_rate(aggregate_rate: f64, n_channels: u32) -> f64 {
        aggregate_rate / n_channels as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timing_config_builder() {
        let config = TimingConfig::builder()
            .sample_rate(10000.0)
            .n_channels(4)
            .build()
            .unwrap();

        assert_eq!(config.sample_rate, 10000.0);
        assert_eq!(config.n_channels, 4);
        // 10kHz = 100,000 ns
        assert_eq!(config.scan_interval_ns(), 100_000);
    }

    #[test]
    fn test_timing_validation() {
        let caps = TimingCapabilities::default();

        // Valid config
        let config = TimingConfig::builder()
            .sample_rate(50000.0)
            .n_channels(4)
            .build_with_caps(&caps)
            .unwrap();

        assert!(config.validate(&caps).is_ok());

        // Invalid: rate too high
        let result = TimingConfig::builder()
            .sample_rate(200000.0)
            .build_with_caps(&caps);

        assert!(result.is_err());
    }

    #[test]
    fn test_convert_interval_calculation() {
        let config = TimingConfig::builder()
            .sample_rate(10000.0) // 100,000 ns scan
            .n_channels(4)
            .build()
            .unwrap();

        // 100,000 ns / 4 channels = 25,000 ns per channel
        assert_eq!(config.convert_interval_ns(), 25_000);
    }

    #[test]
    fn test_nearest_sample_rate() {
        let caps = TimingCapabilities::default();

        // Request 10 kHz
        let nearest = caps.nearest_sample_rate(10000.0);
        // Should be close to 10 kHz
        assert!((nearest - 10000.0).abs() < 1.0);
    }

    #[test]
    fn test_timing_utils() {
        assert_eq!(timing_utils::rate_to_ns(1000.0), 1_000_000);
        assert!((timing_utils::ns_to_rate(1_000_000) - 1000.0).abs() < 0.01);
        assert_eq!(timing_utils::aggregate_rate(10000.0, 4), 40000.0);
    }
}
