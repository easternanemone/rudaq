//! PVCAM Camera Actor (V4 Architecture)
//!
//! Kameo actor implementation wrapping the PVCAM SDK for camera control.
//! Implements the CameraSensor trait for hardware-agnostic camera operations.

use anyhow::Result;
use async_trait::async_trait;
use kameo::{
    actor::{ActorRef, WeakActorRef},
    error::BoxSendError,
    message::{Context, Message},
    Actor,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::hardware::pvcam_adapter::{
    AcquisitionGuard, CameraHandle, MockPvcamAdapter, PvcamAdapter, PvcamFrame, PxRegion,
};
use crate::traits::camera_sensor::{
    BinningConfig, CameraCapabilities, CameraSensor, CameraStreamConfig, CameraTiming, Frame,
    PixelFormat, RegionOfInterest, TriggerMode,
};

/// PVCAM camera actor implementing CameraSensor trait
pub struct PVCAMActor {
    pub id: String,
    pub adapter: Arc<dyn PvcamAdapter>,
    pub camera_handle: Option<CameraHandle>,
    pub camera_name: String,

    // Camera configuration
    pub roi: RegionOfInterest,
    pub binning: BinningConfig,
    pub timing: CameraTiming,
    pub gain: u8,
    pub sensor_width: u32,
    pub sensor_height: u32,

    // Streaming state
    pub streaming: bool,
    pub stream_task: Option<tokio::task::JoinHandle<()>>,
    pub stream_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl PVCAMActor {
    /// Create PVCAM actor with mock adapter
    pub fn mock(id: String, camera_name: String) -> Self {
        Self {
            id,
            adapter: Arc::new(MockPvcamAdapter::new()),
            camera_handle: None,
            camera_name,
            roi: RegionOfInterest::full_sensor(2048, 2048),
            binning: BinningConfig { x_bin: 1, y_bin: 1 },
            timing: CameraTiming {
                exposure_us: 100_000, // 100ms
                frame_period_ms: 110.0,
                trigger_mode: TriggerMode::Internal,
            },
            gain: 1,
            sensor_width: 2048,
            sensor_height: 2048,
            streaming: false,
            stream_task: None,
            stream_shutdown_tx: None,
        }
    }

    /// Convert V4 ROI to PVCAM PxRegion
    fn roi_to_px_region(&self, roi: &RegionOfInterest, binning: &BinningConfig) -> PxRegion {
        PxRegion {
            s1: roi.x as u16,
            s2: (roi.x + roi.width - 1) as u16,
            sbin: binning.x_bin as u16,
            p1: roi.y as u16,
            p2: (roi.y + roi.height - 1) as u16,
            pbin: binning.y_bin as u16,
        }
    }

    /// Convert PVCAM Frame to V4 Frame
    fn pvcam_frame_to_v4(pvcam_frame: PvcamFrame) -> Frame {
        // Convert u16 pixel data to u8 bytes (little-endian)
        let pixel_data: Vec<u8> = pvcam_frame
            .data
            .iter()
            .flat_map(|&pixel| pixel.to_le_bytes())
            .collect();

        Frame {
            timestamp_ns: pvcam_frame.timestamp_ns,
            frame_number: pvcam_frame.frame_number as u64,
            pixel_format: PixelFormat::Mono16,
            width: pvcam_frame.roi.2 as u32,
            height: pvcam_frame.roi.3 as u32,
            roi: RegionOfInterest {
                x: pvcam_frame.roi.0 as u32,
                y: pvcam_frame.roi.1 as u32,
                width: pvcam_frame.roi.2 as u32,
                height: pvcam_frame.roi.3 as u32,
            },
            pixel_data,
        }
    }
}

impl Actor for PVCAMActor {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        info!("PVCAM actor {} starting (camera: {})", args.id, args.camera_name);

        let mut actor = args;

        // Initialize PVCAM adapter - graceful error handling like SCPI
        if let Err(e) = actor.adapter.init() {
            warn!("PVCAM adapter init failed: {}, continuing in degraded mode", e);
            return Ok(actor);
        }

        // Open camera
        match actor.adapter.open_camera(&actor.camera_name) {
            Ok(handle) => {
                actor.camera_handle = Some(handle);

                // Configure initial parameters
                let exposure_ms = (actor.timing.exposure_us / 1000) as u16;
                let _ = actor.adapter.set_exposure(handle, exposure_ms);
                let _ = actor.adapter.set_gain(handle, actor.gain as u16);

                let px_region = actor.roi_to_px_region(&actor.roi, &actor.binning);
                let _ = actor.adapter.set_roi(handle, px_region);

                info!(
                    "PVCAM camera '{}' initialized with handle {:?}",
                    actor.camera_name, handle
                );
            }
            Err(e) => {
                warn!("Failed to open PVCAM camera '{}': {}", actor.camera_name, e);
            }
        }

        Ok(actor)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        info!("PVCAM actor {} stopping", self.id);

        // Stop streaming if active
        if self.streaming {
            if let Some(tx) = self.stream_shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(handle) = self.stream_task.take() {
                let _ = handle.await;
            }
        }

        // Close camera and uninitialize adapter
        if let Some(handle) = self.camera_handle.take() {
            let _ = self.adapter.close_camera(handle);
        }

        let _ = self.adapter.uninit();

        info!("PVCAM actor {} stopped", self.id);
        Ok(())
    }
}

#[async_trait]
impl CameraSensor for PVCAMActor {
    async fn start_stream(&self, config: CameraStreamConfig) -> Result<()> {
        anyhow::bail!("Streaming not yet implemented - use Kameo messages");
    }

    async fn stop_stream(&self) -> Result<()> {
        anyhow::bail!("Streaming not yet implemented - use Kameo messages");
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }

    async fn snap_frame(&self, config: &CameraTiming) -> Result<Frame> {
        anyhow::bail!("Snap frame not yet implemented - use Kameo messages");
    }

    async fn configure_roi(&self, roi: RegionOfInterest) -> Result<()> {
        anyhow::bail!("Configure ROI not yet implemented - use Kameo messages");
    }

    async fn set_timing(&self, timing: CameraTiming) -> Result<()> {
        anyhow::bail!("Set timing not yet implemented - use Kameo messages");
    }

    async fn set_gain(&self, gain: u8) -> Result<()> {
        anyhow::bail!("Set gain not yet implemented - use Kameo messages");
    }

    async fn set_binning(&self, binning: BinningConfig) -> Result<()> {
        anyhow::bail!("Set binning not yet implemented - use Kameo messages");
    }

    fn get_capabilities(&self) -> CameraCapabilities {
        CameraCapabilities {
            sensor_width: self.sensor_width,
            sensor_height: self.sensor_height,
            pixel_formats: vec![PixelFormat::Mono16],
            max_binning_x: 8,
            max_binning_y: 8,
            min_exposure_us: 100,        // 0.1ms
            max_exposure_us: 10_000_000, // 10 seconds
            max_frame_rate_hz: 100.0,
        }
    }
}

// ============================================================================
// Kameo Message Types
// ============================================================================

/// Start continuous frame acquisition
#[derive(Debug, Clone)]
pub struct StartStream {
    pub config: CameraStreamConfig,
}

impl Message<StartStream> for PVCAMActor {
    type Reply = Result<mpsc::Receiver<Frame>>;

    async fn handle(&mut self, msg: StartStream, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        if self.streaming {
            anyhow::bail!("Already streaming");
        }

        let handle = self
            .camera_handle
            .ok_or_else(|| anyhow::anyhow!("Camera not initialized"))?;

        // Apply configuration
        let exposure_ms = (msg.config.timing.exposure_us / 1000) as u16;
        self.adapter.set_exposure(handle, exposure_ms)?;
        self.adapter.set_gain(handle, msg.config.gain as u16)?;

        let px_region = self.roi_to_px_region(&msg.config.roi, &msg.config.binning);
        self.adapter.set_roi(handle, px_region)?;

        // Start PVCAM adapter acquisition
        let (mut pvcam_rx, guard) = self.adapter.clone().start_acquisition(handle)?;

        // Create channel for V4 frames
        let (tx, rx) = mpsc::channel(16);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        // Spawn task to convert PVCAM frames to V4 frames
        let task = tokio::spawn(async move {
            let _guard = guard; // RAII guard stops acquisition when dropped

            loop {
                tokio::select! {
                    Some(pvcam_frame) = pvcam_rx.recv() => {
                        let v4_frame = Self::pvcam_frame_to_v4(pvcam_frame);
                        if tx.send(v4_frame).await.is_err() {
                            info!("Frame receiver dropped, stopping stream");
                            break;
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("Stream shutdown requested");
                        break;
                    }
                    else => {
                        warn!("PVCAM frame channel closed");
                        break;
                    }
                }
            }
        });

        self.streaming = true;
        self.stream_task = Some(task);
        self.stream_shutdown_tx = Some(shutdown_tx);

        // Update internal state
        self.roi = msg.config.roi;
        self.binning = msg.config.binning;
        self.timing = msg.config.timing;
        self.gain = msg.config.gain;

        info!("PVCAM streaming started");
        Ok(rx)
    }
}

/// Stop continuous frame acquisition
#[derive(Debug, Clone)]
pub struct StopStream;

impl Message<StopStream> for PVCAMActor {
    type Reply = Result<()>;

    async fn handle(&mut self, _msg: StopStream, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        if !self.streaming {
            return Ok(());
        }

        // Signal shutdown
        if let Some(tx) = self.stream_shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Wait for task to finish
        if let Some(handle) = self.stream_task.take() {
            let _ = handle.await;
        }

        self.streaming = false;
        info!("PVCAM streaming stopped");
        Ok(())
    }
}

/// Acquire single frame
#[derive(Debug, Clone)]
pub struct SnapFrame {
    pub timing: CameraTiming,
}

impl Message<SnapFrame> for PVCAMActor {
    type Reply = Result<Frame>;

    async fn handle(&mut self, msg: SnapFrame, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        if self.streaming {
            anyhow::bail!("Cannot snap while streaming");
        }

        let handle = self
            .camera_handle
            .ok_or_else(|| anyhow::anyhow!("Camera not initialized"))?;

        // Set exposure
        let exposure_ms = (msg.timing.exposure_us / 1000) as u16;
        self.adapter.set_exposure(handle, exposure_ms)?;

        // Start single-frame acquisition
        let (mut pvcam_rx, guard) = self.adapter.clone().start_acquisition(handle)?;

        // Wait for first frame with timeout
        let frame_result = tokio::time::timeout(
            Duration::from_secs(5),
            pvcam_rx.recv(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for frame"))?
        .ok_or_else(|| anyhow::anyhow!("Frame channel closed"))?;

        // Stop acquisition (guard will stop when dropped)
        drop(guard);

        let v4_frame = Self::pvcam_frame_to_v4(frame_result);
        Ok(v4_frame)
    }
}

/// Configure ROI
#[derive(Debug, Clone)]
pub struct ConfigureROI {
    pub roi: RegionOfInterest,
}

impl Message<ConfigureROI> for PVCAMActor {
    type Reply = Result<()>;

    async fn handle(&mut self, msg: ConfigureROI, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        if self.streaming {
            anyhow::bail!("Cannot change ROI while streaming");
        }

        let handle = self
            .camera_handle
            .ok_or_else(|| anyhow::anyhow!("Camera not initialized"))?;

        // Validate ROI
        if msg.roi.x + msg.roi.width > self.sensor_width {
            anyhow::bail!("ROI exceeds sensor width");
        }
        if msg.roi.y + msg.roi.height > self.sensor_height {
            anyhow::bail!("ROI exceeds sensor height");
        }

        let px_region = self.roi_to_px_region(&msg.roi, &self.binning);
        self.adapter.set_roi(handle, px_region)?;

        self.roi = msg.roi;
        info!("ROI configured: {:?}", self.roi);
        Ok(())
    }
}

/// Set timing parameters
#[derive(Debug, Clone)]
pub struct SetTiming {
    pub timing: CameraTiming,
}

impl Message<SetTiming> for PVCAMActor {
    type Reply = Result<()>;

    async fn handle(&mut self, msg: SetTiming, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        if self.streaming {
            anyhow::bail!("Cannot change timing while streaming");
        }

        let handle = self
            .camera_handle
            .ok_or_else(|| anyhow::anyhow!("Camera not initialized"))?;

        // Set exposure (trigger mode handled by adapter)
        let exposure_ms = (msg.timing.exposure_us / 1000) as u16;
        self.adapter.set_exposure(handle, exposure_ms)?;

        self.timing = msg.timing;
        info!("Timing configured: {:?}", self.timing);
        Ok(())
    }
}

/// Set gain
#[derive(Debug, Clone)]
pub struct SetGain {
    pub gain: u8,
}

impl Message<SetGain> for PVCAMActor {
    type Reply = Result<()>;

    async fn handle(&mut self, msg: SetGain, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        if self.streaming {
            anyhow::bail!("Cannot change gain while streaming");
        }

        let handle = self
            .camera_handle
            .ok_or_else(|| anyhow::anyhow!("Camera not initialized"))?;

        self.adapter.set_gain(handle, msg.gain as u16)?;

        self.gain = msg.gain;
        info!("Gain set to {}", self.gain);
        Ok(())
    }
}

/// Set binning
#[derive(Debug, Clone)]
pub struct SetBinning {
    pub binning: BinningConfig,
}

impl Message<SetBinning> for PVCAMActor {
    type Reply = Result<()>;

    async fn handle(&mut self, msg: SetBinning, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        if self.streaming {
            anyhow::bail!("Cannot change binning while streaming");
        }

        let handle = self
            .camera_handle
            .ok_or_else(|| anyhow::anyhow!("Camera not initialized"))?;

        // Update ROI with new binning
        let px_region = self.roi_to_px_region(&self.roi, &msg.binning);
        self.adapter.set_roi(handle, px_region)?;

        self.binning = msg.binning;
        info!("Binning set to {}x{}", self.binning.x_bin, self.binning.y_bin);
        Ok(())
    }
}

/// Get camera capabilities
#[derive(Debug, Clone)]
pub struct GetCapabilities;

impl Message<GetCapabilities> for PVCAMActor {
    type Reply = Result<CameraCapabilities>;

    async fn handle(&mut self, _msg: GetCapabilities, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        Ok(self.get_capabilities())
    }
}

/// Check if streaming is active
#[derive(Debug, Clone)]
pub struct IsStreaming;

impl Message<IsStreaming> for PVCAMActor {
    type Reply = Result<bool>;

    async fn handle(&mut self, _msg: IsStreaming, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        Ok(self.streaming)
    }
}
