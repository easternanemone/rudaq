//! Camera configuration methods: trigger, gate, MCP, DDG, ROI, binning, cooling, temperature.

use super::AndorCamera;
use anyhow::Result;
use common::core::Roi;
use std::sync::atomic::Ordering;

#[cfg(feature = "camera")]
use andor_sdk3_sys::*;

use crate::types::{ElectronicShutteringMode, GateMode, TriggerMode};

impl AndorCamera {
    /// Get camera information
    pub fn info(&self) -> &crate::types::CameraInfo {
        &self.inner.info
    }

    /// Set trigger mode (cannot change during acquisition)
    pub async fn set_trigger_mode(&self, mode: &str) -> Result<()> {
        self.check_not_streaming()?;
        let trigger_mode = TriggerMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;
        self.inner.trigger_mode.set(trigger_mode).await?;
        Ok(())
    }

    /// Get trigger mode
    pub async fn get_trigger_mode(&self) -> Result<String> {
        Ok(self.inner.trigger_mode.get().to_string())
    }

    /// Set gate mode (CW or DDG)
    pub async fn set_gate_mode(&self, mode: &str) -> Result<()> {
        let gate_mode = GateMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;
        self.inner.gate_mode.set(gate_mode).await?;
        Ok(())
    }

    /// Set MCP gain (0-4095)
    pub async fn set_mcp_gain(&self, gain: u32) -> Result<()> {
        self.inner.mcp_gain.set(gain).await?;
        Ok(())
    }

    /// Get MCP gain
    pub async fn get_mcp_gain(&self) -> Result<u32> {
        Ok(self.inner.mcp_gain.get())
    }

    /// Set DDG output delay in picoseconds
    pub async fn set_ddg_output_delay(&self, delay_ps: u64) -> Result<()> {
        self.inner.ddg_output_delay_ps.set(delay_ps).await?;
        Ok(())
    }

    /// Set DDG output width in picoseconds
    pub async fn set_ddg_output_width(&self, width_ps: u64) -> Result<()> {
        self.inner.ddg_output_width_ps.set(width_ps).await?;
        Ok(())
    }

