//! MockInstrument V3 - Prototype using new architecture
//!
//! This module demonstrates the new unified architecture with:
//! - Direct Instrument trait implementation (no V1/V2 split)
//! - Camera meta trait for polymorphism
//! - Parameter<T> for declarative parameter management
//! - Direct async communication (no actor model)
//!
//! This serves as the reference implementation for migrating other instruments.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;

use crate::core_v3::{
    Camera, Command, ImageData, ImageMetadata, Instrument, InstrumentState, Measurement,
    ParameterBase, PixelBuffer, Response, Roi,
};
use crate::parameter::{Parameter, ParameterBuilder};

/// Mock camera instrument demonstrating new architecture
pub struct MockCameraV3 {
    /// Instrument identifier
    id: String,

    /// Current state
    state: InstrumentState,

    /// Data broadcast channel
    data_tx: broadcast::Sender<Measurement>,

    /// Parameters (managed via Parameter<T>)
    parameters: HashMap<String, Box<dyn ParameterBase>>,

    /// Camera-specific parameters (typed access)
    exposure: Arc<RwLock<Parameter<f64>>>,
    gain: Arc<RwLock<Parameter<f64>>>,
    roi: Arc<RwLock<Parameter<Roi>>>,
    binning: Arc<RwLock<Parameter<(u32, u32)>>>,

    /// Acquisition state
    is_acquiring: bool,

    /// Background acquisition task
    acquisition_task: Option<JoinHandle<()>>,
}

impl MockCameraV3 {
    /// Create new mock camera with default parameters
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        let (data_tx, _) = broadcast::channel(1024);

        // Create typed parameters with constraints
        let exposure = Arc::new(RwLock::new(
            ParameterBuilder::new("exposure_ms", 100.0)
                .description("Camera exposure time")
                .unit("ms")
                .range(1.0, 10000.0)
                .build(),
        ));

        let gain = Arc::new(RwLock::new(
            ParameterBuilder::new("gain", 1.0)
                .description("Camera gain")
                .unit("dB")
                .range(0.0, 24.0)
                .build(),
        ));

        let roi = Arc::new(RwLock::new(
            Parameter::new("roi", Roi::default()).with_description("Region of interest"),
        ));

        let binning = Arc::new(RwLock::new(
            ParameterBuilder::new("binning", (1u32, 1u32))
                .description("Pixel binning (horizontal, vertical)")
                .choices(vec![(1, 1), (2, 2), (4, 4), (8, 8)])
                .build(),
        ));

        // Create parameter map for dynamic access
        let parameters: HashMap<String, Box<dyn ParameterBase>> = HashMap::new();
        // Note: Would need Clone for Parameter to put in map, this is simplified

        Self {
            id,
            state: InstrumentState::Uninitialized,
            data_tx,
            parameters,
            exposure,
            gain,
            roi,
            binning,
            is_acquiring: false,
            acquisition_task: None,
        }
    }

    /// Generate mock image data
    fn generate_mock_image(&self) -> ImageData {
        let roi = futures::executor::block_on(self.roi.read()).get();
        let exposure_ms = futures::executor::block_on(self.exposure.read()).get();
        let gain_db = futures::executor::block_on(self.gain.read()).get();

        // Generate simple gradient pattern
        let size = (roi.width * roi.height) as usize;
        let mut pixels = vec![0u16; size];
        for (i, pixel) in pixels.iter_mut().enumerate() {
            let x = (i % roi.width as usize) as f64;
            let y = (i / roi.width as usize) as f64;
            let value = ((x + y) * gain_db * (exposure_ms / 100.0)) as u16;
            *pixel = value.min(65535);
        }

        ImageData {
            timestamp: chrono::Utc::now(),
            channel: format!("{}_image", self.id),
            width: roi.width as usize,
            height: roi.height as usize,
            pixels: PixelBuffer::U16(pixels),
            unit: "counts".to_string(),
            metadata: Some(serde_json::json!({
                "exposure_ms": exposure_ms,
                "gain_db": gain_db,
                "roi": {
                    "x": roi.x,
                    "y": roi.y,
                    "width": roi.width,
                    "height": roi.height,
                }
            })),
        }
    }

    /// Start background acquisition task
    fn start_acquisition_task(&mut self) {
        if self.acquisition_task.is_some() {
            return; // Already running
        }

        let data_tx = self.data_tx.clone();
        let id = self.id.clone();
        let exposure = self.exposure.clone();
        let gain = self.gain.clone();
        let roi = self.roi.clone();

        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));

            loop {
                interval.tick().await;

                // Generate mock image
                let roi_val = roi.read().await.get();
                let exposure_ms = exposure.read().await.get();
                let gain_db = gain.read().await.get();

                let size = (roi_val.width * roi_val.height) as usize;
                let mut pixels = vec![0u16; size];
                for (i, pixel) in pixels.iter_mut().enumerate() {
                    let x = (i % roi_val.width as usize) as f64;
                    let y = (i / roi_val.width as usize) as f64;
                    let value = ((x + y) * gain_db * (exposure_ms / 100.0)) as u16;
                    *pixel = value.min(65535);
                }

                let measurement = Measurement::Image {
                    name: format!("{}_frame", id),
                    width: roi_val.width,
                    height: roi_val.height,
                    buffer: PixelBuffer::U16(pixels),
                    unit: "counts".to_string(),
                    metadata: ImageMetadata {
                        exposure_ms: Some(exposure_ms),
                        gain: Some(gain_db),
                        binning: None,
                        temperature_c: None,
                        hardware_timestamp_us: None,
                        readout_ms: None,
                        roi_origin: Some((roi_val.x, roi_val.y)),
                    },
                    timestamp: chrono::Utc::now(),
                };

                // Broadcast (non-blocking, drops if no subscribers)
                let _ = data_tx.send(measurement);
            }
        });

        self.acquisition_task = Some(task);
    }

    /// Stop background acquisition task
    fn stop_acquisition_task(&mut self) {
        if let Some(task) = self.acquisition_task.take() {
            task.abort();
        }
    }
}

