//! Driver factories for plugin registration
//!
//! Implements `DriverFactory` trait for both camera and spectrograph drivers,
//! enabling them to be loaded dynamically from TOML configuration.

#[cfg(feature = "hardware")]
use crate::camera::AndorCamera;
use crate::mock::{MockCamera, MockSpectrograph};
#[cfg(feature = "hardware")]
use crate::spectrograph::AndorSpectrograph;
use anyhow::Result;
use common::driver::{Capability, DeviceComponents, DriverFactory};
use futures::future::BoxFuture;
use std::sync::Arc;

/// Driver factory for Andor iStar camera
pub struct AndorCameraFactory;

impl DriverFactory for AndorCameraFactory {
    fn driver_type(&self) -> &'static str {
        "andor_istar"
    }

    fn name(&self) -> &'static str {
        "Andor iStar Intensified Camera"
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::FrameProducer,
            Capability::Triggerable,
            Capability::ExposureControl,
            Capability::Parameterized,
        ]
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        // For hardware mode, camera_index is required
        // For mock mode, it's optional (defaults to 0)
        if cfg!(feature = "hardware") {
            if config.get("camera_index").is_none() {
                anyhow::bail!("Missing required field: camera_index");
            }
        }

        // If camera_index is present, verify it's a non-negative integer
        if let Some(idx) = config.get("camera_index") {
            match idx.as_integer() {
                Some(i) if i >= 0 => { /* valid */ }
                Some(_) => anyhow::bail!("camera_index must be non-negative"),
                None => anyhow::bail!("camera_index must be an integer"),
            }
        }

        // Validate optional cooling configuration.
        //
        // - enable_cooling (bool): Enable TEC on init. Default: true.
        // - temperature_control (string): Value for the SDK3 TemperatureControl
        //   enum (e.g. "0.00"). Valid values are camera-model-specific calibrated
        //   setpoints; validation defers to the SDK at runtime. If the camera
        //   lacks TemperatureControl, the value is parsed as a float for the
        //   TargetSensorTemperature fallback. Default: "0.00".
        // - fan_speed (string): SDK3 FanSpeed enum. Default: "On".
        if let Some(v) = config.get("enable_cooling") {
            if v.as_bool().is_none() {
                anyhow::bail!("enable_cooling must be a boolean");
            }
        }
        if let Some(v) = config.get("temperature_control") {
            match v.as_str() {
                Some(s) if !s.is_empty() => { /* valid — SDK validates the actual enum value */ }
                Some(_) => anyhow::bail!("temperature_control must be a non-empty string"),
                None => anyhow::bail!("temperature_control must be a string (e.g. \"0.00\")"),
            }
        }
        if let Some(v) = config.get("fan_speed") {
            match v.as_str() {
                Some("Off" | "Low" | "On") => { /* valid */ }
                Some(other) => {
                    anyhow::bail!("fan_speed must be \"Off\", \"Low\", or \"On\", got \"{other}\"")
                }
                None => anyhow::bail!("fan_speed must be a string"),
            }
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            #[cfg(feature = "hardware")]
            {
                let camera_index = config
                    .get("camera_index")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(0) as i32;

                let camera = Arc::new(AndorCamera::new_async(camera_index).await?);

                // Configure cooling if enabled (default: true).
                // Uses TemperatureControl enum (calibrated setpoints) as primary
                // mechanism; falls back to TargetSensorTemperature float for
                // cameras that don't implement the enum.
                let enable_cooling = config
                    .get("enable_cooling")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if enable_cooling {
                    let temperature_control = config
                        .get("temperature_control")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.00");
                    let fan_speed = config
                        .get("fan_speed")
                        .and_then(|v| v.as_str())
                        .unwrap_or("On");
                    if let Err(e) = camera
                        .configure_cooling(temperature_control, fan_speed)
                        .await
                    {
                        tracing::warn!(error = %e, "Cooling init failed, camera will operate without cooling");
                    }
                }

                let components = DeviceComponents::new()
                    .with_frame_producer(camera.clone())
                    .with_triggerable(camera.clone())
                    .with_exposure_control(camera.clone())
                    .with_parameterized(camera);

                Ok(components)
            }

            #[cfg(not(feature = "hardware"))]
            {
                tracing::warn!("Using mock Andor camera (hardware feature not enabled)");
                let camera = Arc::new(MockCamera::new());

                // Log cooling config for visibility (mock doesn't talk to real hardware)
                let enable_cooling = config
                    .get("enable_cooling")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if enable_cooling {
                    let temperature_control = config
                        .get("temperature_control")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.00");
                    let fan_speed = config
                        .get("fan_speed")
                        .and_then(|v| v.as_str())
                        .unwrap_or("On");
                    tracing::info!(
                        temperature_control,
                        fan_speed,
                        "Mock camera: cooling configured (no real hardware)"
                    );
                }

                let components = DeviceComponents::new()
                    .with_frame_producer(camera.clone())
                    .with_triggerable(camera.clone())
                    .with_exposure_control(camera.clone())
                    .with_parameterized(camera);

                Ok(components)
            }
        })
    }
}

/// Driver factory for Andor Shamrock spectrograph
pub struct AndorSpectrographFactory;

impl DriverFactory for AndorSpectrographFactory {
    fn driver_type(&self) -> &'static str {
        "andor_shamrock"
    }

    fn name(&self) -> &'static str {
        "Andor Shamrock Spectrograph"
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::WavelengthTunable,
            Capability::ShutterControl,
            Capability::Parameterized,
        ]
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        // device_index is optional (defaults to 0), but must be non-negative if present
        if let Some(idx) = config.get("device_index") {
            match idx.as_integer() {
                Some(i) if i >= 0 => { /* valid */ }
                Some(_) => anyhow::bail!("device_index must be non-negative"),
                None => anyhow::bail!("device_index must be an integer"),
            }
        }
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            #[cfg(feature = "hardware")]
            {
                let device_index = config
                    .get("device_index")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(0) as i32;

                let spectrograph = Arc::new(AndorSpectrograph::new_async(device_index).await?);

                let components = DeviceComponents::new()
                    .with_wavelength_tunable(spectrograph.clone())
                    .with_shutter_control(spectrograph.clone())
                    .with_parameterized(spectrograph);

                Ok(components)
            }

            #[cfg(not(feature = "hardware"))]
            {
                tracing::warn!("Using mock Shamrock spectrograph (hardware feature not enabled)");
                let spectrograph = Arc::new(MockSpectrograph::new());

                let components = DeviceComponents::new()
                    .with_wavelength_tunable(spectrograph.clone())
                    .with_shutter_control(spectrograph.clone())
                    .with_parameterized(spectrograph);

                Ok(components)
            }
        })
    }
}

// Auto-register factories on crate load
#[cfg(feature = "camera")]
#[ctor::ctor]
fn register_camera_factory() {
    // Registration will be done by the drivers crate when linking
}

#[cfg(feature = "spectrograph")]
#[ctor::ctor]
fn register_spectrograph_factory() {
    // Registration will be done by the drivers crate when linking
}
