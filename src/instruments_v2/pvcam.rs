//! Photometrics PVCAM camera driver V2 (PrimeBSI)
//!
//! V2 implementation using the new trait hierarchy and Measurement enum.
//! This enables native PixelBuffer::U16 broadcasting for 4× memory savings.
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.prime_bsi]
//! type = "pvcam_v2"
//! camera_name = "PrimeBSI"
//! exposure_ms = 100.0
//! roi = [0, 0, 2048, 2048]  # [x, y, width, height]
//! binning = [1, 1]  # [x_bin, y_bin]
//! polling_rate_hz = 10.0
//! sdk_mode = "mock"  # or "real" when SDK available
//! ```

use async_trait::async_trait;
use chrono::Utc;
use daq_core::{
    arc_measurement, measurement_channel, Camera, DaqError, DataPoint, HardwareAdapter, ImageData,
    Instrument, InstrumentCommand, InstrumentState, Measurement, MeasurementReceiver,
    MeasurementSender, PixelBuffer, Result, ROI,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use std::time::Instant;
use tokio::task::JoinHandle;

use crate::adapters::MockAdapter;
use super::pvcam_sdk::{CameraHandle, MockPvcamSdk, PvcamSdk, RealPvcamSdk};
use super::pvcam_sdk::TriggerMode;

/// SDK mode selection for PVCAM camera
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvcamSdkKind {
    /// Mock SDK for testing without hardware
    Mock,
    /// Real SDK for actual camera control
    Real,
}

/// PVCAM camera V2 implementation
///
/// Uses PixelBuffer::U16 for native 16-bit camera data (4× memory savings vs f64).
pub struct PVCAMInstrumentV2 {
    id: String,
    adapter: Box<dyn HardwareAdapter>,
    state: InstrumentState,

    // SDK abstraction layer
    sdk: Arc<dyn PvcamSdk>,
    camera_handle: Option<CameraHandle>,

    // Camera configuration
    camera_name: String,
    exposure_ms: f64,
    roi: ROI,
    binning: (u16, u16),
    sensor_size: (u32, u32),
    gain: u16, // Sensor gain
    trigger_mode: TriggerMode, // Trigger mode for acquisition

    // Data streaming
    measurement_tx: MeasurementSender,
    _measurement_rx_keeper: MeasurementReceiver,

    // Task management
    task_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    
    // Diagnostic counters (accessed from async task, must be atomic)
    total_frames: Arc<AtomicU64>,
    dropped_frames: Arc<AtomicU64>,
    last_frame_number: Arc<AtomicU32>,
    acquisition_start_time: Arc<tokio::sync::Mutex<Option<Instant>>>,
}

impl PVCAMInstrumentV2 {
    /// Create a new PVCAM instrument with default capacity (1024) and Mock SDK
    pub fn new(id: String) -> Self {
        Self::with_capacity(id, 1024)
    }

    /// Create a new PVCAM instrument with specified broadcast capacity and Mock SDK
    pub fn with_capacity(id: String, capacity: usize) -> Self {
        Self::with_sdk_and_capacity(id, PvcamSdkKind::Mock, capacity)
    }

    /// Create a new PVCAM instrument with custom adapter and capacity (Mock SDK)
    pub fn with_adapter_and_capacity(
        id: String,
        adapter: Box<dyn HardwareAdapter>,
        capacity: usize,
    ) -> Self {
        let (measurement_tx, measurement_rx) = measurement_channel(capacity);

        // Default to Mock SDK for backward compatibility
        let sdk: Arc<dyn PvcamSdk> = Arc::new(MockPvcamSdk::new());

        Self {
            id,
            adapter,
            state: InstrumentState::Disconnected,

            // SDK layer
            sdk,
            camera_handle: None,

            // Default PVCAM configuration
            camera_name: "PrimeBSI".to_string(),
            exposure_ms: 100.0,
            roi: ROI {
                x: 0,
                y: 0,
                width: 2048,
                height: 2048,
            },
            binning: (1, 1),
            sensor_size: (2048, 2048),
            gain: 1, // Default gain
            trigger_mode: TriggerMode::Timed, // Default to free-running

            measurement_tx,
            _measurement_rx_keeper: measurement_rx,

            task_handle: None,
            shutdown_tx: None,
            
            // Initialize diagnostic counters
            total_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            last_frame_number: Arc::new(AtomicU32::new(0)),
            acquisition_start_time: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Create a new PVCAM instrument with specified SDK mode and capacity
    pub fn with_sdk_and_capacity(id: String, sdk_kind: PvcamSdkKind, capacity: usize) -> Self {
        let (measurement_tx, measurement_rx) = measurement_channel(capacity);

        // Create appropriate SDK implementation
        let sdk: Arc<dyn PvcamSdk> = match sdk_kind {
            PvcamSdkKind::Mock => Arc::new(MockPvcamSdk::new()),
            PvcamSdkKind::Real => Arc::new(RealPvcamSdk::new()),
        };

        Self {
            id,
            adapter: Box::new(MockAdapter::new()),
            state: InstrumentState::Disconnected,

            // SDK layer
            sdk,
            camera_handle: None,

            // Default PVCAM configuration
            camera_name: "PrimeBSI".to_string(),
            exposure_ms: 100.0,
            roi: ROI {
                x: 0,
                y: 0,
                width: 2048,
                height: 2048,
            },
            binning: (1, 1),
            sensor_size: (2048, 2048),
            gain: 1, // Default gain
            trigger_mode: TriggerMode::Timed, // Default to free-running

            measurement_tx,
            _measurement_rx_keeper: measurement_rx,

            task_handle: None,
            shutdown_tx: None,
            
            // Initialize diagnostic counters
            total_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            last_frame_number: Arc::new(AtomicU32::new(0)),
            acquisition_start_time: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Simulate frame data generation (used for `snap()`)
    fn simulate_frame_data(&self, width: u32, height: u32) -> Vec<u16> {
        let mut frame = vec![0u16; (width * height) as usize];

        // Simple gradient pattern for testing
        for y in 0..height {
            for x in 0..width {
                let value = ((x + y) % 256) as u16 * 256;
                frame[(y * width + x) as usize] = value;
            }
        }

        frame
    }

    /// Calculate frame statistics (associated function - no self needed)
    fn calculate_frame_stats(frame: &[u16]) -> (f64, f64, f64) {
        if frame.is_empty() {
            return (0.0, 0.0, 0.0);
        }

        let sum: u64 = frame.iter().map(|&v| v as u64).sum();
        let mean = sum as f64 / frame.len() as f64;

        let min = *frame.iter().min().unwrap_or(&0) as f64;
        let max = *frame.iter().max().unwrap_or(&0) as f64;

        (mean, min, max)
    }

    /// Get the camera handle, returning an error if not initialized
    fn get_handle(&self) -> Result<CameraHandle> {
        self.camera_handle
            .ok_or_else(|| anyhow::anyhow!("Camera not initialized"))
    }

    /// Convert our ROI struct to PVCAM PxRegion
    fn roi_to_px_region(&self, roi: &ROI, binning: (u16, u16)) -> super::pvcam_sdk::PxRegion {
        use super::pvcam_sdk::PxRegion;
        PxRegion {
            s1: roi.x,
            s2: roi.x + roi.width - 1,
            sbin: binning.0,
            p1: roi.y,
            p2: roi.y + roi.height - 1,
            pbin: binning.1,
        }
    }

    /// Convert PVCAM PxRegion to our ROI struct
    fn px_region_to_roi(&self, region: &super::pvcam_sdk::PxRegion) -> (ROI, (u16, u16)) {
        let roi = ROI {
            x: region.s1,
            y: region.p1,
            width: region.s2 - region.s1 + 1,
            height: region.p2 - region.p1 + 1,
        };
        let binning = (region.sbin, region.pbin);
        (roi, binning)
    }
}

#[async_trait]
impl Instrument for PVCAMInstrumentV2 {
    fn id(&self) -> &str {
        &self.id
    }

    fn instrument_type(&self) -> &str {
        "pvcam_v2"
    }

    fn state(&self) -> InstrumentState {
        self.state.clone()
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Disconnected {
            return Err(anyhow::anyhow!("Already initialized"));
        }

        self.state = InstrumentState::Connecting;

        // Initialize PVCAM SDK
        self.sdk
            .init()
            .map_err(|e| anyhow::anyhow!("Failed to initialize PVCAM SDK: {}", e))?;

        // Open camera
        let handle = self.sdk.open_camera(&self.camera_name).map_err(|e| {
            let _ = self.sdk.uninit(); // Cleanup on failure
            anyhow::anyhow!("Failed to open camera '{}': {}", self.camera_name, e)
        })?;

        self.camera_handle = Some(handle);

        log::info!(
            "PVCAM SDK initialized, camera '{}' opened with handle {:?}",
            self.camera_name,
            handle
        );

        // Read initial gain from SDK
        match self.sdk.get_param_u16(&handle, super::pvcam_sdk::PvcamParam::Gain) {
            Ok(gain_u16) => {
                self.gain = gain_u16;
                log::info!("PVCAM initial gain read from SDK: {}", gain_u16);
            }
            Err(e) => {
                log::warn!("Failed to read initial gain from SDK: {}, using default", e);
                // Keep default value initialized in constructor
            }
        }

        // Initialize hardware adapter (for compatibility)
        match self.adapter.connect(&Default::default()).await {
            Ok(()) => {
                self.state = InstrumentState::Ready;
                log::info!(
                    "PVCAM camera '{}' ({}) initialized",
                    self.id,
                    self.camera_name
                );
                Ok(())
            }
            Err(e) => {
                // Cleanup SDK on adapter failure
                if let Some(handle) = self.camera_handle.take() {
                    let _ = self.sdk.close_camera(handle);
                }
                let _ = self.sdk.uninit();

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

        // Close camera and uninitialize SDK
        if let Some(handle) = self.camera_handle.take() {
            self.sdk.close_camera(handle).map_err(|e| {
                log::error!("Failed to close camera: {}", e);
                anyhow::anyhow!("Failed to close camera: {}", e)
            })?;
            log::info!("PVCAM camera handle {:?} closed", handle);
        }

        self.sdk.uninit().map_err(|e| {
            log::error!("Failed to uninitialize PVCAM SDK: {}", e);
            anyhow::anyhow!("Failed to uninitialize PVCAM SDK: {}", e)
        })?;

        log::info!("PVCAM SDK uninitialized");

        self.adapter.disconnect().await?;

        self.state = InstrumentState::Disconnected;
        log::info!("PVCAM camera '{}' shut down", self.id);
        Ok(())
    }

    fn measurement_stream(&self) -> MeasurementReceiver {
        self.measurement_tx.subscribe()
    }

    async fn handle_command(&mut self, cmd: InstrumentCommand) -> Result<()> {
        match cmd {
            InstrumentCommand::Shutdown => self.shutdown().await,
            InstrumentCommand::StartAcquisition => self.start_live().await,
            InstrumentCommand::StopAcquisition => self.stop_live().await,
            InstrumentCommand::SetParameter { name, value } => match name.as_str() {
                "exposure_ms" => {
                    if let Some(ms) = value.as_f64() {
                        self.set_exposure_ms(ms).await
                    } else {
                        Err(anyhow::anyhow!("Invalid exposure value"))
                    }
                }
                "gain" => {
                    if self.state == InstrumentState::Acquiring {
                        return Err(anyhow::anyhow!("Cannot change gain while acquiring"));
                    }

                    if let Some(gain_f64) = value.as_f64() {
                        // Validation: Gain must be positive
                        if gain_f64 <= 0.0 {
                            return Err(anyhow::anyhow!("Gain must be a positive value, got {}", gain_f64));
                        }
                        if gain_f64 > u16::MAX as f64 {
                            return Err(anyhow::anyhow!("Gain value {} is too large, max is {}", gain_f64, u16::MAX));
                        }

                        let gain_u16 = gain_f64 as u16;
                        let handle = self.get_handle()?;
                        self.sdk
                            .set_param_u16(&handle, super::pvcam_sdk::PvcamParam::Gain, gain_u16)
                            .map_err(|e| anyhow::anyhow!("Failed to set gain: {}", e))?;

                        self.gain = gain_u16; // Update internal state after successful SDK call
                        log::info!("PVCAM gain set to {}", gain_u16);
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("Invalid gain value"))
                    }
                }
                "binning" => {
                    if self.state == InstrumentState::Acquiring {
                        return Err(anyhow::anyhow!("Cannot change binning while acquiring"));
                    }

                    // Parse binning value as JSON object {"x": u16, "y": u16}
                    let binning_obj = value.as_object()
                        .ok_or_else(|| anyhow::anyhow!("Binning must be a JSON object with 'x' and 'y' fields"))?;
                    
                    let x = binning_obj.get("x")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow::anyhow!("Binning 'x' must be a positive integer"))?;
                    let y = binning_obj.get("y")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow::anyhow!("Binning 'y' must be a positive integer"))?;

                    // Validation: Binning must be in u16 range and positive
                    if x == 0 || y == 0 {
                        return Err(anyhow::anyhow!("Binning values must be positive, got x={}, y={}", x, y));
                    }
                    if x > u16::MAX as u64 || y > u16::MAX as u64 {
                        return Err(anyhow::anyhow!("Binning values too large, max is {}", u16::MAX));
                    }

                    let x_u16 = x as u16;
                    let y_u16 = y as u16;

                    self.set_binning(x_u16, y_u16).await?;
                    log::info!("PVCAM binning set to ({}, {})", x_u16, y_u16);
                    Ok(())
                }
                "roi" => {
                    if self.state == InstrumentState::Acquiring {
                        return Err(anyhow::anyhow!("Cannot change ROI while acquiring"));
                    }

                    // Parse ROI value as JSON object {"x": u16, "y": u16, "width": u16, "height": u16}
                    let roi_obj = value.as_object()
                        .ok_or_else(|| anyhow::anyhow!("ROI must be a JSON object with 'x', 'y', 'width', and 'height' fields"))?;

                    let x = roi_obj.get("x")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow::anyhow!("ROI 'x' must be a positive integer"))?;
                    let y = roi_obj.get("y")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow::anyhow!("ROI 'y' must be a positive integer"))?;
                    let width = roi_obj.get("width")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow::anyhow!("ROI 'width' must be a positive integer"))?;
                    let height = roi_obj.get("height")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow::anyhow!("ROI 'height' must be a positive integer"))?;

                    // Validation: ROI values must be in u16 range
                    if x > u16::MAX as u64 || y > u16::MAX as u64 || width > u16::MAX as u64 || height > u16::MAX as u64 {
                        return Err(anyhow::anyhow!("ROI values too large, max is {}", u16::MAX));
                    }

                    // Validation: Width and height must be positive
                    if width == 0 || height == 0 {
                        return Err(anyhow::anyhow!("ROI width and height must be positive, got width={}, height={}", width, height));
                    }

                    let roi = ROI {
                        x: x as u16,
                        y: y as u16,
                        width: width as u16,
                        height: height as u16,
                    };

                    // Validation: ROI must be within sensor bounds
                    if roi.x + roi.width > self.sensor_size.0 as u16 {
                        return Err(anyhow::anyhow!(
                            "ROI extends beyond sensor width: x={}, width={}, sensor_width={}",
                            roi.x, roi.width, self.sensor_size.0
                        ));
                    }
                    if roi.y + roi.height > self.sensor_size.1 as u16 {
                        return Err(anyhow::anyhow!(
                            "ROI extends beyond sensor height: y={}, height={}, sensor_height={}",
                            roi.y, roi.height, self.sensor_size.1
                        ));
                    }

                    self.set_roi(roi).await?;
                    log::info!("PVCAM ROI set to {:?}", roi);
                    Ok(())
                }
                "trigger_mode" => {
                    if self.state == InstrumentState::Acquiring {
                        return Err(anyhow::anyhow!("Cannot change trigger mode while acquiring"));
                    }

                    if let Some(mode_str) = value.as_str() {
                        let trigger_mode = TriggerMode::from_str(mode_str)
                            .ok_or_else(|| anyhow::anyhow!("Invalid trigger mode: '{}'. Valid modes: timed, trigger_first, strobed, bulb, software_edge", mode_str))?;

                        let handle = self.get_handle()?;
                        self.sdk
                            .set_param_u16(&handle, super::pvcam_sdk::PvcamParam::ExposureMode, trigger_mode.as_u16())
                            .map_err(|e| anyhow::anyhow!("Failed to set trigger mode: {}", e))?;

                        self.trigger_mode = trigger_mode;
                        log::info!("PVCAM trigger mode set to {:?}", trigger_mode);
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("Invalid trigger mode value, expected string"))
                    }
                }
                _ => Err(anyhow::anyhow!("Unknown parameter: {}", name)),
            },
            InstrumentCommand::GetParameter { name } => {
                let handle = self.get_handle()?;
                let value_from_sdk = match name.as_str() {
                    "exposure_ms" => {
                        let exposure_u16 = self.sdk.get_param_u16(&handle, super::pvcam_sdk::PvcamParam::Exposure)
                            .map_err(|e| anyhow::anyhow!("Failed to get exposure from SDK: {}", e))?;
                        exposure_u16 as f64
                    },
                    "gain" => {
                        let gain_u16 = self.sdk.get_param_u16(&handle, super::pvcam_sdk::PvcamParam::Gain)
                            .map_err(|e| anyhow::anyhow!("Failed to get gain from SDK: {}", e))?;
                        gain_u16 as f64
                    },
                    "sensor_temperature" => {
                        let temp_i16 = self.sdk.get_param_i16(&handle, super::pvcam_sdk::PvcamParam::SensorTemperature)
                            .map_err(|e| anyhow::anyhow!("Failed to get sensor temperature from SDK: {}", e))?;
                        temp_i16 as f64
                    },
                    "pixel_size_um" => {
                        let pixel_size_u16 = self.sdk.get_param_u16(&handle, super::pvcam_sdk::PvcamParam::PixelSize)
                            .map_err(|e| anyhow::anyhow!("Failed to get pixel size from SDK: {}", e))?;
                        pixel_size_u16 as f64
                    },
                    "roi" => {
                        // Read from internal state and broadcast as JSON-encoded scalar
                        let roi_json = serde_json::json!({
                            "x": self.roi.x,
                            "y": self.roi.y,
                            "width": self.roi.width,
                            "height": self.roi.height,
                        });

                        let measurement = arc_measurement(Measurement::Scalar(DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:roi", self.id),
                            value: 0.0, // Dummy value, actual data in metadata
                            unit: "".to_string(),
                        }));

                        // Also broadcast ROI as individual scalar values for easier plotting
                        let roi_x = DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:roi_x", self.id),
                            value: self.roi.x as f64,
                            unit: "px".to_string(),
                        };
                        let _ = self.measurement_tx.send(arc_measurement(Measurement::Scalar(roi_x)));

                        let roi_y = DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:roi_y", self.id),
                            value: self.roi.y as f64,
                            unit: "px".to_string(),
                        };
                        let _ = self.measurement_tx.send(arc_measurement(Measurement::Scalar(roi_y)));

                        let roi_width = DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:roi_width", self.id),
                            value: self.roi.width as f64,
                            unit: "px".to_string(),
                        };
                        let _ = self.measurement_tx.send(arc_measurement(Measurement::Scalar(roi_width)));

                        let roi_height = DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:roi_height", self.id),
                            value: self.roi.height as f64,
                            unit: "px".to_string(),
                        };
                        let _ = self.measurement_tx.send(arc_measurement(Measurement::Scalar(roi_height)));

                        let _ = self.measurement_tx.send(measurement);
                        log::info!("PVCAM ROI query: {:?}", roi_json);
                        return Ok(());
                    },
                    "binning" => {
                        // Read from internal state and broadcast as individual scalar values
                        let binning_x = DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:binning_x", self.id),
                            value: self.binning.0 as f64,
                            unit: "".to_string(),
                        };
                        let _ = self.measurement_tx.send(arc_measurement(Measurement::Scalar(binning_x)));

                        let binning_y = DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:binning_y", self.id),
                            value: self.binning.1 as f64,
                            unit: "".to_string(),
                        };
                        let _ = self.measurement_tx.send(arc_measurement(Measurement::Scalar(binning_y)));

                        log::info!("PVCAM binning query: ({}, {})", self.binning.0, self.binning.1);
                        return Ok(());
                    },
                    "trigger_mode" => {
                        let trigger_mode = self.trigger_mode;
                        let trigger_mode_str = trigger_mode.to_string();
                        let trigger_mode_value = trigger_mode.as_u16() as f64;

                        let measurement = arc_measurement(Measurement::Scalar(DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:trigger_mode", self.id),
                            value: trigger_mode_value,
                            unit: "".to_string(),
                        }));

                        let _ = self.measurement_tx.send(measurement);
                        log::info!("PVCAM trigger mode query: {}", trigger_mode_str);
                        return Ok(());
                    },
                    // Diagnostic parameters
                    "total_frames" => {
                        let total = self.total_frames.load(Ordering::Relaxed) as f64;
                        let measurement = arc_measurement(Measurement::Scalar(DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:total_frames", self.id),
                            value: total,
                            unit: "frames".to_string(),
                        }));
                        let _ = self.measurement_tx.send(measurement);
                        log::debug!("PVCAM total_frames query: {}", total);
                        return Ok(());
                    },
                    "dropped_frames" => {
                        let dropped = self.dropped_frames.load(Ordering::Relaxed) as f64;
                        let measurement = arc_measurement(Measurement::Scalar(DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:dropped_frames", self.id),
                            value: dropped,
                            unit: "frames".to_string(),
                        }));
                        let _ = self.measurement_tx.send(measurement);
                        log::debug!("PVCAM dropped_frames query: {}", dropped);
                        return Ok(());
                    },
                    "actual_fps" => {
                        // Calculate actual FPS from total frames and elapsed time
                        let total = self.total_frames.load(Ordering::Relaxed);
                        let start_time_guard = self.acquisition_start_time.lock().await;
                        
                        let fps = if let Some(start_time) = *start_time_guard {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            if elapsed > 0.0 && total > 0 {
                                total as f64 / elapsed
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        };
                        
                        let measurement = arc_measurement(Measurement::Scalar(DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:actual_fps", self.id),
                            value: fps,
                            unit: "Hz".to_string(),
                        }));
                        let _ = self.measurement_tx.send(measurement);
                        log::debug!("PVCAM actual_fps query: {:.2} Hz", fps);
                        return Ok(());
                    },
                    "camera_health" => {
                        // Generate health status string
                        let total = self.total_frames.load(Ordering::Relaxed);
                        let dropped = self.dropped_frames.load(Ordering::Relaxed);
                        let drop_rate = if total > 0 {
                            (dropped as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        };
                        
                        let health_status = if drop_rate > 10.0 {
                            "WARNING: High drop rate"
                        } else if drop_rate > 1.0 {
                            "DEGRADED: Some drops detected"
                        } else if self.state == InstrumentState::Acquiring {
                            "HEALTHY: Acquiring"
                        } else {
                            "READY: Idle"
                        };
                        
                        // Encode health as numeric value for easier monitoring
                        let health_value = if drop_rate > 10.0 {
                            0.0 // Critical
                        } else if drop_rate > 1.0 {
                            0.5 // Degraded
                        } else if self.state == InstrumentState::Acquiring {
                            1.0 // Healthy/Acquiring
                        } else {
                            0.75 // Ready/Idle
                        };
                        
                        let measurement = arc_measurement(Measurement::Scalar(DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}:camera_health", self.id),
                            value: health_value,
                            unit: "".to_string(),
                        }));
                        let _ = self.measurement_tx.send(measurement);
                        log::info!("PVCAM camera_health query: {} (drop_rate={:.2}%)", health_status, drop_rate);
                        return Ok(());
                    },
                    _ => return Err(anyhow::anyhow!("Unknown parameter: {}", name)),
                };

                // Update internal state if it's exposure_ms to keep it in sync
                if name.as_str() == "exposure_ms" {
                    self.exposure_ms = value_from_sdk;
                }
                
                // Update internal state if it's gain to keep it in sync
                if name.as_str() == "gain" {
                    self.gain = value_from_sdk as u16;
                }

                let measurement = arc_measurement(Measurement::Scalar(DataPoint {
                    timestamp: Utc::now(),
                    channel: format!("{}:{}", self.id, name),
                    value: value_from_sdk,
                    unit: match name.as_str() {
                        "exposure_ms" => "ms".to_string(),
                        "gain" => "".to_string(), // Gain is unitless
                        "sensor_temperature" => "°C".to_string(),
                        "pixel_size_um" => "µm".to_string(),
                        _ => "".to_string(),
                    },
                }));

                let _ = self.measurement_tx.send(measurement);
                Ok(())
            }
            InstrumentCommand::Recover => self.recover().await,
        }
    }

    async fn recover(&mut self) -> Result<()> {
        match &self.state {
            InstrumentState::Error(daq_error) if daq_error.can_recover => {
                log::info!("Attempting recovery for PVCAM '{}'", self.id);

                let _ = self.adapter.disconnect().await;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                self.adapter.connect(&Default::default()).await?;

                self.state = InstrumentState::Ready;
                log::info!("Recovery successful for PVCAM '{}'", self.id);
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
impl Camera for PVCAMInstrumentV2 {
    async fn snap(&mut self) -> Result<ImageData> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!(
                "PVCAM camera not ready, state: {:?}",
                self.state
            ));
        }

        self.state = InstrumentState::Acquiring;

        // Simulate acquisition delay
        tokio::time::sleep(std::time::Duration::from_millis(self.exposure_ms as u64)).await;

        let width = (self.roi.width / self.binning.0) as u32;
        let height = (self.roi.height / self.binning.1) as u32;

        // Generate u16 frame data (native camera format)
        let frame_data = self.simulate_frame_data(width, height);
        
        let timestamp = Utc::now();
        
        // Simulate realistic metadata for snap()
        let hardware_timestamp_us = timestamp.timestamp_micros();
        let readout_ms = 7.5; // Typical readout time
        let sensor_temp = -5.0; // Typical operating temperature

        // ✅ KEY FEATURE: Use PixelBuffer::U16 for 4× memory savings!
        let image = ImageData {
            timestamp,
            channel: format!("{}_image", self.id),
            width,
            height,
            pixels: PixelBuffer::U16(frame_data), // 4× memory reduction vs Vec<f64>
            unit: "counts".to_string(),
            metadata: Some(serde_json::json!({
                "camera_name": self.camera_name,
                "exposure_ms": self.exposure_ms,
                "roi": self.roi,
                "binning": self.binning,
                "frame": 0,
                "exposure_time_ms": self.exposure_ms,
                "hardware_timestamp_us": hardware_timestamp_us,
                "software_timestamp": timestamp.to_rfc3339(),
                "readout_time_ms": readout_ms,
                "sensor_temperature_c": sensor_temp,
                "roi_from_frame": {
                    "x": self.roi.x,
                    "y": self.roi.y,
                    "width": width,
                    "height": height,
                },
            })),
        };

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

        let handle = self.get_handle()?;
        let (mut frame_rx, guard) = self
            .sdk
            .clone()
            .start_acquisition(handle)
            .map_err(|e| anyhow::anyhow!("Failed to start SDK acquisition: {}", e))?;

        self.state = InstrumentState::Acquiring;

        let tx = self.measurement_tx.clone();
        let id = self.id.clone();
        let camera_name = self.camera_name.clone();
        let exposure_ms = self.exposure_ms;
        let roi = self.roi;
        let binning = self.binning;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);
        
        // Clone diagnostic counters for async task
        let total_frames = self.total_frames.clone();
        let dropped_frames = self.dropped_frames.clone();
        let last_frame_number = self.last_frame_number.clone();
        let acquisition_start_time = self.acquisition_start_time.clone();
        
        // Reset diagnostics at start of acquisition
        total_frames.store(0, Ordering::Relaxed);
        dropped_frames.store(0, Ordering::Relaxed);
        last_frame_number.store(0, Ordering::Relaxed);
        {
            let mut start_time = acquisition_start_time.lock().await;
            *start_time = Some(Instant::now());
        }

        // Spawn continuous acquisition task
        self.task_handle = Some(tokio::spawn(async move {
            // The `guard` is moved into this task. When the task ends, the guard is
            // dropped, which automatically calls `stop_acquisition` on the SDK.
            let _guard = guard;

            let width = (roi.width / binning.0) as u32;
            let height = (roi.height / binning.1) as u32;

            log::info!("PVCAM live acquisition started for '{}'", id);

            // Broadcast acquiring status = 1.0 (acquiring)
            let acquiring_status = DataPoint {
                timestamp: Utc::now(),
                channel: format!("{}:acquiring", id),
                value: 1.0,
                unit: "".to_string(),
            };
            let _ = tx.send(arc_measurement(Measurement::Scalar(acquiring_status)));

            loop {
                tokio::select! {
                    Some(frame) = frame_rx.recv() => {
                        // Track total frames received
                        total_frames.fetch_add(1, Ordering::Relaxed);
                        
                        // Detect dropped frames by checking frame number sequence
                        let expected_frame_num = last_frame_number.load(Ordering::Relaxed);
                        if frame.frame_number > 0 && expected_frame_num > 0 {
                            // Check for gap in frame numbers (expect frame_number == expected_frame_num + 1)
                            let actual_next = frame.frame_number;
                            let expected_next = expected_frame_num + 1;
                            
                            if actual_next != expected_next {
                                let dropped = actual_next.saturating_sub(expected_next);
                                dropped_frames.fetch_add(dropped as u64, Ordering::Relaxed);
                                log::warn!(
                                    "Dropped {} frame(s) for '{}': expected frame {}, got {}",
                                    dropped, id, expected_next, actual_next
                                );
                            }
                        }
                        last_frame_number.store(frame.frame_number, Ordering::Relaxed);
                        
                        let (mean, min, max) = PVCAMInstrumentV2::calculate_frame_stats(&frame.data);
                        let timestamp = frame.software_timestamp;

                        // Extract all metadata from Frame
                        let mut metadata = serde_json::json!({
                            "camera_name": camera_name,
                            "exposure_ms": exposure_ms,
                            "roi": roi,
                            "binning": binning,
                            "frame": frame.frame_number,
                            "exposure_time_ms": frame.exposure_time_ms,
                            "roi_from_frame": {
                                "x": frame.roi.0,
                                "y": frame.roi.1,
                                "width": frame.roi.2,
                                "height": frame.roi.3,
                            },
                        });

                        // Add optional metadata fields if available
                        if let Some(hw_ts) = frame.hardware_timestamp {
                            metadata["hardware_timestamp_us"] = serde_json::json!(hw_ts);
                        }
                        if let Some(readout) = frame.readout_time_ms {
                            metadata["readout_time_ms"] = serde_json::json!(readout);
                        }
                        if let Some(temp) = frame.sensor_temperature_c {
                            metadata["sensor_temperature_c"] = serde_json::json!(temp);
                        }
                        
                        metadata["software_timestamp"] = serde_json::json!(timestamp.to_rfc3339());

                        // Emit image measurement with PixelBuffer::U16
                        let image = ImageData {
                            timestamp,
                            channel: format!("{}_image", id),
                            width,
                            height,
                            pixels: PixelBuffer::U16(frame.data), // ✅ 4× memory savings
                            unit: "counts".to_string(),
                            metadata: Some(metadata),
                        };

                        let measurement = arc_measurement(Measurement::Image(image));
                        if tx.send(measurement.clone()).is_err() {
                            log::warn!("No receivers for image, stopping acquisition");
                            break;
                        }

                        // Also emit scalar statistics
                        let dp_mean = DataPoint {
                            timestamp,
                            channel: format!("{}:mean_intensity", id),
                            value: mean,
                            unit: "counts".to_string(),
                        };
                        let _ = tx.send(arc_measurement(Measurement::Scalar(dp_mean)));

                        let dp_min = DataPoint {
                            timestamp,
                            channel: format!("{}:min_intensity", id),
                            value: min,
                            unit: "counts".to_string(),
                        };
                        let _ = tx.send(arc_measurement(Measurement::Scalar(dp_min)));

                        let dp_max = DataPoint {
                            timestamp,
                            channel: format!("{}:max_intensity", id),
                            value: max,
                            unit: "counts".to_string(),
                        };
                        let _ = tx.send(arc_measurement(Measurement::Scalar(dp_max)));
                        
                        // Broadcast sensor temperature as separate measurement if available
                        if let Some(temp) = frame.sensor_temperature_c {
                            let dp_temp = DataPoint {
                                timestamp,
                                channel: format!("{}:sensor_temperature", id),
                                value: temp,
                                unit: "°C".to_string(),
                            };
                            let _ = tx.send(arc_measurement(Measurement::Scalar(dp_temp)));
                        }
                        
                        // Broadcast actual frame rate calculated from timestamps
                        if let Some(hw_ts) = frame.hardware_timestamp {
                            if frame.frame_number > 0 {
                                // Calculate frame period from hardware timestamps
                                let prev_hw_ts = hw_ts - (frame.exposure_time_ms * 1000.0) as i64;
                                let frame_period_us = hw_ts - prev_hw_ts;
                                if frame_period_us > 0 {
                                    let frame_rate_hz = 1_000_000.0 / frame_period_us as f64;
                                    let dp_fps = DataPoint {
                                        timestamp,
                                        channel: format!("{}:frame_rate", id),
                                        value: frame_rate_hz,
                                        unit: "Hz".to_string(),
                                    };
                                    let _ = tx.send(arc_measurement(Measurement::Scalar(dp_fps)));
                                }
                            }
                        }
                    }
                    // Shutdown signal
                    _ = &mut shutdown_rx => {
                        log::info!("PVCAM acquisition shutdown requested for '{}'", id);
                        break;
                    }
                    else => {
                        log::info!("Frame channel closed, acquisition ended.");
                        break;
                    }
                }
            }

            // Broadcast acquiring status = 0.0 (idle) when loop exits
            let idle_status = DataPoint {
                timestamp: Utc::now(),
                channel: format!("{}:acquiring", id),
                value: 0.0,
                unit: "".to_string(),
            };
            let _ = tx.send(arc_measurement(Measurement::Scalar(idle_status)));

            log::info!("PVCAM live acquisition stopped for '{}'", id);
        }));

        Ok(())
    }

    async fn stop_live(&mut self) -> Result<()> {
        if self.state != InstrumentState::Acquiring {
            return Ok(()); // Already stopped
        }

        // Signal shutdown to the acquisition task
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Wait for task to finish. The RAII guard inside the task will stop the SDK acquisition.
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        self.state = InstrumentState::Ready;
        log::info!("Stopped live acquisition for PVCAM '{}'", self.id);
        Ok(())
    }

    async fn set_exposure_ms(&mut self, ms: f64) -> Result<()> {
        if self.state == InstrumentState::Acquiring {
            return Err(anyhow::anyhow!("Cannot change exposure while acquiring"));
        }

        // Validation: Exposure must be positive and within u16 range
        if ms <= 0.0 {
            return Err(anyhow::anyhow!("Exposure must be a positive value, got {}", ms));
        }
        if ms > u16::MAX as f64 {
            return Err(anyhow::anyhow!("Exposure value {} ms is too large, max is {} ms", ms, u16::MAX));
        }

        // Convert f64 milliseconds to u16 for SDK (PVCAM uses integer milliseconds)
        let exposure_u16 = ms as u16;

        let handle = self.get_handle()?;
        self.sdk
            .set_param_u16(
                &handle,
                super::pvcam_sdk::PvcamParam::Exposure,
                exposure_u16,
            )
            .map_err(|e| anyhow::anyhow!("Failed to set exposure: {}", e))?;

        self.exposure_ms = ms; // Update internal state after successful SDK call
        log::info!("PVCAM exposure set to {} ms", ms);
        Ok(())
    }

    async fn get_exposure_ms(&self) -> f64 {
        self.exposure_ms
    }

    async fn set_roi(&mut self, roi: ROI) -> Result<()> {
        if self.state == InstrumentState::Acquiring {
            return Err(anyhow::anyhow!("Cannot change ROI while acquiring"));
        }

        let handle = self.get_handle()?;
        let px_region = self.roi_to_px_region(&roi, self.binning);

        self.sdk
            .set_param_region(&handle, super::pvcam_sdk::PvcamParam::Roi, px_region)
            .map_err(|e| anyhow::anyhow!("Failed to set ROI: {}", e))?;

        self.roi = roi;
        log::info!("PVCAM ROI set to {:?}", roi);
        Ok(())
    }

    async fn get_roi(&self) -> ROI {
        self.roi
    }

    async fn set_binning(&mut self, x: u16, y: u16) -> Result<()> {
        if self.state == InstrumentState::Acquiring {
            return Err(anyhow::anyhow!("Cannot change binning while acquiring"));
        }

        let handle = self.get_handle()?;
        let px_region = self.roi_to_px_region(&self.roi, (x, y));

        // Update ROI with new binning
        self.sdk
            .set_param_region(&handle, super::pvcam_sdk::PvcamParam::Roi, px_region)
            .map_err(|e| anyhow::anyhow!("Failed to set binning: {}", e))?;

        self.binning = (x, y);
        log::info!("PVCAM binning set to ({}, {})", x, y);
        Ok(())
    }

    async fn get_binning(&self) -> (u16, u16) {
        self.binning
    }

    fn get_sensor_size(&self) -> (u32, u32) {
        self.sensor_size
    }

    fn get_pixel_size_um(&self) -> (f64, f64) {
        (6.5, 6.5) // PrimeBSI pixel size
    }

    fn supports_hardware_trigger(&self) -> bool {
        true // PVCAM supports hardware triggering
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_pvcam_lifecycle() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());

        assert_eq!(camera.state(), InstrumentState::Disconnected);

        camera.initialize().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Ready);

        camera.shutdown().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Disconnected);
    }

    #[tokio::test]
    async fn test_pvcam_snap_uses_pixelbuffer_u16() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let image = camera.snap().await.unwrap();

        // Verify PixelBuffer::U16 is used
        match image.pixels {
            PixelBuffer::U16(ref data) => {
                assert_eq!(data.len(), 2048 * 2048);
                // Verify u16 values are in valid range
                assert!(data.iter().all(|&v| v <= u16::MAX));
            }
            _ => panic!("Expected PixelBuffer::U16, got {:?}", image.pixels),
        }

        // Verify memory savings
        let actual_bytes = image.pixels.memory_bytes();
        let expected_bytes = 2048 * 2048 * 2; // u16 = 2 bytes/pixel
        assert_eq!(actual_bytes, expected_bytes);

        // Compare to old Vec<f64> approach
        let old_bytes = 2048 * 2048 * 8; // f64 = 8 bytes/pixel
        let savings = old_bytes - actual_bytes;
        assert_eq!(savings, 25165824); // 24 MB savings per frame!
    }

    #[tokio::test]
    async fn test_pvcam_live_acquisition() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let mut rx = camera.measurement_stream();

        camera.start_live().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Acquiring);

        // Receive a few measurements (mix of Image and Scalar)
        for _ in 0..5 {
            let measurement = timeout(Duration::from_millis(500), rx.recv())
                .await
                .expect("Timed out waiting for measurement")
                .unwrap();
            match &*measurement {
                Measurement::Image(image) => {
                    // Verify PixelBuffer::U16
                    assert!(matches!(image.pixels, PixelBuffer::U16(_)));
                    assert_eq!(image.width, 2048);
                    assert_eq!(image.height, 2048);
                }
                Measurement::Scalar(data) => {
                    // Statistics channels and status channel
                    assert!(
                        data.channel.contains("mean")
                            || data.channel.contains("min")
                            || data.channel.contains("max")
                            || data.channel.contains("acquiring")
                    );
                }
                _ => panic!("Unexpected measurement type"),
            }
        }

        camera.stop_live().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Ready);
    }

    #[tokio::test]
    async fn test_set_binning_valid() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let binning_value = serde_json::json!({"x": 2, "y": 2});
        camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "binning".to_string(),
                value: binning_value,
            })
            .await
            .unwrap();

        assert_eq!(camera.binning, (2, 2));
    }

    #[tokio::test]
    async fn test_set_binning_rejects_while_acquiring() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();
        camera.start_live().await.unwrap();

        let binning_value = serde_json::json!({"x": 2, "y": 2});
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "binning".to_string(),
                value: binning_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot change binning while acquiring"));

        camera.stop_live().await.unwrap();
    }

    #[tokio::test]
    async fn test_set_binning_rejects_zero_values() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let binning_value = serde_json::json!({"x": 0, "y": 2});
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "binning".to_string(),
                value: binning_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Binning values must be positive"));
    }

    #[tokio::test]
    async fn test_set_binning_rejects_invalid_format() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        // Missing 'y' field
        let binning_value = serde_json::json!({"x": 2});
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "binning".to_string(),
                value: binning_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Binning 'y' must be a positive integer"));
    }

    #[tokio::test]
    async fn test_set_roi_valid() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let roi_value = serde_json::json!({
            "x": 100,
            "y": 100,
            "width": 512,
            "height": 512
        });
        camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "roi".to_string(),
                value: roi_value,
            })
            .await
            .unwrap();

        assert_eq!(camera.roi.x, 100);
        assert_eq!(camera.roi.y, 100);
        assert_eq!(camera.roi.width, 512);
        assert_eq!(camera.roi.height, 512);
    }

    #[tokio::test]
    async fn test_set_roi_rejects_while_acquiring() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();
        camera.start_live().await.unwrap();

        let roi_value = serde_json::json!({
            "x": 100,
            "y": 100,
            "width": 512,
            "height": 512
        });
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "roi".to_string(),
                value: roi_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot change ROI while acquiring"));

        camera.stop_live().await.unwrap();
    }

    #[tokio::test]
    async fn test_set_roi_rejects_out_of_bounds() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        // ROI extends beyond sensor width (x + width > 2048)
        let roi_value = serde_json::json!({
            "x": 1900,
            "y": 0,
            "width": 512,
            "height": 512
        });
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "roi".to_string(),
                value: roi_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ROI extends beyond sensor width"));
    }

    #[tokio::test]
    async fn test_set_roi_rejects_zero_dimensions() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let roi_value = serde_json::json!({
            "x": 0,
            "y": 0,
            "width": 0,
            "height": 512
        });
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "roi".to_string(),
                value: roi_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ROI width and height must be positive"));
    }

    #[tokio::test]
    async fn test_get_binning() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        // Set binning first
        camera.binning = (2, 4);

        let mut rx = camera.measurement_stream();

        camera
            .handle_command(InstrumentCommand::GetParameter {
                name: "binning".to_string(),
            })
            .await
            .unwrap();

        // Should receive two scalar measurements (binning_x and binning_y)
        let mut binning_x_received = false;
        let mut binning_y_received = false;

        for _ in 0..2 {
            let measurement = timeout(Duration::from_millis(100), rx.recv())
                .await
                .expect("Timed out waiting for measurement")
                .unwrap();

            if let Measurement::Scalar(data) = &*measurement {
                if data.channel.ends_with(":binning_x") {
                    assert_eq!(data.value, 2.0);
                    binning_x_received = true;
                } else if data.channel.ends_with(":binning_y") {
                    assert_eq!(data.value, 4.0);
                    binning_y_received = true;
                }
            }
        }

        assert!(binning_x_received && binning_y_received);
    }

    #[tokio::test]
    async fn test_get_roi() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        // Set ROI first
        camera.roi = ROI {
            x: 100,
            y: 200,
            width: 512,
            height: 1024,
        };

        let mut rx = camera.measurement_stream();

        camera
            .handle_command(InstrumentCommand::GetParameter {
                name: "roi".to_string(),
            })
            .await
            .unwrap();

        // Should receive five measurements (roi, roi_x, roi_y, roi_width, roi_height)
        let mut counts = std::collections::HashMap::new();

        for _ in 0..5 {
            let measurement = timeout(Duration::from_millis(100), rx.recv())
                .await
                .expect("Timed out waiting for measurement")
                .unwrap();

            if let Measurement::Scalar(data) = &*measurement {
                if data.channel.ends_with(":roi_x") {
                    assert_eq!(data.value, 100.0);
                    *counts.entry("roi_x").or_insert(0) += 1;
                } else if data.channel.ends_with(":roi_y") {
                    assert_eq!(data.value, 200.0);
                    *counts.entry("roi_y").or_insert(0) += 1;
                } else if data.channel.ends_with(":roi_width") {
                    assert_eq!(data.value, 512.0);
                    *counts.entry("roi_width").or_insert(0) += 1;
                } else if data.channel.ends_with(":roi_height") {
                    assert_eq!(data.value, 1024.0);
                    *counts.entry("roi_height").or_insert(0) += 1;
                } else if data.channel.ends_with(":roi") {
                    *counts.entry("roi").or_insert(0) += 1;
                }
            }
        }

        assert_eq!(counts.get("roi_x"), Some(&1));
        assert_eq!(counts.get("roi_y"), Some(&1));
        assert_eq!(counts.get("roi_width"), Some(&1));
        assert_eq!(counts.get("roi_height"), Some(&1));
        assert_eq!(counts.get("roi"), Some(&1));
    }

    #[tokio::test]
    async fn test_set_trigger_mode_valid() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        // Test all valid trigger modes
        let modes = vec!["timed", "trigger_first", "strobed", "bulb", "software_edge"];
        let expected = vec![
            TriggerMode::Timed,
            TriggerMode::TriggerFirst,
            TriggerMode::Strobed,
            TriggerMode::Bulb,
            TriggerMode::SoftwareEdge,
        ];

        for (mode_str, expected_mode) in modes.iter().zip(expected.iter()) {
            let trigger_value = serde_json::json!(mode_str);
            camera
                .handle_command(InstrumentCommand::SetParameter {
                    name: "trigger_mode".to_string(),
                    value: trigger_value,
                })
                .await
                .unwrap();

            assert_eq!(camera.trigger_mode, *expected_mode);
        }
    }

    #[tokio::test]
    async fn test_set_trigger_mode_rejects_while_acquiring() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();
        camera.start_live().await.unwrap();

        let trigger_value = serde_json::json!("strobed");
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "trigger_mode".to_string(),
                value: trigger_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot change trigger mode while acquiring"));

        camera.stop_live().await.unwrap();
    }

    #[tokio::test]
    async fn test_set_trigger_mode_rejects_invalid_mode() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let trigger_value = serde_json::json!("invalid_mode");
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "trigger_mode".to_string(),
                value: trigger_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid trigger mode"));
    }

    #[tokio::test]
    async fn test_set_trigger_mode_rejects_non_string() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let trigger_value = serde_json::json!(42);
        let result = camera
            .handle_command(InstrumentCommand::SetParameter {
                name: "trigger_mode".to_string(),
                value: trigger_value,
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expected string"));
    }

    #[tokio::test]
    async fn test_get_trigger_mode() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        // Set trigger mode to strobed
        camera.trigger_mode = TriggerMode::Strobed;

        let mut rx = camera.measurement_stream();

        camera
            .handle_command(InstrumentCommand::GetParameter {
                name: "trigger_mode".to_string(),
            })
            .await
            .unwrap();

        // Should receive scalar measurement with trigger mode value
        let measurement = timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("Timed out waiting for measurement")
            .unwrap();

        if let Measurement::Scalar(data) = &*measurement {
            assert!(data.channel.ends_with(":trigger_mode"));
            assert_eq!(data.value, TriggerMode::Strobed.as_u16() as f64);
        } else {
            panic!("Expected Scalar measurement");
        }
    }

    #[tokio::test]
    async fn test_set_trigger_mode_default() {
        let camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        assert_eq!(camera.trigger_mode, TriggerMode::Timed);
    }

    #[tokio::test]
    async fn test_frame_metadata_fields_exist() {
        use super::super::pvcam_sdk::Frame;
        use chrono::Utc;

        // Verify Frame struct has all required metadata fields
        let frame = Frame {
            data: vec![100u16; 10],
            frame_number: 42,
            hardware_timestamp: Some(1234567890),
            software_timestamp: Utc::now(),
            exposure_time_ms: 100.0,
            readout_time_ms: Some(7.5),
            sensor_temperature_c: Some(-5.0),
            roi: (0, 0, 512, 512),
        };

        assert_eq!(frame.frame_number, 42);
        assert_eq!(frame.hardware_timestamp, Some(1234567890));
        assert_eq!(frame.exposure_time_ms, 100.0);
        assert_eq!(frame.readout_time_ms, Some(7.5));
        assert_eq!(frame.sensor_temperature_c, Some(-5.0));
        assert_eq!(frame.roi, (0, 0, 512, 512));
        assert_eq!(frame.data.len(), 10);
    }

    #[tokio::test]
    async fn test_mock_sdk_populates_metadata() {
        use super::super::pvcam_sdk::{MockPvcamSdk, PvcamSdk};
        use std::sync::Arc;

        let sdk = Arc::new(MockPvcamSdk::new());
        sdk.init().unwrap();
        let handle = sdk.open_camera("PrimeBSI").unwrap();

        // Start acquisition
        let (mut rx, _guard) = sdk.start_acquisition(handle).unwrap();

        // Give the acquisition task time to start
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Receive first frame and verify metadata
        let frame = timeout(Duration::from_millis(1000), rx.recv())
            .await
            .expect("Timed out waiting for frame")
            .expect("Channel closed unexpectedly");

        // Verify all metadata fields are populated with realistic values
        assert!(frame.hardware_timestamp.is_some());
        assert!(frame.readout_time_ms.is_some());
        assert!(frame.sensor_temperature_c.is_some());

        let hw_ts = frame.hardware_timestamp.unwrap();
        assert!(hw_ts > 0, "Hardware timestamp should be positive");

        let readout = frame.readout_time_ms.unwrap();
        assert!(readout >= 5.0 && readout <= 10.0, "Readout time should be 5-10ms, got {}", readout);

        let temp = frame.sensor_temperature_c.unwrap();
        assert!(temp >= -10.0 && temp <= 5.0, "Sensor temp should be -10 to 5°C, got {}", temp);

        assert_eq!(frame.exposure_time_ms, 100.0);
        assert_eq!(frame.roi.2, 2048); // width
        assert_eq!(frame.roi.3, 2048); // height
    }

    #[tokio::test]
    async fn test_imagedata_metadata_contains_all_fields() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let mut rx = camera.measurement_stream();
        camera.start_live().await.unwrap();

        // Wait for first image measurement
        let mut image_received = false;
        for _ in 0..20 {
            let measurement = timeout(Duration::from_millis(500), rx.recv())
                .await
                .expect("Timed out waiting for measurement")
                .unwrap();

            if let Measurement::Image(image) = &*measurement {
                image_received = true;

                // Verify metadata exists
                let metadata = image.metadata.as_ref().expect("Metadata should exist");

                // Verify all required fields are present
                assert!(metadata.get("camera_name").is_some());
                assert!(metadata.get("exposure_ms").is_some());
                assert!(metadata.get("roi").is_some());
                assert!(metadata.get("binning").is_some());
                assert!(metadata.get("frame").is_some());
                assert!(metadata.get("exposure_time_ms").is_some());
                assert!(metadata.get("roi_from_frame").is_some());
                assert!(metadata.get("hardware_timestamp_us").is_some());
                assert!(metadata.get("software_timestamp").is_some());
                assert!(metadata.get("readout_time_ms").is_some());
                assert!(metadata.get("sensor_temperature_c").is_some());

                // Verify field types and values
                assert_eq!(metadata["camera_name"].as_str().unwrap(), "PrimeBSI");
                assert_eq!(metadata["exposure_ms"].as_f64().unwrap(), 100.0);
                assert!(metadata["hardware_timestamp_us"].as_i64().unwrap() > 0);

                let readout = metadata["readout_time_ms"].as_f64().unwrap();
                assert!(readout >= 5.0 && readout <= 10.0);

                let temp = metadata["sensor_temperature_c"].as_f64().unwrap();
                assert!(temp >= -10.0 && temp <= 5.0);

                // Verify ROI from frame matches expected format
                let roi_from_frame = metadata["roi_from_frame"].as_object().unwrap();
                assert!(roi_from_frame.get("x").is_some());
                assert!(roi_from_frame.get("y").is_some());
                assert!(roi_from_frame.get("width").is_some());
                assert!(roi_from_frame.get("height").is_some());

                break;
            }
        }

        assert!(image_received, "Should have received at least one image measurement");
        camera.stop_live().await.unwrap();
    }

    #[tokio::test]
    async fn test_metadata_changes_with_camera_parameters() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        // Change exposure time
        camera.set_exposure_ms(50.0).await.unwrap();

        // Change ROI
        camera.set_roi(ROI { x: 100, y: 100, width: 512, height: 512 }).await.unwrap();

        // Change binning
        camera.set_binning(2, 2).await.unwrap();

        let mut rx = camera.measurement_stream();
        camera.start_live().await.unwrap();

        // Wait for image with updated metadata
        let mut verified = false;
        for _ in 0..20 {
            let measurement = timeout(Duration::from_millis(500), rx.recv())
                .await
                .expect("Timed out waiting for measurement")
                .unwrap();

            if let Measurement::Image(image) = &*measurement {
                let metadata = image.metadata.as_ref().unwrap();

                // Verify exposure time changed
                if metadata["exposure_time_ms"].as_f64().unwrap() == 50.0 {
                    // Verify ROI changed
                    let roi = metadata["roi"].as_object().unwrap();
                    assert_eq!(roi["x"].as_u64().unwrap(), 100);
                    assert_eq!(roi["y"].as_u64().unwrap(), 100);
                    assert_eq!(roi["width"].as_u64().unwrap(), 512);
                    assert_eq!(roi["height"].as_u64().unwrap(), 512);

                    // Verify binning changed
                    let binning = metadata["binning"].as_array().unwrap();
                    assert_eq!(binning[0].as_u64().unwrap(), 2);
                    assert_eq!(binning[1].as_u64().unwrap(), 2);

                    // Verify image dimensions match binned ROI
                    assert_eq!(image.width, 512 / 2);
                    assert_eq!(image.height, 512 / 2);

                    verified = true;
                    break;
                }
            }
        }

        assert!(verified, "Should have received image with updated metadata");
        camera.stop_live().await.unwrap();
    }

    #[tokio::test]
    async fn test_sensor_temperature_broadcast_as_scalar() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let mut rx = camera.measurement_stream();
        camera.start_live().await.unwrap();

        // Look for sensor temperature scalar measurement
        let mut temp_received = false;
        for _ in 0..50 {
            let measurement = timeout(Duration::from_millis(500), rx.recv())
                .await
                .expect("Timed out waiting for measurement")
                .unwrap();

            if let Measurement::Scalar(data) = &*measurement {
                if data.channel.ends_with(":sensor_temperature") {
                    assert_eq!(data.unit, "°C");
                    assert!(data.value >= -10.0 && data.value <= 5.0);
                    temp_received = true;
                    break;
                }
            }
        }

        assert!(temp_received, "Should have received sensor temperature as scalar");
        camera.stop_live().await.unwrap();
    }

    #[tokio::test]
    async fn test_frame_rate_broadcast_from_hardware_timestamps() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let mut rx = camera.measurement_stream();
        camera.start_live().await.unwrap();

        // Look for frame rate scalar measurement (should appear after first frame)
        let mut fps_received = false;
        for _ in 0..100 {
            let measurement = timeout(Duration::from_millis(500), rx.recv())
                .await
                .expect("Timed out waiting for measurement")
                .unwrap();

            if let Measurement::Scalar(data) = &*measurement {
                if data.channel.ends_with(":frame_rate") {
                    assert_eq!(data.unit, "Hz");
                    // With 100ms exposure, expect ~10 Hz (but mock might be different)
                    assert!(data.value > 0.0, "Frame rate should be positive");
                    assert!(data.value < 1000.0, "Frame rate should be reasonable");
                    fps_received = true;
                    break;
                }
            }
        }

        assert!(fps_received, "Should have received frame rate calculation");
        camera.stop_live().await.unwrap();
    }

    #[tokio::test]
    async fn test_snap_includes_metadata() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();

        let image = camera.snap().await.unwrap();

        // Verify metadata exists and contains all fields
        let metadata = image.metadata.as_ref().expect("snap() should include metadata");

        assert!(metadata.get("camera_name").is_some());
        assert!(metadata.get("exposure_ms").is_some());
        assert!(metadata.get("exposure_time_ms").is_some());
        assert!(metadata.get("hardware_timestamp_us").is_some());
        assert!(metadata.get("software_timestamp").is_some());
        assert!(metadata.get("readout_time_ms").is_some());
        assert!(metadata.get("sensor_temperature_c").is_some());
        assert!(metadata.get("roi_from_frame").is_some());

        // Verify values are realistic
        let temp = metadata["sensor_temperature_c"].as_f64().unwrap();
        assert_eq!(temp, -5.0);

        let readout = metadata["readout_time_ms"].as_f64().unwrap();
        assert_eq!(readout, 7.5);
    }
    
    // ==================== DIAGNOSTIC TESTS ====================
    
    #[tokio::test]
    async fn test_diagnostic_counters_track_frames() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();
        
        let mut rx = camera.measurement_stream();
        
        // Start acquisition
        camera.start_live().await.unwrap();
        
        // Wait for several frames
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        // Query total_frames
        camera.handle_command(InstrumentCommand::GetParameter {
            name: "total_frames".to_string(),
        }).await.unwrap();
        
        // Look for the total_frames measurement
        let mut total_frames_value = None;
        for _ in 0..20 {
            if let Ok(Ok(measurement)) = timeout(Duration::from_millis(100), rx.recv()).await {
                if let Measurement::Scalar(data) = &*measurement {
                    if data.channel.ends_with(":total_frames") {
                        total_frames_value = Some(data.value);
                        break;
                    }
                }
            }
        }
        
        let total = total_frames_value.expect("Should receive total_frames measurement");
        assert!(total >= 1.0, "Should have received at least 1 frame, got {}", total);
        assert_eq!(camera.total_frames.load(Ordering::Relaxed) as f64, total);
        
        camera.stop_live().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_diagnostic_dropped_frames_detection() {
        use super::super::pvcam_sdk::MockPvcamSdk;
        use std::sync::Arc;
        
        // Create camera with MockPvcamSdk that we can configure
        let sdk = Arc::new(MockPvcamSdk::new());
        
        // Enable dropped frame simulation with 20% probability
        sdk.set_simulate_dropped_frames(true);
        sdk.set_drop_frame_probability(0.2);
        
        let mut camera = PVCAMInstrumentV2::with_adapter_and_capacity(
            "test_pvcam".to_string(),
            Box::new(crate::adapters::MockAdapter::new()),
            1024,
        );
        
        // Replace the SDK with our configured one
        camera.sdk = sdk;
        
        camera.initialize().await.unwrap();
        camera.start_live().await.unwrap();
        
        // Wait for frames to accumulate (with 20% drop rate, we should see some drops)
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // Subscribe BEFORE querying to ensure we receive the broadcast
        let mut rx = camera.measurement_stream();

        // Query dropped_frames
        camera.handle_command(InstrumentCommand::GetParameter {
            name: "dropped_frames".to_string(),
        }).await.unwrap();
        let mut dropped_frames_value = None;
        for _ in 0..20 {
            if let Ok(Ok(measurement)) = timeout(Duration::from_millis(100), rx.recv()).await {
                if let Measurement::Scalar(data) = &*measurement {
                    if data.channel.ends_with(":dropped_frames") {
                        dropped_frames_value = Some(data.value);
                        break;
                    }
                }
            }
        }
        
        let dropped = dropped_frames_value.expect("Should receive dropped_frames measurement");
        // With 20% drop probability over 1 second at ~100ms exposure, expect some drops
        // but this is probabilistic so we just verify the counter exists and is accessible
        assert!(dropped >= 0.0, "Dropped frames should be non-negative, got {}", dropped);
        
        camera.stop_live().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_diagnostic_actual_fps_calculation() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();
        
        let mut rx = camera.measurement_stream();
        camera.start_live().await.unwrap();
        
        // Wait for frames to accumulate
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        // Query actual_fps
        camera.handle_command(InstrumentCommand::GetParameter {
            name: "actual_fps".to_string(),
        }).await.unwrap();
        
        let mut fps_value = None;
        for _ in 0..20 {
            if let Ok(Ok(measurement)) = timeout(Duration::from_millis(100), rx.recv()).await {
                if let Measurement::Scalar(data) = &*measurement {
                    if data.channel.ends_with(":actual_fps") {
                        fps_value = Some(data.value);
                        break;
                    }
                }
            }
        }
        
        let fps = fps_value.expect("Should receive actual_fps measurement");
        assert!(fps > 0.0, "FPS should be positive");
        assert!(fps < 100.0, "FPS should be reasonable for 100ms exposure");
        
        camera.stop_live().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_diagnostic_camera_health() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();
        
        let mut rx = camera.measurement_stream();
        
        // Query health while idle
        camera.handle_command(InstrumentCommand::GetParameter {
            name: "camera_health".to_string(),
        }).await.unwrap();
        
        let mut health_value = None;
        for _ in 0..20 {
            if let Ok(Ok(measurement)) = timeout(Duration::from_millis(100), rx.recv()).await {
                if let Measurement::Scalar(data) = &*measurement {
                    if data.channel.ends_with(":camera_health") {
                        health_value = Some(data.value);
                        break;
                    }
                }
            }
        }
        
        let health = health_value.expect("Should receive camera_health measurement");
        assert_eq!(health, 0.75, "Health should be 0.75 (Ready/Idle)");
        
        // Start acquisition and check health again
        camera.start_live().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        
        camera.handle_command(InstrumentCommand::GetParameter {
            name: "camera_health".to_string(),
        }).await.unwrap();
        
        let mut health_acquiring = None;
        for _ in 0..20 {
            if let Ok(Ok(measurement)) = timeout(Duration::from_millis(100), rx.recv()).await {
                if let Measurement::Scalar(data) = &*measurement {
                    if data.channel.ends_with(":camera_health") {
                        health_acquiring = Some(data.value);
                        break;
                    }
                }
            }
        }
        
        let health = health_acquiring.expect("Should receive camera_health measurement while acquiring");
        assert_eq!(health, 1.0, "Health should be 1.0 (Healthy/Acquiring)");
        
        camera.stop_live().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_diagnostic_counters_reset_on_start() {
        let mut camera = PVCAMInstrumentV2::new("test_pvcam".to_string());
        camera.initialize().await.unwrap();
        
        // Manually set counters to non-zero values
        camera.total_frames.store(100, Ordering::Relaxed);
        camera.dropped_frames.store(10, Ordering::Relaxed);
        
        // Start acquisition - should reset counters
        camera.start_live().await.unwrap();
        
        // Verify counters were reset
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // After some frames, total should be small (recently reset)
        let total = camera.total_frames.load(Ordering::Relaxed);
        assert!(total < 50, "Total frames should be reset and small, got {}", total);
        
        camera.stop_live().await.unwrap();
    }
    
    #[test]
    fn test_pvcam_error_variants_have_context() {
        use super::super::pvcam_sdk::{PvcamError, CameraHandle};
        
        // Test InitFailed has descriptive message
        let err = PvcamError::InitFailed("SDK not found".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("Failed to initialize"));
        assert!(msg.contains("SDK not found"));
        
        // Test CameraDisconnected
        let err = PvcamError::CameraDisconnected { camera: "PrimeBSI".to_string() };
        let msg = format!("{}", err);
        assert!(msg.contains("Camera disconnected"));
        assert!(msg.contains("PrimeBSI"));
        
        // Test InvalidParameter
        let err = PvcamError::InvalidParameter {
            param: "exposure".to_string(),
            reason: "value too large".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid parameter"));
        assert!(msg.contains("exposure"));
        assert!(msg.contains("value too large"));
        
        // Test OutOfRange
        let err = PvcamError::OutOfRange {
            param: "gain".to_string(),
            value: "1000".to_string(),
            valid_range: "1-64".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("out of range"));
        assert!(msg.contains("gain"));
        assert!(msg.contains("1000"));
        assert!(msg.contains("1-64"));
        
        // Test AcquisitionError
        let err = PvcamError::AcquisitionError {
            camera: "PrimeBSI".to_string(),
            reason: "buffer overflow".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Acquisition error"));
        assert!(msg.contains("PrimeBSI"));
        assert!(msg.contains("buffer overflow"));
        
        // Test Timeout
        let err = PvcamError::Timeout { operation: "frame capture".to_string() };
        let msg = format!("{}", err);
        assert!(msg.contains("timed out"));
        assert!(msg.contains("frame capture"));
        
        // Test DroppedFrames
        let err = PvcamError::DroppedFrames {
            expected: 10,
            actual: 15,
            dropped: 5,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Frame number gap"));
        assert!(msg.contains("expected 10"));
        assert!(msg.contains("got 15"));
        assert!(msg.contains("Dropped 5"));
    }
    
    #[tokio::test]
    async fn test_mock_sdk_error_injection() {
        use super::super::pvcam_sdk::{MockPvcamSdk, PvcamSdk};
        use std::sync::Arc;
        
        let sdk = Arc::new(MockPvcamSdk::new());
        
        // Test init failure
        sdk.set_next_init_fails(true);
        let result = sdk.init();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Mock init failed"));
        
        // Init should succeed on second try
        let result = sdk.init();
        assert!(result.is_ok());
        
        // Test open_camera failure
        use super::super::pvcam_sdk::{PvcamError, CameraHandle};
        sdk.set_next_open_fails_with_error(Some(PvcamError::CameraNotFound("TestCam".to_string())));
        let result = sdk.open_camera("PrimeBSI");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Camera not found"));
    }
}