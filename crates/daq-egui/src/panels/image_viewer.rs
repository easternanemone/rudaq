//! Image Viewer Panel - 2D camera frame visualization
//!
//! Displays live camera frames from FrameProducer devices with:
//! - Real-time frame streaming via gRPC
//! - Configurable colormaps (grayscale, viridis, etc.)
//! - Zoom/pan controls
//! - Frame metadata display (dimensions, FPS, frame count)
//!
//! ## Async Integration Pattern
//!
//! Uses message-passing for thread-safe async updates:
//! - Background task receives frames from gRPC stream
//! - Frames sent to panel via mpsc channel
//! - Panel drains channel each frame and updates texture

use eframe::egui;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

use crate::client::DaqClient;
use crate::icons;
use crate::layout::{self, colors};
use crate::widgets::{Histogram, HistogramPosition, ParameterCache, RoiSelector};
use daq_proto::compression::decompress_frame;
use daq_proto::daq::{FrameData, StreamQuality};

/// Maximum frame queue depth (prevents memory buildup if GUI is slow)
/// We only keep the latest frame anyway, so 4 frames is sufficient
/// (1 in flight, 1 being processed, 2 buffer for timing jitter)
const MAX_QUEUED_FRAMES: usize = 4;

/// Debounce interval for live exposure updates (200ms)
const EXPOSURE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(200);

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
#[derive(Debug)]
pub struct FrameUpdate {
    pub device_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub data: Vec<u8>,
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
            data: frame.data,
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

/// Request for background RGBA conversion (bd-xifj)
struct RgbaConversionRequest {
    /// Raw frame data
    data: Vec<u8>,
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
    /// Display parameters for conversion
    colormap: Colormap,
    scale_mode: ScaleMode,
    display_min: f32,
    display_max: f32,
    auto_contrast: bool,
}

/// Result of background RGBA conversion (bd-xifj)
struct RgbaConversionResult {
    /// Converted RGBA data
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    /// Frame number for debugging and ordering (kept for future use)
    #[allow(dead_code)]
    frame_number: u64,
    /// Computed display min/max (for auto-contrast feedback)
    computed_min: f32,
    computed_max: f32,
}

/// Convert raw frame data to RGBA, reusing the provided buffer (bd-wdx3, bd-xifj)
///
/// This is a free function that can be called from both the UI thread and background threads.
/// The buffer is resized as needed but not shrunk, avoiding allocations for same-size frames.
fn convert_frame_to_rgba_into(req: &RgbaConversionRequest, buffer: &mut Vec<u8>) -> (f32, f32) {
    let width = req.width;
    let height = req.height;
    let bit_depth = req.bit_depth;
    let colormap = req.colormap;
    let scale_mode = req.scale_mode;
    let display_min = req.display_min;
    let display_max = req.display_max;
    let auto_contrast = req.auto_contrast;
    let data = &req.data;

    // Guard against zero or invalid dimensions
    if width == 0 || height == 0 {
        buffer.clear();
        return (0.0, 1.0);
    }

    // Use checked arithmetic to prevent overflow on large dimensions
    let Some(pixel_count) = (width as u64).checked_mul(height as u64) else {
        buffer.clear();
        return (0.0, 1.0);
    };

    // Cap allocation to reasonable size (256 MB max for RGBA)
    const MAX_PIXELS: u64 = 64 * 1024 * 1024; // 64M pixels = 256MB RGBA
    if pixel_count > MAX_PIXELS {
        tracing::warn!(width, height, "Frame too large, capping allocation");
        buffer.clear();
        return (0.0, 1.0);
    }

    let pixel_count = pixel_count as usize;
    let required_size = pixel_count * 4;

    // bd-wdx3: Resize buffer only when needed (grows but never shrinks during session)
    buffer.resize(required_size, 255); // Pre-fill alpha channel

    // Get the bit depth's max value for normalization
    let bit_max = match bit_depth {
        8 => 255.0f32,
        12 => 4095.0,
        16 => 65535.0,
        _ => 65535.0,
    };

    // Compute min/max for auto-contrast
    let (effective_min, effective_max) = if auto_contrast {
        compute_minmax_from_data(data, bit_depth, bit_max)
    } else {
        (display_min, display_max)
    };

    // Compute contrast range (avoid division by zero)
    let range = (effective_max - effective_min).max(0.001);

    match bit_depth {
        8 => {
            // 8-bit grayscale
            for (i, &pixel) in data.iter().take(pixel_count).enumerate() {
                let normalized = pixel as f32 / bit_max;
                let contrasted = ((normalized - effective_min) / range).clamp(0.0, 1.0);
                let scaled = scale_mode.apply(contrasted);
                let [r, g, b] = colormap.apply(scaled);
                buffer[i * 4] = r;
                buffer[i * 4 + 1] = g;
                buffer[i * 4 + 2] = b;
                // Alpha already set to 255
            }
        }
        12 | 16 => {
            // 16-bit (or 12-bit stored as 16-bit) little-endian
            for i in 0..pixel_count {
                let byte_idx = i * 2;
                if byte_idx + 1 >= data.len() {
                    break;
                }
                let pixel = u16::from_le_bytes([data[byte_idx], data[byte_idx + 1]]);
                let normalized = pixel as f32 / bit_max;
                let contrasted = ((normalized - effective_min) / range).clamp(0.0, 1.0);
                let scaled = scale_mode.apply(contrasted);
                let [r, g, b] = colormap.apply(scaled);
                buffer[i * 4] = r;
                buffer[i * 4 + 1] = g;
                buffer[i * 4 + 2] = b;
            }
        }
        _ => {
            // Unknown bit depth - show error pattern (checkerboard)
            let width_usize = width as usize;
            for i in 0..pixel_count {
                let checkerboard = ((i % width_usize) / 16 + (i / width_usize) / 16) % 2;
                let color = if checkerboard == 0 { 255u8 } else { 128u8 };
                buffer[i * 4] = color;
                buffer[i * 4 + 1] = 0;
                buffer[i * 4 + 2] = color;
            }
        }
    }

    (effective_min, effective_max)
}

/// Compute min/max values from frame data for auto-contrast (free function version)
fn compute_minmax_from_data(data: &[u8], bit_depth: u32, bit_max: f32) -> (f32, f32) {
    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;

    match bit_depth {
        8 => {
            for &pixel in data {
                let val = pixel as f32;
                min_val = min_val.min(val);
                max_val = max_val.max(val);
            }
        }
        12 | 16 => {
            for chunk in data.chunks_exact(2) {
                let pixel = u16::from_le_bytes([chunk[0], chunk[1]]);
                let val = pixel as f32;
                min_val = min_val.min(val);
                max_val = max_val.max(val);
            }
        }
        _ => {
            return (0.0, 1.0);
        }
    }

    // Normalize to 0.0-1.0 range
    if min_val < max_val {
        (min_val / bit_max, max_val / bit_max)
    } else {
        (0.0, 1.0)
    }
}

/// Whitelist for quick access camera parameters
const QUICK_ACCESS_PARAMS: &[&str] = &[
    "exposure",    // Matches exposure_ms, exposure_mode, etc.
    "gain",        // Matches gain_index, gain_mode
    "speed",       // Matches speed_index, speed_mode
    "temperature", // Matches temperature, temperature_setpoint
    "fan",         // Matches fan_speed
    "trigger",     // Matches trigger_mode
    "roi",         // Matches roi (hardware)
    "binning",     // Matches binning
];

/// Sender for pushing frame updates from async tasks
pub type FrameUpdateSender = mpsc::SyncSender<FrameUpdate>;

/// Receiver for frame updates in the panel
pub type FrameUpdateReceiver = mpsc::Receiver<FrameUpdate>;

/// Create a new bounded channel pair for frame updates
/// Using a small buffer prevents memory growth when UI can't keep up
pub fn frame_channel() -> (FrameUpdateSender, FrameUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_FRAMES)
}

/// Colormap for image display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    Viridis,
    Inferno,
    Plasma,
    Magma,
}

impl Colormap {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Grayscale => "Grayscale",
            Self::Viridis => "Viridis",
            Self::Inferno => "Inferno",
            Self::Plasma => "Plasma",
            Self::Magma => "Magma",
        }
    }

    /// Apply colormap to a normalized value (0.0-1.0) returning RGB
    /// Uses pre-computed LUT for performance (bd-7rk0)
    #[inline]
    pub fn apply(&self, value: f32) -> [u8; 3] {
        // Convert to 8-bit index (0-255)
        let idx = (value.clamp(0.0, 1.0) * 255.0) as usize;
        self.lut()[idx]
    }

    /// Get the pre-computed LUT for this colormap (256 RGB entries)
    #[inline]
    fn lut(&self) -> &'static [[u8; 3]; 256] {
        match self {
            Self::Grayscale => &GRAYSCALE_LUT,
            Self::Viridis => &VIRIDIS_LUT,
            Self::Inferno => &INFERNO_LUT,
            Self::Plasma => &PLASMA_LUT,
            Self::Magma => &MAGMA_LUT,
        }
    }
}

// Pre-computed colormap lookup tables (bd-7rk0: performance optimization)
// Each LUT has 256 entries for O(1) intensity-to-color mapping

