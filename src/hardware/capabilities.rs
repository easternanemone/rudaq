//! Atomic Hardware Capabilities
//!
//! This module defines fine-grained capability traits that hardware devices can implement.
//! Instead of monolithic traits like `Camera` or `Instrument`, devices implement
//! specific capabilities they actually support:
//!
//! - A camera might implement: `Triggerable + ExposureControl + FrameProducer`
//! - A stage might implement: `Movable + Triggerable`
//! - A power meter might implement: `Readable`
//!
//! This approach enables:
//! - Better composition (devices can mix capabilities)
//! - Clearer contracts (traits are small and focused)
//! - Easier testing (mock individual capabilities)
//! - Hardware-agnostic code (functions work with trait bounds)
//!
//! # Design Philosophy
//!
//! Each capability trait:
//! - Is async (uses #[async_trait])
//! - Is thread-safe (requires Send + Sync)
//! - Uses anyhow::Result for errors
//! - Focuses on ONE thing
//!
//! # Example
//!
//! ```rust,ignore
//! // A triggered camera implements multiple capabilities
//! struct SimulatedCamera {
//!     exposure_ms: f64,
//!     armed: bool,
//!     frame_count: u32,
//! }
//!
//! #[async_trait]
//! impl ExposureControl for SimulatedCamera {
//!     async fn set_exposure(&self, seconds: f64) -> Result<()> {
//!         self.exposure_ms = seconds * 1000.0;
//!         Ok(())
//!     }
//!
//!     async fn get_exposure(&self) -> Result<f64> {
//!         Ok(self.exposure_ms / 1000.0)
//!     }
//! }
//!
//! #[async_trait]
//! impl Triggerable for SimulatedCamera {
//!     async fn arm(&self) -> Result<()> {
//!         self.armed = true;
//!         Ok(())
//!     }
//!
//!     async fn trigger(&self) -> Result<()> {
//!         if !self.armed {
//!             anyhow::bail!("Camera not armed");
//!         }
//!         // Capture frame...
//!         Ok(())
//!     }
//! }
//!
//! #[async_trait]
//! impl FrameProducer for SimulatedCamera {
//!     async fn start_stream(&self) -> Result<()> { Ok(()) }
//!     async fn stop_stream(&self) -> Result<()> { Ok(()) }
//!     fn resolution(&self) -> (u32, u32) { (1024, 1024) }
//! }
//!
//! // Use in generic code
//! async fn triggered_acquisition<T>(device: &T) -> Result<()>
//! where
//!     T: Triggerable + ExposureControl + FrameProducer
//! {
//!     device.set_exposure(0.1).await?;
//!     device.arm().await?;
//!     device.trigger().await?;
//!     Ok(())
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;

/// Capability: Motion Control
///
/// Devices that can move to positions (stages, actuators, goniometers).
///
/// # Contract
/// - Positions are in device-native units (typically mm or degrees)
/// - `move_abs` and `move_rel` initiate motion but may return before completion
/// - `wait_settled` blocks until motion completes
/// - `position` returns current position (may be approximate during motion)
///
/// # Thread Safety
/// - All methods are async and require `&self` (immutable reference)
/// - Interior mutability (Mutex/RwLock) should be used for state
#[async_trait]
pub trait Movable: Send + Sync {
    /// Move to absolute position
    ///
    /// # Arguments
    /// * `position` - Target position in device-native units
    ///
    /// # Returns
    /// - Ok(()) if motion initiated successfully
    /// - Err if position is out of range or hardware error
    async fn move_abs(&self, position: f64) -> Result<()>;

    /// Move relative to current position
    ///
    /// # Arguments
    /// * `distance` - Distance to move (positive or negative)
    ///
    /// # Returns
    /// - Ok(()) if motion initiated successfully
    /// - Err if resulting position would be out of range
    async fn move_rel(&self, distance: f64) -> Result<()>;

    /// Get current position
    ///
    /// # Returns
    /// Current position in device-native units.
    /// May be approximate if device is currently moving.
    async fn position(&self) -> Result<f64>;