    /// Query the current valid range for ExposureTime (in seconds).
    ///
    /// The valid range changes dynamically based on trigger mode, gate mode,
    /// and other camera settings. Always query after changing modes.
    pub async fn get_exposure_range(&self) -> Result<(f64, f64)> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let (min, max) = crate::ffi_timeout::ffi_call(move || {
                let min = Self::get_float_min(handle, "ExposureTime")?;
                let max = Self::get_float_max(handle, "ExposureTime")?;
                Ok::<(f64, f64), anyhow::Error>((min, max))
            }, crate::ffi_timeout::FFI_QUERY_TIMEOUT, "get_exposure_range")
            .await??;
            return Ok((min, max));
        }

        #[cfg(not(feature = "camera"))]
        Ok((0.0000001, 30.0))
    }

    /// Check if a named SDK feature is implemented on this camera model.
    ///
    /// This is a static capability check (`AT_IsImplemented`). Use it to
    /// determine whether the camera model supports a feature at all, not
    /// whether the feature is currently writable in the present state.
    pub async fn is_feature_implemented_on_camera(&self, feature: &str) -> Result<bool> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let feature = feature.to_string();
            return crate::ffi_timeout::ffi_call(move || {
                Self::is_feature_implemented(handle, &feature)
            }, crate::ffi_timeout::FFI_QUERY_TIMEOUT, "is_feature_implemented_on_camera")
            .await?;
        }

        #[cfg(not(feature = "camera"))]
        {
            let _ = feature;
            Ok(true)
        }
    }

    /// Set electronic shuttering mode (cannot change during acquisition)
    pub async fn set_electronic_shuttering(&self, mode: &str) -> Result<()> {
        self.check_not_streaming()?;
        let shuttering_mode =
            ElectronicShutteringMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;
        self.inner
            .electronic_shuttering
            .set(shuttering_mode)
            .await?;
        Ok(())
    }

    /// Set ROI (cannot change during acquisition)
    pub async fn set_roi(&self, roi: Roi) -> Result<()> {
        self.check_not_streaming()?;
        self.inner.roi.set(roi).await?;
        Ok(())
    }

    /// Set binning (cannot change during acquisition)
    pub async fn set_binning(&self, x: u32, y: u32) -> Result<()> {
        self.check_not_streaming()?;
        self.inner.binning.set((x, y)).await?;
        Ok(())
    }

    /// Enable/disable sensor cooling
    pub async fn set_cooling(&self, enabled: bool) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_bool_feature(handle, "SensorCooling", enabled)
            })
            .await??;
        }

        self.inner.cooling_enabled.store(enabled, Ordering::Relaxed);
        Ok(())
    }

    /// Configure cooling at initialization time.
    ///
    /// Sequence (per Andor SDK3 manual — order matters for writability):
    /// 1. Enable `SensorCooling = true` — **must be first**, as
    ///    `TemperatureControl` is not writable while the cooler is off
    /// 2. Set `FanSpeed` (typically `"On"` for air-cooled)
    /// 3. Set temperature target via `TemperatureControl` enum (preferred) or
    ///    `TargetSensorTemperature` float (fallback for cameras without the enum)
    ///
    /// Skips entirely if `SensorCooling` is not supported on this camera model.
    /// Individual step errors are logged as warnings but do not prevent camera use.
    pub async fn configure_cooling(
        &self,
        temperature_control: &str,
        fan_speed: &str,
    ) -> Result<()> {
        if !self.info().features.sensor_cooling {
            tracing::info!(
                "SensorCooling not supported on this camera model, skipping cooling init"
            );
            return Ok(());
        }

        // 1. Enable cooling FIRST — TemperatureControl is not writable while
        //    the cooler is off (SDK3 manual requirement).
        if let Err(e) = self.set_cooling(true).await {
            tracing::warn!(error = %e, "Failed to enable sensor cooling");
        } else {
            tracing::info!("Sensor cooling enabled");
        }

        // 2. Set fan speed (maximizes cooling efficiency before setting target)
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let fan = fan_speed.to_string();
            match tokio::task::spawn_blocking(move || {
                Self::set_enum_feature(handle, "FanSpeed", &fan)
            })
            .await
            {
                Ok(Ok(())) => tracing::info!(fan_speed, "Fan speed configured"),
                Ok(Err(e)) => tracing::warn!(error = %e, fan_speed, "Failed to set fan speed"),
                Err(e) => tracing::warn!(error = %e, "spawn_blocking failed for FanSpeed"),
            }
        }

        // 3. Set temperature target
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let tc_value = temperature_control.to_string();
            match tokio::task::spawn_blocking(move || -> anyhow::Result<&str> {
                // Try TemperatureControl enum (calibrated setpoints)
                if Self::is_feature_implemented(handle, "TemperatureControl")? {
                    if Self::is_feature_writable(handle, "TemperatureControl")? {
                        Self::set_enum_feature(handle, "TemperatureControl", &tc_value)?;
                        return Ok("enum");
                    }
                    let current = Self::get_enum_string(handle, "TemperatureControl")
                        .unwrap_or_else(|_| "unknown".to_string());
                    tracing::info!(
                        current_setpoint = %current,
                        requested = %tc_value,
                        "TemperatureControl is not writable \
                         (camera uses hardware-managed setpoint)"
                    );
                    return Ok("skipped");
                }

                // TemperatureControl not available — try float fallback
                if Self::is_feature_implemented(handle, "TargetSensorTemperature")?
                    && Self::is_feature_writable(handle, "TargetSensorTemperature")?
                {
                    if let Ok(target_c) = tc_value.parse::<f64>() {
                        Self::set_float_feature(handle, "TargetSensorTemperature", target_c)?;
                        return Ok("float");
                    }
                }

                tracing::info!(
                    "No writable temperature target feature available \
                     (camera manages its own setpoint)"
                );
                Ok("skipped")
            })
            .await
            {
                Ok(Ok(method)) => match method {
                    "enum" => tracing::info!(
                        temperature_control,
                        "Temperature target set via TemperatureControl enum"
                    ),
                    "float" => tracing::info!(
                        temperature_control,
                        "Temperature target set via TargetSensorTemperature float (fallback)"
                    ),
                    _ => { /* already logged inside spawn_blocking */ }
                },
                Ok(Err(e)) => tracing::warn!(
                    error = %e,
                    temperature_control,
                    "Failed to set temperature target"
                ),
                Err(e) => {
                    tracing::warn!(error = %e, "spawn_blocking failed for temperature control")
                }
            }
        }

        // Log current temperature for visibility
        match self.get_temperature().await {
            Ok(temp) => tracing::info!(
                current_temp_c = temp,
                temperature_control,
                "Cooling initialized, waiting for stabilization"
            ),
            Err(e) => tracing::warn!(error = %e, "Could not read current temperature"),
        }

        Ok(())
    }

    /// Get sensor temperature in Celsius
    pub async fn get_temperature(&self) -> Result<f64> {
        #[cfg(feature = "camera")]
        self.inner.temperature_c.read_from_hardware().await?;

        Ok(self.inner.temperature_c.get())
    }

    /// Set target cooling temperature in Celsius via `TargetSensorTemperature`.
    pub async fn set_target_temperature(&self, target_c: f64) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_float_feature(handle, "TargetSensorTemperature", target_c)
            })
            .await??;
        }

        self.inner.target_temperature_c.set(target_c).await?;
        Ok(())
    }

    /// Get the cooling status from the SDK.
    pub async fn get_cooling_status(&self) -> Result<crate::types::CoolingStatus> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let status_str = tokio::task::spawn_blocking(move || {
                Self::get_enum_string(handle, "TemperatureStatus")
            })
            .await??;

            return match status_str.as_str() {
                "Cooler Off" => Ok(crate::types::CoolingStatus::Off),
                "Stabilised" => Ok(crate::types::CoolingStatus::Stabilised),
                "Cooling" | "Not Stabilised" => Ok(crate::types::CoolingStatus::Stabilising),
                "Fault" => Ok(crate::types::CoolingStatus::Fault),
                other => {
                    tracing::warn!(status = other, "Unknown cooling status, treating as Off");
                    Ok(crate::types::CoolingStatus::Off)
                }
            };
        }

        #[cfg(not(feature = "camera"))]
        {
            if self.inner.cooling_enabled.load(Ordering::Relaxed) {
                Ok(crate::types::CoolingStatus::Stabilised)
            } else {
                Ok(crate::types::CoolingStatus::Off)
            }
        }
    }

    /// Get total frames dropped during current/last acquisition.
    pub fn frames_dropped(&self) -> u64 {
        self.inner.frames_dropped.load(Ordering::Relaxed)
    }

    /// Start a background task that periodically reads sensor temperature.
    pub async fn start_drift_polling(&self, interval: std::time::Duration) {
        // Stop existing task if any
        self.stop_drift_polling().await;

        let inner = self.inner.clone();
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;

                #[cfg(feature = "camera")]
                {
                    if let Ok(()) = inner.temperature_c.read_from_hardware().await {
                        // Parameter updated in-place
                    }
                }

                #[cfg(not(feature = "camera"))]
                {
                    // Mock: simulate slow drift towards target
                    let current = inner.temperature_c.get();
                    let target = inner.target_temperature_c.get();
                    let step = (target - current) * 0.1;
                    if step.abs() > 0.01 {
                        let _ = inner.temperature_c.set(current + step).await;
                    }
                }
            }
        });

        *self.inner.drift_task_handle.lock().await = Some(handle);
    }

    /// Stop the background drift polling task.
    pub async fn stop_drift_polling(&self) {
        if let Some(handle) = self.inner.drift_task_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
    }

    /// Returns true if the last acquisition ended due to an error.
    pub fn has_error(&self) -> bool {
        self.inner
            .last_error
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Returns the error message from the last failed acquisition.
    pub fn last_error(&self) -> Option<String> {
        self.inner
            .last_error
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Clear the error state. Call before retrying acquisition.
    pub fn clear_error(&self) {
        if let Ok(mut guard) = self.inner.last_error.lock() {
            *guard = None;
        }
    }

    /// Record an acquisition error (used internally by the acq loop).
    pub(super) fn set_error(inner: &super::AndorCameraInner, msg: String) {
        if let Ok(mut guard) = inner.last_error.lock() {
            *guard = Some(msg);
        }
        // Drop all observer senders so recv() returns None in gRPC tasks
        inner.tap_registry.clear_all();
    }

    /// Check if MCP gain is supported (iStar only).
    pub fn supports_mcp_gain(&self) -> bool {
        self.inner.info.features.mcp_gain
    }

    /// Check if gate mode is supported (iStar only).
    pub fn supports_gate_mode(&self) -> bool {
        self.inner.info.features.gate_mode
    }

    /// Check if DDG output is supported (iStar only).
    pub fn supports_ddg(&self) -> bool {
        self.inner.info.features.ddg_output_delay && self.inner.info.features.ddg_output_width
    }

    /// Set MCP gain with feature guard. Returns error on non-iStar cameras.
    pub async fn set_mcp_gain_guarded(&self, gain: u32) -> Result<()> {
        if !self.supports_mcp_gain() {
            anyhow::bail!(
                "MCP gain not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_mcp_gain(gain).await
    }

    /// Set gate mode with feature guard. Returns error on non-iStar cameras.
    pub async fn set_gate_mode_guarded(&self, mode: &str) -> Result<()> {
        if !self.supports_gate_mode() {
            anyhow::bail!(
                "Gate mode not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_gate_mode(mode).await
    }

    /// Set DDG output delay with feature guard. Returns error on non-iStar cameras.
    pub async fn set_ddg_output_delay_guarded(&self, delay_ps: u64) -> Result<()> {
        if !self.supports_ddg() {
            anyhow::bail!(
                "DDG output not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_ddg_output_delay(delay_ps).await
    }

    /// Set DDG output width with feature guard. Returns error on non-iStar cameras.
    pub async fn set_ddg_output_width_guarded(&self, width_ps: u64) -> Result<()> {
        if !self.supports_ddg() {
            anyhow::bail!(
                "DDG output not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_ddg_output_width(width_ps).await
    }
}