static GRAYSCALE_LUT: [[u8; 3]; 256] = {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        lut[i] = [i as u8, i as u8, i as u8];
        i += 1;
    }
    lut
};

static VIRIDIS_LUT: [[u8; 3]; 256] = compute_viridis_lut();
static INFERNO_LUT: [[u8; 3]; 256] = compute_inferno_lut();
static PLASMA_LUT: [[u8; 3]; 256] = compute_plasma_lut();
static MAGMA_LUT: [[u8; 3]; 256] = compute_magma_lut();

const fn compute_viridis_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        let v = i as f64 / 255.0;
        // Viridis: purple -> blue -> green -> yellow
        let r = (0.267 + v * (0.993 - 0.267)) * 255.0;
        let g = v * 0.906 * 255.0;
        let b = (0.329 + v * 0.186) * 255.0; // Simplified for const fn
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

const fn compute_inferno_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        let v = i as f64 / 255.0;
        // Inferno: black -> purple -> red -> yellow (using sqrt/pow approximations)
        let r = const_sqrt(v) * 255.0;
        let g = v * v * v * 200.0; // powf(1.5) approximated
        let b = (1.0 - v) * v * 4.0 * 255.0;
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

const fn compute_plasma_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        let v = i as f64 / 255.0;
        // Plasma: blue -> purple -> orange -> yellow
        let r = (0.05 + v * 0.95) * 255.0;
        let g = v * v * 255.0;
        let b = (1.0 - v * 0.7) * 255.0;
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

const fn compute_magma_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        let v = i as f64 / 255.0;
        // Magma: black -> purple -> pink -> white
        let r = const_pow_0_7(v) * 255.0;
        let g = v * v * 200.0;
        let b = (0.3 + v * 0.7) * v * 255.0;
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

/// Clamp f64 to u8 range (const fn compatible)
const fn clamp_u8(v: f64) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v as u8
    }
}

/// Const-compatible sqrt approximation using Newton-Raphson
const fn const_sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut guess = x / 2.0;
    let mut i = 0;
    while i < 10 {
        guess = (guess + x / guess) / 2.0;
        i += 1;
    }
    guess
}

/// Const-compatible x^0.7 approximation
const fn const_pow_0_7(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    // x^0.7 ≈ x * x^(-0.3) ≈ x / x^0.3
    // Using sqrt approximation: x^0.5 then interpolate
    let sqrt_x = const_sqrt(x);
    // x^0.7 ≈ sqrt(x) * x^0.2 ≈ sqrt(x) * sqrt(sqrt(x))^0.4
    // Simplified: use linear interpolation between x and sqrt(x)
    // x^0.7 ≈ 0.4*x + 0.6*sqrt(x) (rough approximation)
    sqrt_x * 0.7 + x * 0.3
}

/// Scale mode for pixel intensity mapping
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScaleMode {
    #[default]
    Linear,
    Log,
    Sqrt,
}

impl ScaleMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Log => "Log",
            Self::Sqrt => "Sqrt",
        }
    }

    /// Apply scaling to a normalized value (0.0-1.0)
    pub fn apply(&self, value: f32) -> f32 {
        match self {
            Self::Linear => value,
            Self::Log => (1.0 + value * 99.0).log10() / 2.0, // log10(1-100) -> 0-2 -> 0-1
            Self::Sqrt => value.sqrt(),
        }
    }
}

/// Get display label for stream quality
fn stream_quality_label(quality: StreamQuality) -> &'static str {
    match quality {
        StreamQuality::Full => "Full",
        StreamQuality::Preview => "Preview (2x)",
        StreamQuality::Fast => "Fast (4x)",
    }
}

/// Stream subscription handle (for future external stream control)
#[allow(dead_code)]
pub struct FrameStreamSubscription {
    cancel_tx: tokio::sync::mpsc::Sender<()>,
    device_id: String,
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
struct FpsCounter {
    frame_times: std::collections::VecDeque<Instant>,
    max_samples: usize,
}

impl FpsCounter {
    fn new(max_samples: usize) -> Self {
        Self {
            frame_times: std::collections::VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.frame_times.push_back(now);
        while self.frame_times.len() > self.max_samples {
            self.frame_times.pop_front();
        }
    }

    fn fps(&self) -> f32 {
        if self.frame_times.len() < 2 {
            return 0.0;
        }
        let first = self.frame_times.front().unwrap();
        let last = self.frame_times.back().unwrap();
        let duration = last.duration_since(*first).as_secs_f32();
        if duration > 0.0 {
            (self.frame_times.len() - 1) as f32 / duration
        } else {
            0.0
        }
    }
}

/// Async action result for ImageViewerPanel
enum ImageViewerAction {
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
    RecordingStatus(Option<daq_proto::daq::RecordingStatus>),
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

/// Image Viewer Panel state
pub struct ImageViewerPanel {
    /// Currently selected device ID
    device_id: Option<String>,
    /// Current frame dimensions
    width: u32,
    height: u32,
    /// Current frame bit depth
    bit_depth: u32,
    /// Frame counter
    frame_count: u64,
    /// Cached texture handle
    texture: Option<egui::TextureHandle>,
    /// Current colormap
    colormap: Colormap,
    /// Current scale mode
    scale_mode: ScaleMode,
    /// Zoom level (1.0 = fit to window)
    zoom: f32,
    /// Pan offset
    pan: egui::Vec2,
    /// Frame update receiver
    frame_rx: Option<FrameUpdateReceiver>,
    /// Frame update sender (for cloning to async tasks)
    frame_tx: Option<FrameUpdateSender>,
    /// Active stream subscription
    subscription: Option<FrameStreamSubscription>,
    /// FPS counter
    fps_counter: FpsCounter,
    /// Auto-fit zoom on next frame
    auto_fit: bool,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Max FPS for streaming (rate limit)
    max_fps: u32,
    /// ROI selector state
    roi_selector: RoiSelector,
    /// Last frame raw data (for ROI statistics computation)
    last_frame_data: Option<Vec<u8>>,
    /// Show ROI statistics panel
    show_roi_panel: bool,
    /// Histogram for intensity distribution
    histogram: Histogram,
    /// Histogram display position
    histogram_position: HistogramPosition,
    /// Available camera devices
    available_cameras: Vec<String>,
    /// Display minimum (0.0-1.0 normalized) - pixels at or below this are black
    display_min: f32,
    /// Display maximum (0.0-1.0 normalized) - pixels at or above this are white
    display_max: f32,
    /// Auto-contrast mode - automatically compute min/max from frame data
    auto_contrast: bool,
    /// Async action receiver
    action_rx: std::sync::mpsc::Receiver<ImageViewerAction>,
    /// Async action sender
    action_tx: std::sync::mpsc::Sender<ImageViewerAction>,
    /// Last refresh time
    last_refresh: Option<Instant>,

    // -- Camera Control Fields --
    /// Camera parameters (cached)
    camera_params: Vec<ParameterCache>,
    /// Parameter edit buffers (device_id, param_name) -> value
    param_edit_buffers: std::collections::HashMap<(String, String), String>,
    /// Parameter errors (device_id, param_name) -> error
    param_errors: std::collections::HashMap<(String, String), String>,
    /// Show controls side panel
    show_controls: bool,
    /// Receiver for parameter load results
    param_load_rx: Option<mpsc::Receiver<ParamLoadResult>>,
    /// Sender for parameter set results (persistent, cloned per request)
    param_set_tx: mpsc::Sender<ParamSetResult>,
    /// Receiver for parameter set results
    param_set_rx: mpsc::Receiver<ParamSetResult>,
    /// Parameters currently being set
    setting_params: std::collections::HashSet<(String, String)>,
    /// Pending parameter updates to execute
    pending_param_updates: Vec<(String, String, String)>,
    /// Device ID currently loading parameters
    loading_params_device: Option<String>,
    /// Live exposure preview mode (updates during drag)
    live_exposure: bool,
    /// Last time exposure was sent (for debounce)
    exposure_last_sent: Option<Instant>,

    // -- Connection Resilience Fields (bd-12qt) --
    /// Connection state for the current device
    connection_state: ConnectionState,
    /// Number of consecutive connection failures
    retry_count: u32,
    /// Time of last disconnect (for auto-retry backoff)
    last_disconnect: Option<Instant>,
    /// Enable automatic reconnection attempts
    auto_reconnect: bool,

    // -- Stream Metrics (bd-7rk0: gRPC improvements) --
    /// Latest streaming metrics from server
    stream_metrics: Option<StreamMetrics>,

    // -- Recording Fields (bd-3pdi.5.3) --
    /// Current recording state
    recording_state: RecordingState,
    /// Recording name input
    recording_name: String,
    /// Current output path (when recording)
    recording_output_path: Option<String>,
    /// Recording status from server
    recording_status: Option<daq_proto::daq::RecordingStatus>,
    /// Last recording status poll time
    last_recording_poll: Option<Instant>,

    // -- Stream Quality Settings --
    /// Stream quality level for server-side downsampling
    stream_quality: StreamQuality,

