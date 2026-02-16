//! Mock implementations for cross-platform development
//!
//! Provides full mock implementations of camera and spectrograph drivers
//! that work without the Andor SDK3 installed. Useful for:
//!
//! - Development on non-Windows platforms
//! - CI/CD pipelines
//! - Unit testing
//! - GUI development without hardware
//!
//! The mock drivers simulate realistic behavior including:
//! - Synthetic gradient frame generation
//! - Wavelength-dependent spectra
//! - Realistic parameter ranges
//! - Async delays to simulate hardware timing

use crate::types::{CameraInfo, Grating, SpectrographInfo, WavelengthCalibration};
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use common::capabilities::{
    ExposureControl, FrameObserver, FrameProducer, ObserverHandle, Parameterized, ShutterControl,
    Triggerable, WavelengthTunable,
};
use common::data::{Frame, FrameView};
use common::observable::ParameterSet;
use common::parameter::Parameter;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Duration;

/// Mock iStar camera for testing
#[derive(Clone)]
pub struct MockCamera {
    inner: Arc<MockCameraInner>,
}

struct MockCameraInner {
    info: CameraInfo,
    streaming: AtomicBool,
    armed: AtomicBool,
    frame_count: AtomicU32,
    exposure_s: Parameter<f64>,
    mcp_gain: Parameter<u32>,
    observers: Mutex<Vec<(ObserverHandle, Box<dyn FrameObserver>)>>,
    next_observer_id: AtomicU64,
    params: ParameterSet,
}

impl MockCamera {
    pub fn new() -> Self {
        let info = CameraInfo {
            model: "Mock iStar".to_string(),
            serial_number: "MOCK-12345".to_string(),
            firmware_version: "1.0.0-mock".to_string(),
            sensor_width: 2048,
            sensor_height: 2048,
            features: crate::types::FeatureSupport {
                mcp_gain: true,
                gate_mode: true,
                ddg_output_delay: true,
                ddg_output_width: true,
                sensor_cooling: true,
                sensor_temperature: true,
                pixel_encoding: true,
                external_trigger_modes: true,
                electronic_shuttering_mode: true,
                frame_count: true,
            },
        };

        let exposure_s = Parameter::new("exposure_s", 0.001)
            .with_unit("s")
            .with_description("Integration time");
        let mcp_gain = Parameter::new("mcp_gain", 1000u32)
            .with_range(0, 4095)
            .with_description("MCP intensifier gain");

        let mut params = ParameterSet::new();
        params.register(exposure_s.clone());
        params.register(mcp_gain.clone());

        let inner = Arc::new(MockCameraInner {
            info,
            streaming: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            frame_count: AtomicU32::new(0),
            exposure_s,
            mcp_gain,
            observers: Mutex::new(Vec::new()),
            next_observer_id: AtomicU64::new(1),
            params,
        });

        Self { inner }
    }

    /// Generate synthetic gradient frame
    fn generate_frame(&self, width: u32, height: u32, frame_number: u32) -> Frame {
        let size = (width * height) as usize;
        let mut pixels = vec![0u16; size];

        // Generate gradient pattern with frame number as offset
        let offset = (frame_number % 100) as u16;
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                let value = ((x + y + offset as u32) % 65535) as u16;
                pixels[idx] = value;
            }
        }

        // Convert u16 pixels to bytes
        let mut data = Vec::with_capacity(pixels.len() * 2);
        for pixel in pixels {
            data.extend_from_slice(&pixel.to_le_bytes());
        }

        let timestamp_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|_| std::time::Duration::from_secs(0))
            .as_nanos() as u64;

        Frame {
            width,
            height,
            bit_depth: 16,
            data: Bytes::from(data),
            timestamp_ns,
            frame_number: frame_number as u64,
            exposure_ms: Some(self.inner.exposure_s.get() * 1000.0),
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    /// Notify registered observers with a FrameView of the given frame.
    async fn notify_observers(&self, frame: &Frame) {
        let observers = self.inner.observers.lock().await;
        if !observers.is_empty() {
            let frame_view = FrameView::from_frame(frame);
            for (_, observer) in observers.iter() {
                observer.on_frame(&frame_view);
            }
        }
    }
}