    /// Wait for motion to settle
    ///
    /// Blocks until device reports motion is complete.
    /// Should have internal timeout to prevent infinite blocking.
    ///
    /// # Returns
    /// - Ok(()) when settled
    /// - Err on timeout or hardware error
    async fn wait_settled(&self) -> Result<()>;
}

/// Capability: External Triggering
///
/// Devices that can be armed and triggered (cameras, detectors, pulse generators).
///
/// # Contract
/// - `arm()` prepares device for trigger (may configure hardware buffers)
/// - `trigger()` initiates acquisition/output
/// - Some devices require arm before every trigger, others stay armed
/// - Calling `trigger()` on unarmed device should return Err
#[async_trait]
pub trait Triggerable: Send + Sync {
    /// Arm device for trigger
    ///
    /// Prepares hardware to respond to trigger signal.
    /// May configure buffers, clear counters, or enter standby mode.
    ///
    /// # Returns
    /// - Ok(()) if armed successfully
    /// - Err if device is busy or in error state
    async fn arm(&self) -> Result<()>;

    /// Send software trigger
    ///
    /// Initiates acquisition/output. Device must be armed first.
    ///
    /// # Returns
    /// - Ok(()) if trigger accepted
    /// - Err if not armed or hardware error
    async fn trigger(&self) -> Result<()>;
}

/// Capability: Exposure Time Control
///
/// Devices with configurable integration time (cameras, spectrometers, photodetectors).
///
/// # Contract
/// - Exposure is in seconds (not milliseconds)
/// - Setting exposure does not start acquisition
/// - Exposure applies to next acquisition
#[async_trait]
pub trait ExposureControl: Send + Sync {
    /// Set exposure/integration time
    ///
    /// # Arguments
    /// * `seconds` - Exposure time in seconds
    ///
    /// # Returns
    /// - Ok(()) if exposure set successfully
    /// - Err if value is out of hardware range
    async fn set_exposure(&self, seconds: f64) -> Result<()>;

    /// Get current exposure setting
    ///
    /// # Returns
    /// Current exposure time in seconds
    async fn get_exposure(&self) -> Result<f64>;
}

/// Capability: Frame/Image Production
///
/// Devices that produce 2D image frames (cameras, beam profilers).
///
/// # Contract
/// - `start_stream()` begins continuous acquisition
/// - `stop_stream()` halts acquisition
/// - Frames are delivered via separate channel (not returned from these methods)
/// - `resolution()` is immutable (cannot be changed via this trait)
///
/// # Design Note
/// Frame data is NOT returned from these methods. Instead, devices should
/// broadcast frames via tokio channels. This trait only controls the stream.
#[async_trait]
pub trait FrameProducer: Send + Sync {
    /// Start continuous frame acquisition
    ///
    /// # Returns
    /// - Ok(()) if streaming started
    /// - Err if already streaming or hardware error
    async fn start_stream(&self) -> Result<()>;

    /// Stop frame acquisition
    ///
    /// # Returns
    /// - Ok(()) if streaming stopped
    /// - Err on hardware error
    async fn stop_stream(&self) -> Result<()>;

    /// Get frame resolution (width, height)
    ///
    /// Returns sensor resolution in pixels.
    /// This is immutable - use separate ROI trait for cropping.
    fn resolution(&self) -> (u32, u32);
}

/// Capability: Scalar Readout
///
/// Devices that produce single scalar values (power meters, temperature sensors,
/// voltmeters, pressure gauges).
///
/// # Contract
/// - `read()` performs measurement and returns value
/// - Units are device-specific (document in implementation)
/// - Reading should be fast (<100ms typical)
#[async_trait]
pub trait Readable: Send + Sync {
    /// Read current value
    ///
    /// Performs measurement and returns scalar value.
    /// Units depend on device type (watts, volts, celsius, etc.)
    ///
    /// # Returns
    /// - Ok(value) on successful read
    /// - Err on hardware error or timeout
    async fn read(&self) -> Result<f64>;
}

// =============================================================================
// Trait Composition Examples (Documentation)
// =============================================================================

