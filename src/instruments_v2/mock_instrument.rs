//! Mock instrument implementation using new trait hierarchy
//!
//! This validates the three-tier architecture with a simple, hardware-free
//! implementation. Serves as a reference for migrating real instruments.

use async_trait::async_trait;
use daq_core::{
    Camera, DataPoint, HardwareAdapter, ImageData, Instrument, InstrumentCommand,
    InstrumentState, Measurement, MeasurementReceiver, MeasurementSender, PowerMeter, PowerRange,
    ROI, Result, arc_measurement, measurement_channel,
};
use chrono::Utc;
use tokio::task::JoinHandle;

use crate::adapters::MockAdapter;

/// Mock instrument for testing the new trait system
///
/// Demonstrates:
/// - Composition of HardwareAdapter (MockAdapter)
/// - InstrumentState state machine
/// - Arc<Measurement> zero-copy pattern
/// - Async acquisition loop
/// - Error recovery
pub struct MockInstrumentV2 {
    id: String,
    adapter: Box<dyn HardwareAdapter>,
    state: InstrumentState,

    // Configuration (for Camera trait)
    exposure_ms: f64,
    roi: ROI,
    binning: (u16, u16),
    sensor_size: (u32, u32),

    // Data streaming
    measurement_tx: MeasurementSender,
    _measurement_rx_keeper: MeasurementReceiver, // Keeps channel open

    // Task management
    task_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockInstrumentV2 {
    /// Create a new mock instrument with default MockAdapter
    pub fn new(id: String) -> Self {
        Self::with_adapter(id, Box::new(MockAdapter::new()))
    }

    /// Create a new mock instrument with custom adapter (for testing)
    pub fn with_adapter(id: String, adapter: Box<dyn HardwareAdapter>) -> Self {
        let (measurement_tx, measurement_rx) = measurement_channel(1024);

        Self {
            id,
            adapter,
            state: InstrumentState::Disconnected,

            // Default camera configuration
            exposure_ms: 100.0,
            roi: ROI::default(),
            binning: (1, 1),
            sensor_size: (512, 512),

            measurement_tx,
            _measurement_rx_keeper: measurement_rx,

            task_handle: None,
            shutdown_tx: None,
        }
    }

    /// Generate test pattern image
    fn generate_test_image(&self, width: u32, height: u32) -> ImageData {
        let pixels: Vec<f64> = (0..width * height)
            .map(|i| {
                let x = i % width;
                let y = i / width;
                ((x + y) % 256) as f64
            })
            .collect();

        ImageData {
            timestamp: Utc::now(),
            channel: format!("{}_image", self.id),
            width,
            height,
            pixels,
            unit: "counts".to_string(),
            metadata: Some(serde_json::json!({
                "exposure_ms": self.exposure_ms,
                "roi": self.roi,
                "binning": self.binning,
            })),
        }
    }
}

#[async_trait]
impl Instrument for MockInstrumentV2 {
    fn id(&self) -> &str {
        &self.id
    }

    fn instrument_type(&self) -> &str {
        "mock_v2"
    }

    fn state(&self) -> InstrumentState {
        self.state
    }


    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Disconnected {
            return Err(anyhow::anyhow!("Already initialized"));
        }

        self.state = InstrumentState::Connecting;

        // Connect hardware adapter
        match self.adapter.connect(&Default::default()).await {
            Ok(()) => {
                self.state = InstrumentState::Ready;
                log::info!("MockInstrumentV2 '{}' initialized", self.id);
                Ok(())
            }
            Err(e) => {
                self.state = InstrumentState::Error(DaqError {
                    message: e.to_string(),
                    can_recover: true,
                });
                Err(e)
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.state = InstrumentState::ShuttingDown;

        // Stop any running acquisition
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        // Disconnect hardware
        self.adapter.disconnect().await?;

        self.state = InstrumentState::Disconnected;
        log::info!("MockInstrumentV2 '{}' shut down", self.id);
        Ok(())
    }

    fn measurement_stream(&self) -> MeasurementReceiver {
        self.measurement_tx.subscribe()
    }

    async fn handle_command(&mut self, cmd: InstrumentCommand) -> Result<()> {
        if !self.can_execute(&cmd) {
            return Err(anyhow::anyhow!(
                "Cannot execute command {:?} in state {:?}",
                cmd,
                self.state
            ));
        }

        match cmd {
            InstrumentCommand::Shutdown => self.shutdown().await,
            InstrumentCommand::StartAcquisition => self.start_live().await,
            InstrumentCommand::StopAcquisition => self.stop_live().await,
            InstrumentCommand::SetParameter { name, value } => {
                match name.as_str() {
                    "exposure_ms" => {
                        if let Some(ms) = value.as_f64() {
                            self.set_exposure_ms(ms).await
                        } else {
                            Err(anyhow::anyhow!("Invalid exposure value"))
                        }
                    }
                    _ => Err(anyhow::anyhow!("Unknown parameter: {}", name)),
                }
            }
            InstrumentCommand::GetParameter { .. } => {
                // Not implemented for mock
                Ok(())
            }
            InstrumentCommand::Recover => self.recover().await,
        }
    }

    async fn recover(&mut self) -> Result<()> {
        match &self.state {
            InstrumentState::Error(daq_error) if daq_error.can_recover => {
                log::info!("Attempting recovery for '{}'", self.id);

                // Disconnect if connected
                let _ = self.adapter.disconnect().await;

                // Wait briefly
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                // Reconnect
                self.adapter.connect(&Default::default()).await?;

                self.state = InstrumentState::Ready;

                log::info!("Recovery successful for '{}'", self.id);
                Ok(())
            }
            InstrumentState::Error(_) => {
                Err(anyhow::anyhow!("Cannot recover from unrecoverable error"))
            }
            _ => Err(anyhow::anyhow!(
                "Cannot recover from state: {:?}",
                self.state
            )),
        }
    }
}

#[async_trait]
impl Camera for MockInstrumentV2 {
    async fn snap(&mut self) -> Result<ImageData> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!("Camera not ready, state: {:?}", self.state));
        }