#[async_trait]
impl Instrument for MockCameraV3 {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> InstrumentState {
        self.state
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Uninitialized {
            return Err(anyhow!("Instrument already initialized"));
        }

        self.state = InstrumentState::Idle;
        log::info!("MockCameraV3 '{}' initialized", self.id);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.stop_acquisition_task();
        self.state = InstrumentState::ShuttingDown;
        log::info!("MockCameraV3 '{}' shutdown", self.id);
        Ok(())
    }

    fn data_channel(&self) -> broadcast::Receiver<Measurement> {
        self.data_tx.subscribe()
    }

    async fn execute(&mut self, cmd: Command) -> Result<Response> {
        match cmd {
            Command::Start => {
                if self.state != InstrumentState::Idle {
                    return Err(anyhow!("Cannot start from {:?} state", self.state));
                }
                self.start_acquisition_task();
                self.is_acquiring = true;
                self.state = InstrumentState::Running;
                Ok(Response::Ok)
            }

            Command::Stop => {
                self.stop_acquisition_task();
                self.is_acquiring = false;
                self.state = InstrumentState::Idle;
                Ok(Response::Ok)
            }

            Command::Pause => {
                if self.state != InstrumentState::Running {
                    return Err(anyhow!("Cannot pause from {:?} state", self.state));
                }
                self.stop_acquisition_task();
                self.state = InstrumentState::Paused;
                Ok(Response::Ok)
            }

            Command::Resume => {
                if self.state != InstrumentState::Paused {
                    return Err(anyhow!("Cannot resume from {:?} state", self.state));
                }
                self.start_acquisition_task();
                self.state = InstrumentState::Running;
                Ok(Response::Ok)
            }

            Command::GetState => Ok(Response::State(self.state)),

            Command::GetParameter(name) => {
                let value = match name.as_str() {
                    "exposure_ms" => {
                        serde_json::to_value(self.exposure.read().await.get()).unwrap()
                    }
                    "gain" => serde_json::to_value(self.gain.read().await.get()).unwrap(),
                    "roi" => serde_json::to_value(self.roi.read().await.get()).unwrap(),
                    "binning" => serde_json::to_value(self.binning.read().await.get()).unwrap(),
                    _ => return Ok(Response::Error(format!("Unknown parameter: {}", name))),
                };
                Ok(Response::Parameter(value))
            }

            Command::SetParameter(name, value) => {
                match name.as_str() {
                    "exposure_ms" => {
                        let val: f64 = serde_json::from_value(value)?;
                        self.exposure.write().await.set(val).await?;
                    }
                    "gain" => {
                        let val: f64 = serde_json::from_value(value)?;
                        self.gain.write().await.set(val).await?;
                    }
                    "roi" => {
                        let val: Roi = serde_json::from_value(value)?;
                        self.roi.write().await.set(val).await?;
                    }
                    "binning" => {
                        let val: (u32, u32) = serde_json::from_value(value)?;
                        self.binning.write().await.set(val).await?;
                    }
                    _ => return Ok(Response::Error(format!("Unknown parameter: {}", name))),
                }
                Ok(Response::Ok)
            }

            Command::Custom(_, _) => {
                Ok(Response::Error("Custom commands not supported".to_string()))
            }
            _ => Ok(Response::Ok),
        }
    }

    fn parameters(&self) -> &HashMap<String, Box<dyn ParameterBase>> {
        &self.parameters
    }

