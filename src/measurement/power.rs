//! Power measurement abstractions for optical power meters.
//!
//! This module defines the trait interface for instruments that measure
//! optical power, such as Newport 1830-C power meters and similar devices.
//!
//! # Units
//!
//! All power measurements are returned in **watts (W)** as floating-point
//! values. Implementations are responsible for converting from device-native
//! units (e.g., dBm, microwatts) to watts.
//!
//! # Precision
//!
//! Typical power meter precision varies by wavelength and power level:
//! - High power (>1 mW): ±1-2% accuracy
//! - Low power (<1 µW): ±5-10% accuracy
//! - Consult specific instrument documentation for calibrated ranges
//!
//! # Example
//!
//! ```rust,no_run
//! use rust_daq::measurement::power::PowerMeasure;
//! use anyhow::Result;
//!
//! # struct MyPowerMeter;
//! # #[async_trait::async_trait]
//! # impl PowerMeasure for MyPowerMeter {
//! #     async fn read_power(&mut self) -> Result<f64> { Ok(0.001) }
//! # }
//! #
//! # async fn example(mut meter: MyPowerMeter) -> Result<()> {
//! let power_watts = meter.read_power().await?;
//! println!("Power: {:.6} W ({:.2} mW)", power_watts, power_watts * 1000.0);
//! # Ok(())
//! # }
//! ```

use anyhow::Result;
use async_trait::async_trait;

/// Capability trait for instruments that measure optical power.
///
/// Implement this trait for power meters, photodetectors, and other devices
/// that provide power readings. All measurements must be returned in watts (W).
///
/// # Implementation Notes
///
/// - Convert device-native units (dBm, µW, mW) to watts before returning
/// - Handle auto-ranging and zero-offset internally
/// - Return errors for out-of-range or invalid readings
/// - Implementations should be thread-safe (requires `&mut self` for state)
///
/// # Measurement Timing
///
/// The `read_power` method blocks until a stable reading is available.
/// Integration time varies by instrument (typically 10-100ms). For
/// continuous streaming, consider using a data acquisition loop with
/// appropriate delays.
///
/// # Example Implementation
///
/// ```rust,ignore
/// use rust_daq::measurement::power::PowerMeasure;
/// use anyhow::Result;
/// use async_trait::async_trait;
///
/// struct Newport1830C {
///     handle: SerialPort,
/// }
///
/// #[async_trait]
/// impl PowerMeasure for Newport1830C {
///     async fn read_power(&mut self) -> Result<f64> {
///         // Query device for power reading in dBm
///         let response = self.handle.query("PM:P?").await?;
///         let dbm: f64 = response.parse()?;
///         
///         // Convert dBm to watts: P(W) = 10^(dBm/10) / 1000
///         let watts = 10_f64.powf(dbm / 10.0) / 1000.0;
///         Ok(watts)
///     }
/// }
/// ```
#[async_trait]
pub trait PowerMeasure {
    /// Reads the current optical power from the instrument.
    ///
    /// Blocks until a measurement is complete and returns the power in watts.
    /// The reading is instantaneous (not averaged) unless the instrument has
    /// built-in integration enabled.
    ///
    /// # Returns
    ///
    /// Power in watts (W) as a positive floating-point value. For very low
    /// signals, may return values close to zero (e.g., 1e-9 W = 1 nW).
    ///
    /// # Errors
    ///
    /// - Communication errors if the device is disconnected or unresponsive
    /// - Parse errors if the device returns malformed data
    /// - Range errors if the signal is outside the instrument's measurement range
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rust_daq::measurement::power::PowerMeasure;
    /// # use anyhow::Result;
    /// # struct MyMeter;
    /// # #[async_trait::async_trait]
    /// # impl PowerMeasure for MyMeter {
    /// #     async fn read_power(&mut self) -> Result<f64> { Ok(0.0015) }
    /// # }
    /// # async fn example(mut meter: MyMeter) -> Result<()> {
    /// let power_w = meter.read_power().await?;
    ///
    /// // Convert to milliwatts for display
    /// let power_mw = power_w * 1000.0;
    /// println!("Optical power: {:.3} mW", power_mw);
    /// # Ok(())
    /// # }
    /// ```
    async fn read_power(&mut self) -> Result<f64>;
}