        self.state = InstrumentState::Acquiring;

        // Simulate acquisition delay
        tokio::time::sleep(std::time::Duration::from_millis(self.exposure_ms as u64)).await;

        // Generate test image
        let width = (self.roi.width / self.binning.0) as u32;
        let height = (self.roi.height / self.binning.1) as u32;
        let image = self.generate_test_image(width, height);

        // Emit measurement
        let measurement = arc_measurement(Measurement::Image(image.clone()));
        let _ = self.measurement_tx.send(measurement);

        self.state = InstrumentState::Ready;

        Ok(image)
    }

    async fn start_live(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!(
                "Cannot start live from state: {:?}",
                self.state
            ));
        }

        self.state = InstrumentState::Acquiring;

        let tx = self.measurement_tx.clone();
        let id = self.id.clone();
        let exposure_ms = self.exposure_ms;
        let roi = self.roi;
        let binning = self.binning;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Spawn continuous acquisition task
        self.task_handle = Some(tokio::spawn(async move {
            let width = (roi.width / binning.0) as u32;
            let height = (roi.height / binning.1) as u32;

            loop {
                tokio::select! {
                    // Simulate frame acquisition
                    _ = tokio::time::sleep(std::time::Duration::from_millis(exposure_ms as u64)) => {
                        // Generate test pattern
                        let pixels: Vec<f64> = (0..width * height)
                            .map(|i| {
                                let x = i % width;
                                let y = i / width;
                                ((x + y + (Utc::now().timestamp() % 256) as u32) % 256) as f64
                            })
                            .collect();

                        let image = ImageData {
                            timestamp: Utc::now(),
                            channel: format!("{}_image", id),
                            width,
                            height,
                            pixels,
                            unit: "counts".to_string(),
                            metadata: Some(serde_json::json!({
                                "exposure_ms": exposure_ms,
                                "roi": roi,
                                "binning": binning,
                            })),
                        };

                        let measurement = arc_measurement(Measurement::Image(image));

                        if tx.send(measurement).is_err() {
                            log::info!("No receivers, stopping acquisition");
                            break;
                        }
                    }
                    // Shutdown signal
                    _ = &mut shutdown_rx => {
                        log::info!("Acquisition shutdown requested");
                        break;
                    }
                }
            }
        }));

        log::info!("Started live acquisition for '{}'", self.id);
        Ok(())
    }

    async fn stop_live(&mut self) -> Result<()> {
        if self.state != InstrumentState::Acquiring {
            return Ok(()); // Already stopped
        }

        // Signal shutdown
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Wait for task to finish
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        self.state = InstrumentState::Ready;
        log::info!("Stopped live acquisition for '{}'", self.id);
        Ok(())
    }

    async fn set_exposure_ms(&mut self, ms: f64) -> Result<()> {
        if self.state == InstrumentState::Acquiring {
            return Err(anyhow::anyhow!("Cannot change exposure while acquiring"));
        }
        self.exposure_ms = ms;
        Ok(())
    }

    async fn get_exposure_ms(&self) -> f64 {
        self.exposure_ms
    }

    async fn set_roi(&mut self, roi: ROI) -> Result<()> {
        if self.state == InstrumentState::Acquiring {
            return Err(anyhow::anyhow!("Cannot change ROI while acquiring"));
        }
        self.roi = roi;
        Ok(())
    }

    async fn get_roi(&self) -> ROI {
        self.roi
    }

    async fn set_binning(&mut self, x: u16, y: u16) -> Result<()> {
        if self.state == InstrumentState::Acquiring {
            return Err(anyhow::anyhow!("Cannot change binning while acquiring"));
        }
        self.binning = (x, y);
        Ok(())
    }

    async fn get_binning(&self) -> (u16, u16) {
        self.binning
    }

    fn get_sensor_size(&self) -> (u32, u32) {
        self.sensor_size
    }

    fn get_pixel_size_um(&self) -> (f64, f64) {
        (6.5, 6.5) // Typical for scientific cameras
    }

    fn supports_hardware_trigger(&self) -> bool {
        false // Mock doesn't support hardware trigger
    }
}