/// Example: Triggered Camera
///
/// A camera that supports external triggering would implement:
/// ```rust,ignore
/// struct TriggeredCamera { /* ... */ }
///
/// impl Triggerable for TriggeredCamera { /* ... */ }
/// impl ExposureControl for TriggeredCamera { /* ... */ }
/// impl FrameProducer for TriggeredCamera { /* ... */ }
///
/// // Use in generic scan code
/// async fn scan_with_camera<C>(camera: &C) -> Result<()>
/// where
///     C: Triggerable + ExposureControl + FrameProducer
/// {
///     camera.set_exposure(0.1).await?;
///     camera.arm().await?;
///     camera.trigger().await?;
///     Ok(())
/// }
/// ```

// =============================================================================
// Combined Traits (for trait objects)
// =============================================================================

/// Combined trait for cameras that support both triggering and frame production
///
/// This trait exists solely to enable trait objects. Implement the individual
/// traits (Triggerable, FrameProducer) and get this automatically via blanket impl.
///
/// # Usage
/// ```rust,ignore
/// // In function signatures
/// fn use_camera(camera: Arc<dyn Camera>) { /* ... */ }
///
/// // Blanket impl means you just implement the individual traits
/// impl Triggerable for MyCamera { /* ... */ }
/// impl FrameProducer for MyCamera { /* ... */ }
/// // Camera is automatically implemented!
/// ```
pub trait Camera: Triggerable + FrameProducer {}

/// Blanket implementation - any type implementing both traits gets Camera for free
impl<T: Triggerable + FrameProducer> Camera for T {}

/// Example: Motion Stage
///
/// A motorized stage would implement:
/// ```rust,ignore
/// struct ESP300Stage { /* ... */ }
///
/// impl Movable for ESP300Stage { /* ... */ }
///
/// // Optionally also triggerable for synchronized scans
/// impl Triggerable for ESP300Stage { /* ... */ }
///
/// // Use in generic scan code
/// async fn line_scan<S>(stage: &S, start: f64, end: f64, steps: usize) -> Result<()>
/// where
///     S: Movable
/// {
///     for position in linspace(start, end, steps) {
///         stage.move_abs(position).await?;
///         stage.wait_settled().await?;
///         // Acquire data...
///     }
///     Ok(())
/// }
/// ```

/// Example: Power Meter
///
/// A simple power meter implements only Readable:
/// ```rust,ignore
/// struct NewportPowerMeter { /* ... */ }
///
/// impl Readable for NewportPowerMeter {
///     async fn read(&self) -> Result<f64> {
///         // SCPI query, return watts
///         Ok(0.042)
///     }
/// }
///
/// // Use in generic monitoring code
/// async fn monitor<R>(sensor: &R) -> Result<Vec<f64>>
/// where
///     R: Readable
/// {
///     let mut readings = Vec::new();
///     for _ in 0..100 {
///         readings.push(sensor.read().await?);
///         tokio::time::sleep(Duration::from_millis(10)).await;
///     }
///     Ok(readings)
/// }
/// ```

#[cfg(test)]
mod tests {
    use super::*;

    // Mock implementations for testing

    struct MockStage {
        position: std::sync::Mutex<f64>,
    }

    #[async_trait]
    impl Movable for MockStage {
        async fn move_abs(&self, position: f64) -> Result<()> {
            *self.position.lock().unwrap() = position;
            Ok(())
        }

        async fn move_rel(&self, distance: f64) -> Result<()> {
            *self.position.lock().unwrap() += distance;
            Ok(())
        }

        async fn position(&self) -> Result<f64> {
            Ok(*self.position.lock().unwrap())
        }

        async fn wait_settled(&self) -> Result<()> {
            // Simulate settling time
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_movable_trait() {
        let stage = MockStage {
            position: std::sync::Mutex::new(0.0),
        };

        // Test absolute move
        stage.move_abs(10.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 10.0);

        // Test relative move
        stage.move_rel(5.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 15.0);

        // Test settle
        stage.wait_settled().await.unwrap();
    }

    struct MockPowerMeter;

    #[async_trait]
    impl Readable for MockPowerMeter {
        async fn read(&self) -> Result<f64> {
            Ok(0.123)
        }
    }

    #[tokio::test]
    async fn test_readable_trait() {
        let meter = MockPowerMeter;
        let reading = meter.read().await.unwrap();
        assert_eq!(reading, 0.123);
    }
}
