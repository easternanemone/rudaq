//! Safe Dover Motion axis driver implementation
//!
//! This module provides a safe async wrapper around the unsafe FFI bindings
//! from dover-motion-sys. All blocking FFI calls are wrapped in spawn_blocking
//! to avoid blocking the tokio runtime.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use common::capabilities::{Movable, Parameterized, TriggerOnPosition};
use common::error::DaqError;
use common::observable::ParameterSet;
use common::parameter::Parameter;
use dover_motion_sys::*;
use std::ffi::CString;
use std::os::raw::c_void;
use std::ptr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::instrument;

/// Newtype wrapper for FFI axis handle pointer
///
/// # Safety
///
/// This type wraps a raw pointer from the Dover Motion C++ SDK.
/// The caller must ensure the Dover Motion SDK library remains loaded
/// for the lifetime of this handle.
///
/// ## Thread Safety
///
/// - **Send**: Handles can be moved between threads. The Dover Motion SDK
///   documentation indicates it has an internal dispatcher thread for callbacks
///   (docs/reference/markdown/dover-motion-api-manual.md line 7858), which
///   suggests handles can be transferred between threads.
///
/// - **Sync is NOT implemented**: The SDK documentation does not provide
///   explicit guarantees about concurrent access to the same handle from
///   multiple threads. Thread-safe access is ensured by wrapping AxisHandle
///   in `Arc<Mutex<>>` in DoverAxisDriver, which serializes all operations.
#[repr(transparent)]
struct AxisHandle(*mut c_void);

// SAFETY: AxisHandle is a raw pointer to SDK-managed state.
// - Send: Handles can be moved between threads; SDK supports this via its dispatcher.
// - Sync is NOT implemented because no concurrent access guarantees are documented.
// - Thread-safe access is ensured by wrapping in Arc<Mutex<>> in DoverAxisDriver.
unsafe impl Send for AxisHandle {}
// Note: Sync intentionally not implemented

/// Safe Dover Motion axis driver
///
/// Wraps the unsafe C++ MotionSynergyAPI with async Rust interface.
/// All blocking FFI calls are executed in `spawn_blocking` to prevent
/// blocking the tokio runtime.
pub struct DoverAxisDriver {
    /// Opaque pointer to IAxisDevice (from FFI)
    /// Protected by Mutex for thread-safe access
    axis_handle: Arc<Mutex<AxisHandle>>,

    /// Axis name for logging/debugging
    axis_name: String,

    /// Position parameter (mm or µm depending on configuration)
    position_param: Parameter<f64>,

    /// Velocity parameter (mm/s)
    velocity_param: Parameter<f64>,

    /// Acceleration parameter (mm/s²)
    acceleration_param: Parameter<f64>,

    /// Parameter registry
    params: Arc<ParameterSet>,

    /// Trigger-on-position enabled state
    top_enabled_param: Parameter<bool>,
}

impl DoverAxisDriver {
    /// Create a new Dover Motion axis driver asynchronously.
    ///
    /// This is the **preferred constructor** for production use.
    /// Initializes the Dover Motion SDK and connects to the specified axis.
    ///
    /// # Arguments
    ///
    /// * `device_path` - Path to Dover Motion device configuration file
    ///   (e.g., "C:\\ProgramData\\Dover Motion\\SmartStage.xml")
    /// * `axis_name` - Name of the axis to control (e.g., "X", "Y", "Z")
    /// * `communication_type` - Communication method ("USB", "Ethernet", etc.)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Dover Motion SDK is not installed
    /// - Device configuration file doesn't exist
    /// - Axis name doesn't match any configured axis
    /// - Hardware initialization fails
    ///
    /// # Safety
    ///
    /// This function wraps unsafe FFI calls to the Dover Motion SDK.
    /// The SDK must be properly installed and the device must be connected.
    #[instrument(skip_all, fields(axis_name = %axis_name))]
    pub async fn new_async(
        device_path: &str,
        axis_name: &str,
        communication_type: &str,
    ) -> Result<Self> {
        let device_path = device_path.to_string();
        let axis_name_copy = axis_name.to_string();
        let communication_type = communication_type.to_string();

        // Initialize Dover Motion SDK in blocking context
        let axis_handle = tokio::task::spawn_blocking(move || {
            Self::initialize_sdk(&device_path, &axis_name_copy, &communication_type)
        })
        .await
        .context("Failed to spawn SDK initialization task")??;

        // Create parameter registry
        let mut params = ParameterSet::new();

        let position = Parameter::new("position", 0.0)
            .with_description("Axis position")
            .with_unit("mm");

        let velocity = Parameter::new("velocity", 1.0)
            .with_description("Axis velocity")
            .with_unit("mm/s");

        let acceleration = Parameter::new("acceleration", 10.0)
            .with_description("Axis acceleration")
            .with_unit("mm/s²");

        let top_enabled =
            Parameter::new("top_enabled", false).with_description("Trigger-on-position enabled");

        params.register(position.clone());
        params.register(velocity.clone());
        params.register(acceleration.clone());
        params.register(top_enabled.clone());

        Ok(Self {
            axis_handle: Arc::new(Mutex::new(axis_handle)),
            axis_name: axis_name.to_string(),
            position_param: position,
            velocity_param: velocity,
            acceleration_param: acceleration,
            params: Arc::new(params),
            top_enabled_param: top_enabled,
        })
    }

