//! Type definitions for the image viewer panel.
//!
//! Contains supporting types, enums, and channel helpers used by the
//! image viewer panel and its submodules.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use crate::widgets::ParameterCache;
use protocol::daq::FrameData;

/// Maximum frame queue depth (prevents memory buildup if GUI is slow)
/// We only keep the latest frame anyway, so 4 frames is sufficient
/// (1 in flight, 1 being processed, 2 buffer for timing jitter)
pub(super) const MAX_QUEUED_FRAMES: usize = 4;

/// Debounce interval for live exposure updates (200ms)
pub(super) const EXPOSURE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(200);

/// Streaming metrics from server (bd-7rk0: gRPC improvements)
///
/// Note: Some fields populated from proto but not yet displayed in UI.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct StreamMetrics {
    /// Current frames per second
    pub current_fps: f64,
    /// Total frames sent by server
    pub frames_sent: u64,
    /// Frames dropped by server (slow client)
    pub frames_dropped: u64,
    /// Average latency from capture to send (server-side)
    pub avg_latency_ms: f64,
}

/// Frame update message for async integration
#[derive(Debug, Clone)]
pub struct FrameUpdate {
    pub device_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub data: Arc<[u8]>,
    pub frame_number: u64,
    /// Timestamp in nanoseconds (for frame timing analysis)
    #[allow(dead_code)]
    pub timestamp_ns: u64,
    /// Streaming metrics from server (bd-7rk0)
    pub metrics: Option<StreamMetrics>,
}

impl From<FrameData> for FrameUpdate {
    fn from(frame: FrameData) -> Self {
        let metrics = frame.metrics.map(|m| StreamMetrics {
            current_fps: m.current_fps,
            frames_sent: m.frames_sent,
            frames_dropped: m.frames_dropped,
            avg_latency_ms: m.avg_latency_ms,
        });

        Self {
            device_id: frame.device_id,
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            data: frame.data.into(),
            frame_number: frame.frame_number,
            timestamp_ns: frame.timestamp_ns,
            metrics,
        }
    }
}

/// Result of an async parameter load operation
pub struct ParamLoadResult {
    pub device_id: String,
    pub params: Vec<ParameterCache>,
    pub errors: Vec<(String, String)>, // (param_name, error)
}

/// Result of an async parameter set operation
pub struct ParamSetResult {
    pub device_id: String,
    pub param_name: String,
    pub success: bool,
    pub actual_value: String,
    pub error: Option<String>,
}

/// Sender for pushing frame updates from async tasks
pub type FrameUpdateSender = mpsc::SyncSender<FrameUpdate>;

/// Receiver for frame updates in the panel
pub type FrameUpdateReceiver = mpsc::Receiver<FrameUpdate>;

/// Create a new bounded channel pair for frame updates
/// Using a small buffer prevents memory growth when UI can't keep up
pub fn frame_channel() -> (FrameUpdateSender, FrameUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_FRAMES)
}

/// Get display label for stream quality
pub(super) fn stream_quality_label(quality: protocol::daq::StreamQuality) -> &'static str {
    match quality {
        protocol::daq::StreamQuality::Full => "Full",
        protocol::daq::StreamQuality::Preview => "Preview (2x)",
        protocol::daq::StreamQuality::Fast => "Fast (4x)",
    }
}

/// Stream subscription handle (for future external stream control)
#[allow(dead_code)]
pub struct FrameStreamSubscription {
    pub(super) cancel_tx: tokio::sync::mpsc::Sender<()>,
    pub(super) device_id: String,
}

#[allow(dead_code)]
impl FrameStreamSubscription {
    /// Cancel this subscription
    pub async fn cancel(self) {
        let _ = self.cancel_tx.send(()).await;
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }
}

/// FPS calculation state
pub(super) struct FpsCounter {
    frame_times: std::collections::VecDeque<Instant>,
    max_samples: usize,
}

impl FpsCounter {
    pub(super) fn new(max_samples: usize) -> Self {
        Self {
            frame_times: std::collections::VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    pub(super) fn tick(&mut self) {
        let now = Instant::now();
        self.frame_times.push_back(now);
        while self.frame_times.len() > self.max_samples {
            self.frame_times.pop_front();
        }
    }

    pub(super) fn fps(&self) -> f32 {
        if self.frame_times.len() < 2 {
            return 0.0;
        }
        let (Some(first), Some(last)) = (self.frame_times.front(), self.frame_times.back()) else {
            return 0.0;
        };
        let duration = last.duration_since(*first).as_secs_f32();
        if duration > 0.0 {
            (self.frame_times.len() - 1) as f32 / duration
        } else {
            0.0
        }
    }
}

/// Async action result for ImageViewerPanel
pub(super) enum ImageViewerAction {
    /// List of available camera devices
    CamerasLoaded(Vec<String>),
    /// Error from async operation
    Error(String),
    /// Reconnection attempt result (bd-12qt) - construction TODO
    #[allow(dead_code)]
    ReconnectResult { device_id: String, success: bool },
    /// Recording started (bd-3pdi.5.3)
    RecordingStarted { output_path: String },
    /// Recording stopped (bd-3pdi.5.3)
    RecordingStopped {
        output_path: String,
        file_size_bytes: u64,
        total_samples: u64,
    },
    /// Recording status update (bd-3pdi.5.3)
    RecordingStatus(Option<protocol::daq::RecordingStatus>),
}

/// Connection state for camera device (bd-12qt)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// No device selected or initial state
    #[default]
    Idle,
    /// Connected and streaming normally
    Connected,
    /// Device disconnected or error occurred
    Disconnected,
    /// Attempting to reconnect
    Reconnecting,
}

/// Recording state for camera frames (bd-3pdi.5.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecordingState {
    /// Not recording
    #[default]
    Idle,
    /// Actively recording frames
    Recording,
    /// Starting recording (async in progress)
    Starting,
    /// Stopping recording (async in progress)
    Stopping,
}
