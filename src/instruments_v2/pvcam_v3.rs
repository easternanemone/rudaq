//! Photometrics PVCAM camera driver V3 (Unified Architecture)
//!
//! V3 implementation using the unified core_v3 traits:
//! - Implements `core_v3::Instrument` trait (replaces V1/V2 split)
//! - Implements `core_v3::Camera` trait for polymorphism
//! - Uses `Parameter<T>` for declarative parameter management
//! - Direct async methods (no InstrumentCommand message passing)
//! - Single broadcast channel (no double-broadcast overhead)
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.prime_bsi]
//! type = "pvcam_v3"
//! camera_name = "PrimeBSI"
//! exposure_ms = 100.0
//! roi = [0, 0, 2048, 2048]  # [x, y, width, height]
//! binning = [1, 1]  # [x_bin, y_bin]
//! sdk_mode = "mock"  # or "real" when SDK available
//! ```
//!
//! ## Migration from V2
//!
//! V3 eliminates the actor model and message passing:
//! - V2: `handle_command(InstrumentCommand)` → Complex enum matching
//! - V3: Direct trait methods (`set_exposure()`, `start_acquisition()`, etc.)
//!
//! Performance improvements:
//! - Single broadcast (was: instrument → actor → GUI)
//! - Direct async calls (was: GUI → actor → instrument → actor → GUI)
//! - Parameter validation at compile time (was: runtime JSON parsing)

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::JoinHandle;

use super::pvcam_sdk::{
    AcquisitionGuard, CameraHandle, Frame, MockPvcamSdk, PvcamParam, PvcamSdk, PxRegion,
    RealPvcamSdk, TriggerMode,
};
use crate::core_v3::{
    Camera, Command, ImageData, ImageMetadata, Instrument, InstrumentState, Measurement,
    ParameterBase, PixelBuffer, Response, Roi,
};
use crate::parameter::{Parameter, ParameterBuilder};

/// SDK mode selection for PVCAM camera
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvcamSdkKind {
    /// Mock SDK for testing without hardware
    Mock,
    /// Real SDK for actual camera control
    Real,
}

/// PVCAM camera V3 implementation
///
/// Unified architecture implementation demonstrating:
/// - Direct `Instrument` + `Camera` trait implementation
/// - `Parameter<T>` for declarative camera settings
/// - Single broadcast channel for data streaming
/// - Direct async methods (no message passing)
/// - SDK abstraction layer (Mock/Real)
pub struct PVCAMCameraV3 {
    /// Instrument identifier
    id: String,

    /// Current state
    state: InstrumentState,

    /// Data broadcast channel
    data_tx: broadcast::Sender<Measurement>,

    /// Parameters (for dynamic access via ParameterBase)
    parameters: HashMap<String, Box<dyn ParameterBase>>,

    // SDK abstraction layer
    sdk: Arc<dyn PvcamSdk>,
    camera_handle: Option<CameraHandle>,
    sdk_kind: PvcamSdkKind,

    // Typed camera parameters (for direct access via Camera trait)
    camera_name: String,
    exposure_ms: Arc<RwLock<Parameter<f64>>>,
    roi: Arc<RwLock<Parameter<Roi>>>,
    binning: Arc<RwLock<Parameter<(u32, u32)>>>,
    gain: Arc<RwLock<Parameter<u32>>>,
    trigger_mode: Arc<RwLock<Parameter<String>>>, // "internal", "external_edge", "external_level"

    // Sensor metadata (read-only)
    sensor_size: (u32, u32),

    // Acquisition state
    is_acquiring: bool,
    frame_receiver: Option<mpsc::Receiver<Frame>>,
    acquisition_guard: Option<AcquisitionGuard>,

    // Background streaming task
    streaming_task: Option<JoinHandle<()>>,

    // Diagnostic counters (atomic for thread-safe access)
    total_frames: Arc<AtomicU64>,
    dropped_frames: Arc<AtomicU64>,
    last_frame_number: Arc<AtomicU32>,
    acquisition_start_time: Arc<tokio::sync::Mutex<Option<Instant>>>,
}

impl PVCAMCameraV3 {
    /// Create new PVCAM camera with Mock SDK
    pub fn new(id: impl Into<String>, camera_name: impl Into<String>) -> Self {
        Self::with_sdk(id, camera_name, PvcamSdkKind::Mock)
    }