    // -- Background RGBA Conversion (bd-xifj: move CPU work off UI thread) --
    /// Receiver for completed RGBA conversions from background thread
    rgba_rx: Option<std::sync::mpsc::Receiver<RgbaConversionResult>>,
    /// Sender for RGBA conversion requests (cloned to background thread)
    rgba_request_tx: Option<std::sync::mpsc::SyncSender<RgbaConversionRequest>>,
    /// Pending RGBA data ready to be applied to texture
    pending_rgba: Option<RgbaConversionResult>,
    /// Sender to recycle used buffers back to the converter thread (bd-wdx3)
    rgba_recycle_tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
}

impl Default for ImageViewerPanel {
    fn default() -> Self {
        let (tx, rx) = frame_channel();
        let (action_tx, action_rx) = std::sync::mpsc::channel();
        // Persistent channel for parameter set results - sender is cloned per request
        let (param_set_tx, param_set_rx) = mpsc::channel();
        Self {
            device_id: None,
            width: 0,
            height: 0,
            bit_depth: 0,
            frame_count: 0,
            texture: None,
            colormap: Colormap::default(),
            scale_mode: ScaleMode::default(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            frame_rx: Some(rx),
            frame_tx: Some(tx),
            subscription: None,
            fps_counter: FpsCounter::new(30),
            auto_fit: true,
            error: None,
            status: None,
            max_fps: 30,
            roi_selector: RoiSelector::new(),
            last_frame_data: None,
            show_roi_panel: true,
            histogram: Histogram::new(),
            histogram_position: HistogramPosition::BottomRight,
            available_cameras: Vec::new(),
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: true,
            action_rx,
            action_tx,
            last_refresh: None,

            camera_params: Vec::new(),
            param_edit_buffers: std::collections::HashMap::new(),
            param_errors: std::collections::HashMap::new(),
            show_controls: true,
            param_load_rx: None,
            param_set_tx,
            param_set_rx,
            setting_params: std::collections::HashSet::new(),
            pending_param_updates: Vec::new(),
            loading_params_device: None,
            live_exposure: true,
            exposure_last_sent: None,

            // Connection resilience (bd-12qt)
            connection_state: ConnectionState::Idle,
            retry_count: 0,
            last_disconnect: None,
            auto_reconnect: true,

            // Stream metrics (bd-7rk0)
            stream_metrics: None,

            // Recording (bd-3pdi.5.3)
            recording_state: RecordingState::Idle,
            recording_name: String::new(),
            recording_output_path: None,
            recording_status: None,
            last_recording_poll: None,

            // Stream quality for bandwidth control
            stream_quality: StreamQuality::Full,

            // Background RGBA conversion (bd-xifj)
            // Buffer reuse via recycling channel (bd-wdx3)
            rgba_rx: None,
            rgba_request_tx: None,
            pending_rgba: None,
            rgba_recycle_tx: None,
        }
    }
}

impl ImageViewerPanel {
    /// Create a new image viewer panel
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn background thread for RGBA conversion (bd-xifj)
    ///
    /// This moves CPU-intensive pixel conversion off the UI thread to prevent
    /// UI freezes on 4K 16-bit images at high frame rates.
    fn spawn_rgba_converter(&mut self) {
        // Use bounded channel to prevent unbounded queue growth
        // Queue size of 2 is sufficient: 1 processing, 1 waiting
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<RgbaConversionRequest>(2);
        let (result_tx, result_rx) = std::sync::mpsc::channel::<RgbaConversionResult>();
        // Channel for recycling buffers from UI thread back to converter (bd-wdx3)
        let (recycle_tx, recycle_rx) = std::sync::mpsc::channel::<Vec<u8>>();

        self.rgba_request_tx = Some(request_tx);
        self.rgba_rx = Some(result_rx);
        self.rgba_recycle_tx = Some(recycle_tx);

        // Spawn dedicated thread for RGBA conversion
        std::thread::Builder::new()
            .name("rgba-converter".into())
            .spawn(move || {
                tracing::debug!("RGBA converter thread started");

                while let Ok(req) = request_rx.recv() {
                    // Get a buffer to reuse: prefer recycled, else allocate new (bd-wdx3)
                    let mut buffer = recycle_rx
                        .try_recv()
                        .unwrap_or_else(|_| Vec::with_capacity(1920 * 1080 * 4));

                    // Perform CPU-intensive conversion
                    let (computed_min, computed_max) =
                        convert_frame_to_rgba_into(&req, &mut buffer);

                    // Send result back to UI thread - move buffer ownership (no clone!)
                    let result = RgbaConversionResult {
                        rgba: buffer,
                        width: req.width,
                        height: req.height,
                        frame_number: req.frame_number,
                        computed_min,
                        computed_max,
                    };

                    if result_tx.send(result).is_err() {
                        // Receiver dropped, exit thread
                        tracing::debug!("RGBA converter result receiver dropped, exiting");
                        break;
                    }
                }

                tracing::debug!("RGBA converter thread exiting");
            })
            .expect("Failed to spawn RGBA converter thread");
    }

    /// Poll for completed RGBA conversions from background thread (bd-xifj)
    fn poll_rgba_results(&mut self) {
        if let Some(rx) = &self.rgba_rx {
            // Drain all available results, keeping only the most recent
            let mut latest: Option<RgbaConversionResult> = None;
            while let Ok(result) = rx.try_recv() {
                latest = Some(result);
            }
            if latest.is_some() {
                self.pending_rgba = latest;
            }
        }
    }

    /// Submit frame for background RGBA conversion (bd-xifj)
    ///
    /// Returns true if frame was submitted, false if queue is full (frame dropped)
    fn submit_for_rgba_conversion(&mut self, frame: &FrameUpdate) -> bool {
        // Spawn converter thread lazily on first use
        if self.rgba_request_tx.is_none() {
            self.spawn_rgba_converter();
        }

        if let Some(tx) = &self.rgba_request_tx {
            let request = RgbaConversionRequest {
                data: frame.data.clone(),
                width: frame.width,
                height: frame.height,
                bit_depth: frame.bit_depth,
                frame_number: frame.frame_number,
                colormap: self.colormap,
                scale_mode: self.scale_mode,
                display_min: self.display_min,
                display_max: self.display_max,
                auto_contrast: self.auto_contrast,
            };

            match tx.try_send(request) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_)) => {
                    // Queue full, frame will be dropped (normal under load)
                    false
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    // Thread died, clear sender to trigger respawn
                    self.rgba_request_tx = None;
                    false
                }
            }
        } else {
            false
        }
    }

    /// Apply pending RGBA result to texture (bd-xifj)
    fn apply_pending_rgba(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.pending_rgba.take() {
            // Update auto-contrast display values
            if self.auto_contrast {
                self.display_min = result.computed_min;
                self.display_max = result.computed_max;
            }

            // Create or update texture
            let size = [result.width as usize, result.height as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &result.rgba);

            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.texture =
                    Some(ctx.load_texture("camera_frame", image, egui::TextureOptions::NEAREST));
            }

            // Recycle the buffer back to the converter thread (bd-wdx3)
            if let Some(tx) = &self.rgba_recycle_tx {
                let _ = tx.send(result.rgba);
            }
        }
    }

