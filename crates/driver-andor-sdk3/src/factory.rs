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