    /// Initialize the Dover Motion SDK and return axis handle.
    ///
    /// # Safety
    ///
    /// This function calls unsafe FFI functions. Caller must ensure:
    /// - Dover Motion SDK is installed
    /// - Device configuration file is valid
    /// - Axis name exists in the configuration
    fn initialize_sdk(
        device_path: &str,
        axis_name: &str,
        communication_type: &str,
    ) -> Result<AxisHandle> {
        unsafe {
            // Convert Rust strings to C strings
            let path_cstr =
                CString::new(device_path).context("Failed to convert device path to CString")?;
            let comm_cstr = CString::new(communication_type)
                .context("Failed to convert communication type to CString")?;
            let axis_cstr =
                CString::new(axis_name).context("Failed to convert axis name to CString")?;

            // Initialize MotionSynergyAPI
            // Note: Actual API calls depend on bindings generated by dover-motion-sys
            // This is a placeholder implementation showing the pattern

            // In a real implementation, you would call:
            // let api = MotionSynergyAPI_Create();
            // if api.is_null() { return Err(anyhow!("Failed to create MotionSynergyAPI")); }
            //
            // let result = MotionSynergyAPI_Configure(api, path_cstr.as_ptr());
            // if result != 0 { return Err(anyhow!("Configure failed with code {}", result)); }
            //
            // let result = MotionSynergyAPI_Connect(api, comm_cstr.as_ptr());
            // if result != 0 { return Err(anyhow!("Connect failed with code {}", result)); }
            //
            // let axis_device = MotionSynergyAPI_GetAxisDevice(api, axis_cstr.as_ptr());
            // if axis_device.is_null() { return Err(anyhow!("Axis '{}' not found", axis_name)); }

            // For now, return a non-null dummy pointer
            // In real implementation, this would be the actual IAxisDevice pointer
            let dummy_handle = AxisHandle(0x1 as *mut c_void);

            tracing::info!(
                "Dover Motion axis '{}' initialized (device: {}, comm: {})",
                axis_name,
                device_path,
                communication_type
            );

            // SAFETY: In real implementation, this would call the MotionSynergyAPI FFI
            // to initialize the device. The handle would be validated by the SDK.
            // This placeholder is safe as it only returns a dummy value for testing.
            Ok(dummy_handle)
        }
    }

    /// Set velocity asynchronously.
    #[instrument(skip(self), fields(axis = %self.axis_name, velocity), err)]
    pub async fn set_velocity(&self, velocity: f64) -> Result<()> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::SetVelocity(velocity)
                // In real implementation:
                // let result = IAxisDevice_SetVelocity(handle as *mut IAxisDevice, velocity);
                // if result != 0 { return Err(anyhow!("SetVelocity failed with code {}", result)); }

                tracing::debug!("Set velocity to {} mm/s", velocity);
                Ok(())
            }
        })
        .await
        .context("Failed to spawn velocity setter task")??;

        self.velocity_param.inner().set(velocity);
        Ok(())
    }

    /// Set acceleration asynchronously.
    #[instrument(skip(self), fields(axis = %self.axis_name, acceleration), err)]
    pub async fn set_acceleration(&self, acceleration: f64) -> Result<()> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::SetAcceleration(acceleration)
                tracing::debug!("Set acceleration to {} mm/s²", acceleration);
                Ok(())
            }
        })
        .await
        .context("Failed to spawn acceleration setter task")??;

        self.acceleration_param.inner().set(acceleration);
        Ok(())
    }

    /// Set deceleration asynchronously.
    #[instrument(skip(self), fields(axis = %self.axis_name, deceleration), err)]
    pub async fn set_deceleration(&self, deceleration: f64) -> Result<()> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::SetDeceleration(deceleration)
                tracing::debug!("Set deceleration to {} mm/s²", deceleration);
                Ok(())
            }
        })
        .await
        .context("Failed to spawn deceleration setter task")?
    }

    /// Check if axis is in motion.
    async fn is_moving(&self) -> Result<bool> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<bool> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Query motion status from hardware
                // In real implementation:
                // let mut is_moving: bool = false;
                // IAxisDevice_IsMoving(handle as *mut IAxisDevice, &mut is_moving);
                // Ok(is_moving)

                Ok(false) // Placeholder
            }
        })
        .await
        .context("Failed to spawn motion status query")?
    }
}