#[async_trait]
impl FrameProducer for MockCamera {
    async fn start_stream(&self) -> Result<()> {
        self.inner.streaming.store(true, Ordering::Relaxed);

        // Spawn frame generation task
        let camera = self.clone();
        tokio::spawn(async move {
            let mut frame_number = 0u32;

            while camera.inner.streaming.load(Ordering::Relaxed) {
                let exposure = camera.inner.exposure_s.get();
                tokio::time::sleep(Duration::from_secs_f64(exposure)).await;

                let frame = camera.generate_frame(2048, 2048, frame_number);
                camera.notify_observers(&frame).await;
                frame_number += 1;
            }
        });

        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        self.inner.streaming.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        (2048, 2048)
    }

    async fn register_observer(&self, observer: Box<dyn FrameObserver>) -> Result<ObserverHandle> {
        let handle = ObserverHandle(self.inner.next_observer_id.fetch_add(1, Ordering::Relaxed));
        self.inner.observers.lock().await.push((handle, observer));
        Ok(handle)
    }

    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()> {
        let mut observers = self.inner.observers.lock().await;
        observers.retain(|(h, _)| *h != handle);
        Ok(())
    }
}

#[async_trait]
impl Triggerable for MockCamera {
    async fn arm(&self) -> Result<()> {
        self.inner.armed.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        if !self.inner.armed.load(Ordering::Relaxed) {
            anyhow::bail!("Camera not armed");
        }

        let frame_number = self.inner.frame_count.fetch_add(1, Ordering::Relaxed);
        let frame = self.generate_frame(2048, 2048, frame_number);
        self.notify_observers(&frame).await;

        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.inner.armed.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl ExposureControl for MockCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        self.inner.exposure_s.set(seconds).await?;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(self.inner.exposure_s.get())
    }
}

impl Parameterized for MockCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}

/// Mock Shamrock spectrograph for testing
#[derive(Clone)]
pub struct MockSpectrograph {
    inner: Arc<MockSpectrographInner>,
}

struct MockSpectrographInner {
    info: SpectrographInfo,
    wavelength_nm: Parameter<f64>,
    grating: Parameter<Grating>,
    shutter_open: AtomicBool,
    params: ParameterSet,
}

impl MockSpectrograph {
    pub fn new() -> Self {
        let info = SpectrographInfo {
            model: "Mock Shamrock".to_string(),
            serial_number: "MOCK-SPEC-001".to_string(),
            num_gratings: 3,
        };

        let wavelength_nm = Parameter::new("wavelength_nm", 310.0)
            .with_unit("nm")
            .with_description("Center wavelength");
        let grating =
            Parameter::new("grating", Grating::Grating2).with_description("Active grating");

        let mut params = ParameterSet::new();
        params.register(wavelength_nm.clone());
        params.register(grating.clone());

        let inner = Arc::new(MockSpectrographInner {
            info,
            wavelength_nm,
            grating,
            shutter_open: AtomicBool::new(false),
            params,
        });

        Self { inner }
    }

    pub async fn get_wavelength_calibration(
        &self,
        num_pixels: u32,
    ) -> Result<WavelengthCalibration> {
        let center = self.inner.wavelength_nm.get();
        let dispersion = 0.05; // nm per pixel

        let wavelengths: Vec<f64> = (0..num_pixels)
            .map(|i| center + (i as f64 - num_pixels as f64 / 2.0) * dispersion)
            .collect();

        Ok(WavelengthCalibration::new(wavelengths))
    }
}

#[async_trait]
impl WavelengthTunable for MockSpectrograph {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.inner.wavelength_nm.set(wavelength_nm).await?;
        // Simulate hardware delay
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        Ok(self.inner.wavelength_nm.get())
    }

    fn wavelength_range(&self) -> (f64, f64) {
        (200.0, 1000.0)
    }
}

#[async_trait]
impl ShutterControl for MockSpectrograph {
    async fn open_shutter(&self) -> Result<()> {
        self.inner.shutter_open.store(true, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        self.inner.shutter_open.store(false, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        Ok(self.inner.shutter_open.load(Ordering::Relaxed))
    }
}

impl Parameterized for MockSpectrograph {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}
