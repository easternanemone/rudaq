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
use crate::observable::ParameterSet;

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

    /// Stop motion immediately
    ///
    /// Issues an emergency stop command to halt motion in progress.
    /// Not all devices support this - check capability before calling.
    ///
    /// # Returns
    /// - Ok(()) if stop command issued successfully
    /// - Err if device doesn't support stop or hardware error
    ///
    /// # Default Implementation
    /// Returns an error indicating stop is not supported.
    async fn stop(&self) -> Result<()> {
        anyhow::bail!("Stop not supported by this device")
    }
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

    /// Check if device is currently armed
    ///
    /// # Returns
    /// - Ok(true) if device is armed and ready for trigger
    /// - Ok(false) if device is not armed
    /// - Err if state cannot be determined or not supported
    ///
    /// # Default Implementation
    /// Returns an error indicating state query is not supported.
    async fn is_armed(&self) -> Result<bool> {
        anyhow::bail!("Armed state query not supported by this device")
    }
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
/// - Frames are delivered via `take_frame_receiver()` channel
/// - `resolution()` is immutable (cannot be changed via this trait)
///
/// # Frame Delivery
/// Call `take_frame_receiver()` BEFORE `start_stream()` to get the channel
/// that will receive Frame objects during streaming.
#[async_trait]
pub trait FrameProducer: Send + Sync {
    /// Start continuous frame acquisition
    ///
    /// # Returns
    /// - Ok(()) if streaming started
    /// - Err if already streaming or hardware error
    async fn start_stream(&self) -> Result<()>;

    /// Start finite frame acquisition with a maximum frame count
    ///
    /// # Arguments
    /// - `frame_limit`: Maximum number of frames to acquire.
    ///   - `Some(n)` where n > 0: acquire exactly n frames then stop
    ///   - `Some(0)` or `None`: continuous acquisition (same as `start_stream()`)
    ///
    /// # Returns
    /// - Ok(()) if streaming started
    /// - Err if already streaming or hardware error
    ///
    /// # Default Implementation
    /// Calls `start_stream()` for continuous acquisition. Drivers that support
    /// finite acquisition should override this method.
    async fn start_stream_finite(&self, frame_limit: Option<u32>) -> Result<()> {
        match frame_limit {
            Some(n) if n > 0 => {
                tracing::warn!(
                    "Device does not support finite acquisition; starting continuous stream \
                     (requested {} frames)",
                    n
                );
                self.start_stream().await
            }
            _ => self.start_stream().await,
        }
    }

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

    /// Take the frame receiver for consuming streamed frames
    ///
    /// **DEPRECATED**: Use `subscribe_frames()` instead for multi-subscriber support.
    ///
    /// This can only be called once - subsequent calls return None.
    /// Call this BEFORE `start_stream()` to receive frames.
    ///
    /// # Returns
    /// - Some(receiver) if receiver is available
    /// - None if receiver was already taken or not supported by this device
    #[deprecated(since = "0.2.0", note = "Use subscribe_frames() for multi-subscriber support")]
    async fn take_frame_receiver(
        &self,
    ) -> Option<tokio::sync::mpsc::Receiver<crate::hardware::Frame>> {
        // Default: no frame receiver support
        None
    }