#[async_trait]
impl PowerMeter for MockInstrumentV2 {
    async fn read_power(&mut self) -> Result<f64> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!("PowerMeter not ready"));
        }

        // Simulate reading
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Generate simulated power reading
        let power = 1.0 + (Utc::now().timestamp() % 100) as f64 / 1000.0;

        // Emit measurement
        let data = DataPoint {
            timestamp: Utc::now(),
            channel: format!("{}_power", self.id),
            value: power,
            unit: "W".to_string(),
        };
        let measurement = arc_measurement(Measurement::Scalar(data));
        let _ = self.measurement_tx.send(measurement);

        Ok(power)
    }

    async fn set_wavelength_nm(&mut self, _nm: f64) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!("Cannot set wavelength in current state"));
        }
        Ok(())
    }

    async fn set_range(&mut self, _range: PowerRange) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!("Cannot set range in current state"));
        }
        Ok(())
    }

    async fn zero(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!("Cannot zero in current state"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_mock_instrument_lifecycle() {
        let mut instrument = MockInstrumentV2::new("test".to_string());

        // Initial state
        assert_eq!(instrument.state(), InstrumentState::Disconnected);

        // Initialize
        instrument.initialize().await.unwrap();
        assert_eq!(instrument.state(), InstrumentState::Ready);

        // Shutdown
        instrument.shutdown().await.unwrap();
        assert_eq!(instrument.state(), InstrumentState::Disconnected);
    }

    #[tokio::test]
    async fn test_camera_snap() {
        let mut camera = MockInstrumentV2::new("test_camera".to_string());
        camera.initialize().await.unwrap();

        let image = camera.snap().await.unwrap();

        assert_eq!(image.width, 512);
        assert_eq!(image.height, 512);
        assert_eq!(image.pixels.len(), 512 * 512);
    }

    #[tokio::test]
    async fn test_camera_live_acquisition() {
        let mut camera = MockInstrumentV2::new("test_camera".to_string());
        camera.initialize().await.unwrap();

        let mut rx = camera.measurement_stream();

        // Start live
        camera.start_live().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Acquiring);

        // Receive a few frames
        for _ in 0..3 {
            let measurement = rx.recv().await.unwrap();
            if let Measurement::Image(image) = &*measurement {
                assert_eq!(image.width, 512);
                assert_eq!(image.height, 512);
            } else {
                panic!("Expected Image measurement");
            }
        }

        // Stop live
        camera.stop_live().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Ready);
    }

    #[tokio::test]
    async fn test_state_machine_transitions() {
        let mut instrument = MockInstrumentV2::new("test".to_string());

        // Cannot snap when disconnected
        assert!(instrument.snap().await.is_err());

        // Initialize
        instrument.initialize().await.unwrap();

        // Can snap when ready
        assert!(instrument.snap().await.is_ok());

        // Start live
        instrument.start_live().await.unwrap();

        // Cannot change exposure while acquiring
        assert!(instrument.set_exposure_ms(200.0).await.is_err());

        // Stop live
        instrument.stop_live().await.unwrap();

        // Can change exposure when ready
        assert!(instrument.set_exposure_ms(200.0).await.is_ok());
    }

    #[tokio::test]
    async fn test_error_recovery() {
        // Create instrument with custom adapter that will fail
        let mut adapter = MockAdapter::new();
        adapter.trigger_failure();
        let mut instrument =
            MockInstrumentV2::with_adapter("test".to_string(), Box::new(adapter));

        // Trigger failure during initialization
        assert!(instrument.initialize().await.is_err());

        if let InstrumentState::Error(err) = instrument.state() {
            assert!(err.can_recover);
            assert!(err.message.contains("Adapter forced to fail"));
        } else {
            panic!("Expected Error state, got {:?}", instrument.state());
        }

        // Recover
        instrument.recover().await.unwrap();
        assert_eq!(instrument.state(), InstrumentState::Ready);
    }

    #[tokio::test]
    async fn test_power_meter_trait() {
        let mut power_meter = MockInstrumentV2::new("test_pm".to_string());
        power_meter.initialize().await.unwrap();

        let power = power_meter.read_power().await.unwrap();
        assert!(power > 0.0);
        assert!(power < 2.0); // Reasonable range for simulated data
    }

    #[tokio::test]
    async fn test_arc_measurement_zero_copy() {
        let camera = MockInstrumentV2::new("test_camera".to_string());

        let mut rx1 = camera.measurement_stream();
        let mut rx2 = camera.measurement_stream();

        let mut camera = camera;
        camera.initialize().await.unwrap();
        camera.snap().await.unwrap();

        let m1 = rx1.recv().await.unwrap();
        let m2 = rx2.recv().await.unwrap();

        // Both should point to same Arc
        assert!(Arc::ptr_eq(&m1, &m2));
    }
}