impl Drop for DoverAxisDriver {
    fn drop(&mut self) {
        // TODO(bd-qude): dover-motion-sys does not yet expose Stop/Shutdown FFI functions
        // in mock mode (dummy bindings). When the `dover-sdk` feature provides real bindings,
        // this Drop impl MUST be updated to:
        //
        // 1. Stop axis motion:
        //    unsafe { IAxisDevice_Stop(handle) }
        //    A moving stage that outlives its driver is a collision/damage risk.
        //
        // 2. Release the axis handle / shutdown the SDK:
        //    unsafe { MotionSynergyAPI_Shutdown(api) }
        //
        // All FFI calls must be wrapped in unsafe blocks with SAFETY comments,
        // errors must be logged (not panicked), and the handle must be nulled.

        let handle_ptr = {
            // try_lock: Drop must not block indefinitely. If the mutex is poisoned
            // or contended we still warn and bail — never panic in Drop.
            match self.axis_handle.try_lock() {
                Ok(guard) => guard.0,
                Err(_) => {
                    tracing::warn!(
                        axis = %self.axis_name,
                        "Dover Motion Drop: could not acquire axis handle lock — \
                         skipping cleanup (stage may still be in motion)"
                    );
                    return;
                }
            }
        };

        // Guard: the dummy handle (0x1) produced by initialize_sdk is not a real
        // SDK pointer. Skip FFI teardown when running without the real SDK.
        if handle_ptr.is_null() || handle_ptr == 0x1 as *mut std::os::raw::c_void {
            tracing::debug!(
                axis = %self.axis_name,
                "Dover Motion axis dropped (no real SDK handle — skipping FFI teardown)"
            );
            return;
        }

        // --- Real SDK path (currently unreachable without dover-sdk feature) ---
        //
        // SAFETY: handle_ptr was returned by the Dover Motion SDK during initialize_sdk
        // and has not been freed. We hold the Mutex lock so no concurrent access is
        // possible. Stop() is documented as safe to call at any time (Section 6.2.2).
        //
        // When real bindings are available, uncomment:
        // unsafe {
        //     // Step 1: Halt motion — prevents collision damage on teardown.
        //     let stop_result = IAxisDevice_Stop(handle_ptr as *mut IAxisDevice);
        //     if stop_result != 0 {
        //         tracing::warn!(
        //             axis = %self.axis_name,
        //             code = stop_result,
        //             "Dover Motion Drop: Stop() failed"
        //         );
        //     }
        //
        //     // Step 2: Release the axis / shutdown the API session.
        //     let shutdown_result = MotionSynergyAPI_Shutdown(handle_ptr as *mut MotionSynergyAPI);
        //     if shutdown_result != 0 {
        //         tracing::warn!(
        //             axis = %self.axis_name,
        //             code = shutdown_result,
        //             "Dover Motion Drop: Shutdown() failed"
        //         );
        //     }
        // }

        tracing::warn!(
            axis = %self.axis_name,
            "Dover Motion axis dropped with a live SDK handle but safe-state \
             shutdown is not yet implemented — stage may still be in motion. \
             See TODO(bd-qude) in DoverAxisDriver::Drop."
        );
    }
}

impl Parameterized for DoverAxisDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for DoverAxisDriver {
    #[instrument(skip(self), fields(axis = %self.axis_name, position), err)]
    async fn move_abs(&self, position: f64) -> Result<()> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::MoveAbsolute(position)
                // In real implementation:
                // let result = IAxisDevice_MoveAbsolute(handle as *mut IAxisDevice, position);
                // if result != 0 { return Err(anyhow!("MoveAbsolute failed with code {}", result)); }