    /// Create new PVCAM camera with specified SDK kind
    pub fn with_sdk(
        id: impl Into<String>,
        camera_name: impl Into<String>,
        sdk_kind: PvcamSdkKind,
    ) -> Self {
        let id = id.into();
        let camera_name = camera_name.into();
        let (data_tx, _) = broadcast::channel(1024);

        // Create SDK based on kind
        let sdk: Arc<dyn PvcamSdk> = match sdk_kind {
            PvcamSdkKind::Mock => Arc::new(MockPvcamSdk::new()),
            PvcamSdkKind::Real => Arc::new(RealPvcamSdk::new()),
        };

        // Default sensor size (will be updated from SDK on initialization)
        let sensor_size = (2048, 2048);

        // Create parameters with constraints
        let exposure_ms = Arc::new(RwLock::new(
            ParameterBuilder::new("exposure_ms", 100.0)
                .description("Camera exposure time")
                .unit("ms")
                .range(1.0, 10000.0)
                .build(),
        ));

        let roi = Arc::new(RwLock::new(
            Parameter::new(
                "roi",
                Roi {
                    x: 0,
                    y: 0,
                    width: sensor_size.0,
                    height: sensor_size.1,
                },
            )
            .with_description("Region of interest for acquisition"),
        ));

        let binning = Arc::new(RwLock::new(
            ParameterBuilder::new("binning", (1u32, 1u32))
                .description("Pixel binning (horizontal, vertical)")
                .choices(vec![(1, 1), (2, 2), (4, 4), (8, 8)])
                .build(),
        ));

        let gain = Arc::new(RwLock::new(
            ParameterBuilder::new("gain", 1u32)
                .description("Sensor gain index")
                .range(1, 4) // Typical gain range, will be updated from SDK
                .build(),
        ));

        let trigger_mode = Arc::new(RwLock::new(
            ParameterBuilder::new("trigger_mode", "timed".to_string())
                .description("Trigger mode for acquisition")
                .choices(vec![
                    "timed".to_string(),
                    "trigger_first".to_string(),
                    "strobed".to_string(),
                    "bulb".to_string(),
                    "software_edge".to_string(),
                ])
                .build(),
        ));

        Self {
            id,
            state: InstrumentState::Uninitialized,
            data_tx,
            parameters: HashMap::new(),
            sdk,
            camera_handle: None,
            sdk_kind,
            camera_name,
            exposure_ms,
            roi,
            binning,
            gain,
            trigger_mode,
            sensor_size,
            is_acquiring: false,
            frame_receiver: None,
            acquisition_guard: None,
            streaming_task: None,
            total_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            last_frame_number: Arc::new(AtomicU32::new(0)),
            acquisition_start_time: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    fn roi_to_region(roi: &Roi, binning: (u32, u32)) -> Result<PxRegion> {
        if roi.width == 0 || roi.height == 0 {
            return Err(anyhow!("ROI width and height must be greater than zero"));
        }
        if binning.0 == 0 || binning.1 == 0 {
            return Err(anyhow!("Binning values must be greater than zero"));
        }

        let s2 = roi
            .x
            .checked_add(roi.width.saturating_sub(1))
            .ok_or_else(|| anyhow!("ROI width causes overflow"))?;
        let p2 = roi
            .y
            .checked_add(roi.height.saturating_sub(1))
            .ok_or_else(|| anyhow!("ROI height causes overflow"))?;

        Ok(PxRegion {
            s1: roi.x as u16,
            s2: s2 as u16,
            sbin: binning.0 as u16,
            p1: roi.y as u16,
            p2: p2 as u16,
            pbin: binning.1 as u16,
        })
    }

    fn region_to_roi(region: PxRegion) -> Result<(Roi, (u32, u32))> {
        if region.sbin == 0 || region.pbin == 0 {
            return Err(anyhow!("Hardware reported zero binning which is invalid"));
        }

        let width = region
            .s2
            .checked_sub(region.s1)
            .ok_or_else(|| anyhow!("Invalid ROI region: s2 < s1"))?
            .saturating_add(1) as u32;
        let height = region
            .p2
            .checked_sub(region.p1)
            .ok_or_else(|| anyhow!("Invalid ROI region: p2 < p1"))?
            .saturating_add(1) as u32;

        let roi = Roi {
            x: region.s1 as u32,
            y: region.p1 as u32,
            width,
            height,
        };
        let binning = (region.sbin as u32, region.pbin as u32);
        Ok((roi, binning))
    }

    fn exposure_to_u16(ms: f64) -> Result<u16> {
        if ms <= 0.0 {
            return Err(anyhow!("Exposure must be positive"));
        }
        let rounded = ms.round();
        if !(0.0..=u16::MAX as f64).contains(&rounded) {
            return Err(anyhow!("Exposure {}ms out of hardware range", ms));
        }
        Ok(rounded as u16)
    }

    fn parse_trigger_mode(value: &str) -> Result<TriggerMode> {
        TriggerMode::from_str(value).ok_or_else(|| {
            anyhow!(
                "Unsupported trigger mode '{}'. Expected one of timed, trigger_first, strobed, bulb, software_edge",
                value
            )
        })
    }

    fn trigger_mode_to_string(mode_value: u16) -> Result<String> {
        let mode = TriggerMode::from_u16(mode_value)
            .ok_or_else(|| anyhow!("Unknown trigger mode value {}", mode_value))?;
        Ok(mode.as_str().to_string())
    }

    fn apply_roi_to_hardware(
        &self,
        handle: &CameraHandle,
        roi: Roi,
        binning: (u32, u32),
    ) -> Result<(Roi, (u32, u32))> {
        let region = Self::roi_to_region(&roi, binning)?;
        self.sdk
            .set_param_region(handle, PvcamParam::Roi, region)
            .with_context(|| "Failed to apply ROI to PVCAM")?;
        let confirmed = self
            .sdk
            .get_param_region(handle, PvcamParam::Roi)
            .with_context(|| "Failed to read back ROI from PVCAM")?;
        Self::region_to_roi(confirmed)
    }

    async fn refresh_parameters_from_hardware(&mut self, handle: &CameraHandle) -> Result<()> {
        let exposure_hw =
            self.sdk
                .get_param_u16(handle, PvcamParam::Exposure)
                .with_context(|| "Failed to read exposure from PVCAM")? as f64;
        self.exposure_ms.write().await.set(exposure_hw).await?;

        let gain_hw = self
            .sdk
            .get_param_u16(handle, PvcamParam::Gain)
            .with_context(|| "Failed to read gain from PVCAM")? as u32;
        self.gain.write().await.set(gain_hw).await?;

        let region = self
            .sdk
            .get_param_region(handle, PvcamParam::Roi)
            .with_context(|| "Failed to read ROI from PVCAM")?;
        let (roi_hw, binning_hw) = Self::region_to_roi(region)?;
        self.roi.write().await.set(roi_hw).await?;
        self.binning.write().await.set(binning_hw).await?;
        self.sensor_size = (roi_hw.width, roi_hw.height);

        let trigger_hw = self
            .sdk
            .get_param_u16(handle, PvcamParam::ExposureMode)
            .with_context(|| "Failed to read trigger mode from PVCAM")?;
        let trigger_str = Self::trigger_mode_to_string(trigger_hw)?;
        self.trigger_mode.write().await.set(trigger_str).await?;

        Ok(())
    }

    async fn set_gain(&mut self, gain: u32) -> Result<()> {
        {
            let param = self.gain.read().await;
            param.constraints().validate(&gain)?;
        }

        if let Some(ref handle) = self.camera_handle {
            self.sdk
                .set_param_u16(handle, PvcamParam::Gain, gain as u16)
                .with_context(|| "Failed to write gain to PVCAM")?;
            let confirmed =
                self.sdk
                    .get_param_u16(handle, PvcamParam::Gain)
                    .with_context(|| "Failed to verify gain from PVCAM")? as u32;
            self.gain.write().await.set(confirmed).await?;
        } else {
            self.gain.write().await.set(gain).await?;
        }
        Ok(())
    }

    async fn set_trigger_mode_internal(&mut self, mode: &str) -> Result<()> {
        let trigger_mode = Self::parse_trigger_mode(mode)?;

        if let Some(ref handle) = self.camera_handle {
            self.sdk
                .set_param_u16(handle, PvcamParam::ExposureMode, trigger_mode.as_u16())
                .with_context(|| "Failed to write trigger mode to PVCAM")?;
            self.trigger_mode
                .write()
                .await
                .set(trigger_mode.as_str().to_string())
                .await?;
        } else {
            self.trigger_mode
                .write()
                .await
                .set(trigger_mode.as_str().to_string())
                .await?;
        }

        Ok(())
    }

    /// Get diagnostic frame counters
    pub fn frame_stats(&self) -> (u64, u64, u32) {
        (
            self.total_frames.load(Ordering::Relaxed),
            self.dropped_frames.load(Ordering::Relaxed),
            self.last_frame_number.load(Ordering::Relaxed),
        )
    }

    /// Start background streaming task that receives frames from SDK
    fn start_streaming_task(&mut self) {
        if self.streaming_task.is_some() {
            return; // Already running
        }

        let mut receiver = match self.frame_receiver.take() {
            Some(rx) => rx,
            None => {
                log::error!("PVCAM '{}': No frame receiver available", self.id);
                return;
            }
        };

        let data_tx = self.data_tx.clone();
        let id = self.id.clone();
        let exposure_ms = self.exposure_ms.clone();
        let roi = self.roi.clone();
        let binning = self.binning.clone();
        let gain = self.gain.clone();
        let total_frames = self.total_frames.clone();
        let dropped_frames = self.dropped_frames.clone();
        let last_frame_number = self.last_frame_number.clone();

        let task = tokio::spawn(async move {
            log::info!("PVCAM '{}': Streaming task started", id);

            while let Some(frame) = receiver.recv().await {
                // Update counters
                total_frames.fetch_add(1, Ordering::Relaxed);
                let prev_frame_num = last_frame_number.swap(frame.frame_number, Ordering::Relaxed);

                // Detect dropped frames (use u32::MAX as sentinel for "no previous frame")
                if prev_frame_num != u32::MAX && frame.frame_number > prev_frame_num + 1 {
                    let dropped = frame.frame_number - prev_frame_num - 1;
                    dropped_frames.fetch_add(dropped as u64, Ordering::Relaxed);
                    log::warn!(
                        "PVCAM '{}': Dropped {} frames (#{} → #{})",
                        id,
                        dropped,
                        prev_frame_num,
                        frame.frame_number
                    );
                }

                // Get current parameters
                let current_roi = roi.read().await.get();
                let current_binning = binning.read().await.get();
                let current_gain = gain.read().await.get();

                // Create measurement from frame
                let measurement = Measurement::Image {
                    name: format!("{}_frame", id),
                    width: current_roi.width,
                    height: current_roi.height,
                    buffer: PixelBuffer::U16(frame.data),
                    unit: "counts".to_string(),
                    metadata: ImageMetadata {
                        exposure_ms: Some(frame.exposure_time_ms),
                        gain: Some(current_gain as f64),
                        binning: Some(current_binning),
                        temperature_c: frame.sensor_temperature_c,
                    },
                    timestamp: frame.software_timestamp,
                };

                // Broadcast (non-blocking)
                if data_tx.send(measurement).is_err() {
                    log::debug!("PVCAM '{}': No subscribers, stopping stream", id);
                    break;
                }
            }

            log::info!("PVCAM '{}': Streaming task ended", id);
        });

        self.streaming_task = Some(task);
    }

    /// Stop background streaming task
    fn stop_streaming_task(&mut self) {
        // Drop the acquisition guard, which stops the SDK acquisition
        self.acquisition_guard = None;

        // Abort the streaming task
        if let Some(task) = self.streaming_task.take() {
            task.abort();
        }
    }
}

#[async_trait]
impl Instrument for PVCAMCameraV3 {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> InstrumentState {
        self.state
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Uninitialized {
            return Err(anyhow!("Camera already initialized"));
        }

        // Initialize SDK
        self.sdk.init()?;

        // Open camera
        let handle = self.sdk.open_camera(&self.camera_name)?;

        // TODO: Get sensor size from SDK once API is available
        // For now, use default from constructor

        // Update ROI parameter constraint to match sensor
        let mut roi_param = self.roi.write().await;
        let default_roi = Roi {
            x: 0,
            y: 0,
            width: self.sensor_size.0,
            height: self.sensor_size.1,
        };
        roi_param.set(default_roi).await?;
        drop(roi_param);

        self.refresh_parameters_from_hardware(&handle)
            .await
            .with_context(|| "Failed to synchronize PVCAM parameters during initialization")?;
        self.camera_handle = Some(handle);
        self.state = InstrumentState::Idle;

        log::info!(
            "PVCAM '{}' initialized: camera '{}', sensor {}x{}",
            self.id,
            self.camera_name,
            self.sensor_size.0,
            self.sensor_size.1
        );

        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        // Stop acquisition if running
        if self.is_acquiring {
            self.stop_streaming_task();
            self.is_acquiring = false;
        }

        // Close camera
        if let Some(handle) = self.camera_handle.take() {
            self.sdk.close_camera(handle)?;
        }

        // Uninitialize SDK
        self.sdk.uninit()?;

        self.state = InstrumentState::ShuttingDown;
        log::info!("PVCAM '{}' shutdown", self.id);

        Ok(())
    }

    fn data_channel(&self) -> broadcast::Receiver<Measurement> {
        self.data_tx.subscribe()
    }

    async fn execute(&mut self, cmd: Command) -> Result<Response> {
        match cmd {
            Command::Start => {
                self.start_acquisition().await?;
                Ok(Response::Ok)
            }

            Command::Stop => {
                self.stop_acquisition().await?;
                Ok(Response::Ok)
            }

            Command::Pause => {
                if self.state != InstrumentState::Running {
                    return Err(anyhow!("Cannot pause from {:?} state", self.state));
                }
                self.stop_streaming_task();
                self.state = InstrumentState::Paused;
                Ok(Response::Ok)
            }

            Command::Resume => {
                if self.state != InstrumentState::Paused {
                    return Err(anyhow!("Cannot resume from {:?} state", self.state));
                }
                self.start_streaming_task();
                self.state = InstrumentState::Running;
                Ok(Response::Ok)
            }

            Command::GetState => Ok(Response::State(self.state)),

            Command::GetParameter(name) => {
                let value = match name.as_str() {
                    "exposure_ms" => serde_json::to_value(self.exposure_ms.read().await.get())?,
                    "roi" => serde_json::to_value(self.roi.read().await.get())?,
                    "binning" => serde_json::to_value(self.binning.read().await.get())?,
                    "gain" => serde_json::to_value(self.gain.read().await.get())?,
                    "trigger_mode" => serde_json::to_value(self.trigger_mode.read().await.get())?,
                    "camera_name" => serde_json::to_value(&self.camera_name)?,
                    "sensor_size" => serde_json::to_value(self.sensor_size)?,
                    _ => return Ok(Response::Error(format!("Unknown parameter: {}", name))),
                };
                Ok(Response::Parameter(value))
            }

            Command::SetParameter(name, value) => {
                match name.as_str() {
                    "exposure_ms" => {
                        let val: f64 = serde_json::from_value(value)?;
                        self.set_exposure(val).await?;
                    }
                    "roi" => {
                        let val: Roi = serde_json::from_value(value)?;
                        self.set_roi(val).await?;
                    }
                    "binning" => {
                        let val: (u32, u32) = serde_json::from_value(value)?;
                        self.set_binning(val.0, val.1).await?;
                    }
                    "gain" => {
                        let val: u32 = serde_json::from_value(value)?;
                        self.set_gain(val).await?;
                    }
                    "trigger_mode" => {
                        let val: String = serde_json::from_value(value)?;
                        self.set_trigger_mode_internal(&val).await?;
                    }
                    _ => return Ok(Response::Error(format!("Unknown parameter: {}", name))),
                }
                Ok(Response::Ok)
            }

            Command::Custom(cmd, _data) => match cmd.as_str() {
                "get_frame_stats" => {
                    let (total, dropped, last) = self.frame_stats();
                    let stats = serde_json::json!({
                        "total_frames": total,
                        "dropped_frames": dropped,
                        "last_frame_number": last,
                    });
                    Ok(Response::Custom(stats))
                }
                _ => Ok(Response::Error(format!("Unknown custom command: {}", cmd))),
            },
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
impl Camera for PVCAMCameraV3 {
    async fn set_exposure(&mut self, ms: f64) -> Result<()> {
        {
            let param = self.exposure_ms.read().await;
            param.constraints().validate(&ms)?;
        }

        if let Some(ref handle) = self.camera_handle {
            let hardware_value = Self::exposure_to_u16(ms)?;
            self.sdk
                .set_param_u16(handle, PvcamParam::Exposure, hardware_value)
                .with_context(|| "Failed to write exposure to PVCAM")?;
            let confirmed = self
                .sdk
                .get_param_u16(handle, PvcamParam::Exposure)
                .with_context(|| "Failed to verify exposure from PVCAM")?
                as f64;
            self.exposure_ms.write().await.set(confirmed).await?;
        } else {
            self.exposure_ms.write().await.set(ms).await?;
        }
        Ok(())
    }

    async fn set_roi(&mut self, roi: Roi) -> Result<()> {
        // Validate ROI is within sensor bounds
        if roi.x + roi.width > self.sensor_size.0 || roi.y + roi.height > self.sensor_size.1 {
            return Err(anyhow!(
                "ROI {}x{} at ({},{}) exceeds sensor size {}x{}",
                roi.width,
                roi.height,
                roi.x,
                roi.y,
                self.sensor_size.0,
                self.sensor_size.1
            ));
        }

        if let Some(ref handle) = self.camera_handle {
            let binning = self.binning.read().await.get();
            let (roi_actual, binning_actual) = self.apply_roi_to_hardware(handle, roi, binning)?;
            self.roi.write().await.set(roi_actual).await?;
            self.binning.write().await.set(binning_actual).await?;
        } else {
            self.roi.write().await.set(roi).await?;
        }

        Ok(())
    }

    async fn roi(&self) -> Roi {
        self.roi.read().await.get()
    }

    async fn set_binning(&mut self, h: u32, v: u32) -> Result<()> {
        if h == 0 || v == 0 {
            return Err(anyhow!("Binning values must be greater than zero"));
        }

        if let Some(ref handle) = self.camera_handle {
            let roi_current = self.roi.read().await.get();
            let (roi_actual, binning_actual) =
                self.apply_roi_to_hardware(handle, roi_current, (h, v))?;
            self.roi.write().await.set(roi_actual).await?;
            self.binning.write().await.set(binning_actual).await?;
        } else {
            self.binning.write().await.set((h, v)).await?;
        }

        Ok(())
    }

    async fn start_acquisition(&mut self) -> Result<()> {
        if self.state != InstrumentState::Idle {
            return Err(anyhow!(
                "Cannot start acquisition from {:?} state",
                self.state
            ));
        }

        if self.camera_handle.is_none() {
            return Err(anyhow!("Camera not initialized"));
        }

        let handle = self.camera_handle.unwrap();

        // Get current settings and apply to SDK
        let exposure = self.exposure_ms.read().await.get();
        let roi = self.roi.read().await.get();
        let binning = self.binning.read().await.get();

        // Set exposure
        self.sdk.set_param_u16(
            &handle,
            PvcamParam::Exposure,
            Self::exposure_to_u16(exposure)?,
        )?;

        // Set ROI with binning
        let region = Self::roi_to_region(&roi, binning)?;
        self.sdk
            .set_param_region(&handle, PvcamParam::Roi, region)?;

        // Apply gain
        let gain_value = self.gain.read().await.get();
        self.sdk
            .set_param_u16(&handle, PvcamParam::Gain, gain_value as u16)?;

        // Apply trigger mode
        let trigger_value = self.trigger_mode.read().await.get();
        let trigger_mode = Self::parse_trigger_mode(&trigger_value)?;
        self.sdk
            .set_param_u16(&handle, PvcamParam::ExposureMode, trigger_mode.as_u16())?;

        // Start acquisition (returns receiver and guard)
        let (receiver, guard) = self.sdk.clone().start_acquisition(handle)?;

        // Store receiver and guard
        self.frame_receiver = Some(receiver);
        self.acquisition_guard = Some(guard);

        // Reset counters
        self.total_frames.store(0, Ordering::Relaxed);
        self.dropped_frames.store(0, Ordering::Relaxed);
        self.last_frame_number.store(u32::MAX, Ordering::Relaxed); // Use sentinel value
        *self.acquisition_start_time.lock().await = Some(Instant::now());

        // Start streaming task
        self.start_streaming_task();
        self.is_acquiring = true;
        self.state = InstrumentState::Running;

        log::info!(
            "PVCAM '{}' started acquisition: {}x{} @ {} ms, bin {}x{}",
            self.id,
            roi.width,
            roi.height,
            exposure,
            binning.0,
            binning.1
        );

        Ok(())
    }

    async fn stop_acquisition(&mut self) -> Result<()> {
        // Stop streaming task (which drops acquisition guard)
        self.stop_streaming_task();

        self.is_acquiring = false;
        self.state = InstrumentState::Idle;

        let (total, dropped, _) = self.frame_stats();
        log::info!(
            "PVCAM '{}' stopped acquisition: {} frames ({} dropped)",
            self.id,
            total,
            dropped
        );

        Ok(())
    }

    async fn arm_trigger(&mut self) -> Result<()> {
        // PVCAM SDK doesn't have separate arm_trigger method
        // Trigger is configured via ExposureMode parameter
        log::debug!(
            "PVCAM '{}': arm_trigger is no-op (trigger configured via mode)",
            self.id
        );
        Ok(())
    }

    async fn trigger(&mut self) -> Result<()> {
        // Software trigger not directly exposed in SDK trait
        // Would require trigger mode = SoftwareEdge and frame waiting
        log::warn!(
            "PVCAM '{}': software_trigger not implemented in SDK",
            self.id
        );
        Err(anyhow!("Software trigger not implemented"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pvcam_v3_initialization() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
        assert_eq!(camera.state(), InstrumentState::Uninitialized);

        camera.initialize().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Idle);
    }

    #[tokio::test]
    async fn test_pvcam_v3_exposure_setting() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
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
    async fn test_pvcam_v3_roi() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
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
    async fn test_pvcam_v3_acquisition() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
        camera.initialize().await.unwrap();

        // Subscribe to data
        let mut rx = camera.data_channel();

        // Start acquisition
        camera.start_acquisition().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Running);

        // Receive a few frames
        for _ in 0..3 {
            let measurement = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .expect("Timeout waiting for frame")
                .unwrap();

            match measurement {
                Measurement::Image { name, buffer, .. } => {
                    assert!(name.contains("test_pvcam"));
                    match buffer {
                        PixelBuffer::U16(data) => {
                            assert!(!data.is_empty());
                        }
                        _ => panic!("Expected U16 buffer"),
                    }
                }
                _ => panic!("Expected Image measurement"),
            }
        }

        // Stop acquisition
        camera.stop_acquisition().await.unwrap();
        assert_eq!(camera.state(), InstrumentState::Idle);
    }

    #[tokio::test]
    async fn test_pvcam_v3_frame_stats() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
        camera.initialize().await.unwrap();

        let mut rx = camera.data_channel();

        camera.start_acquisition().await.unwrap();

        // Receive 5 frames
        for _ in 0..5 {
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .unwrap()
                .unwrap();
        }

        camera.stop_acquisition().await.unwrap();

        // Check stats
        let (total, _dropped, _last) = camera.frame_stats();
        assert_eq!(total, 5);
    }

    #[tokio::test]
    async fn test_pvcam_v3_parameter_validation() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
        camera.initialize().await.unwrap();

        // Invalid exposure should fail
        assert!(camera.set_exposure(0.0).await.is_err());

        // Valid exposure should work
        camera.set_exposure(1000.0).await.unwrap();
    }

    #[tokio::test]
    async fn test_pvcam_v3_gain_command_path() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
        camera.initialize().await.unwrap();

        camera
            .execute(Command::SetParameter(
                "gain".to_string(),
                serde_json::json!(3),
            ))
            .await
            .unwrap();

        let response = camera
            .execute(Command::GetParameter("gain".to_string()))
            .await
            .unwrap();

        match response {
            Response::Parameter(val) => {
                let gain: u32 = serde_json::from_value(val).unwrap();
                assert_eq!(gain, 3);
            }
            other => panic!("Expected gain parameter response, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_pvcam_v3_trigger_mode_updates() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
        camera.initialize().await.unwrap();

        camera
            .execute(Command::SetParameter(
                "trigger_mode".to_string(),
                serde_json::json!("strobed"),
            ))
            .await
            .unwrap();

        let response = camera
            .execute(Command::GetParameter("trigger_mode".to_string()))
            .await
            .unwrap();

        match response {
            Response::Parameter(val) => {
                let mode: String = serde_json::from_value(val).unwrap();
                assert_eq!(mode, "strobed");
            }
            other => panic!("Expected trigger_mode parameter response, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_pvcam_v3_set_binning_updates_parameters() {
        let mut camera = PVCAMCameraV3::new("test_pvcam", "PrimeBSI");
        camera.initialize().await.unwrap();

        camera.set_binning(2, 2).await.unwrap();

        let binning = camera.binning.read().await.get();
        assert_eq!(binning, (2, 2));

        let response = camera
            .execute(Command::GetParameter("roi".to_string()))
            .await
            .unwrap();
        match response {
            Response::Parameter(val) => {
                let roi: Roi = serde_json::from_value(val).unwrap();
                assert_eq!(roi.x, 0);
                assert!(roi.width >= 1);
            }
            other => panic!("Expected ROI parameter response, got {:?}", other),
        }
    }
}
