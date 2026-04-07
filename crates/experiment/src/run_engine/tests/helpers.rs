//! Shared mock types and helpers for RunEngine tests.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use common::capabilities::{
    DeviceCategory, FrameProducer, GatedCamera, Parameterized, SpectrometerControl,
    TemperatureStatus,
};
use common::driver::{Capability as DeviceCapability, DeviceComponents, DriverFactory};
use common::observable::{Observable, ParameterSet};
use hardware::registry::DeviceRegistry;

// ---- MockSpectrometer ----

pub(super) struct MockSpectrometer {
    pub grating: i32,
    pub wavelength_nm: f64,
}

#[async_trait]
impl SpectrometerControl for MockSpectrometer {
    async fn set_grating(&self, _grating_num: i32) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_grating(&self) -> anyhow::Result<i32> {
        Ok(self.grating)
    }

    async fn set_wavelength(&self, _nm: f64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_wavelength(&self) -> anyhow::Result<f64> {
        Ok(self.wavelength_nm)
    }

    async fn set_slit_width(&self, _slit_id: i32, _width_um: f64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_calibration(&self, num_pixels: usize) -> anyhow::Result<Vec<f64>> {
        #[allow(clippy::cast_precision_loss)] // test pixel indices are small
        Ok((0..num_pixels).map(|idx| 300.0 + idx as f64).collect())
    }

    async fn is_at_zero_order(&self) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn set_shutter(&self, _open: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

pub(super) struct MockSpectrometerFactory {
    pub grating: i32,
    pub wavelength_nm: f64,
}

impl DriverFactory for MockSpectrometerFactory {
    fn driver_type(&self) -> &'static str {
        "mock_spectrometer"
    }

    fn name(&self) -> &'static str {
        "Mock Spectrometer"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::SpectrometerControl]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let grating = self.grating;
        let wavelength_nm = self.wavelength_nm;
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Detector)
                .with_spectrometer_control(Arc::new(MockSpectrometer {
                    grating,
                    wavelength_nm,
                })))
        })
    }
}

pub(super) async fn make_spectroscopy_registry() -> DeviceRegistry {
    make_spectroscopy_registry_with_spectrometer(1, 300.0).await
}

pub(super) async fn make_spectroscopy_registry_with_spectrometer(
    grating: i32,
    wavelength_nm: f64,
) -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(MockSpectrometerFactory {
        grating,
        wavelength_nm,
    }));
    registry
        .register_from_toml(
            "spectrometer",
            "Mock Spectrometer",
            "mock_spectrometer",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register mock spectrometer");
    registry
}

// ---- MockGatedCamera ----

#[derive(Clone, Copy)]
pub(super) struct MockEchelleCameraConfig {
    pub frame_width: u32,
    pub frame_height: u32,
    pub roi_x: u32,
    pub roi_y: u32,
    pub binning_x: u32,
    pub binning_y: u32,
    pub bit_depth: Option<u32>,
}

impl Default for MockEchelleCameraConfig {
    fn default() -> Self {
        Self {
            frame_width: 1024,
            frame_height: 512,
            roi_x: 0,
            roi_y: 0,
            binning_x: 1,
            binning_y: 1,
            bit_depth: Some(16),
        }
    }
}

pub(super) struct MockGatedCamera {
    params: ParameterSet,
    resolution: (u32, u32),
}

impl MockGatedCamera {
    pub fn new(config: MockEchelleCameraConfig) -> Self {
        let mut params = ParameterSet::new();
        params.register(Observable::new("AOIWidth", i64::from(config.frame_width)));
        params.register(Observable::new("AOIHeight", i64::from(config.frame_height)));
        params.register(Observable::new("AOILeft", i64::from(config.roi_x + 1)));
        params.register(Observable::new("AOITop", i64::from(config.roi_y + 1)));
        params.register(Observable::new("AOIHBin", i64::from(config.binning_x)));
        params.register(Observable::new("AOIVBin", i64::from(config.binning_y)));
        if let Some(bit_depth) = config.bit_depth {
            params.register(Observable::new("BitDepth", i64::from(bit_depth)));
        }

        Self {
            params,
            resolution: (config.frame_width, config.frame_height),
        }
    }
}