                tracing::debug!("Moving to absolute position {} mm", position);
                Ok(())
            }
        })
        .await
        .context("Failed to spawn absolute move task")??;

        self.position_param.inner().set(position);
        Ok(())
    }

    #[instrument(skip(self), fields(axis = %self.axis_name, distance), err)]
    async fn move_rel(&self, distance: f64) -> Result<()> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::MoveRelative(distance)
                tracing::debug!("Moving relative distance {} mm", distance);
                Ok(())
            }
        })
        .await
        .context("Failed to spawn relative move task")?
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn position(&self) -> Result<f64> {
        let axis_handle = self.axis_handle.clone();

        let pos = tokio::task::spawn_blocking(move || -> Result<f64> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::GetActualPosition()
                // In real implementation:
                // let mut position: f64 = 0.0;
                // let result = IAxisDevice_GetActualPosition(handle as *mut IAxisDevice, &mut position);
                // if result != 0 { return Err(anyhow!("GetActualPosition failed")); }
                // Ok(position)

                Ok(0.0) // Placeholder
            }
        })
        .await
        .context("Failed to spawn position query task")??;

        self.position_param.inner().set(pos);
        Ok(pos)
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn wait_settled(&self) -> Result<()> {
        let timeout = std::time::Duration::from_secs(60);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!(
                    "Dover axis wait_settled timed out after 60 seconds"
                ));
            }

            if !self.is_moving().await? {
                return Ok(());
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn stop(&self) -> Result<()> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::Stop()
                tracing::debug!("Stopping axis motion");
                Ok(())
            }
        })
        .await
        .context("Failed to spawn stop command")?
    }
}

#[async_trait]
impl TriggerOnPosition for DoverAxisDriver {
    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn enable_top(
        &self,
        start_position: f64,
        end_position: f64,
        increment: f64,
        bidirectional: bool,
        pulse_width_ns: u64,
    ) -> Result<()> {
        // Validate parameters
        if increment <= 0.0 {
            return Err(anyhow!(
                "Trigger increment must be positive, got {}",
                increment
            ));
        }

        if pulse_width_ns < 50 || pulse_width_ns > 204800 {
            return Err(anyhow!(
                "Pulse width must be 50-204,800 ns, got {}",
                pulse_width_ns
            ));
        }

        if pulse_width_ns % 50 != 0 {
            return Err(anyhow!(
                "Pulse width must be a multiple of 50ns, got {}",
                pulse_width_ns
            ));
        }

        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::EnableTriggerOnPosition(...)
                // In real implementation:
                // let result = IAxisDevice_EnableTriggerOnPosition(
                //     handle as *mut IAxisDevice,
                //     start_position,
                //     end_position,
                //     increment,
                //     bidirectional as i32,
                //     pulse_width_ns,
                // );
                // if result != 0 { return Err(anyhow!("EnableTriggerOnPosition failed")); }

                tracing::info!(
                    "Enabled TOP: start={}, end={}, inc={}, bidir={}, pulse_width={}ns",
                    start_position,
                    end_position,
                    increment,
                    bidirectional,
                    pulse_width_ns
                );
                Ok(())
            }
        })
        .await
        .context("Failed to spawn enable TOP task")??;

        self.top_enabled_param.inner().set(true);
        Ok(())
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn disable_top(&self) -> Result<()> {
        let axis_handle = self.axis_handle.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // SAFETY: Lock is held for entire FFI call duration to serialize access
            let handle = axis_handle.blocking_lock().0;
            unsafe {
                // Call IAxisDevice::DisableTriggerOnPosition()
                tracing::info!("Disabled TOP");
                Ok(())
            }
        })
        .await
        .context("Failed to spawn disable TOP task")??;

        self.top_enabled_param.inner().set(false);
        Ok(())
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn is_top_enabled(&self) -> Result<bool> {
        Ok(self.top_enabled_param.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_drop_does_not_panic() {
        // Create a DoverAxisDriver with the dummy handle and immediately drop it.
        // This exercises the Drop path with the placeholder 0x1 handle.
        let driver = DoverAxisDriver::new_async("/dev/null", "test_axis", "USB").await;
        // new_async should succeed with the dummy implementation
        assert!(driver.is_ok(), "new_async should succeed with dummy SDK");
        let driver = driver.expect("already checked");
        // Explicit drop — must not panic
        drop(driver);
    }

    #[tokio::test]
    async fn test_drop_with_null_handle() {
        // Construct a driver with a null handle to exercise that branch of Drop.
        let mut params = ParameterSet::new();
        let position = Parameter::new("position", 0.0);
        let velocity = Parameter::new("velocity", 1.0);
        let acceleration = Parameter::new("acceleration", 10.0);
        let top_enabled = Parameter::new("top_enabled", false);
        params.register(position.clone());
        params.register(velocity.clone());
        params.register(acceleration.clone());
        params.register(top_enabled.clone());

        let driver = DoverAxisDriver {
            axis_handle: Arc::new(Mutex::new(AxisHandle(ptr::null_mut()))),
            axis_name: "null_test".to_string(),
            position_param: position,
            velocity_param: velocity,
            acceleration_param: acceleration,
            params: Arc::new(params),
            top_enabled_param: top_enabled,
        };
        // Must not panic
        drop(driver);
    }
}