    /// Subscribe to the frame stream
    ///
    /// Returns a broadcast receiver that will receive `Arc<Frame>` for each captured frame.
    /// Multiple subscribers can receive the same frames without copying pixel data.
    /// Can be called multiple times to create additional subscribers.
    ///
    /// # Returns
    /// - Some(receiver) if subscription succeeded
    /// - None if streaming is not supported by this device
    ///
    /// # Example
    /// ```rust,ignore
    /// let rx = camera.subscribe_frames().await?;
    /// while let Ok(frame) = rx.recv().await {
    ///     // Process Arc<Frame> without copying pixel data
    ///     println!("Frame: {}x{}", frame.width, frame.height);
    /// }
    /// ```
    async fn subscribe_frames(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<std::sync::Arc<crate::hardware::Frame>>> {
        // Default: no broadcast support
        None
    }

    /// Check if device is currently streaming frames
    ///
    /// # Returns
    /// - Ok(true) if actively streaming
    /// - Ok(false) if not streaming
    /// - Err if state cannot be determined or not supported
    ///
    /// # Default Implementation
    /// Returns an error indicating state query is not supported.
    async fn is_streaming(&self) -> Result<bool> {
        anyhow::bail!("Streaming state query not supported by this device")
    }

    /// Get the number of frames captured since streaming started
    ///
    /// # Returns
    /// - Count of frames captured during the current or last stream
    ///
    /// # Default Implementation
    /// Returns 0 (no frame count tracking)
    fn frame_count(&self) -> u64 {
        0
    }
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

/// Capability: Wavelength Tuning
///
/// Devices with tunable wavelength output (lasers, monochromators, OPOs).
///
/// # Contract
/// - Wavelength is in nanometers (nm)
/// - `set_wavelength()` may block while tuning (device-specific)
/// - Implementation should validate wavelength is within device range
///
/// # Safety
/// CAUTION: Wavelength changes on high-power lasers may affect
/// beam alignment and optical safety equipment effectiveness.
#[async_trait]
pub trait WavelengthTunable: Send + Sync {
    /// Set output wavelength
    ///
    /// # Arguments
    /// * `wavelength_nm` - Target wavelength in nanometers
    ///
    /// # Returns
    /// - Ok(()) if wavelength set successfully
    /// - Err if value is out of hardware range or tuning failed
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()>;

    /// Get current wavelength setting
    ///
    /// # Returns
    /// Current wavelength in nanometers
    async fn get_wavelength(&self) -> Result<f64>;

    /// Get wavelength tuning range
    ///
    /// # Returns
    /// (min_nm, max_nm) tuple defining the valid wavelength range
    ///
    /// # Default Implementation
    /// Returns a typical NIR range. Override for specific devices.
    fn wavelength_range(&self) -> (f64, f64) {
        (700.0, 1000.0)
    }
}

/// Capability: Shutter Control
///
/// Devices with controllable beam shutter (lasers, light sources).
///
/// # Contract
/// - `open_shutter()` allows beam to pass
/// - `close_shutter()` blocks beam
/// - Shutter state should be queryable
///
/// # Safety
/// CAUTION: Always verify shutter state before assuming beam is blocked.
/// Use hardware interlocks for laser safety, never rely on software alone.
#[async_trait]
pub trait ShutterControl: Send + Sync {
    /// Open the shutter (allow beam to pass)
    ///
    /// # Returns
    /// - Ok(()) if shutter opened successfully
    /// - Err if shutter cannot be opened or hardware error
    ///
    /// # Safety
    /// Opening the shutter on a high-power laser creates an immediate
    /// eye/skin hazard. Verify safety interlocks before calling.
    async fn open_shutter(&self) -> Result<()>;

    /// Close the shutter (block beam)
    ///
    /// # Returns
    /// - Ok(()) if shutter closed successfully
    /// - Err if shutter cannot be closed or hardware error
    async fn close_shutter(&self) -> Result<()>;

    /// Query shutter state
    ///
    /// # Returns
    /// - Ok(true) if shutter is open (beam can pass)
    /// - Ok(false) if shutter is closed (beam blocked)
    /// - Err if state cannot be determined
    async fn is_shutter_open(&self) -> Result<bool>;
}

/// Capability: Emission Control
///
/// Devices with controllable emission (lasers, light sources).
///
/// # Contract
/// - `enable_emission()` activates the source
/// - `disable_emission()` deactivates the source
/// - Emission state should be queryable when possible
///
/// # Safety
/// CAUTION: Enabling emission on a high-power laser creates immediate
/// hazards. Always verify safety interlocks and shutter state first.
#[async_trait]
pub trait EmissionControl: Send + Sync {
    /// Enable emission (turn on the source)
    ///
    /// # Returns
    /// - Ok(()) if emission enabled successfully
    /// - Err if emission cannot be enabled or hardware error
    ///
    /// # Safety
    /// Enabling emission on high-power sources requires:
    /// - Proper PPE (safety glasses, etc.)
    /// - Verified beam path
    /// - Interlock systems active
    async fn enable_emission(&self) -> Result<()>;

    /// Disable emission (turn off the source)
    ///
    /// # Returns
    /// - Ok(()) if emission disabled successfully
    /// - Err if emission cannot be disabled or hardware error
    async fn disable_emission(&self) -> Result<()>;

    /// Query emission state
    ///
    /// # Returns
    /// - Ok(true) if emission is active
    /// - Ok(false) if emission is inactive
    /// - Err if state cannot be determined
    ///
    /// # Default Implementation
    /// Returns error indicating state query is not supported.
    async fn is_emission_enabled(&self) -> Result<bool> {
        anyhow::bail!("Emission state query not supported by this device")
    }
}

/// Capability: Device Staging (Bluesky-style lifecycle)
///
/// Devices that require preparation before acquisition sequences and cleanup after.
/// This follows the Bluesky/ophyd device lifecycle pattern.
///
/// # Contract
/// - `stage()` prepares device for acquisition (e.g., configure buffers, enable triggers)
/// - `unstage()` cleans up after acquisition (e.g., release resources, reset state)
/// - Staging/unstaging may be nested (count references internally if needed)
///
/// # Usage Pattern
/// ```rust,ignore
/// // Before scan
/// device.stage().await?;
///
/// // Perform acquisition
/// for position in scan_positions {
///     stage.move_abs(position).await?;
///     camera.trigger().await?;
/// }
///
/// // After scan
/// device.unstage().await?;
/// ```
#[async_trait]
pub trait Stageable: Send + Sync {
    /// Prepare device for acquisition sequence
    ///
    /// Called before a scan or acquisition sequence begins.
    /// May configure hardware buffers, enable triggers, or set parameters.
    ///
    /// # Returns
    /// - Ok(()) if staging successful
    /// - Err if device cannot be staged or is in error state
    async fn stage(&self) -> Result<()>;

    /// Clean up after acquisition sequence
    ///
    /// Called after a scan or acquisition sequence completes.
    /// Should release resources, disable triggers, and reset state.
    ///
    /// # Returns
    /// - Ok(()) if unstaging successful
    /// - Err if device cannot be unstaged or is in error state
    async fn unstage(&self) -> Result<()>;

    /// Query staging state
    ///
    /// # Returns
    /// - Ok(true) if device is currently staged
    /// - Ok(false) if device is not staged
    /// - Err if state cannot be determined or not supported
    ///
    /// # Default Implementation
    /// Returns an error indicating state query is not supported.
    async fn is_staged(&self) -> Result<bool> {
        anyhow::bail!("Staged state query not supported by this device")
    }
}

/// Capability: Settable (Configurable Parameters)
///
/// Devices that have parameters which can be set and optionally queried.
///
/// # Contract
/// - `set_value()` sets the parameter to a new value.
/// - `get_value()` queries the current value of the parameter.
/// - Values are represented as `serde_json::Value` to allow flexibility (f64, i64, bool, string, enum).
/// - Methods take `&self` (not `&mut self`) to allow use with `Arc<dyn Settable>`.
///   Implementations should use interior mutability (e.g., `Mutex`) for state changes.
#[async_trait]
pub trait Settable: Send + Sync {
    /// Set a named parameter to a new value.
    ///
    /// # Arguments
    /// * `name` - The identifier for the parameter to set.
    /// * `value` - The new value for the parameter.
    async fn set_value(&self, name: &str, value: serde_json::Value) -> Result<()>;

    /// Get the current value of a named parameter.
    ///
    /// # Arguments
    /// * `name` - The identifier for the parameter to query.
    async fn get_value(&self, name: &str) -> Result<serde_json::Value> {
        anyhow::bail!("Get value for '{}' not supported by this device", name)
    }
}

/// Capability: Switchable (On/Off States)
///
/// Devices that can be turned on or off.
///
/// # Contract
/// - `turn_on()` activates the device/feature.
/// - `turn_off()` deactivates the device/feature.
/// - `is_on()` queries the current on/off state.
#[async_trait]
pub trait Switchable: Send + Sync {
    /// Turn on a named switchable feature.
    ///
    /// # Arguments
    /// * `name` - The identifier for the feature to turn on.
    async fn turn_on(&mut self, name: &str) -> Result<()>;

    /// Turn off a named switchable feature.
    ///
    /// # Arguments
    /// * `name` - The identifier for the feature to turn off.
    async fn turn_off(&mut self, name: &str) -> Result<()>;

    /// Query the on/off state of a named switchable feature.
    ///
    /// # Arguments
    /// * `name` - The identifier for the feature to query.
    ///
    /// # Returns
    /// - `Ok(true)` if the feature is on.
    /// - `Ok(false)` if the feature is off.
    /// - `Err` if the state cannot be determined or is not supported.
    async fn is_on(&self, name: &str) -> Result<bool> {
        anyhow::bail!("State query for '{}' not supported by this device", name)
    }
}

/// Capability: Actionable (One-Time Commands)
///
/// Devices that can perform one-time actions.
///
/// # Contract
/// - `execute_action()` triggers a specific action.
/// - Actions are typically fire-and-forget or block until completion.
#[async_trait]
pub trait Actionable: Send + Sync {
    /// Execute a named one-time action.
    ///
    /// # Arguments
    /// * `name` - The identifier for the action to execute.
    async fn execute_action(&mut self, name: &str) -> Result<()>;
}

/// Capability: Loggable (Static Metadata)
///
/// Devices that provide static, typically read-only, identification or configuration data.
/// This data is usually read once at initialization and logged.
///
/// # Contract
/// - `get_log_value()` retrieves a specific piece of loggable data.
/// - Values are typically strings (e.g., serial number, firmware version).
#[async_trait]
pub trait Loggable: Send + Sync {
    /// Get a named piece of static loggable data.
    ///
    /// # Arguments
    /// * `name` - The identifier for the loggable data (e.g., "serial_number", "firmware_version").
    async fn get_log_value(&self, name: &str) -> Result<String>;
}

/// Capability: Parameter Registry Access
///
/// Devices that expose their parameters for introspection and control.
///
/// This trait enables generic code (gRPC, presets, HDF5 writers) to:
/// - List all parameters of a device
/// - Subscribe to parameter changes
/// - Snapshot device state for reproducibility
///
/// # Contract
/// - `parameters()` returns a reference to the device's parameter registry
/// - The ParameterSet should contain all mutable device parameters
/// - Parameters must use Parameter<T> for hardware-backed state
///
/// # Example
///
/// ```rust,ignore
/// impl Parameterized for MockCamera {
///     fn parameters(&self) -> &ParameterSet {
///         &self.params
///     }
/// }
///
/// // Generic code can now enumerate parameters
/// fn list_all_parameters<D: Parameterized>(device: &D) {
///     for name in device.parameters().names() {
///         println!("Parameter: {}", name);
///     }
/// }
/// ```
pub trait Parameterized: Send + Sync {
    /// Get device's parameter registry
    fn parameters(&self) -> &ParameterSet;
}

// =============================================================================
// Trait Composition Examples (Documentation)
// =============================================================================
//
// Example: Triggered Camera
//
// A camera that supports external triggering would implement:
// ```rust,ignore
// struct TriggeredCamera { /* ... */ }
//
// impl Triggerable for TriggeredCamera { /* ... */ }
// impl ExposureControl for TriggeredCamera { /* ... */ }
// impl FrameProducer for TriggeredCamera { /* ... */ }
//
// // Use in generic scan code
// async fn scan_with_camera<C>(camera: &C) -> Result<()>
// where
//     C: Triggerable + ExposureControl + FrameProducer
// {
//     camera.set_exposure(0.1).await?;
//     camera.arm().await?;
//     camera.trigger().await?;
//     Ok(())
// }
// ```
//
// =============================================================================
// Combined Traits (for trait objects)
// =============================================================================

/// Composite trait for cameras (convenience)
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