    /// Poll for async action results
    fn poll_actions(&mut self) {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                ImageViewerAction::CamerasLoaded(cameras) => {
                    self.available_cameras = cameras;
                    self.status = Some(format!("Found {} camera(s)", self.available_cameras.len()));
                }
                ImageViewerAction::Error(msg) => {
                    self.error = Some(msg);
                    // Clear subscription state on error to allow restart
                    self.subscription = None;
                    // bd-12qt: Update connection state on error
                    if self.connection_state == ConnectionState::Connected {
                        self.connection_state = ConnectionState::Disconnected;
                        self.last_disconnect = Some(Instant::now());
                        self.retry_count = 0;
                    }
                }
                ImageViewerAction::ReconnectResult { device_id, success } => {
                    // bd-12qt: Handle reconnection result
                    if success {
                        self.connection_state = ConnectionState::Connected;
                        self.retry_count = 0;
                        self.error = None;
                        self.status = Some(format!("Reconnected to {}", device_id));
                    } else {
                        self.connection_state = ConnectionState::Disconnected;
                        self.retry_count += 1;
                        self.status =
                            Some(format!("Reconnect failed (attempt {})", self.retry_count));
                    }
                }
                // bd-3pdi.5.3: Recording action handlers
                ImageViewerAction::RecordingStarted { output_path } => {
                    self.recording_state = RecordingState::Recording;
                    self.recording_output_path = Some(output_path.clone());
                    self.status = Some(format!("Recording to {}", output_path));
                    self.error = None;
                }
                ImageViewerAction::RecordingStopped {
                    output_path,
                    file_size_bytes,
                    total_samples,
                } => {
                    self.recording_state = RecordingState::Idle;
                    let size_mb = file_size_bytes as f64 / 1_000_000.0;
                    self.status = Some(format!(
                        "Saved: {} ({:.2} MB, {} frames)",
                        output_path, size_mb, total_samples
                    ));
                    self.error = None;
                }
                ImageViewerAction::RecordingStatus(status) => {
                    if let Some(s) = status {
                        self.recording_status = Some(s);
                        // Update recording state based on status
                        self.recording_state = match self.recording_status.as_ref().map(|s| s.state)
                        {
                            Some(2) => RecordingState::Recording, // RECORDING_ACTIVE
                            _ => RecordingState::Idle,
                        };
                    }
                }
            }
        }
    }

    /// Refresh the list of available cameras
    fn refresh_cameras(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        let action_tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.list_devices().await {
                Ok(devices) => {
                    // Filter for camera devices (FrameProducer capability)
                    let cameras: Vec<String> = devices
                        .into_iter()
                        .filter(|d| {
                            // Check is_frame_producer flag or camera category
                            d.is_frame_producer
                                || d.category == daq_proto::daq::DeviceCategory::Camera as i32
                        })
                        .map(|d| d.id)
                        .collect();
                    let _ = action_tx.send(ImageViewerAction::CamerasLoaded(cameras));
                }
                Err(e) => {
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to list cameras: {}",
                        e
                    )));
                }
            }
        });

        self.last_refresh = Some(Instant::now());
    }

    /// Load parameters for the selected camera (filtered for quick access)
    fn load_camera_params(&mut self, client: &mut DaqClient, runtime: &Runtime, device_id: &str) {
        // Don't start another load if already loading
        if self.loading_params_device.as_deref() == Some(device_id) {
            return;
        }

        let mut client = client.clone();
        let device_id_str = device_id.to_string();

        // Clear existing edit buffers and errors for this device
        self.param_edit_buffers
            .retain(|(dev_id, _), _| dev_id != device_id);
        self.param_errors
            .retain(|(dev_id, _), _| dev_id != device_id);

        // Set loading state
        self.loading_params_device = Some(device_id_str.clone());

        // Create channel for result
        let (tx, rx) = mpsc::channel();
        self.param_load_rx = Some(rx);

        // Spawn async task to load parameters in background
        runtime.spawn(async move {
            let device_id_for_error = device_id_str.clone();

            let result = async {
                let descriptors = client.list_parameters(&device_id_str).await?;

                // Filter for quick access parameters FIRST to reduce fetch volume
                let relevant_descriptors: Vec<_> = descriptors
                    .into_iter()
                    .filter(|d| {
                        let name_lower = d.name.to_lowercase();
                        QUICK_ACCESS_PARAMS
                            .iter()
                            .any(|&keyword| name_lower.contains(keyword))
                    })
                    .collect();

                // Parallel fetch of relevant parameter values
                let fetch_futures: Vec<_> = relevant_descriptors
                    .iter()
                    .map(|desc| {
                        let mut client = client.clone();
                        let device_id = device_id_str.clone();
                        let param_name = desc.name.clone();
                        async move {
                            let value = client.get_parameter(&device_id, &param_name).await;
                            (param_name, value)
                        }
                    })
                    .collect();

                let fetch_results = futures::future::join_all(fetch_futures).await;

                // Combine descriptors with fetched values
                let mut params = Vec::new();
                let mut load_errors = Vec::new();

                for (desc, (param_name, value_result)) in
                    relevant_descriptors.into_iter().zip(fetch_results)
                {
                    match value_result {
                        Ok(v) => {
                            params.push(ParameterCache::new(desc, v.value));
                        }
                        Err(e) => {
                            load_errors.push((param_name, e.to_string()));
                            params.push(ParameterCache::new(desc, String::new()));
                        }
                    }
                }

                Ok::<_, anyhow::Error>(ParamLoadResult {
                    device_id: device_id_str,
                    params,
                    errors: load_errors,
                })
            }
            .await;

            match result {
                Ok(load_result) => {
                    let _ = tx.send(load_result);
                }
                Err(e) => {
                    let _ = tx.send(ParamLoadResult {
                        device_id: device_id_for_error,
                        params: Vec::new(),
                        errors: vec![("_load".to_string(), e.to_string())],
                    });
                }
            }
        });
    }

    /// Set a camera parameter value
    fn set_camera_parameter(
        &mut self,
        client: &mut DaqClient,
        runtime: &Runtime,
        device_id: &str,
        name: &str,
        value: &str,
    ) {
        let mut client = client.clone();
        let device_id_str = device_id.to_string();
        let name_str = name.to_string();
        let value_str = value.to_string();
        let buffer_key = (device_id_str.clone(), name_str.clone());

        // Clear any previous error
        self.param_errors.remove(&buffer_key);
        // Mark as setting
        self.setting_params.insert(buffer_key);

        // Clone the persistent sender - this preserves all in-flight responses
        let tx = self.param_set_tx.clone();

        runtime.spawn(async move {
            let result = client
                .set_parameter(&device_id_str, &name_str, &value_str)
                .await;

            let set_result = match result {
                Ok(response) => ParamSetResult {
                    device_id: device_id_str,
                    param_name: name_str,
                    success: response.success,
                    actual_value: response.actual_value,
                    error: if response.success {
                        None
                    } else {
                        Some(response.error_message)
                    },
                },
                Err(e) => ParamSetResult {
                    device_id: device_id_str,
                    param_name: name_str,
                    success: false,
                    actual_value: String::new(),
                    error: Some(e.to_string()),
                },
            };

            let _ = tx.send(set_result);
        });
    }

    /// Poll for parameter async results
    fn poll_param_results(&mut self, ctx: &egui::Context) {
        // Poll loads
        if let Some(rx) = &self.param_load_rx {
            if let Ok(result) = rx.try_recv() {
                // If this result matches our current device, update
                if Some(&result.device_id) == self.device_id.as_ref() {
                    self.camera_params = result.params;
                    self.loading_params_device = None;

                    for (name, err) in result.errors {
                        self.param_errors
                            .insert((result.device_id.clone(), name), err);
                    }
                }
                self.param_load_rx = None; // One-shot load
                ctx.request_repaint();
            }
        }

        // Poll sets (persistent channel - drain all available)
        while let Ok(result) = self.param_set_rx.try_recv() {
            let key = (result.device_id.clone(), result.param_name.clone());
            self.setting_params.remove(&key);

            if result.success {
                // Update cache if device matches
                if Some(&result.device_id) == self.device_id.as_ref() {
                    if let Some(param) = self
                        .camera_params
                        .iter_mut()
                        .find(|p| p.descriptor.name == result.param_name)
                    {
                        param.update_value(result.actual_value.clone());
                    }
                }
                // Update buffer
                let unquoted = result.actual_value.trim_matches('"').to_string();
                self.param_edit_buffers.insert(key.clone(), unquoted);
                self.param_errors.remove(&key);
            } else if let Some(err) = result.error {
                self.param_errors.insert(key, err);
            }
            ctx.request_repaint();
        }

        // Request repaint if we're waiting for parameter set results
        if !self.setting_params.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Render a single camera parameter control
    fn render_camera_control(&mut self, ui: &mut egui::Ui, device_id: &str, param_idx: usize) {
        // Safe access to parameter to avoid borrowing self for the whole method
        let param = &self.camera_params[param_idx];
        let desc = &param.descriptor;
        let buffer_key = (device_id.to_string(), desc.name.clone());

        // Check if setting
        if self.setting_params.contains(&buffer_key) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&desc.name);
            });
            return;
        }

        // Read-only
        if !desc.writable {
            ui.horizontal(|ui| {
                ui.label(&desc.name);
                ui.label(format!(": {}", param.current_value));
                if !desc.units.is_empty() {
                    ui.weak(&desc.units);
                }
            });
            return;
        }

        let mut pending_update: Option<String> = None;

        // Enums
        if !desc.enum_values.is_empty() {
            let current = param.current_value.trim_matches('"').to_string();
            let mut selected = current.clone();

            ui.horizontal(|ui| {
                ui.label(&desc.name);
                let id = egui::Id::new("cam_ctrl").with(device_id).with(&desc.name);
                egui::ComboBox::from_id_salt(id)
                    .selected_text(&selected)
                    .show_ui(ui, |ui| {
                        for val in &desc.enum_values {
                            ui.selectable_value(&mut selected, val.clone(), val);
                        }
                    });
            });

            if selected != current {
                pending_update = Some(format!("\"{}\"", selected));
            }
        }
        // Boolean
        else if desc.dtype == "bool" {
            let mut val = param.current_value.parse::<bool>().unwrap_or(false);
            if ui.checkbox(&mut val, &desc.name).changed() {
                pending_update = Some(val.to_string());
            }
        }
        // Integer
        else if desc.dtype == "int" {
            // Get edit buffer or init from current
            let buffer = self
                .param_edit_buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| param.current_value.clone());

            let mut val: i64 = buffer.parse().unwrap_or(0);
            let original = val;

            ui.horizontal(|ui| {
                ui.label(&desc.name);
                let mut drag = egui::DragValue::new(&mut val).speed(1);
                if let Some(min) = desc.min_value {
                    drag = drag.range(min as i64..=i64::MAX);
                }
                if let Some(max) = desc.max_value {
                    drag = drag.range(i64::MIN..=max as i64);
                }

                let response = ui.add(drag);
                if !desc.units.is_empty() {
                    ui.weak(&desc.units);
                }

                // Update buffer immediately for visual feedback
                if val != original {
                    *self.param_edit_buffers.get_mut(&buffer_key).unwrap() = val.to_string();
                }

                // Commit on release or focus lost
                if (response.drag_stopped() || response.lost_focus())
                    && val != param.current_value.parse().unwrap_or(0)
                {
                    pending_update = Some(val.to_string());
                }
            });
        }
        // Float
        else if desc.dtype == "float" {
            let buffer = self
                .param_edit_buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| param.current_value.clone());

            let mut val: f64 = buffer.parse().unwrap_or(0.0);
            let original = val;

            // Check if this is an exposure parameter
            let is_exposure = desc.name.to_lowercase().contains("exposure");

            ui.horizontal(|ui| {
                ui.label(&desc.name);
                let mut drag = egui::DragValue::new(&mut val).speed(0.1);
                if let Some(min) = desc.min_value {
                    drag = drag.range(min..=f64::MAX);
                }
                if let Some(max) = desc.max_value {
                    drag = drag.range(f64::MIN..=max);
                }

                let response = ui.add(drag);
                if !desc.units.is_empty() {
                    ui.weak(&desc.units);
                }

                // Live toggle for exposure parameters
                if is_exposure {
                    ui.checkbox(&mut self.live_exposure, "Live");
                }

                if (val - original).abs() > f64::EPSILON {
                    *self.param_edit_buffers.get_mut(&buffer_key).unwrap() = val.to_string();
                }

                let current_float: f64 = param.current_value.parse().unwrap_or(0.0);
                let value_changed = (val - current_float).abs() > f64::EPSILON;

                // Live exposure: send during drag with debounce
                if is_exposure && self.live_exposure && response.dragged() && value_changed {
                    let now = Instant::now();
                    let should_send = self
                        .exposure_last_sent
                        .map(|t| now.duration_since(t) >= EXPOSURE_DEBOUNCE)
                        .unwrap_or(true);

                    if should_send {
                        pending_update = Some(val.to_string());
                        self.exposure_last_sent = Some(now);
                    }
                }

                // Always send on drag stop or focus lost (for all floats, including exposure)
                if (response.drag_stopped() || response.lost_focus()) && value_changed {
                    pending_update = Some(val.to_string());
                    if is_exposure {
                        self.exposure_last_sent = Some(Instant::now());
                    }
                }
            });
        }
        // String
        else if desc.dtype == "string" {
            let buffer = self
                .param_edit_buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| param.current_value.clone());

            ui.horizontal(|ui| {
                ui.label(&desc.name);
                let response = ui.text_edit_singleline(buffer);

                if response.lost_focus() && buffer != &param.current_value {
                    pending_update = Some(format!("\"{}\"", buffer));
                }
            });
        }
        // Fallback
        else {
            ui.horizontal(|ui| {
                ui.label(&desc.name);
                ui.label(&param.current_value);
            });
        }

        // Show error
        if let Some(err) = self.param_errors.get(&buffer_key) {
            ui.colored_label(egui::Color32::RED, err);
        }

        // Apply update if needed
        if let Some(val) = pending_update {
            self.pending_param_updates
                .push((device_id.to_string(), desc.name.clone(), val));
        }
    }

    /// Get sender for async frame updates (for external frame producers)
    ///
    /// Allows external code to push frames directly without going through gRPC.
    /// Useful for local frame sources or testing.
    #[allow(dead_code)]
    pub fn get_sender(&self) -> Option<FrameUpdateSender> {
        self.frame_tx.clone()
    }

    /// Start streaming frames from a device (public API for external control)
    pub fn start_stream(&mut self, device_id: &str, client: &mut DaqClient, runtime: &Runtime) {
        // Cancel existing subscription and ensure server-side stream is stopped
        if let Some(sub) = self.subscription.take() {
            let cancel_tx = sub.cancel_tx.clone();
            let mut client = client.clone();
            let old_device_id = sub.device_id.clone();
            runtime.spawn(async move {
                let _ = cancel_tx.send(()).await;
                let _ = client.stop_stream(&old_device_id).await;
            });
        }

        self.device_id = Some(device_id.to_string());
        self.error = None;
        self.status = Some(format!("Connecting to {}...", device_id));
        // bd-12qt: Update connection state
        self.connection_state = ConnectionState::Reconnecting;

        let Some(frame_tx) = self.frame_tx.clone() else {
            self.error = Some("Internal error: no frame channel".to_string());
            return;
        };

        let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut client = client.clone();
        let action_tx = self.action_tx.clone();
        let device_id_clone = device_id.to_string();
        let max_fps = self.max_fps;
        let stream_quality = self.stream_quality;

        runtime.spawn(async move {
            use futures::StreamExt;

            // 1. Start hardware-side streaming on the daemon
            // Treat "already streaming" as success (idempotent behavior)
            let start_result = client.start_stream(&device_id_clone, None).await;
            if let Err(e) = &start_result {
                // Check if this is "already streaming" - treat as non-fatal
                let error_str = e.to_string().to_lowercase();
                let is_already_streaming = error_str.contains("already streaming")
                    || error_str.contains("failedprecondition");

                if is_already_streaming {
                    tracing::info!(
                        device_id = %device_id_clone,
                        "Device already streaming; proceeding to subscribe"
                    );
                } else {
                    tracing::error!(device_id = %device_id_clone, error = %e, "Failed to start hardware stream");
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to start hardware stream: {}",
                        e
                    )));
                    return;
                }
            }

            // 2. Subscribe to the frame stream with quality setting
            let stream = match client.stream_frames(&device_id_clone, max_fps, stream_quality).await {
                Ok(s) => s,
                Err(e) => {
                    // Clean up: stop stream if we started it successfully
                    if start_result.is_ok() {
                        let _ = client.stop_stream(&device_id_clone).await;
                    }
                    tracing::error!(device_id = %device_id_clone, error = %e, "Failed to subscribe to frame stream");
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to subscribe to frames: {}",
                        e
                    )));
                    return;
                }
            };

            tokio::pin!(stream);

            tracing::info!(
                device_id = %device_id_clone,
                max_fps = max_fps,
                "Frame streaming started"
            );

            let mut frames_received = 0u64;
            let mut frames_dropped = 0u64;

            // Timeout for stream inactivity (30s) to prevent hanging on network faults (bd-7rk0)
            const STREAM_TIMEOUT: Duration = Duration::from_secs(30);

            loop {
                tokio::select! {
                    _ = cancel_rx.recv() => {
                        tracing::info!(device_id = %device_id_clone, "Frame stream cancelled");
                        break;
                    }
                    _ = tokio::time::sleep(STREAM_TIMEOUT) => {
                        tracing::warn!(
                            device_id = %device_id_clone,
                            timeout_secs = STREAM_TIMEOUT.as_secs(),
                            "Frame stream timeout - no frames received"
                        );
                        let _ = action_tx.send(ImageViewerAction::Error(format!(
                            "Frame stream timeout (no frames for {}s)", STREAM_TIMEOUT.as_secs()
                        )));
                        break;
                    }
                    frame_result = stream.next() => {
                        match frame_result {
                            Some(Ok(mut frame_data)) => {
                                frames_received += 1;

                                // Decompress frame if compressed (bd-7rk0: gRPC improvements)
                                if let Err(e) = decompress_frame(&mut frame_data) {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        error = %e,
                                        "Frame decompression failed, skipping frame"
                                    );
                                    continue;
                                }

                                if frames_received % 30 == 0 {
                                    tracing::debug!(
                                        device_id = %device_id_clone,
                                        frame = frames_received,
                                        bytes = frame_data.data.len(),
                                        "Received frame from gRPC"
                                    );
                                }

                                let update = FrameUpdate::from(frame_data);
                                // Use try_send to avoid blocking when queue is full
                                // Dropping frames is preferred over blocking the stream
                                match frame_tx.try_send(update) {
                                    Ok(()) => {}
                                    Err(mpsc::TrySendError::Full(_)) => {
                                        frames_dropped += 1;
                                        if frames_dropped % 10 == 0 {
                                            tracing::warn!(
                                                device_id = %device_id_clone,
                                                dropped = frames_dropped,
                                                "Frame dropped - UI queue full (slow render loop?)"
                                            );
                                        }
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => {
                                        // Receiver dropped
                                        tracing::info!("Frame receiver disconnected, stopping stream");
                                        break;
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                tracing::warn!(device_id = %device_id_clone, error = %e, "Frame stream error");
                                let _ = action_tx.send(ImageViewerAction::Error(format!(
                                    "Frame stream error: {}", e
                                )));
                                break;
                            }
                            None => {
                                // Stream ended
                                tracing::info!(device_id = %device_id_clone, "Frame stream ended");
                                let _ = action_tx.send(ImageViewerAction::Error(format!(
                                    "Frame stream from {} ended unexpectedly", device_id_clone
                                )));
                                break;
                            }
                        }
                    }
                }
            }

            // Log stream statistics
            tracing::info!(
                device_id = %device_id_clone,
                frames_received = frames_received,
                frames_dropped = frames_dropped,
                "Frame streaming stopped"
            );

            // Cleanup: Ensure server-side stream is stopped when subscriber task exits
            let _ = client.stop_stream(&device_id_clone).await;
        });

        self.subscription = Some(FrameStreamSubscription {
            cancel_tx,
            device_id: device_id.to_string(),
        });
    }

    /// Stop streaming and notify server to stop hardware capture
    pub fn stop_stream(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        if let Some(sub) = self.subscription.take() {
            let cancel_tx = sub.cancel_tx.clone();
            let device_id = sub.device_id.clone();

            // If client is available, also tell server to stop hardware capture
            if let Some(client) = client {
                let mut client = client.clone();
                runtime.spawn(async move {
                    let _ = cancel_tx.send(()).await;
                    let _ = client.stop_stream(&device_id).await;
                });
            } else {
                runtime.spawn(async move {
                    let _ = cancel_tx.send(()).await;
                });
            }
        }
        self.status = Some("Stream stopped".to_string());
    }

    // -- Recording Methods (bd-3pdi.5.3) --

    /// Start recording camera frames to HDF5
    fn start_recording(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        if self.recording_state != RecordingState::Idle {
            return;
        }

        self.recording_state = RecordingState::Starting;
        self.error = None;

        let action_tx = self.action_tx.clone();
        let mut client = client.clone();
        let name = if self.recording_name.is_empty() {
            // Generate name with device ID and timestamp
            let device_suffix = self
                .device_id
                .as_ref()
                .map(|d| format!("_{}", d.replace('/', "_")))
                .unwrap_or_default();
            format!(
                "camera{}_{}",
                device_suffix,
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            )
        } else {
            self.recording_name.clone()
        };

        runtime.spawn(async move {
            match client.start_recording(&name).await {
                Ok(response) => {
                    let _ = action_tx.send(ImageViewerAction::RecordingStarted {
                        output_path: response.output_path,
                    });
                }
                Err(e) => {
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to start recording: {}",
                        e
                    )));
                }
            }
        });
    }

    /// Stop recording camera frames
    fn stop_recording(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        if self.recording_state != RecordingState::Recording {
            return;
        }

        self.recording_state = RecordingState::Stopping;
        self.error = None;

        let action_tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.stop_recording().await {
                Ok(response) => {
                    let _ = action_tx.send(ImageViewerAction::RecordingStopped {
                        output_path: response.output_path,
                        file_size_bytes: response.file_size_bytes,
                        total_samples: response.total_samples,
                    });
                }
                Err(e) => {
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to stop recording: {}",
                        e
                    )));
                }
            }
        });
    }

    /// Poll recording status from server
    fn poll_recording_status(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        // Only poll every 500ms to avoid spamming
        let should_poll = self
            .last_recording_poll
            .map_or(true, |t| t.elapsed().as_millis() > 500);
        if !should_poll {
            return;
        }

        self.last_recording_poll = Some(Instant::now());

        let action_tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.get_recording_status().await {
                Ok(status) => {
                    let _ = action_tx.send(ImageViewerAction::RecordingStatus(Some(status)));
                }
                Err(_) => {
                    // Silently ignore status poll errors
                }
            }
        });
    }

    /// Drain pending frame updates, keeping only the most recent
    ///
    /// Fully drains the channel to prevent latency buildup.
    /// With bounded channel, producer blocks when queue is full.
    fn drain_updates(&mut self, ctx: &egui::Context) {
        // bd-xifj: Poll for completed RGBA conversions from background thread
        self.poll_rgba_results();
        self.apply_pending_rgba(ctx);

        let Some(rx) = &self.frame_rx else { return };

        // Drain ALL pending frames, keeping only the last one
        // This ensures we always display the most recent frame
        let mut latest_frame: Option<FrameUpdate> = None;

        while let Ok(frame) = rx.try_recv() {
            latest_frame = Some(frame);
        }

        // Process only the latest frame
        if let Some(frame) = latest_frame {
            self.process_frame(ctx, frame);
        }
    }

    /// Process a single frame update
    fn process_frame(&mut self, _ctx: &egui::Context, frame: FrameUpdate) {
        // Validate frame belongs to currently selected device (bd-tjwm.3)
        if let Some(expected_device) = &self.device_id {
            if &frame.device_id != expected_device {
                tracing::warn!(
                    expected = %expected_device,
                    received = %frame.device_id,
                    "Dropping frame from unexpected device: mismatch"
                );
                return;
            }
        }

        // Trace processed frames (throttled)
        if frame.frame_number % 30 == 0 {
            tracing::debug!(
                frame = frame.frame_number,
                width = frame.width,
                height = frame.height,
                "Processing frame for display"
            );
        }

        self.fps_counter.tick();
        self.width = frame.width;
        self.height = frame.height;
        self.bit_depth = frame.bit_depth;
        self.frame_count = frame.frame_number;
        self.error = None;

        // bd-7rk0: Update stream metrics from server
        if frame.metrics.is_some() {
            self.stream_metrics = frame.metrics.clone();
        }

        // bd-12qt: Update connection state when receiving frames
        if self.connection_state != ConnectionState::Connected {
            self.connection_state = ConnectionState::Connected;
            self.retry_count = 0;
            self.status = Some("Connected".to_string());
        }
        self.status = None;

        // Store frame data for ROI statistics
        self.last_frame_data = Some(frame.data.clone());

        // Update ROI statistics if we have an active ROI
        self.roi_selector.update_statistics(
            &frame.data,
            frame.width,
            frame.height,
            frame.bit_depth,
        );

        // Update histogram
        self.histogram
            .from_frame_data(&frame.data, frame.width, frame.height, frame.bit_depth);

        // bd-xifj: Submit frame for background RGBA conversion to prevent UI freezes
        // The converted RGBA will be applied to texture when polled in drain_updates
        let _submitted = self.submit_for_rgba_conversion(&frame);
        // Note: If submission fails (queue full), frame is dropped which is acceptable
        // under high load - we'll display the next successful frame
    }

    /// Render the image viewer panel
    pub fn ui(&mut self, ui: &mut egui::Ui, mut client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Poll for async action results
        self.poll_actions();
        self.poll_param_results(ui.ctx());

        // Drain pending frame updates
        self.drain_updates(ui.ctx());

        // Request continuous repaint while streaming
        if self.subscription.is_some() {
            ui.ctx().request_repaint();
        }

        // bd-12qt + bd-7rk0: Auto-reconnect logic with exponential backoff
        // Pattern inspired by Rerun's well-tested gRPC implementation:
        // - Initial delay: 100ms
        // - Max delay: 10 seconds
        // - Backoff factor: 2x per retry
        let mut should_auto_reconnect = false;
        if self.auto_reconnect
            && self.connection_state == ConnectionState::Disconnected
            && self.device_id.is_some()
            && self.subscription.is_none()
        {
            // Exponential backoff: 100ms * 2^retry_count, capped at 10 seconds
            let backoff_ms = (100u64 * 2u64.pow(self.retry_count.min(7))).min(10_000);
            if let Some(last_disconnect) = self.last_disconnect {
                if last_disconnect.elapsed().as_millis() as u64 >= backoff_ms {
                    should_auto_reconnect = true;
                    tracing::debug!(
                        retry_count = self.retry_count,
                        backoff_ms = backoff_ms,
                        "Auto-reconnecting with exponential backoff"
                    );
                }
            }
        }

        // Auto-refresh camera list on first load or if stale
        let should_refresh = self.last_refresh.is_none()
            || self
                .last_refresh
                .map(|t| t.elapsed().as_secs() > 30)
                .unwrap_or(false);

        // Track actions to take after UI rendering (avoid borrow issues)
        let mut start_stream_device: Option<String> = None;
        let mut stop_stream = false;
        let mut refresh_cameras = false;
        let mut start_recording = false;
        let mut stop_recording = false;

        // Header with connection state indicator
        ui.horizontal(|ui| {
            // Connection state indicator (colored dot)
            let (state_color, state_text) = match self.connection_state {
                ConnectionState::Idle => (colors::MUTED, ""),
                ConnectionState::Connected => (colors::CONNECTED, ""),
                ConnectionState::Disconnected => (colors::ERROR, ""),
                ConnectionState::Reconnecting => (colors::CONNECTING, ""),
            };
            if self.connection_state != ConnectionState::Idle {
                ui.colored_label(state_color, "●");
            }

            ui.heading("Image Viewer");

            if !state_text.is_empty() {
                ui.weak(state_text);
            }
        });

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Main toolbar in card frame
        layout::card_frame(ui).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = layout::ITEM_SPACING;

                // === Camera Selection Group ===
                ui.label(format!("{} Camera:", icons::device::CAMERA));

                let selected_text = self
                    .device_id
                    .clone()
                    .unwrap_or_else(|| "Select...".to_string());

                egui::ComboBox::from_id_salt("camera_selector")
                    .selected_text(&selected_text)
                    .show_ui(ui, |ui| {
                        if self.available_cameras.is_empty() {
                            ui.label("No cameras found");
                        } else {
                            for cam_id in &self.available_cameras.clone() {
                                let is_selected = self.device_id.as_ref() == Some(cam_id);
                                if ui.selectable_label(is_selected, cam_id).clicked() {
                                    if self.device_id.as_ref() != Some(cam_id) {
                                        self.device_id = Some(cam_id.clone());
                                        self.camera_params.clear();
                                    }
                                }
                            }
                        }
                    });

                if ui
                    .button(icons::action::REFRESH)
                    .on_hover_text("Refresh camera list")
                    .clicked()
                {
                    refresh_cameras = true;
                }

                // Auto-load parameters if needed
                if let Some(device_id) = &self.device_id {
                    if self.camera_params.is_empty() && self.loading_params_device.is_none() {
                        let device_id_clone = device_id.clone();
                        if let Some(client) = client.as_deref_mut() {
                            self.load_camera_params(client, runtime, &device_id_clone);
                        }
                    }
                }

                ui.separator();

                // === Stream Controls Group ===
                let is_streaming = self.subscription.is_some();
                if is_streaming {
                    if ui
                        .button(format!("{} Stop", icons::action::STOP))
                        .on_hover_text("Stop streaming")
                        .clicked()
                    {
                        stop_stream = true;
                    }
                } else if self.device_id.is_some() {
                    if ui
                        .button(format!("{} Start", icons::action::START))
                        .on_hover_text("Start streaming")
                        .clicked()
                    {
                        if let Some(device_id) = &self.device_id {
                            start_stream_device = Some(device_id.clone());
                        }
                    }
                }

                // Reconnect button when disconnected
                if self.connection_state == ConnectionState::Disconnected {
                    if ui
                        .button(format!("{} Reconnect", icons::action::REFRESH))
                        .on_hover_text("Attempt to reconnect to camera")
                        .clicked()
                    {
                        if let Some(device_id) = &self.device_id {
                            start_stream_device = Some(device_id.clone());
                            self.connection_state = ConnectionState::Reconnecting;
                        }
                    }
                    ui.checkbox(&mut self.auto_reconnect, "Auto")
                        .on_hover_text("Automatically attempt reconnection");
                }

                // === Recording Controls ===
                ui.separator();
                match self.recording_state {
                    RecordingState::Idle => {
                        if is_streaming {
                            if ui
                                .button(icons::action::RECORD)
                                .on_hover_text("Start recording frames to HDF5")
                                .clicked()
                            {
                                start_recording = true;
                            }
                        }
                    }
                    RecordingState::Recording => {
                        // Pulsing recording indicator
                        let time = ui.ctx().input(|i| i.time);
                        let pulse = ((time * 2.0).sin() * 0.5 + 0.5) as f32;
                        let record_color = egui::Color32::from_rgb(
                            (200.0 + pulse * 55.0) as u8,
                            (20.0 + pulse * 20.0) as u8,
                            (20.0 + pulse * 20.0) as u8,
                        );

                        if ui
                            .add(
                                egui::Button::new(format!("{} Stop", icons::action::STOP))
                                    .fill(record_color),
                            )
                            .on_hover_text("Stop recording")
                            .clicked()
                        {
                            stop_recording = true;
                        }

                        // Pulsing recording dot
                        ui.colored_label(record_color, icons::action::RECORD);
                        if let Some(status) = &self.recording_status {
                            ui.monospace(format!("{} frames", status.samples_recorded));
                        }

                        // Request repaint for animation
                        ui.ctx().request_repaint();
                    }
                    RecordingState::Starting => {
                        ui.add_enabled(false, egui::Button::new("Starting..."));
                        ui.spinner();
                    }
                    RecordingState::Stopping => {
                        ui.add_enabled(false, egui::Button::new("Stopping..."));
                        ui.spinner();
                    }
                }
            });
        });

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Display controls toolbar
        layout::card_frame(ui).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = layout::ITEM_SPACING;

                // Stream quality selector (server-side downsampling)
                egui::ComboBox::from_id_salt("stream_quality")
                    .selected_text(stream_quality_label(self.stream_quality))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.stream_quality, StreamQuality::Full, "Full");
                        ui.selectable_value(
                            &mut self.stream_quality,
                            StreamQuality::Preview,
                            "Preview (2x)",
                        );
                        ui.selectable_value(&mut self.stream_quality, StreamQuality::Fast, "Fast (4x)");
                    });

                ui.separator();

                // === Colormap & Scale ===
                ui.label("Color:");
                egui::ComboBox::from_id_salt("colormap_selector")
                    .width(80.0)
                    .selected_text(self.colormap.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.colormap, Colormap::Grayscale, "Grayscale");
                        ui.selectable_value(&mut self.colormap, Colormap::Viridis, "Viridis");
                        ui.selectable_value(&mut self.colormap, Colormap::Inferno, "Inferno");
                        ui.selectable_value(&mut self.colormap, Colormap::Plasma, "Plasma");
                        ui.selectable_value(&mut self.colormap, Colormap::Magma, "Magma");
                    });

                egui::ComboBox::from_id_salt("scale_mode")
                    .width(60.0)
                    .selected_text(self.scale_mode.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.scale_mode, ScaleMode::Linear, "Linear");
                        ui.selectable_value(&mut self.scale_mode, ScaleMode::Log, "Log");
                        ui.selectable_value(&mut self.scale_mode, ScaleMode::Sqrt, "Sqrt");
                    });

                ui.separator();

                // === Contrast ===
                ui.checkbox(&mut self.auto_contrast, "Auto Contrast");
                if !self.auto_contrast {
                    ui.add(
                        egui::DragValue::new(&mut self.display_min)
                            .speed(0.01)
                            .range(0.0..=1.0)
                            .prefix("Min: ")
                            .max_decimals(2),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.display_max)
                            .speed(0.01)
                            .range(0.0..=1.0)
                            .prefix("Max: ")
                            .max_decimals(2),
                    );
                } else {
                    ui.weak(format!(
                        "{:.0}%-{:.0}%",
                        self.display_min * 100.0,
                        self.display_max * 100.0
                    ));
                }

                ui.separator();

                // === Zoom Controls with Icons ===
                if ui
                    .button(icons::action::FIT)
                    .on_hover_text("Fit to window")
                    .clicked()
                {
                    self.auto_fit = true;
                }
                if ui
                    .button(icons::action::ZOOM_OUT)
                    .on_hover_text("Zoom out")
                    .clicked()
                {
                    self.zoom = (self.zoom * 0.8).max(0.1);
                    self.auto_fit = false;
                }
                ui.monospace(format!("{:>3.0}%", self.zoom * 100.0));
                if ui
                    .button(icons::action::ZOOM_IN)
                    .on_hover_text("Zoom in")
                    .clicked()
                {
                    self.zoom = (self.zoom * 1.25).min(10.0);
                    self.auto_fit = false;
                }

                ui.separator();

                // === ROI & Panel Controls ===
                let roi_selected = self.roi_selector.selection_mode;
                if ui
                    .selectable_label(roi_selected, if roi_selected { "ROI [ON]" } else { "ROI" })
                    .on_hover_text("Toggle ROI selection mode")
                    .clicked()
                {
                    self.roi_selector.selection_mode = !self.roi_selector.selection_mode;
                }
                if self.roi_selector.roi().is_some() {
                    if ui
                        .button(icons::action::DELETE)
                        .on_hover_text("Clear ROI")
                        .clicked()
                    {
                        self.roi_selector.clear();
                    }
                }

                ui.separator();

                ui.checkbox(&mut self.show_roi_panel, "Stats");
                ui.checkbox(&mut self.show_controls, "Controls");

                // === Histogram Position ===
                egui::ComboBox::from_id_salt("histogram_pos")
                    .width(100.0)
                    .selected_text(format!("Hist: {}", self.histogram_position.label()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::Hidden,
                            "Hidden",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::BottomRight,
                            "Bottom Right",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::BottomLeft,
                            "Bottom Left",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::TopRight,
                            "Top Right",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::TopLeft,
                            "Top Left",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::SidePanel,
                            "Side Panel",
                        );
                    });
                if self.histogram_position.is_visible() {
                    ui.checkbox(&mut self.histogram.log_scale, "Log");
                }
            });
        });

        // Execute collected actions after UI rendering
        let client = if let Some(client_val) = client {
            // Auto-refresh on first load
            if should_refresh {
                self.refresh_cameras(client_val, runtime);
            }

            // Handle manual refresh
            if refresh_cameras {
                self.refresh_cameras(client_val, runtime);
            }

            // Handle start stream (manual or auto-reconnect)
            if let Some(device_id) = start_stream_device {
                self.start_stream(&device_id, client_val, runtime);
            } else if should_auto_reconnect {
                // bd-12qt: Auto-reconnect
                if let Some(device_id) = self.device_id.clone() {
                    self.connection_state = ConnectionState::Reconnecting;
                    self.last_disconnect = Some(Instant::now()); // Reset timer for next attempt
                    self.start_stream(&device_id, client_val, runtime);
                }
            }

            // Handle pending param updates
            let updates: Vec<_> = self.pending_param_updates.drain(..).collect();
            for (dev, name, val) in updates {
                self.set_camera_parameter(client_val, runtime, &dev, &name, &val);
            }

            Some(client_val)
        } else {
            self.pending_param_updates.clear();
            None
        };

        // Handle stop stream and recording actions
        if let Some(client) = client {
            if stop_stream {
                self.stop_stream(Some(client), runtime);
            } else {
                // Handle recording actions (bd-3pdi.5.3)
                if start_recording {
                    self.start_recording(client, runtime);
                }
                if stop_recording {
                    self.stop_recording(client, runtime);
                }
                // Poll recording status while recording
                if matches!(self.recording_state, RecordingState::Recording) {
                    let should_poll = self
                        .last_recording_poll
                        .map(|t| t.elapsed() > std::time::Duration::from_millis(500))
                        .unwrap_or(true);
                    if should_poll {
                        self.poll_recording_status(client, runtime);
                    }
                }
            }
        } else if stop_stream {
            self.stop_stream(None, runtime);
        }

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Status bar with frame info
        ui.horizontal(|ui| {
            if self.width > 0 {
                ui.monospace(format!(
                    "{}x{} @ {}bit",
                    self.width, self.height, self.bit_depth
                ));
                ui.separator();
                ui.monospace(format!("Frame: {}", self.frame_count));
                ui.separator();
                ui.monospace(format!("{:.1} FPS", self.fps_counter.fps()));

                if let Some(ref metrics) = self.stream_metrics {
                    ui.separator();
                    ui.weak(format!("{:.1}ms latency", metrics.avg_latency_ms));
                    if metrics.frames_dropped > 0 {
                        ui.separator();
                        ui.colored_label(
                            colors::WARNING,
                            format!("{} dropped", metrics.frames_dropped),
                        );
                    }
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(err) = &self.error {
                    ui.colored_label(colors::ERROR, format!("{} {}", icons::status::ERROR, err));
                }
                if let Some(status) = &self.status {
                    ui.colored_label(
                        colors::WARNING,
                        format!("{} {}", icons::status::WARNING, status),
                    );
                }
            });
        });

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Image display area with optional statistics panel
        // Calculate side panel width based on what's visible
        let has_roi_panel = self.show_roi_panel && self.roi_selector.roi().is_some();
        let has_histogram_panel = matches!(self.histogram_position, HistogramPosition::SidePanel);
        let has_controls_panel = self.show_controls && !self.camera_params.is_empty();

        let stats_panel_width = if has_roi_panel || has_histogram_panel || has_controls_panel {
            if has_controls_panel {
                280.0
            } else {
                200.0
            }
        } else {
            0.0
        };

        // Get full available space BEFORE any layout calls
        let full_available = ui.available_size();
        let image_area_size =
            egui::vec2(full_available.x - stats_panel_width - 8.0, full_available.y);

        // Use columns to split horizontally while filling vertical space
        ui.columns(if stats_panel_width > 0.0 { 2 } else { 1 }, |columns| {
            // Image column (first/only column)
            let ui = &mut columns[0];
            let available_size = if stats_panel_width > 0.0 {
                // In column mode, use calculated image area
                egui::vec2(image_area_size.x, ui.available_height())
            } else {
                ui.available_size()
            };

            if let Some(texture) = &self.texture {
                // Calculate fit zoom if needed - continuously fit when auto_fit is enabled
                if self.auto_fit && self.width > 0 && self.height > 0 {
                    let scale_x = available_size.x / self.width as f32;
                    let scale_y = available_size.y / self.height as f32;
                    // Allow upscaling to fill available space (remove .min(1.0) cap)
                    self.zoom = scale_x.min(scale_y);
                    self.pan = egui::Vec2::ZERO;
                    // Keep auto_fit true for continuous fitting as window resizes
                }

                let image_size = egui::vec2(
                    self.width as f32 * self.zoom,
                    self.height as f32 * self.zoom,
                );

                // Scrollable/pannable area
                egui::ScrollArea::both()
                    .id_salt("image_scroll")
                    .show(ui, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            available_size.max(image_size),
                            egui::Sense::click_and_drag(),
                        );

                        // Calculate image offset (centered)
                        let offset = (available_size - image_size) / 2.0 + self.pan;
                        let image_rect = egui::Rect::from_min_size(rect.min + offset, image_size);

                        // Handle ROI selection or pan depending on mode
                        if self.roi_selector.selection_mode {
                            // ROI selection mode
                            let roi_finalized = self.roi_selector.handle_input(
                                &response,
                                rect,
                                (self.width, self.height),
                                self.zoom,
                                self.pan,
                            );

                            // If ROI was finalized and we have frame data, compute statistics
                            if roi_finalized {
                                if let (Some(roi), Some(frame_data)) =
                                    (self.roi_selector.roi(), &self.last_frame_data)
                                {
                                    self.roi_selector.set_roi_from_frame(
                                        *roi,
                                        frame_data,
                                        self.width,
                                        self.height,
                                        self.bit_depth,
                                    );
                                }
                            }
                        } else {
                            // Pan mode
                            if response.dragged() {
                                self.pan += response.drag_delta();
                            }
                        }

                        // Handle zoom with scroll wheel (always active)
                        if response.hovered() {
                            let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
                            if scroll_delta != 0.0 {
                                let zoom_factor = 1.0 + scroll_delta * 0.001;
                                self.zoom = (self.zoom * zoom_factor).clamp(0.1, 10.0);
                            }
                        }

                        // Draw the image
                        ui.painter().image(
                            texture.id(),
                            image_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );

                        // Draw ROI overlay
                        self.roi_selector.draw_overlay(
                            ui.painter(),
                            rect,
                            (self.width, self.height),
                            self.zoom,
                            self.pan,
                        );

                        // Draw histogram overlay if positioned on image
                        if self.histogram_position.is_overlay() {
                            let hist_size = egui::vec2(180.0, 80.0);
                            let hist_rect =
                                self.histogram_position.overlay_rect(image_rect, hist_size);

                            // Create a child UI at the overlay position
                            let mut hist_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(hist_rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Min)),
                            );
                            self.histogram.show_overlay(&mut hist_ui, hist_size);
                        }

                        // Show pixel coordinates on hover
                        if let Some(pos) = response.hover_pos() {
                            let image_pos = pos - rect.min - offset;
                            let pixel_x = (image_pos.x / self.zoom) as i32;
                            let pixel_y = (image_pos.y / self.zoom) as i32;
                            if pixel_x >= 0
                                && pixel_x < self.width as i32
                                && pixel_y >= 0
                                && pixel_y < self.height as i32
                            {
                                response.on_hover_text(format!("({}, {})", pixel_x, pixel_y));
                            }
                        }
                    });
            } else {
                // No image - show placeholder
                ui.centered_and_justified(|ui| {
                    ui.label("No image. Select a camera device and start streaming.");
                });
            }

            if stats_panel_width > 0.0 && columns.len() > 1 {
                let side_ui = &mut columns[1];
                egui::ScrollArea::vertical()
                    .id_salt("side_panel_scroll")
                    .show(side_ui, |ui| {
                        if has_controls_panel {
                            layout::card_frame(ui).show(ui, |ui| {
                                egui::CollapsingHeader::new(format!(
                                    "{} Camera Settings",
                                    icons::action::SETTINGS
                                ))
                                .default_open(true)
                                .show(ui, |ui| {
                                    if let Some(device_id_ref) = &self.device_id {
                                        let device_id = device_id_ref.clone();
                                        for i in 0..self.camera_params.len() {
                                            self.render_camera_control(ui, &device_id, i);
                                            if i < self.camera_params.len() - 1 {
                                                ui.add_space(4.0);
                                            }
                                        }
                                    }
                                });
                            });
                            ui.add_space(layout::SECTION_SPACING);
                        }

                        if has_roi_panel {
                            layout::card_frame(ui).show(ui, |ui| {
                                egui::CollapsingHeader::new("ROI Statistics")
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        self.roi_selector.show_statistics_panel(ui);

                                        ui.add_space(4.0);
                                        if ui
                                            .button("Apply as Hardware ROI")
                                            .on_hover_text(
                                                "Update camera acquisition ROI (restarts stream)",
                                            )
                                            .clicked()
                                        {
                                            if let Some(roi) = self.roi_selector.roi() {
                                                if let Some(dev_id) = self.device_id.clone() {
                                                    let roi_json = serde_json::json!({
                                                        "x": roi.x as u32,
                                                        "y": roi.y as u32,
                                                        "width": roi.width as u32,
                                                        "height": roi.height as u32
                                                    });
                                                    self.pending_param_updates.push((
                                                        dev_id,
                                                        "roi".to_string(),
                                                        roi_json.to_string(),
                                                    ));
                                                }
                                            }
                                        }
                                    });
                            });
                            ui.add_space(layout::SECTION_SPACING);
                        }

                        if has_histogram_panel {
                            layout::card_frame(ui).show(ui, |ui| {
                                egui::CollapsingHeader::new("Histogram")
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        self.histogram.show_panel(ui);
                                    });
                            });
                        }
                    });
            }
        }); // close ui.columns
    }

    // =========================================================================
    // Public API for programmatic control
    // =========================================================================

    /// Set the device to stream from (for external control)
    ///
    /// This allows programmatic selection of which camera to stream.
    /// Use in automated workflows or scripted interactions.
    #[allow(dead_code)]
    pub fn set_device(&mut self, device_id: &str, client: &mut DaqClient, runtime: &Runtime) {
        self.start_stream(device_id, client, runtime);
    }

    /// Check if currently streaming
    #[allow(dead_code)]
    pub fn is_streaming(&self) -> bool {
        self.subscription.is_some()
    }

    /// Get current device ID being streamed
    #[allow(dead_code)]
    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }
}