impl Parameterized for MockGatedCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl FrameProducer for MockGatedCamera {
    async fn start_stream(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_stream(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        self.resolution
    }
}

#[async_trait]
impl GatedCamera for MockGatedCamera {
    async fn set_gate_mode(&self, _mode: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn set_ddg_timing(&self, _delay_ps: u64, _width_ps: u64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn set_mcp_gain(&self, _gain: u32) -> anyhow::Result<()> {
        Ok(())
    }

    async fn set_intelligate(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_temperature_status(&self) -> anyhow::Result<TemperatureStatus> {
        Ok(TemperatureStatus::Stabilized)
    }
}

struct MockGatedCameraFactory {
    config: MockEchelleCameraConfig,
}

impl DriverFactory for MockGatedCameraFactory {
    fn driver_type(&self) -> &'static str {
        "mock_gated_camera"
    }

    fn name(&self) -> &'static str {
        "Mock Gated Camera"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[
            DeviceCapability::Parameterized,
            DeviceCapability::GatedCamera,
        ]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let config = self.config;
        Box::pin(async move {
            let driver = Arc::new(MockGatedCamera::new(config));
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Camera)
                .with_parameterized(driver.clone())
                .with_gated_camera(driver))
        })
    }
}

pub(super) async fn make_echelle_registry(config: MockEchelleCameraConfig) -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(MockGatedCameraFactory { config }));
    registry
        .register_from_toml(
            "camera",
            "Mock Gated Camera",
            "mock_gated_camera",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register mock gated camera");
    registry
}

// ---- FixedReadable ----

pub(super) struct FixedReadable(pub f64);

#[async_trait]
impl common::capabilities::Readable for FixedReadable {
    async fn read(&self) -> anyhow::Result<f64> {
        Ok(self.0)
    }
}

pub(super) struct FixedReadableFactory {
    pub value: f64,
}

impl DriverFactory for FixedReadableFactory {
    fn driver_type(&self) -> &'static str {
        "fixed_readable"
    }

    fn name(&self) -> &'static str {
        "Fixed Readable"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::Readable]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let value = self.value;
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Detector)
                .with_readable(Arc::new(FixedReadable(value))))
        })
    }
}

pub(super) async fn make_readable_registry(device_id: &str, value: f64) -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(FixedReadableFactory { value }));
    registry
        .register_from_toml(
            device_id,
            "Fixed Readable",
            "fixed_readable",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register fixed readable device");
    registry
}

pub(super) async fn make_two_readable_registry(
    id_a: &str,
    val_a: f64,
    id_b: &str,
    val_b: f64,
) -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(FixedReadableFactory { value: val_a }));
    registry
        .register_from_toml(
            id_a,
            "Readable A",
            "fixed_readable",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register device A");

    // Need a second factory with different value. Re-register with different type.
    struct FixedReadableFactoryB {
        value: f64,
    }

    #[async_trait]
    impl common::capabilities::Readable for FixedReadableB {
        async fn read(&self) -> anyhow::Result<f64> {
            Ok(self.0)
        }
    }

    struct FixedReadableB(f64);

    impl DriverFactory for FixedReadableFactoryB {
        fn driver_type(&self) -> &'static str {
            "fixed_readable_b"
        }
        fn name(&self) -> &'static str {
            "Fixed Readable B"
        }
        fn capabilities(&self) -> &'static [DeviceCapability] {
            &[DeviceCapability::Readable]
        }
        fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
            Ok(())
        }
        fn build(
            &self,
            _config: toml::Value,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
            let value = self.value;
            Box::pin(async move {
                Ok(DeviceComponents::new()
                    .with_category(DeviceCategory::Detector)
                    .with_readable(Arc::new(FixedReadableB(value))))
            })
        }
    }

    registry.register_factory(Box::new(FixedReadableFactoryB { value: val_b }));
    registry
        .register_from_toml(
            id_b,
            "Readable B",
            "fixed_readable_b",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register device B");

    registry
}