    fn parameters_mut(&mut self) -> &mut HashMap<String, Box<dyn ParameterBase>> {
        &mut self.parameters
    }
}

#[async_trait]
impl Camera for MockCameraV3 {
    async fn set_exposure(&mut self, ms: f64) -> Result<()> {
        self.exposure.write().await.set(ms).await
    }

    async fn set_roi(&mut self, roi: Roi) -> Result<()> {
        self.roi.write().await.set(roi).await
    }

    async fn roi(&self) -> Roi {
        self.roi.read().await.get()
    }

    async fn set_binning(&mut self, h: u32, v: u32) -> Result<()> {
        self.binning.write().await.set((h, v)).await
    }

    async fn start_acquisition(&mut self) -> Result<()> {
        self.execute(Command::Start).await?;
        Ok(())
    }

    async fn stop_acquisition(&mut self) -> Result<()> {
        self.execute(Command::Stop).await?;
        Ok(())
    }

    async fn arm_trigger(&mut self) -> Result<()> {
        // Mock implementation - just prepare state
        log::info!("MockCameraV3 '{}' armed for trigger", self.id);
        Ok(())
    }

    async fn trigger(&mut self) -> Result<()> {
        // Mock implementation - generate single frame
        if self.state != InstrumentState::Running {
            return Err(anyhow!("Camera not running, cannot trigger"));
        }

        let image = self.generate_mock_image();
        let exposure = self.exposure.read().await.get();
        let gain = self.gain.read().await.get();
        let binning = self.binning.read().await.get();
        let roi_val = self.roi.read().await.get();
        let metadata = ImageMetadata {
            exposure_ms: Some(exposure),
            gain: Some(gain),
            binning: Some(binning),
            temperature_c: None,
            hardware_timestamp_us: None,
            readout_ms: None,
            roi_origin: Some((roi_val.x, roi_val.y)),
        };

        let measurement = Measurement::Image {
            name: format!("{}_frame", self.id),
            width: u32::try_from(image.width).unwrap_or(u32::MAX),
            height: u32::try_from(image.height).unwrap_or(u32::MAX),
            buffer: image.pixels,
            unit: image.unit,
            metadata,
            timestamp: image.timestamp,
        };

        self.data_tx
            .send(measurement)
            .map_err(|e| anyhow!("Failed to broadcast: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_camera_v3_initialization() {
        let mut camera = MockCameraV3::new("test_cam");
        assert_eq!(camera.state(), InstrumentState::Uninitialized);

        camera.initialize().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Idle);
    }

    #[tokio::test]
    async fn test_mock_camera_v3_parameter_setting() {
        let mut camera = MockCameraV3::new("test_cam");
        camera.initialize().await.unwrap();

        // Set exposure via Camera trait
        camera.set_exposure(250.0).await.unwrap();

        // Verify via Command interface
        let response = camera
            .execute(Command::GetParameter("exposure_ms".to_string()))
            .await
            .unwrap();

        match response {
            Response::Parameter(val) => {
                let exposure: f64 = serde_json::from_value(val).unwrap();
                assert_eq!(exposure, 250.0);
            }
            _ => panic!("Expected Parameter response"),
        }
    }

    #[tokio::test]
    async fn test_mock_camera_v3_acquisition() {
        let mut camera = MockCameraV3::new("test_cam");
        camera.initialize().await.unwrap();

        // Subscribe to data
        let mut rx = camera.data_channel();

        // Start acquisition
        camera.start_acquisition().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Running);

        // Wait for at least one frame
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("Timeout waiting for frame")
            .expect("Channel closed");

        // Stop acquisition
        camera.stop_acquisition().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Idle);
    }

    #[tokio::test]
    async fn test_mock_camera_v3_roi() {
        let mut camera = MockCameraV3::new("test_cam");
        camera.initialize().await.unwrap();

        let custom_roi = Roi {
            x: 100,
            y: 100,
            width: 512,
            height: 512,
        };

        camera.set_roi(custom_roi).await.unwrap();
        assert_eq!(camera.roi().await, custom_roi);
    }

    #[tokio::test]
    async fn test_mock_camera_v3_parameter_validation() {
        let mut camera = MockCameraV3::new("test_cam");
        camera.initialize().await.unwrap();

        // Try to set exposure out of range (should fail)
        let result = camera.set_exposure(20000.0).await;
        assert!(result.is_err());

        // Valid exposure should work
        camera.set_exposure(1000.0).await.unwrap();
    }

    #[tokio::test]
    async fn test_mock_camera_v3_shutdown() {
        let mut camera = MockCameraV3::new("test_cam");
        camera.initialize().await.unwrap();
        camera.start_acquisition().await.unwrap();

        // Shutdown should stop acquisition
        camera.shutdown().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::ShuttingDown);
    }
}
