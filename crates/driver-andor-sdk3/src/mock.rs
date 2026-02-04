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

use crate::types::{CameraInfo, SpectrographInfo, WavelengthCalibration};
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use common::capabilities::{
    ExposureControl, FrameObserver, FrameProducer, ObserverHandle, Parameterized, ShutterControl,
    Triggerable, WavelengthTunable,
};
use common::data::Frame;
use common::observable::ParameterSet;
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
    exposure_s: Mutex<f64>,
    mcp_gain: Mutex<u32>,
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
        };

        let inner = Arc::new(MockCameraInner {
            info,
            streaming: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            frame_count: AtomicU32::new(0),
            exposure_s: Mutex::new(0.001),
            mcp_gain: Mutex::new(1000),
            observers: Mutex::new(Vec::new()),
            next_observer_id: AtomicU64::new(1),
            params: ParameterSet::new(),
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

        Frame {
            width,
            height,
            bit_depth: 16,
            data: Bytes::from(data),
            timestamp_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_else(|_| std::time::Duration::from_secs(0))
                .as_nanos() as u64,
            frame_number: frame_number as u64,
            exposure_ms: None,
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }
}

#[async_trait]
impl FrameProducer for MockCamera {
    async fn start_stream(&self) -> Result<()> {
        self.inner.streaming.store(true, Ordering::Relaxed);

        // Spawn frame generation task
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut frame_number = 0u32;

            while inner.streaming.load(Ordering::Relaxed) {
                let exposure = *inner.exposure_s.lock().await;
                tokio::time::sleep(Duration::from_secs_f64(exposure)).await;

                let _frame = MockCamera {
                    inner: inner.clone(),
                }
                .generate_frame(2048, 2048, frame_number);
                frame_number += 1;

                // TODO: Notify observers with FrameView
                // The observer API changed to use FrameView instead of Frame
                // which requires borrowing the frame data. Skipping for mock mode.
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
        let _frame = self.generate_frame(2048, 2048, frame_number);

        // TODO: Notify observers with FrameView
        // Skipping for mock mode

        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.inner.armed.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl ExposureControl for MockCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        *self.inner.exposure_s.lock().await = seconds;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(*self.inner.exposure_s.lock().await)
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
    wavelength_nm: Mutex<f64>,
    grating: Mutex<i32>,
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

        let inner = Arc::new(MockSpectrographInner {
            info,
            wavelength_nm: Mutex::new(310.0),
            grating: Mutex::new(2),
            shutter_open: AtomicBool::new(false),
            params: ParameterSet::new(),
        });

        Self { inner }
    }

    pub async fn get_wavelength_calibration(
        &self,
        num_pixels: u32,
    ) -> Result<WavelengthCalibration> {
        let center = *self.inner.wavelength_nm.lock().await;
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
        *self.inner.wavelength_nm.lock().await = wavelength_nm;
        // Simulate hardware delay
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        Ok(*self.inner.wavelength_nm.lock().await)
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
