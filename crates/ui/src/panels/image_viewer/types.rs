//! Type definitions for the image viewer panel.
//!
//! Contains supporting types, enums, and channel helpers used by the
//! image viewer panel and its submodules.

use crate::time::Instant;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;

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
///
/// Uses `Arc<Vec<u8>>` instead of `Arc<[u8]>` to avoid a memcpy during
/// `Vec<u8> → Arc` conversion. `Arc<[u8]>` is a DST requiring layout
/// conversion (header + inline data), while `Arc<Vec<u8>>` wraps the
/// existing heap allocation with only a pointer-sized indirection.
#[derive(Debug, Clone)]
pub struct FrameUpdate {
    pub device_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub data: Arc<Vec<u8>>,
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
            data: Arc::new(frame.data),
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
    pub favorites: Vec<String>,        // favorited parameter names (bd-4wf7)
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
pub(crate) struct FpsCounter {
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

    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn fps(&self) -> f32 {
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
    CamerasLoaded {
        ids: Vec<String>,
        full_frame_dims: HashMap<String, (u32, u32)>,
    },
    /// Error from async operation
    Error(String),
    /// Reconnection attempt result (bd-12qt)
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
    /// Result of syncing the active echelle profile into RunEngine readiness state.
    EchelleCalibrationSynced { message: String },
    /// Error while syncing the active echelle profile into RunEngine readiness state.
    EchelleCalibrationSyncError(String),
}

/// Pixel statistics for the current frame (bd-li4i)
#[derive(Debug, Clone, Default)]
pub struct PixelStatistics {
    /// Total number of pixels
    pub count: u64,
    /// Minimum pixel value
    pub min: f64,
    /// Maximum pixel value
    pub max: f64,
    /// Mean pixel value
    pub mean: f64,
    /// Standard deviation
    pub std_dev: f64,
    /// Median pixel value
    pub median: f64,
    /// Sum of all pixel values
    pub sum: f64,
    /// 1st percentile
    pub p1: f64,
    /// 5th percentile
    pub p5: f64,
    /// 25th percentile (Q1)
    pub p25: f64,
    /// 50th percentile (same as median)
    pub p50: f64,
    /// 75th percentile (Q3)
    pub p75: f64,
    /// 95th percentile
    pub p95: f64,
    /// 99th percentile
    pub p99: f64,
}

impl PixelStatistics {
    /// Format statistics as a human-readable text block for clipboard copy.
    pub fn to_clipboard_text(&self) -> String {
        format!(
            "Pixel Statistics\n\
             ================\n\
             Count:    {}\n\
             Min:      {:.1}\n\
             Max:      {:.1}\n\
             Mean:     {:.2}\n\
             Std Dev:  {:.2}\n\
             Median:   {:.1}\n\
             Sum:      {:.0}\n\
             \n\
             Percentiles\n\
             -----------\n\
             P1:       {:.1}\n\
             P5:       {:.1}\n\
             P25 (Q1): {:.1}\n\
             P50:      {:.1}\n\
             P75 (Q3): {:.1}\n\
             P95:      {:.1}\n\
             P99:      {:.1}",
            self.count,
            self.min,
            self.max,
            self.mean,
            self.std_dev,
            self.median,
            self.sum,
            self.p1,
            self.p5,
            self.p25,
            self.p50,
            self.p75,
            self.p95,
            self.p99,
        )
    }
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

/// State machine for remote profile load/activate operations (bd-zy7y.1).
///
/// Replaces the previous `pending_remote_profile_load: Option<String>` +
/// `remote_profile_load_rx: Option<Receiver>` pair with explicit states
/// so the UI can distinguish idle, loading, success, and failure.
#[derive(Debug, Default)]
pub(in crate::panels::image_viewer) enum RemoteProfileLoadState {
    /// No load in progress.
    #[default]
    Idle,
    /// A load request has been submitted but the async task hasn't started yet.
    /// The path is stored so the UI can show what's being loaded.
    Pending { path: String },
    /// The async gRPC call is in flight. The receiver will deliver the result.
    Loading {
        path: String,
        rx: std::sync::mpsc::Receiver<Result<String, String>>,
    },
    /// Load completed successfully. Caller should transition to Idle after
    /// processing the profile content.
    Succeeded { path: String },
    /// Load failed with an error message.
    Failed { path: String, error: String },
}

impl RemoteProfileLoadState {
    /// Returns true if a load is pending or in flight.
    pub(super) fn is_busy(&self) -> bool {
        matches!(self, Self::Pending { .. } | Self::Loading { .. })
    }

    /// Returns a user-visible status message for active (in-flight) states only.
    /// Terminal states (Succeeded/Failed) return None because their outcomes
    /// are communicated via `echelle_cal_ui.status_message`/`last_error`.
    pub(super) fn status_message(&self) -> Option<String> {
        match self {
            Self::Pending { path } => Some(format!("Requesting profile: {path}...")),
            Self::Loading { path, .. } => Some(format!("Loading profile from daemon: {path}...")),
            _ => None,
        }
    }

    /// Returns true if the state is terminal (Succeeded/Failed) and should
    /// be reset to Idle after the caller has processed the outcome.
    pub(super) fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded { .. } | Self::Failed { .. })
    }
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
