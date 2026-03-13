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

pub mod colormap;
mod echelle_extraction;
mod echelle_profile_cache;
#[cfg(not(target_arch = "wasm32"))]
mod echelle_sidecar;
mod processing;
mod types;

pub use colormap::*;
use echelle_extraction::*;
use echelle_profile_cache::*;
use processing::*;
pub use types::*;

use crate::runtime::Runtime;
use crate::time::{Duration, Instant};
use eframe::egui;
use egui_extras::{Size, StripBuilder};
use egui_plot::{Line, Plot, PlotPoints, Points};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use crate::device_ext::DeviceInfoExt;
use crate::icons;
use crate::layout::{self, colors};
use crate::widgets::{Histogram, HistogramPosition, ParameterCache, RoiSelector};
use client::DaqClient;
use common::core::Measurement;
use common::echelle::{
    AxisDirection, DetectorAxis, EchelleArtifactRef, EchelleCalibrationProfile,
    EchelleExtractionConfig, EchelleFrameCompatibility, EchelleOrderCalibration,
    EchelleOrientation, EchelleProvenance, EchelleSchemaVersion, EchelleSummationMode,
    EchelleTraceModel, EchelleWavelengthModel, PolynomialBasis,
};
use protocol::compression::decompress_frame_into;
use protocol::daq::StreamQuality;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum EchellePlotXAxisMode {
    #[default]
    Wavelength,
    SampleIndex,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct EchellePlotHoverLink {
    relative_index: u32,
    sample_index: usize,
    wavelength: f64,
    flux: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum EchelleCalibrationTab {
    #[default]
    Profile,
    Trace,
    LinePoints,
    WavelengthFit,
    BlazeFlat,
    MechelleNotes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EchelleCalibrationPointUi {
    enabled: bool,
    order_relative_index: u32,
    x_sample: f64,
    y_pixel: f64,
    wavelength: f64,
    note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EchelleLineListEntryUi {
    enabled: bool,
    wavelength: f64,
    label: String,
}

#[derive(Debug, Clone, Default)]
struct EchelleCalibrationUiState {
    tab: EchelleCalibrationTab,
    editor_profile: Option<EchelleCalibrationProfile>,
    editor_dirty: bool,
    editor_last_loaded_path: Option<std::path::PathBuf>,
    save_as_path_text: String,
    points_path_text: String,
    line_list_path_text: String,
    blaze_export_path_text: String,
    calibration_points: Vec<EchelleCalibrationPointUi>,
    line_list: Vec<EchelleLineListEntryUi>,
    selected_order_edit_idx: usize,
    selected_point_idx: usize,
    trace_overlay_enabled: bool,
    trace_overlay_all_orders: bool,
    trace_overlay_sample_step: u32,
    trace_overlay_max_orders: u32,
    trace_nudge_px: f64,
    trace_auto_detect_min_separation_px: u32,
    trace_auto_detect_threshold_fraction: f64,
    fit_outlier_sigma: f64,
    fit_rms_acceptance_px: f64,
    blaze_preview_enabled: bool,
    blaze_preview_scale: f64,
    status_message: Option<String>,
    last_error: Option<String>,
}

impl EchelleCalibrationUiState {
    fn with_defaults() -> Self {
        Self {
            trace_overlay_enabled: true,
            trace_overlay_all_orders: false,
            trace_overlay_sample_step: 32,
            trace_overlay_max_orders: 32,
            trace_nudge_px: 0.25,
            trace_auto_detect_min_separation_px: 16,
            trace_auto_detect_threshold_fraction: 0.25,
            fit_outlier_sigma: 3.0,
            fit_rms_acceptance_px: 0.25,
            blaze_preview_enabled: false,
            blaze_preview_scale: 1.0,
            ..Default::default()
        }
    }
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
    last_frame_data: Option<Arc<Vec<u8>>>,
    /// Show ROI statistics panel
    show_roi_panel: bool,
    /// Show pixel statistics panel (bd-li4i)
    show_pixel_stats: bool,
    /// Cached pixel statistics for current frame (bd-li4i)
    pixel_statistics: Option<PixelStatistics>,
    /// Histogram for intensity distribution
    histogram: Histogram,
    /// Histogram display position
    histogram_position: HistogramPosition,
    /// Available camera devices
    available_cameras: Vec<String>,
    /// Full sensor dimensions by camera ID (from device metadata)
    camera_full_frame_dims: std::collections::HashMap<String, (u32, u32)>,
    /// Display minimum (0.0-1.0 normalized) - pixels at or below this are black
    display_min: f32,
    /// Display maximum (0.0-1.0 normalized) - pixels at or above this are white
    display_max: f32,
    /// Auto-contrast mode - automatically compute min/max from frame data (deprecated, use contrast_mode)
    auto_contrast: bool,
    /// Contrast enhancement mode (bd-j6xm)
    contrast_mode: ContrastMode,
    /// Low percentile for auto-percentile mode (0.0-100.0) (bd-j6xm)
    percentile_low: f32,
    /// High percentile for auto-percentile mode (0.0-100.0) (bd-j6xm)
    percentile_high: f32,
    /// Async action receiver
    action_rx: std::sync::mpsc::Receiver<ImageViewerAction>,
    /// Async action sender
    action_tx: std::sync::mpsc::Sender<ImageViewerAction>,
    /// Last refresh time
    last_refresh: Option<Instant>,
    /// Stream generation counter — incremented on each start_stream() call.
    /// Used by streaming tasks to detect if they've been superseded, preventing
    /// stale tasks from calling stop_stream() and killing a newer stream.
    stream_generation: Arc<AtomicU64>,

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

    // -- Physical Coordinate Calibration (bd-4088.6) --
    /// Pixel to physical unit calibration in X direction (units per pixel)
    pixel_scale_x: Option<f64>,
    /// Pixel to physical unit calibration in Y direction (units per pixel)
    pixel_scale_y: Option<f64>,
    /// Physical unit label (e.g., "µm", "mm")
    scale_unit: String,

    // -- Recording Fields (bd-3pdi.5.3) --
    /// Current recording state
    recording_state: RecordingState,
    /// Recording name input
    recording_name: String,
    /// Current output path (when recording)
    recording_output_path: Option<String>,
    /// Recording status from server
    recording_status: Option<protocol::daq::RecordingStatus>,
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
    /// True when thread spawn failed (e.g., WASM); skip retry and convert synchronously
    rgba_sync_mode: bool,

    // -- Background Echelle Extraction (bd-fwyp: move extraction off UI thread) --
    /// Receiver for completed echelle extractions from background thread
    echelle_extract_rx: Option<std::sync::mpsc::Receiver<EchelleExtractionResult>>,
    /// Sender for echelle extraction requests
    echelle_extract_tx: Option<std::sync::mpsc::SyncSender<EchelleExtractionRequest>>,
    /// Pending echelle extraction result ready to be applied
    pending_echelle: Option<EchelleExtractionResult>,
    /// True when thread spawn failed (e.g., WASM); extract synchronously
    echelle_sync_mode: bool,

    // -- Crosshair Feature (bd-pgcb) --
    /// Enable crosshair cursor display
    crosshair_enabled: bool,
    /// Locked crosshair position (pixel coordinates)
    crosshair_locked_pos: Option<(i32, i32)>,

    // -- Interactive Colorbar (bd-07j1) --
    /// Interactive colorbar widget for midpoint adjustment
    colorbar: crate::widgets::Colorbar,
    /// Show colorbar in the image viewer
    show_colorbar: bool,

    // -- Metadata Overlay (bd-6h1c) --
    /// Show acquisition metadata overlay on the image
    show_metadata_overlay: bool,

    // -- Scale Bar Overlay (bd-0tcg) --
    /// Show scale bar overlay on the image
    show_scale_bar: bool,
    /// Last frame timestamp in nanoseconds (for overlay display)
    last_frame_timestamp_ns: u64,

    // -- Echelle Calibration Profile Cache (bd-2kla.2.4) --
    /// Optional cached echelle calibration profile with hot-reload-safe semantics.
    echelle_profile_cache: EchelleProfileCache,
    /// Last extracted echelle spectrum preview (MVP local extraction path).
    echelle_preview: Option<EchelleExtractionPreview>,
    /// Most recent extraction error (kept separate from general panel errors).
    echelle_preview_error: Option<String>,
    /// Extract every Nth frame to bound CPU cost on UI path.
    echelle_extract_every_n_frames: u32,
    /// Toggle echelle extraction preview while profile is loaded.
    echelle_extraction_enabled: bool,
    /// Selected order index for side-panel plot (0 = first extracted order).
    echelle_selected_order_plot: usize,
    /// Show merged wavelength-sorted preview when available.
    echelle_show_merged_plot: bool,
    /// Reusable scratch buffer for 12/16-bit decode fallback path (allocation control).
    echelle_decode_scratch_u16: Vec<u16>,
    /// Debug/export hook: latest preview spectra materialized as Measurement::Spectrum values.
    echelle_preview_measurements: Vec<Measurement>,
    /// Display-only moving-average smoothing for the spectrum preview plot.
    echelle_plot_smoothing_window: u32,
    /// X-axis display mode for the spectrum preview.
    echelle_plot_x_axis_mode: EchellePlotXAxisMode,
    /// Developer diagnostics counters for the local extractor.
    echelle_extract_runs: u64,
    echelle_extract_errors: u64,
    echelle_extract_skipped_frames: u64,
    echelle_last_extract_ms: Option<f64>,
    /// Hover cross-link from spectrum plot to image sample marker.
    echelle_plot_hover_link: Option<EchellePlotHoverLink>,
    /// Calibration authoring workspace state (bd-2kla.8 scaffolding).
    echelle_cal_ui: EchelleCalibrationUiState,
    /// True when the active echelle profile snapshot should be resynced into RunEngine state.
    echelle_run_engine_sync_dirty: bool,
    /// True while an async echelle snapshot sync request is in flight.
    echelle_run_engine_sync_in_flight: bool,
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
            show_pixel_stats: false,
            pixel_statistics: None,
            histogram: Histogram::new(),
            histogram_position: HistogramPosition::SidePanel,
            available_cameras: Vec::new(),
            camera_full_frame_dims: std::collections::HashMap::new(),
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: true,
            contrast_mode: ContrastMode::AutoPercentile,
            percentile_low: 0.1,
            percentile_high: 99.9,
            action_rx,
            action_tx,
            last_refresh: None,
            stream_generation: Arc::new(AtomicU64::new(0)),

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
            stream_quality: StreamQuality::Fast,

            // Physical coordinate calibration (bd-4088.6)
            pixel_scale_x: None,
            pixel_scale_y: None,
            scale_unit: "µm".to_string(),

            // Background RGBA conversion (bd-xifj)
            // Buffer reuse via recycling channel (bd-wdx3)
            rgba_rx: None,
            rgba_request_tx: None,
            pending_rgba: None,
            rgba_recycle_tx: None,
            rgba_sync_mode: false,

            // Background echelle extraction (bd-fwyp)
            echelle_extract_rx: None,
            echelle_extract_tx: None,
            pending_echelle: None,
            echelle_sync_mode: false,

            // Crosshair (bd-pgcb)
            crosshair_enabled: false,
            crosshair_locked_pos: None,

            // Interactive colorbar (bd-07j1)
            colorbar: crate::widgets::Colorbar::new()
                .orientation(crate::widgets::ColorbarOrientation::Vertical)
                .units("counts"),
            show_colorbar: true,

            // Metadata overlay (bd-6h1c)
            show_metadata_overlay: false,

            // Scale bar overlay (bd-0tcg)
            show_scale_bar: false,
            last_frame_timestamp_ns: 0,

            echelle_profile_cache: EchelleProfileCache::default(),
            echelle_preview: None,
            echelle_preview_error: None,
            echelle_extract_every_n_frames: 5,
            echelle_extraction_enabled: true,
            echelle_selected_order_plot: 0,
            echelle_show_merged_plot: true,
            echelle_decode_scratch_u16: Vec::new(),
            echelle_preview_measurements: Vec::new(),
            echelle_plot_smoothing_window: 1,
            echelle_plot_x_axis_mode: EchellePlotXAxisMode::Wavelength,
            echelle_extract_runs: 0,
            echelle_extract_errors: 0,
            echelle_extract_skipped_frames: 0,
            echelle_last_extract_ms: None,
            echelle_plot_hover_link: None,
            echelle_cal_ui: EchelleCalibrationUiState::with_defaults(),
            echelle_run_engine_sync_dirty: false,
            echelle_run_engine_sync_in_flight: false,
        }
    }
}

impl ImageViewerPanel {
    /// Create a new image viewer panel
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the echelle calibration profile path used for extraction preview features.
    ///
    /// The profile is loaded lazily and reloaded on modification while preserving
    /// the last-good profile if a subsequent reload fails.
    pub fn set_echelle_profile_path(&mut self, path: std::path::PathBuf) {
        self.echelle_cal_ui.save_as_path_text = path.display().to_string();
        self.echelle_profile_cache.set_path(path);
    }

    /// Clear the active echelle calibration profile cache/path.
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn clear_echelle_profile_path(&mut self) {
        if let EchelleProfileCacheEvent::Cleared = self.echelle_profile_cache.clear() {
            self.echelle_cal_ui.editor_profile = None;
            self.echelle_cal_ui.editor_dirty = false;
            self.echelle_cal_ui.editor_last_loaded_path = None;
            self.echelle_cal_ui.save_as_path_text.clear();
            self.mark_echelle_run_engine_sync_dirty();
            // Do not clobber persistent statuses such as recording completion.
            if self
                .status
                .as_deref()
                .map(|s| s.starts_with("Echelle profile"))
                .unwrap_or(false)
            {
                self.status = None;
            }
        }
    }

    /// Expose the last echelle profile loader error for future UI presentation.
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn echelle_profile_last_error(&self) -> Option<&str> {
        self.echelle_profile_cache.last_error()
    }

    /// Returns the configured echelle calibration profile path, if any.
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn echelle_profile_path(&self) -> Option<&std::path::Path> {
        self.echelle_profile_cache.path()
    }

    /// Returns the latest extracted echelle preview spectra as `Measurement::Spectrum` values.
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn echelle_preview_measurements(&self) -> &[Measurement] {
        &self.echelle_preview_measurements
    }

    /// Developer hook: export latest preview spectra (orders + merged if present) as JSON.
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn save_echelle_preview_measurements_json(
        &self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        if self.echelle_preview_measurements.is_empty() {
            return Err("no echelle preview measurements available".to_string());
        }
        let json = serde_json::to_string_pretty(&self.echelle_preview_measurements)
            .map_err(|e| format!("failed to serialize preview measurements: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("failed to write {}: {e}", path.display()))
    }

    /// Developer hook: export the merged preview spectrum as CSV (`wavelength,flux`).
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn save_echelle_preview_merged_csv(&self, path: &std::path::Path) -> Result<(), String> {
        let merged = self
            .echelle_preview
            .as_ref()
            .and_then(|p| p.merged.as_ref())
            .ok_or_else(|| "no merged echelle preview available".to_string())?;
        use std::fmt::Write;
        let mut out = String::from("wavelength,flux\n");
        for (w, f) in merged.wavelengths.iter().zip(&merged.flux) {
            let _ = writeln!(out, "{w},{f}");
        }
        std::fs::write(path, out).map_err(|e| format!("failed to write {}: {e}", path.display()))
    }

    /// Set whether the local echelle extraction preview is enabled.
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn set_echelle_extraction_enabled(&mut self, enabled: bool) {
        self.echelle_extraction_enabled = enabled;
    }

    /// Set extraction cadence for the local preview (`1` = every frame).
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn set_echelle_extract_every_n_frames(&mut self, every_n_frames: u32) {
        self.echelle_extract_every_n_frames = every_n_frames.max(1);
    }

    /// Configure whether the preview plot defaults to merged mode.
    #[allow(dead_code)] // Echelle UI wiring pending
    pub fn set_echelle_preview_show_merged(&mut self, show_merged: bool) {
        self.echelle_show_merged_plot = show_merged;
    }

    /// Poll the profile cache for changes and surface loader results through UI status/error.
    fn poll_echelle_profile_cache(&mut self) {
        match self.echelle_profile_cache.poll_reload_if_changed() {
            EchelleProfileCacheEvent::Unchanged => {}
            EchelleProfileCacheEvent::Loaded(path) => {
                self.mark_echelle_run_engine_sync_dirty();
                self.error = None;
                self.echelle_preview_error = None;
                self.echelle_cal_ui.save_as_path_text = path.display().to_string();
                if !self.echelle_cal_ui.editor_dirty {
                    if let Some(profile) = self.echelle_profile_cache.profile() {
                        self.echelle_cal_ui.editor_profile = Some((**profile).clone());
                        self.echelle_cal_ui.editor_last_loaded_path = Some(path.clone());
                        self.echelle_cal_ui.status_message =
                            Some(format!("Editor synced from {}", path.display()));
                        self.echelle_cal_ui.last_error = None;
                    }
                } else {
                    self.echelle_cal_ui.status_message = Some(format!(
                        "Active profile reloaded from {} (editor has unsaved changes)",
                        path.display()
                    ));
                }
                self.status = Some(format!("Echelle profile loaded: {}", path.display()));
            }
            EchelleProfileCacheEvent::Error(msg) => {
                // Preserve last-good profile inside the cache; only surface the error.
                self.error = Some(msg);
            }
            EchelleProfileCacheEvent::Cleared => {
                self.mark_echelle_run_engine_sync_dirty();
                self.echelle_cal_ui.editor_last_loaded_path = None;
                self.status = Some("Echelle profile cleared".to_string());
            }
        }
    }

    fn mark_echelle_run_engine_sync_dirty(&mut self) {
        self.echelle_run_engine_sync_dirty = true;
    }

    fn build_echelle_run_engine_snapshot(&self) -> Option<protocol::daq::CalibrationSnapshot> {
        let profile = self.echelle_profile_cache.profile()?;
        let target_device_id = self.device_id.clone()?;
        Some(protocol::daq::CalibrationSnapshot {
            device_type: "spectroscopy".to_string(),
            target_device_id: Some(target_device_id),
            calibration_timestamp_rfc3339: None,
            grating_wavelength_coverage: Vec::new(),
            echelle_frame_compatibility: Some(protocol::daq::EchelleFrameCompatibility {
                sensor_width: profile.compatibility.sensor_width,
                sensor_height: profile.compatibility.sensor_height,
                frame_width: profile.compatibility.frame_width,
                frame_height: profile.compatibility.frame_height,
                roi_x: profile.compatibility.roi_x,
                roi_y: profile.compatibility.roi_y,
                binning_x: profile.compatibility.binning_x,
                binning_y: profile.compatibility.binning_y,
                bit_depth: profile.compatibility.bit_depth,
            }),
        })
    }

    fn sync_echelle_profile_to_run_engine(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        if !self.echelle_run_engine_sync_dirty || self.echelle_run_engine_sync_in_flight {
            return;
        }
        let Some(client) = client else {
            return;
        };

        let snapshot = self.build_echelle_run_engine_snapshot();
        let action_tx = self.action_tx.clone();
        let mut client = client.clone();
        self.echelle_run_engine_sync_dirty = false;
        self.echelle_run_engine_sync_in_flight = true;

        runtime.spawn(async move {
            let result = if let Some(snapshot) = snapshot {
                client.set_calibration_snapshot(snapshot).await.map(|_| {
                    "Synced active echelle profile into RunEngine readiness gate".to_string()
                })
            } else {
                client
                    .clear_calibration_snapshot("spectroscopy", false, true)
                    .await
                    .map(|_| {
                        "Cleared echelle profile state from RunEngine readiness gate".to_string()
                    })
            };

            match result {
                Ok(message) => {
                    let _ = action_tx.send(ImageViewerAction::EchelleCalibrationSynced { message });
                }
                Err(error) => {
                    let _ = action_tx.send(ImageViewerAction::EchelleCalibrationSyncError(
                        format!("Failed to sync echelle calibration gate: {error}"),
                    ));
                }
            }
        });
    }

    fn ensure_echelle_calibration_editor_profile(&mut self) {
        if self.echelle_cal_ui.editor_profile.is_some() {
            return;
        }
        if let Some(profile) = self.echelle_profile_cache.profile() {
            self.echelle_cal_ui.editor_profile = Some((**profile).clone());
            self.echelle_cal_ui.editor_last_loaded_path =
                self.echelle_profile_cache.path().map(|p| p.to_path_buf());
            return;
        }
        self.echelle_cal_ui.editor_profile = Some(self.default_echelle_calibration_profile());
        self.echelle_cal_ui.editor_dirty = true;
        self.echelle_cal_ui.status_message =
            Some("Created new draft calibration profile from current frame metadata".to_string());
    }

    fn default_echelle_calibration_profile(&self) -> EchelleCalibrationProfile {
        let frame_width = self.width.max(1);
        let frame_height = self.height.max(1);
        let sample_end = frame_width.saturating_sub(1).min(1023);
        let center_y = (f64::from(frame_height) / 2.0).max(1.0);
        EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: None,
            display_name: format!("Mechelle Draft {}x{}", frame_width, frame_height),
            compatibility: EchelleFrameCompatibility {
                sensor_width: frame_width,
                sensor_height: frame_height,
                frame_width,
                frame_height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: (self.bit_depth > 0).then_some(self.bit_depth),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Positive,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: 4.0,
                background: None,
            },
            orders: vec![EchelleOrderCalibration {
                relative_index: 0,
                physical_order_number: None,
                sample_start: 0,
                sample_end,
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![center_y],
                    domain_start: 0.0,
                    domain_end: f64::from(frame_width.saturating_sub(1)) + 1.0,
                },
                wavelength: EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![0.0, 1.0],
                    domain_start: 0.0,
                    domain_end: f64::from(frame_width.saturating_sub(1)) + 1.0,
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: Some(4.0),
                enabled: true,
                notes: Some("Draft placeholder order; replace with traced Mechelle orders".into()),
            }],
            corrections: Default::default(),
            provenance: EchelleProvenance {
                creator_tool: "rust-daq-image-viewer".to_string(),
                creator_version: None,
                created_at_utc: chrono::Utc::now(),
                source_frame_ids: Vec::new(),
                notes: Some("Draft created from Image Viewer calibration workspace".to_string()),
            },
        }
    }

    fn mark_echelle_editor_dirty(&mut self) {
        self.echelle_cal_ui.editor_dirty = true;
        self.echelle_cal_ui.last_error = None;
    }

    fn save_echelle_editor_profile_to_path(
        &mut self,
        activate_after_save: bool,
    ) -> Result<std::path::PathBuf, String> {
        let mut profile = self
            .echelle_cal_ui
            .editor_profile
            .clone()
            .ok_or_else(|| "No calibration profile in editor".to_string())?;
        profile.provenance.created_at_utc = chrono::Utc::now();
        let mut note = String::from("Saved from Image Viewer calibration workspace");
        if let Some(existing) = profile.provenance.notes.as_deref() {
            if !existing.trim().is_empty() && !existing.contains("Saved from Image Viewer") {
                note = format!("{existing}\n{note}");
            } else if !existing.trim().is_empty() {
                note = existing.to_string();
            }
        }
        profile.provenance.notes = Some(note);
        let path_text = self.echelle_cal_ui.save_as_path_text.trim();
        if path_text.is_empty() {
            return Err("Enter a .toml or .json path to save the calibration profile".to_string());
        }
        let path = std::path::PathBuf::from(path_text);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create calibration profile directory {}: {e}",
                        parent.display()
                    )
                })?;
            }
        }
        profile
            .save_to_path(&path)
            .map_err(|e| format!("Failed to save calibration profile {}: {e}", path.display()))?;
        self.echelle_cal_ui.editor_dirty = false;
        self.echelle_cal_ui.editor_last_loaded_path = Some(path.clone());
        self.echelle_cal_ui.status_message =
            Some(format!("Saved calibration profile to {}", path.display()));
        self.echelle_cal_ui.last_error = None;
        if activate_after_save {
            self.set_echelle_profile_path(path.clone());
            self.poll_echelle_profile_cache();
        }
        Ok(path)
    }

    fn import_echelle_calibration_points_from_path(
        &mut self,
        path: &std::path::Path,
    ) -> Result<usize, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let items = serde_json::from_str::<Vec<EchelleCalibrationPointUi>>(&text)
            .map_err(|e| format!("Failed to parse calibration points JSON: {e}"))?;
        let count = items.len();
        self.echelle_cal_ui.calibration_points = items;
        self.echelle_cal_ui.selected_point_idx = self
            .echelle_cal_ui
            .selected_point_idx
            .min(count.saturating_sub(1));
        Ok(count)
    }

    fn export_echelle_calibration_points_to_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.echelle_cal_ui.calibration_points)
            .map_err(|e| format!("Failed to serialize calibration points: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    fn import_echelle_line_list_from_path(
        &mut self,
        path: &std::path::Path,
    ) -> Result<usize, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let items = serde_json::from_str::<Vec<EchelleLineListEntryUi>>(&text)
            .map_err(|e| format!("Failed to parse line list JSON: {e}"))?;
        let count = items.len();
        self.echelle_cal_ui.line_list = items;
        Ok(count)
    }

    fn export_echelle_line_list_to_path(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.echelle_cal_ui.line_list)
            .map_err(|e| format!("Failed to serialize line list: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    #[allow(clippy::cast_possible_truncation)]
    fn auto_detect_trace_seeds_from_current_frame(&mut self) -> Result<usize, String> {
        self.ensure_echelle_calibration_editor_profile();
        let frame = self
            .last_frame_data
            .as_ref()
            .ok_or_else(|| "No frame available for auto-detect".to_string())?;
        let profile = self
            .echelle_cal_ui
            .editor_profile
            .as_ref()
            .ok_or_else(|| "No editor profile loaded".to_string())?;

        let centers = detect_cross_dispersion_peaks_from_frame(
            frame.as_ref(),
            self.width,
            self.height,
            self.bit_depth,
            profile.orientation.dispersion_axis,
            self.echelle_cal_ui
                .trace_auto_detect_min_separation_px
                .max(1),
            self.echelle_cal_ui.trace_overlay_max_orders.max(1) as usize,
            self.echelle_cal_ui
                .trace_auto_detect_threshold_fraction
                .clamp(0.01, 0.95),
        )?;
        if centers.is_empty() {
            return Err("No candidate order peaks detected in the current frame".to_string());
        }

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
            let dispersion_len = match profile.orientation.dispersion_axis {
                DetectorAxis::X => profile.compatibility.frame_width.max(1),
                DetectorAxis::Y => profile.compatibility.frame_height.max(1),
            };
            let cross_roi_offset = match profile.orientation.cross_dispersion_axis {
                DetectorAxis::X => f64::from(profile.compatibility.roi_x),
                DetectorAxis::Y => f64::from(profile.compatibility.roi_y),
            };
            let template_wavelength = profile
                .orders
                .get(self.echelle_cal_ui.selected_order_edit_idx)
                .map(|o| o.wavelength.clone())
                .or_else(|| profile.orders.first().map(|o| o.wavelength.clone()))
                .unwrap_or(EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![0.0, 1.0],
                    domain_start: 0.0,
                    domain_end: f64::from(dispersion_len.saturating_sub(1)) + 1.0,
                    unit: "nm".to_string(),
                });

            let mut new_orders = Vec::with_capacity(centers.len());
            for (idx, center_local) in centers.iter().enumerate() {
                new_orders.push(EchelleOrderCalibration {
                    relative_index: idx as u32,
                    physical_order_number: None,
                    sample_start: 0,
                    sample_end: dispersion_len.saturating_sub(1),
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![*center_local + cross_roi_offset],
                        domain_start: 0.0,
                        domain_end: f64::from(dispersion_len.saturating_sub(1)) + 1.0,
                    },
                    wavelength: template_wavelength.clone(),
                    aperture_half_width_px: None,
                    enabled: true,
                    notes: Some("Auto-detected constant trace seed from frame peaks".to_string()),
                });
            }
            profile.orders = new_orders;
        }
        self.echelle_cal_ui.selected_order_edit_idx = 0;
        self.mark_echelle_editor_dirty();
        Ok(centers.len())
    }

    fn export_selected_order_blaze_preview_artifact(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let preview = self
            .echelle_preview
            .as_ref()
            .ok_or_else(|| "No extracted preview is available".to_string())?;
        let order = preview
            .orders
            .get(self.echelle_selected_order_plot)
            .ok_or_else(|| "No selected order preview available".to_string())?;
        if order.flux.is_empty() {
            return Err("Selected order preview has empty flux".to_string());
        }

        let max_flux = order
            .flux
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
            .max(1e-12);
        use std::fmt::Write;
        let mut out = String::from("sample,wavelength,raw_flux,normalized_flux\n");
        for (i, (&w, &f)) in order.wavelengths.iter().zip(&order.flux).enumerate() {
            let _ = writeln!(out, "{i},{w},{f},{}", f / max_flux);
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create blaze artifact directory {}: {e}",
                        parent.display()
                    )
                })?;
            }
        }
        std::fs::write(path, out)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
            profile.corrections.blaze = Some(EchelleArtifactRef {
                path: path.display().to_string(),
                sha256: None,
                format: Some("csv".to_string()),
            });
            self.mark_echelle_editor_dirty();
        }
        Ok(())
    }

    fn render_echelle_calibration_workspace(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Calibration Workspace (Mechelle / Echelle)")
            .default_open(true)
            .show(ui, |ui| {
                ui.small(
                    "Author and validate echelle calibration profiles, order traces, arc picks, and fit diagnostics.",
                );

                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (EchelleCalibrationTab::Profile, "Profile"),
                        (EchelleCalibrationTab::Trace, "Trace"),
                        (EchelleCalibrationTab::LinePoints, "Arc/Points"),
                        (EchelleCalibrationTab::WavelengthFit, "Wavelength Fit"),
                        (EchelleCalibrationTab::BlazeFlat, "Blaze/Flat"),
                        (EchelleCalibrationTab::MechelleNotes, "Mechelle UX"),
                    ] {
                        ui.selectable_value(&mut self.echelle_cal_ui.tab, tab, label);
                    }
                });

                if let Some(msg) = &self.echelle_cal_ui.status_message {
                    ui.small(msg);
                }
                if let Some(err) = &self.echelle_cal_ui.last_error {
                    ui.colored_label(colors::ERROR, err);
                }

                ui.separator();

                let mut trigger_load_editor = false;
                let mut trigger_save_editor = false;
                let mut trigger_save_activate = false;
                let mut trigger_activate_only = false;

                ui.horizontal_wrapped(|ui| {
                    ui.label("Profile path:");
                    ui.text_edit_singleline(&mut self.echelle_cal_ui.save_as_path_text);
                    if ui.button("Load Editor").clicked() {
                        trigger_load_editor = true;
                    }
                    if ui.button("Save").clicked() {
                        trigger_save_editor = true;
                    }
                    if ui.button("Save + Activate").clicked() {
                        trigger_save_activate = true;
                    }
                    if ui.button("Activate Path").clicked() {
                        trigger_activate_only = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Clone Active -> Editor").clicked() {
                        if let Some(profile) = self.echelle_profile_cache.profile() {
                            self.echelle_cal_ui.editor_profile = Some((**profile).clone());
                            self.echelle_cal_ui.editor_dirty = false;
                            self.echelle_cal_ui.editor_last_loaded_path =
                                self.echelle_profile_cache.path().map(|p| p.to_path_buf());
                            if self.echelle_cal_ui.save_as_path_text.is_empty() {
                                if let Some(path) = self.echelle_profile_cache.path() {
                                    self.echelle_cal_ui.save_as_path_text = path.display().to_string();
                                }
                            }
                            self.echelle_cal_ui.status_message =
                                Some("Editor cloned from active profile".to_string());
                            self.echelle_cal_ui.last_error = None;
                        } else {
                            self.echelle_cal_ui.last_error =
                                Some("No active profile available to clone".to_string());
                        }
                    }
                    if ui.button("New Draft From Frame").clicked() {
                        self.echelle_cal_ui.editor_profile = Some(self.default_echelle_calibration_profile());
                        self.echelle_cal_ui.editor_dirty = true;
                        self.echelle_cal_ui.editor_last_loaded_path = None;
                        self.echelle_cal_ui.status_message =
                            Some("Created new draft calibration profile".to_string());
                        self.echelle_cal_ui.last_error = None;
                    }
                    if ui.button("Reset Editor From Active").clicked() {
                        if let Some(profile) = self.echelle_profile_cache.profile() {
                            self.echelle_cal_ui.editor_profile = Some((**profile).clone());
                            self.echelle_cal_ui.editor_dirty = false;
                            self.echelle_cal_ui.status_message =
                                Some("Editor reset from active profile".to_string());
                            self.echelle_cal_ui.last_error = None;
                        }
                    }
                    if let Some(path) = &self.echelle_cal_ui.editor_last_loaded_path {
                        ui.small(format!("Editor source: {}", path.display()));
                    } else {
                        ui.small("Editor source: draft");
                    }
                    if self.echelle_cal_ui.editor_dirty {
                        ui.colored_label(colors::WARNING, "Unsaved editor changes");
                    }
                });

                if trigger_load_editor {
                    let path = std::path::PathBuf::from(self.echelle_cal_ui.save_as_path_text.trim());
                    if self.echelle_cal_ui.save_as_path_text.trim().is_empty() {
                        self.echelle_cal_ui.last_error =
                            Some("Enter a profile path before loading".to_string());
                    } else {
                        match EchelleCalibrationProfile::load_from_path(&path) {
                            Ok(profile) => {
                                self.echelle_cal_ui.editor_profile = Some(profile);
                                self.echelle_cal_ui.editor_dirty = false;
                                self.echelle_cal_ui.editor_last_loaded_path = Some(path.clone());
                                self.echelle_cal_ui.status_message =
                                    Some(format!("Loaded editor profile from {}", path.display()));
                                self.echelle_cal_ui.last_error = None;
                            }
                            Err(err) => {
                                self.echelle_cal_ui.last_error = Some(format!(
                                    "Failed to load editor profile {}: {err}",
                                    path.display()
                                ));
                            }
                        }
                    }
                }
                if trigger_save_editor {
                    if let Err(err) = self.save_echelle_editor_profile_to_path(false) {
                        self.echelle_cal_ui.last_error = Some(err);
                    }
                }
                if trigger_save_activate {
                    if let Err(err) = self.save_echelle_editor_profile_to_path(true) {
                        self.echelle_cal_ui.last_error = Some(err);
                    }
                }
                if trigger_activate_only {
                    let path_text = self.echelle_cal_ui.save_as_path_text.trim();
                    if path_text.is_empty() {
                        self.echelle_cal_ui.last_error =
                            Some("Enter a profile path before activation".to_string());
                    } else {
                        self.set_echelle_profile_path(std::path::PathBuf::from(path_text));
                        self.poll_echelle_profile_cache();
                    }
                }

                match self.echelle_cal_ui.tab {
                    EchelleCalibrationTab::Profile => self.render_echelle_calibration_profile_tab(ui),
                    EchelleCalibrationTab::Trace => self.render_echelle_calibration_trace_tab(ui),
                    EchelleCalibrationTab::LinePoints => {
                        self.render_echelle_calibration_line_points_tab(ui);
                    }
                    EchelleCalibrationTab::WavelengthFit => {
                        self.render_echelle_calibration_wavelength_fit_tab(ui);
                    }
                    EchelleCalibrationTab::BlazeFlat => self.render_echelle_calibration_blaze_tab(ui),
                    EchelleCalibrationTab::MechelleNotes => {
                        self.render_echelle_calibration_mechelle_notes_tab(ui);
                    }
                }
            });
    }

    fn render_echelle_calibration_profile_tab(&mut self, ui: &mut egui::Ui) {
        self.ensure_echelle_calibration_editor_profile();
        let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() else {
            ui.weak("No editor profile loaded.");
            return;
        };

        let mut changed = false;
        egui::Grid::new("echelle_cal_profile_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Display name");
                changed |= ui.text_edit_singleline(&mut profile.display_name).changed();
                ui.end_row();

                ui.label("Profile ID");
                let mut id_text = profile.profile_id.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut id_text).changed() {
                    profile.profile_id = if id_text.trim().is_empty() {
                        None
                    } else {
                        Some(id_text)
                    };
                    changed = true;
                }
                ui.end_row();

                ui.label("Schema");
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(egui::DragValue::new(&mut profile.schema_version.major).range(1..=9))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut profile.schema_version.minor).range(0..=99))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut profile.schema_version.patch).range(0..=99))
                        .changed();
                });
                ui.end_row();

                ui.label("Compatibility");
                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.frame_width)
                                .range(1..=8192)
                                .prefix("frame_w "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.frame_height)
                                .range(1..=8192)
                                .prefix("frame_h "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.roi_x)
                                .range(0..=8192)
                                .prefix("roi_x "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.roi_y)
                                .range(0..=8192)
                                .prefix("roi_y "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.binning_x)
                                .range(1..=16)
                                .prefix("bin_x "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.binning_y)
                                .range(1..=16)
                                .prefix("bin_y "),
                        )
                        .changed();
                });
                ui.end_row();

                ui.label("Provenance");
                ui.vertical(|ui| {
                    changed |= ui
                        .text_edit_singleline(&mut profile.provenance.creator_tool)
                        .changed();
                    let mut version = profile
                        .provenance
                        .creator_version
                        .clone()
                        .unwrap_or_default();
                    if ui.text_edit_singleline(&mut version).changed() {
                        profile.provenance.creator_version = if version.trim().is_empty() {
                            None
                        } else {
                            Some(version)
                        };
                        changed = true;
                    }
                    let notes = profile.provenance.notes.get_or_insert_with(String::new);
                    changed |= ui.text_edit_multiline(notes).changed();
                    ui.small(format!(
                        "created_at_utc: {}",
                        profile.provenance.created_at_utc
                    ));
                });
                ui.end_row();
            });

        ui.separator();
        match profile.validate() {
            Ok(()) => ui.colored_label(colors::SUCCESS, "Profile validates"),
            Err(err) => ui.colored_label(colors::WARNING, format!("Validation issue: {err}")),
        };

        if changed {
            self.mark_echelle_editor_dirty();
        }
    }

    fn render_echelle_calibration_trace_tab(&mut self, ui: &mut egui::Ui) {
        self.ensure_echelle_calibration_editor_profile();

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.echelle_cal_ui.trace_overlay_enabled,
                "Show trace overlays on image",
            );
            ui.checkbox(
                &mut self.echelle_cal_ui.trace_overlay_all_orders,
                "All orders",
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_overlay_sample_step)
                    .range(1..=256)
                    .prefix("step "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_overlay_max_orders)
                    .range(1..=256)
                    .prefix("max "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_nudge_px)
                    .range(0.01..=20.0)
                    .speed(0.05)
                    .prefix("nudge "),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_auto_detect_min_separation_px)
                    .range(1..=512)
                    .prefix("auto min-sep "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_auto_detect_threshold_fraction)
                    .range(0.01..=0.95)
                    .speed(0.01)
                    .prefix("auto thr "),
            );
            if ui
                .button("Auto-Detect Trace Seeds From Current Frame")
                .clicked()
            {
                match self.auto_detect_trace_seeds_from_current_frame() {
                    Ok(count) => {
                        self.echelle_cal_ui.status_message = Some(format!(
                            "Auto-detected {count} trace seed(s) from current frame"
                        ));
                        self.echelle_cal_ui.last_error = None;
                    }
                    Err(err) => self.echelle_cal_ui.last_error = Some(err),
                }
            }
            ui.small("Creates constant-trace seeds from cross-dispersion peaks; refine manually.");
        });

        let Some(profile_ref) = self.echelle_cal_ui.editor_profile.as_ref() else {
            ui.weak("No editor profile loaded.");
            return;
        };

        let order_labels: Vec<String> = profile_ref
            .orders
            .iter()
            .enumerate()
            .map(|(idx, order)| {
                format!(
                    "#{idx} rel={}{}{}",
                    order.relative_index,
                    order
                        .physical_order_number
                        .map(|m| format!(" m={m}"))
                        .unwrap_or_default(),
                    if order.enabled { "" } else { " (disabled)" }
                )
            })
            .collect();

        if order_labels.is_empty() {
            ui.weak("No orders in editor profile.");
            return;
        }

        if self.echelle_cal_ui.selected_order_edit_idx >= order_labels.len() {
            self.echelle_cal_ui.selected_order_edit_idx = 0;
        }

        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("echelle_cal_trace_order_select")
                .selected_text(
                    order_labels
                        .get(self.echelle_cal_ui.selected_order_edit_idx)
                        .cloned()
                        .unwrap_or_else(|| "order".to_string()),
                )
                .show_ui(ui, |ui| {
                    for (idx, label) in order_labels.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.echelle_cal_ui.selected_order_edit_idx,
                            idx,
                            label,
                        );
                    }
                });
            if ui.button("Add Order (Clone)").clicked() {
                if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
                    let selected = self
                        .echelle_cal_ui
                        .selected_order_edit_idx
                        .min(profile.orders.len() - 1);
                    let mut new_order = profile.orders[selected].clone();
                    let next_rel = profile
                        .orders
                        .iter()
                        .map(|o| o.relative_index)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    new_order.relative_index = next_rel;
                    new_order.physical_order_number = None;
                    new_order.notes = Some("Cloned for manual trace adjustment".to_string());
                    profile.orders.push(new_order);
                    self.echelle_cal_ui.selected_order_edit_idx = profile.orders.len() - 1;
                    self.mark_echelle_editor_dirty();
                }
            }
            if ui.button("Remove Selected Order").clicked() {
                if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
                    if profile.orders.len() > 1 {
                        let idx = self
                            .echelle_cal_ui
                            .selected_order_edit_idx
                            .min(profile.orders.len() - 1);
                        profile.orders.remove(idx);
                        self.echelle_cal_ui.selected_order_edit_idx = self
                            .echelle_cal_ui
                            .selected_order_edit_idx
                            .min(profile.orders.len().saturating_sub(1));
                        self.mark_echelle_editor_dirty();
                    } else {
                        self.echelle_cal_ui.last_error =
                            Some("Profile must contain at least one order".to_string());
                    }
                }
            }
        });

        let selected_idx = self.echelle_cal_ui.selected_order_edit_idx;
        let mut changed = false;
        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
            if let Some(order) = profile.orders.get_mut(selected_idx) {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    changed |= ui.checkbox(&mut order.enabled, "Enabled").changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut order.relative_index)
                                .range(0..=999)
                                .prefix("rel "),
                        )
                        .changed();
                    let mut physical_text = order
                        .physical_order_number
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    ui.label("m:");
                    if ui.text_edit_singleline(&mut physical_text).changed() {
                        order.physical_order_number = if physical_text.trim().is_empty() {
                            None
                        } else {
                            physical_text.parse::<i32>().ok()
                        };
                        changed = true;
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut order.sample_start)
                                .range(0..=8191)
                                .prefix("sample_start "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut order.sample_end)
                                .range(0..=8191)
                                .prefix("sample_end "),
                        )
                        .changed();
                    let mut aperture_enabled = order.aperture_half_width_px.is_some();
                    if ui
                        .checkbox(&mut aperture_enabled, "Order aperture override")
                        .changed()
                    {
                        if aperture_enabled && order.aperture_half_width_px.is_none() {
                            order.aperture_half_width_px = Some(4.0);
                        }
                        if !aperture_enabled {
                            order.aperture_half_width_px = None;
                        }
                        changed = true;
                    }
                    if let Some(ap) = &mut order.aperture_half_width_px {
                        changed |= ui
                            .add(
                                egui::DragValue::new(ap)
                                    .range(0.1..=128.0)
                                    .speed(0.1)
                                    .prefix("half-width "),
                            )
                            .changed();
                    }
                });

                let notes = order.notes.get_or_insert_with(String::new);
                changed |= ui.text_edit_multiline(notes).changed();

                let EchelleTraceModel::Polynomial {
                    basis,
                    coefficients,
                    domain_start,
                    domain_end,
                } = &mut order.trace;
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Trace basis:");
                    ui.selectable_value(basis, PolynomialBasis::Monomial, "Monomial");
                    ui.selectable_value(basis, PolynomialBasis::Chebyshev, "Chebyshev");
                    changed |= ui
                        .add(
                            egui::DragValue::new(domain_start)
                                .speed(0.5)
                                .prefix("domain_start "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(domain_end)
                                .speed(0.5)
                                .prefix("domain_end "),
                        )
                        .changed();
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Nudge -Y").clicked() {
                        if let Some(c0) = coefficients.first_mut() {
                            *c0 -= self.echelle_cal_ui.trace_nudge_px;
                            changed = true;
                        }
                    }
                    if ui.button("Nudge +Y").clicked() {
                        if let Some(c0) = coefficients.first_mut() {
                            *c0 += self.echelle_cal_ui.trace_nudge_px;
                            changed = true;
                        }
                    }
                    if ui.button("Add Coeff").clicked() {
                        coefficients.push(0.0);
                        changed = true;
                    }
                    if ui.button("Pop Coeff").clicked() && coefficients.len() > 1 {
                        coefficients.pop();
                        changed = true;
                    }
                });
                egui::Grid::new("echelle_cal_trace_coeff_grid").show(ui, |ui| {
                    for (i, coeff) in coefficients.iter_mut().enumerate() {
                        ui.label(format!("c{i}"));
                        changed |= ui.add(egui::DragValue::new(coeff).speed(0.01)).changed();
                        ui.end_row();
                    }
                });
            }
        }

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_ref() {
            match profile.validate() {
                Ok(()) => ui.colored_label(colors::SUCCESS, "Trace/order edits validate"),
                Err(err) => ui.colored_label(colors::WARNING, format!("Validation issue: {err}")),
            };
        }
        if changed {
            self.mark_echelle_editor_dirty();
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn render_echelle_calibration_line_points_tab(&mut self, ui: &mut egui::Ui) {
        self.ensure_echelle_calibration_editor_profile();

        ui.horizontal_wrapped(|ui| {
            ui.label("Points JSON:");
            ui.text_edit_singleline(&mut self.echelle_cal_ui.points_path_text);
            if ui.button("Import Points").clicked() {
                let path_text = self.echelle_cal_ui.points_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error = Some("Enter a points JSON path".to_string());
                } else {
                    match self.import_echelle_calibration_points_from_path(std::path::Path::new(
                        &path_text,
                    )) {
                        Ok(count) => {
                            self.echelle_cal_ui.status_message =
                                Some(format!("Imported {count} calibration points"));
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
            if ui.button("Export Points").clicked() {
                let path_text = self.echelle_cal_ui.points_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error = Some("Enter a points JSON path".to_string());
                } else {
                    match self
                        .export_echelle_calibration_points_to_path(std::path::Path::new(&path_text))
                    {
                        Ok(()) => {
                            self.echelle_cal_ui.status_message =
                                Some("Exported calibration points".to_string());
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Line list JSON:");
            ui.text_edit_singleline(&mut self.echelle_cal_ui.line_list_path_text);
            if ui.button("Import Line List").clicked() {
                let path_text = self.echelle_cal_ui.line_list_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error =
                        Some("Enter a line list JSON path".to_string());
                } else {
                    match self.import_echelle_line_list_from_path(std::path::Path::new(&path_text))
                    {
                        Ok(count) => {
                            self.echelle_cal_ui.status_message =
                                Some(format!("Imported {count} line-list entries"));
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
            if ui.button("Export Line List").clicked() {
                let path_text = self.echelle_cal_ui.line_list_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error =
                        Some("Enter a line list JSON path".to_string());
                } else {
                    match self.export_echelle_line_list_to_path(std::path::Path::new(&path_text)) {
                        Ok(()) => {
                            self.echelle_cal_ui.status_message =
                                Some("Exported line list".to_string());
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
        });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add Point").clicked() {
                let (order_relative_index, x_sample, y_pixel, wavelength) =
                    if let Some(link) = self.echelle_plot_hover_link {
                        (
                            link.relative_index,
                            link.sample_index as f64,
                            0.0,
                            link.wavelength,
                        )
                    } else {
                        (0, 0.0, 0.0, 0.0)
                    };
                self.echelle_cal_ui
                    .calibration_points
                    .push(EchelleCalibrationPointUi {
                        enabled: true,
                        order_relative_index,
                        x_sample,
                        y_pixel,
                        wavelength,
                        note: String::new(),
                    });
                self.echelle_cal_ui.selected_point_idx = self
                    .echelle_cal_ui
                    .calibration_points
                    .len()
                    .saturating_sub(1);
            }
            if ui.button("Remove Point").clicked()
                && !self.echelle_cal_ui.calibration_points.is_empty()
            {
                let idx = self
                    .echelle_cal_ui
                    .selected_point_idx
                    .min(self.echelle_cal_ui.calibration_points.len() - 1);
                self.echelle_cal_ui.calibration_points.remove(idx);
                self.echelle_cal_ui.selected_point_idx =
                    self.echelle_cal_ui.selected_point_idx.min(
                        self.echelle_cal_ui
                            .calibration_points
                            .len()
                            .saturating_sub(1),
                    );
            }
            ui.small(format!(
                "{} calibration points",
                self.echelle_cal_ui.calibration_points.len()
            ));
        });
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                egui::Grid::new("echelle_cal_points_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Sel");
                        ui.strong("On");
                        ui.strong("Order");
                        ui.strong("x");
                        ui.strong("y");
                        ui.strong("λ");
                        ui.strong("Note");
                        ui.end_row();
                        for (idx, p) in self
                            .echelle_cal_ui
                            .calibration_points
                            .iter_mut()
                            .enumerate()
                        {
                            ui.selectable_value(
                                &mut self.echelle_cal_ui.selected_point_idx,
                                idx,
                                "•",
                            );
                            ui.checkbox(&mut p.enabled, "");
                            ui.add(
                                egui::DragValue::new(&mut p.order_relative_index).range(0..=999),
                            );
                            ui.add(egui::DragValue::new(&mut p.x_sample).speed(0.1));
                            ui.add(egui::DragValue::new(&mut p.y_pixel).speed(0.1));
                            ui.add(egui::DragValue::new(&mut p.wavelength).speed(0.001));
                            ui.text_edit_singleline(&mut p.note);
                            ui.end_row();
                        }
                    });
            });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add Line").clicked() {
                self.echelle_cal_ui.line_list.push(EchelleLineListEntryUi {
                    enabled: true,
                    wavelength: 0.0,
                    label: String::new(),
                });
            }
            if ui.button("Remove Line").clicked() && !self.echelle_cal_ui.line_list.is_empty() {
                self.echelle_cal_ui.line_list.pop();
            }
            ui.small(format!(
                "{} line-list entries",
                self.echelle_cal_ui.line_list.len()
            ));
        });
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                egui::Grid::new("echelle_cal_line_list_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("On");
                        ui.strong("λ");
                        ui.strong("Label");
                        ui.end_row();
                        for line in &mut self.echelle_cal_ui.line_list {
                            ui.checkbox(&mut line.enabled, "");
                            ui.add(egui::DragValue::new(&mut line.wavelength).speed(0.001));
                            ui.text_edit_singleline(&mut line.label);
                            ui.end_row();
                        }
                    });
            });
    }

    fn render_echelle_calibration_wavelength_fit_tab(&mut self, ui: &mut egui::Ui) {
        self.ensure_echelle_calibration_editor_profile();
        let Some(_) = self.echelle_cal_ui.editor_profile.as_ref() else {
            ui.weak("No editor profile loaded.");
            return;
        };

        let mut trigger_fit_selected = false;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.fit_outlier_sigma)
                    .range(0.5..=10.0)
                    .speed(0.1)
                    .prefix("outlier σ "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.fit_rms_acceptance_px)
                    .range(0.01..=10.0)
                    .speed(0.01)
                    .prefix("accept RMS "),
            );
            if ui.button("Fit Selected Order (LSQ)").clicked() {
                trigger_fit_selected = true;
            }
            ui.small("Manual picks only (assisted matching planned later).");
        });

        if trigger_fit_selected {
            let selected_idx = self.echelle_cal_ui.selected_order_edit_idx;
            let points = self.echelle_cal_ui.calibration_points.clone();
            let fit_result = if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
                if let Some(order) = profile.orders.get_mut(selected_idx) {
                    fit_wavelength_model_for_order_from_points(order, &points)
                } else {
                    Err("Selected order is out of range".to_string())
                }
            } else {
                Err("No editor profile loaded".to_string())
            };
            match fit_result {
                Ok(summary) => {
                    self.echelle_cal_ui.status_message = Some(summary);
                    self.echelle_cal_ui.last_error = None;
                    self.mark_echelle_editor_dirty();
                }
                Err(err) => {
                    self.echelle_cal_ui.last_error = Some(err);
                }
            }
        }

        let Some(profile) = self.echelle_cal_ui.editor_profile.as_ref() else {
            ui.weak("No editor profile loaded.");
            return;
        };

        let mut global_count = 0usize;
        let mut global_sum_sq = 0.0f64;
        for order in &profile.orders {
            let residuals = compute_wavelength_fit_residuals_for_order(
                order,
                &self.echelle_cal_ui.calibration_points,
            );
            global_count += residuals.len();
            global_sum_sq += residuals.iter().map(|(_, r)| r * r).sum::<f64>();
        }
        if global_count > 0 {
            #[allow(clippy::cast_precision_loss)]
            let global_rms = (global_sum_sq / global_count as f64).sqrt();
            ui.small(format!(
                "Global residual summary across editor orders: {} points | RMS {:.6}",
                global_count, global_rms
            ));
        }

        let selected_order = profile
            .orders
            .get(self.echelle_cal_ui.selected_order_edit_idx)
            .or_else(|| profile.orders.first());
        let Some(order) = selected_order else {
            ui.weak("No orders available.");
            return;
        };

        let residuals = compute_wavelength_fit_residuals_for_order(
            order,
            &self.echelle_cal_ui.calibration_points,
        );
        let count = residuals.len();
        if count == 0 {
            ui.weak("No enabled calibration points for the selected order.");
            return;
        }

        #[allow(clippy::cast_precision_loss)]
        let rms = (residuals.iter().map(|(_, r)| r * r).sum::<f64>() / count as f64).sqrt();
        #[allow(clippy::cast_precision_loss)]
        let mean_abs = residuals.iter().map(|(_, r)| r.abs()).sum::<f64>() / count as f64;
        #[allow(clippy::cast_precision_loss)]
        let mean = residuals.iter().map(|(_, r)| *r).sum::<f64>() / count as f64;
        #[allow(clippy::cast_precision_loss)]
        let stddev = (residuals
            .iter()
            .map(|(_, r)| {
                let d = *r - mean;
                d * d
            })
            .sum::<f64>()
            / count as f64)
            .sqrt();
        let sigma = self.echelle_cal_ui.fit_outlier_sigma.max(0.1);
        let outliers = residuals
            .iter()
            .filter(|(_, r)| stddev > 0.0 && ((*r - mean).abs() / stddev) > sigma)
            .count();

        ui.horizontal_wrapped(|ui| {
            ui.small(format!("Order rel={}", order.relative_index));
            ui.separator();
            ui.small(format!("points: {count}"));
            ui.separator();
            ui.small(format!("RMS: {:.6}", rms));
            ui.separator();
            ui.small(format!("mean|resid|: {:.6}", mean_abs));
            ui.separator();
            ui.small(format!("outliers@{sigma:.1}σ: {outliers}"));
            ui.separator();
            if rms <= self.echelle_cal_ui.fit_rms_acceptance_px {
                ui.colored_label(colors::SUCCESS, "Within acceptance threshold");
            } else {
                ui.colored_label(colors::WARNING, "Exceeds acceptance threshold");
            }
        });

        let points: Vec<[f64; 2]> = residuals.iter().map(|(x, r)| [*x, *r]).collect();
        Plot::new("echelle_wavelength_fit_residual_plot")
            .height(160.0)
            .allow_scroll(false)
            .allow_zoom(true)
            .allow_drag(true)
            .x_axis_label("sample")
            .y_axis_label("λ residual")
            .show(ui, |plot_ui| {
                plot_ui.points(
                    Points::new("residuals", PlotPoints::new(points))
                        .radius(3.0)
                        .color(egui::Color32::from_rgb(255, 185, 60)),
                );
                plot_ui.line(Line::new(
                    "zero",
                    PlotPoints::new(vec![
                        [f64::from(order.sample_start), 0.0],
                        [f64::from(order.sample_end), 0.0],
                    ]),
                ));
            });
    }

    fn render_echelle_calibration_blaze_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.echelle_cal_ui.blaze_preview_enabled,
                "Preview blaze-corrected overlay",
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.blaze_preview_scale)
                    .range(0.05..=100.0)
                    .speed(0.05)
                    .prefix("scale "),
            );
            ui.small("MVP: scalar preview overlay while blaze artifact generation UI is staged.");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Blaze export CSV:");
            ui.text_edit_singleline(&mut self.echelle_cal_ui.blaze_export_path_text);
            if ui.button("Generate From Selected Order Preview").clicked() {
                let path_text = self
                    .echelle_cal_ui
                    .blaze_export_path_text
                    .trim()
                    .to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error =
                        Some("Enter a blaze export CSV path".to_string());
                } else {
                    match self.export_selected_order_blaze_preview_artifact(std::path::Path::new(
                        &path_text,
                    )) {
                        Ok(()) => {
                            self.echelle_cal_ui.status_message =
                                Some(format!("Generated blaze preview artifact {}", path_text));
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
        });

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_ref() {
            ui.small(format!(
                "Blaze artifact: {}",
                profile
                    .corrections
                    .blaze
                    .as_ref()
                    .map(|b| b.path.as_str())
                    .unwrap_or("<not set>")
            ));
            ui.small(format!(
                "Flat-field artifact: {}",
                profile
                    .corrections
                    .flat_field
                    .as_ref()
                    .map(|f| f.path.as_str())
                    .unwrap_or("<not set>")
            ));
        }

        let Some(preview) = &self.echelle_preview else {
            ui.weak("No extracted preview available for blaze/flat comparison.");
            return;
        };
        let Some(order) = preview.orders.get(self.echelle_selected_order_plot) else {
            ui.weak("No selected order preview available.");
            return;
        };
        if order.flux.is_empty() {
            ui.weak("Selected order flux is empty.");
            return;
        }

        #[allow(clippy::cast_precision_loss)]
        let xs: Vec<f64> = if self.echelle_plot_x_axis_mode == EchellePlotXAxisMode::SampleIndex {
            (0..order.flux.len()).map(|i| i as f64).collect()
        } else {
            order.wavelengths.clone()
        };
        let raw_points: Vec<[f64; 2]> = xs
            .iter()
            .copied()
            .zip(order.flux.iter().copied())
            .map(|(x, y)| [x, y])
            .collect();
        let corrected_points: Vec<[f64; 2]> = if self.echelle_cal_ui.blaze_preview_enabled {
            let s = self.echelle_cal_ui.blaze_preview_scale.max(1e-9);
            xs.iter()
                .copied()
                .zip(order.flux.iter().copied())
                .map(|(x, y)| [x, y / s])
                .collect()
        } else {
            Vec::new()
        };

        Plot::new("echelle_blaze_preview_compare_plot")
            .height(170.0)
            .allow_scroll(false)
            .x_axis_label(
                if self.echelle_plot_x_axis_mode == EchellePlotXAxisMode::SampleIndex {
                    "sample"
                } else {
                    order.wavelength_unit.as_str()
                },
            )
            .y_axis_label("counts")
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("Uncorrected", PlotPoints::new(raw_points))
                        .color(egui::Color32::from_rgb(120, 200, 255)),
                );
                if !corrected_points.is_empty() {
                    plot_ui.line(
                        Line::new("Preview corrected", PlotPoints::new(corrected_points))
                            .color(egui::Color32::from_rgb(255, 160, 60)),
                    );
                }
            });
    }

    fn render_echelle_calibration_mechelle_notes_tab(&mut self, ui: &mut egui::Ui) {
        self.ensure_echelle_calibration_editor_profile();
        ui.small(
            "Mechelle-specific UX planning for image slicer / multi-trace-per-order complexity.",
        );
        ui.separator();
        ui.label("Current design assumptions:");
        ui.small("1. A single physical echelle order may present multiple visible traces/slices.");
        ui.small(
            "2. Trace editing UI must support grouping multiple traces under one physical order.",
        );
        ui.small("3. Arc picks should support slice association and confidence labels.");
        ui.small("4. Blaze/flat correction previews should compare per-slice and merged views.");
        ui.small("5. Profile format may need future schema extension for sub-trace components.");
        ui.separator();
        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
            ui.label("Operator notes / vendor quirks / slicer observations:");
            let notes = profile.provenance.notes.get_or_insert_with(String::new);
            if ui.text_edit_multiline(notes).changed() {
                self.mark_echelle_editor_dirty();
            }
        }
    }

    fn build_echelle_trace_overlay_paths(&self) -> Vec<(u32, Vec<(f32, f32)>)> {
        if !self.echelle_cal_ui.trace_overlay_enabled {
            return Vec::new();
        }
        let profile = self
            .echelle_cal_ui
            .editor_profile
            .clone()
            .or_else(|| self.echelle_profile_cache.profile().map(|p| (**p).clone()));
        let Some(profile) = profile else {
            return Vec::new();
        };
        let sample_step = self.echelle_cal_ui.trace_overlay_sample_step.max(1) as usize;
        let max_orders = self.echelle_cal_ui.trace_overlay_max_orders.max(1) as usize;
        let selected_relative = self
            .echelle_cal_ui
            .editor_profile
            .as_ref()
            .and_then(|p| p.orders.get(self.echelle_cal_ui.selected_order_edit_idx))
            .map(|o| o.relative_index);

        let mut out = Vec::new();
        for order in profile.orders.iter().filter(|o| o.enabled) {
            if !self.echelle_cal_ui.trace_overlay_all_orders
                && selected_relative.is_some()
                && Some(order.relative_index) != selected_relative
            {
                continue;
            }
            if out.len() >= max_orders {
                break;
            }
            let mut pts = Vec::new();
            let total_samples = order
                .sample_end
                .saturating_sub(order.sample_start)
                .saturating_add(1) as usize;
            for sample_idx in (0..total_samples).step_by(sample_step) {
                if let Some((x, y)) = order_sample_image_position(&profile, order, sample_idx) {
                    if x.is_finite() && y.is_finite() {
                        pts.push((x, y));
                    }
                }
            }
            if total_samples > 0 {
                let last_idx = total_samples - 1;
                if let Some((x, y)) = order_sample_image_position(&profile, order, last_idx) {
                    if x.is_finite()
                        && y.is_finite()
                        && pts
                            .last()
                            .map(|(px, py)| (*px - x).abs() > 1e-3 || (*py - y).abs() > 1e-3)
                            .unwrap_or(true)
                    {
                        pts.push((x, y));
                    }
                }
            }
            if pts.len() >= 2 {
                out.push((order.relative_index, pts));
            }
        }
        out
    }

    /// Spawn background thread for RGBA conversion (bd-xifj)
    ///
    /// This moves CPU-intensive pixel conversion off the UI thread to prevent
    /// UI freezes on 4K 16-bit images at high frame rates.
    ///
    /// Returns true if the converter thread was spawned successfully, false otherwise.
    /// On failure, RGBA conversion will fall back to synchronous mode.
    fn spawn_rgba_converter(&mut self) -> bool {
        // Use bounded channel to prevent unbounded queue growth
        // Queue size of 2 is sufficient: 1 processing, 1 waiting
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<RgbaConversionRequest>(2);
        let (result_tx, result_rx) = std::sync::mpsc::channel::<RgbaConversionResult>();
        // Channel for recycling buffers from UI thread back to converter (bd-wdx3)
        let (recycle_tx, recycle_rx) = std::sync::mpsc::channel::<Vec<u8>>();

        // Spawn dedicated thread for RGBA conversion
        let spawn_result = std::thread::Builder::new()
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
            });

        match spawn_result {
            Ok(_handle) => {
                self.rgba_request_tx = Some(request_tx);
                self.rgba_rx = Some(result_rx);
                self.rgba_recycle_tx = Some(recycle_tx);
                true
            }
            Err(e) => {
                tracing::error!("Failed to spawn RGBA converter thread: {}. Falling back to synchronous conversion.", e);
                self.rgba_sync_mode = true;
                false
            }
        }
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
        // Spawn converter thread lazily on first use (skip if already known unavailable)
        if self.rgba_request_tx.is_none() && !self.rgba_sync_mode {
            self.spawn_rgba_converter();
        }

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
            contrast_mode: self.contrast_mode,
            percentile_low: self.percentile_low,
            percentile_high: self.percentile_high,
            colorbar_midpoint: self.colorbar.midpoint,
        };

        if let Some(tx) = &self.rgba_request_tx {
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
            // No background thread (e.g., WASM): convert synchronously on the UI thread
            let mut buffer = Vec::with_capacity(frame.width as usize * frame.height as usize * 4);
            let (computed_min, computed_max) = convert_frame_to_rgba_into(&request, &mut buffer);
            self.pending_rgba = Some(RgbaConversionResult {
                rgba: buffer,
                width: request.width,
                height: request.height,
                frame_number: request.frame_number,
                computed_min,
                computed_max,
            });
            true
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

    // -- Background Echelle Extraction (bd-fwyp) --

    /// Spawn a dedicated thread for echelle extraction.
    ///
    /// Mirrors the RGBA converter pattern: bounded request channel, unbounded result channel.
    /// The worker thread owns its own u16 scratch buffer for 12/16-bit decode.
    fn spawn_echelle_extractor(&mut self) -> bool {
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<EchelleExtractionRequest>(2);
        let (result_tx, result_rx) = std::sync::mpsc::channel::<EchelleExtractionResult>();

        let spawn_result = std::thread::Builder::new()
            .name("echelle-extractor".into())
            .spawn(move || {
                tracing::debug!("Echelle extractor thread started");
                let mut u16_scratch = Vec::new();

                while let Ok(req) = request_rx.recv() {
                    let t0 = std::time::Instant::now();
                    let preview = extract_preview_with_u16_scratch(
                        &req.profile,
                        &req.data,
                        req.width,
                        req.height,
                        req.bit_depth,
                        req.frame_number,
                        &mut u16_scratch,
                    );
                    let extract_ms = t0.elapsed().as_secs_f64() * 1000.0;

                    let result = EchelleExtractionResult {
                        preview,
                        extract_ms,
                        frame_number: req.frame_number,
                    };

                    if result_tx.send(result).is_err() {
                        tracing::debug!("Echelle extractor result receiver dropped, exiting");
                        break;
                    }
                }

                tracing::debug!("Echelle extractor thread exiting");
            });

        match spawn_result {
            Ok(_handle) => {
                self.echelle_extract_tx = Some(request_tx);
                self.echelle_extract_rx = Some(result_rx);
                true
            }
            Err(e) => {
                tracing::error!(
                    "Failed to spawn echelle extractor thread: {}. Falling back to synchronous extraction.",
                    e
                );
                self.echelle_sync_mode = true;
                false
            }
        }
    }

    /// Poll for completed echelle extractions from background thread (bd-fwyp)
    fn poll_echelle_results(&mut self) {
        if let Some(rx) = &self.echelle_extract_rx {
            let mut latest: Option<EchelleExtractionResult> = None;
            while let Ok(result) = rx.try_recv() {
                latest = Some(result);
            }
            if latest.is_some() {
                self.pending_echelle = latest;
            }
        }
    }

    /// Apply pending echelle extraction result to panel state (bd-fwyp)
    fn apply_pending_echelle(&mut self) {
        if let Some(result) = self.pending_echelle.take() {
            self.echelle_last_extract_ms = Some(result.extract_ms);
            match result.preview {
                Ok(preview) => {
                    self.echelle_extract_runs = self.echelle_extract_runs.saturating_add(1);
                    let order_count = preview.orders.len();
                    if order_count == 0 {
                        self.echelle_preview = None;
                        self.echelle_preview_error = Some(
                            "Echelle profile has no enabled orders for extraction".to_string(),
                        );
                        return;
                    }
                    if self.echelle_selected_order_plot >= order_count {
                        self.echelle_selected_order_plot = 0;
                    }
                    self.echelle_preview_measurements = preview.to_measurements();
                    self.echelle_preview = Some(preview);
                    self.echelle_preview_error = None;
                }
                Err(err) => {
                    self.echelle_extract_errors = self.echelle_extract_errors.saturating_add(1);
                    self.echelle_preview_error = Some(err);
                }
            }
        }
    }

    /// Submit frame for background echelle extraction (bd-fwyp)
    ///
    /// Handles decimation gating, profile lookup, and submission.
    /// Falls back to synchronous extraction on WASM or thread spawn failure.
    fn submit_for_echelle_extraction(&mut self, frame: &FrameUpdate) {
        if !self.echelle_extraction_enabled {
            return;
        }

        let decimation = u64::from(self.echelle_extract_every_n_frames.max(1));
        if decimation > 1 && !frame.frame_number.is_multiple_of(decimation) {
            self.echelle_extract_skipped_frames =
                self.echelle_extract_skipped_frames.saturating_add(1);
            return;
        }

        let Some(profile) = self.echelle_profile_cache.profile().cloned() else {
            self.echelle_preview = None;
            self.echelle_preview_error = None;
            return;
        };

        // Spawn extractor thread lazily on first use
        if self.echelle_extract_tx.is_none() && !self.echelle_sync_mode {
            self.spawn_echelle_extractor();
        }

        let request = EchelleExtractionRequest {
            data: frame.data.clone(),
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            frame_number: frame.frame_number,
            profile,
        };

        if let Some(tx) = &self.echelle_extract_tx {
            match tx.try_send(request) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    // Queue full, frame dropped (acceptable under load)
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.echelle_extract_tx = None;
                }
            }
        } else {
            // No background thread (e.g., WASM): extract synchronously
            let t0 = Instant::now();
            let preview = extract_preview_with_u16_scratch(
                &request.profile,
                &request.data,
                request.width,
                request.height,
                request.bit_depth,
                request.frame_number,
                &mut self.echelle_decode_scratch_u16,
            );
            self.pending_echelle = Some(EchelleExtractionResult {
                preview,
                extract_ms: t0.elapsed().as_secs_f64() * 1000.0,
                frame_number: request.frame_number,
            });
            self.apply_pending_echelle();
        }
    }

    /// Poll for async action results
    fn poll_actions(&mut self) {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                ImageViewerAction::CamerasLoaded {
                    ids,
                    full_frame_dims,
                } => {
                    self.available_cameras = ids;
                    self.camera_full_frame_dims = full_frame_dims;
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
                    #[allow(clippy::cast_precision_loss)]
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
                ImageViewerAction::EchelleCalibrationSynced { message } => {
                    self.echelle_run_engine_sync_in_flight = false;
                    self.echelle_cal_ui.status_message = Some(message);
                    self.echelle_cal_ui.last_error = None;
                }
                ImageViewerAction::EchelleCalibrationSyncError(message) => {
                    self.echelle_run_engine_sync_in_flight = false;
                    self.echelle_run_engine_sync_dirty = true;
                    self.echelle_cal_ui.last_error = Some(message);
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
                    let mut cameras: Vec<String> = Vec::new();
                    let mut full_frame_dims: std::collections::HashMap<String, (u32, u32)> =
                        std::collections::HashMap::new();

                    for d in devices.into_iter().filter(|d| {
                        // Check is_frame_producer flag or camera category
                        d.is_frame_producer()
                            || d.category == protocol::daq::DeviceCategory::Camera as i32
                    }) {
                        let id = d.id.clone();
                        if let Some(meta) = d.metadata {
                            if let (Some(w), Some(h)) = (meta.frame_width, meta.frame_height) {
                                if w > 0 && h > 0 {
                                    full_frame_dims.insert(id.clone(), (w, h));
                                }
                            }
                        }
                        cameras.push(id);
                    }

                    let _ = action_tx.send(ImageViewerAction::CamerasLoaded {
                        ids: cameras,
                        full_frame_dims,
                    });
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
        tracing::debug!(
            device_id = %device_id,
            param = %name,
            value = %value,
            "set_camera_parameter: sending parameter update"
        );

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
            tracing::debug!(
                device_id = %result.device_id,
                param = %result.param_name,
                success = result.success,
                actual_value = ?result.actual_value,
                error = ?result.error,
                "poll_param_results: received ParamSetResult"
            );

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
    #[allow(clippy::cast_possible_truncation)]
    fn render_camera_control(&mut self, ui: &mut egui::Ui, device_id: &str, param_idx: usize) {
        ui.set_max_width(ui.available_width());

        // Safe access to parameter to avoid borrowing self for the whole method
        let param = &self.camera_params[param_idx];
        let desc = &param.descriptor;
        let buffer_key = (device_id.to_string(), desc.name.clone());

        // Check if setting
        if self.setting_params.contains(&buffer_key) {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.label(&desc.name);
            });
            return;
        }

        // Read-only
        if !desc.writable {
            ui.vertical(|ui| {
                ui.label(&desc.name);
                let mut value = param.current_value.clone();
                if !desc.units.is_empty() {
                    value.push(' ');
                    value.push_str(&desc.units);
                }
                ui.add(egui::Label::new(value).wrap());
            });
            return;
        }

        let mut pending_update: Option<String> = None;

        // Enums
        if !desc.enum_values.is_empty() {
            let current = param.current_value.trim_matches('"').to_string();
            let mut selected = current.clone();

            ui.horizontal_wrapped(|ui| {
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

            ui.horizontal_wrapped(|ui| {
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
                    self.param_edit_buffers
                        .insert(buffer_key.clone(), val.to_string());
                }

                // Commit on drag stop, focus loss, Enter, or step-button click.
                let commit_now = (response.changed()
                    && !response.dragged()
                    && ui.input(|i| i.pointer.any_released() || i.key_pressed(egui::Key::Enter)))
                    || response.drag_stopped()
                    || response.lost_focus();

                if commit_now && val != param.current_value.parse().unwrap_or(0) {
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

            ui.horizontal_wrapped(|ui| {
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
                    self.param_edit_buffers
                        .insert(buffer_key.clone(), val.to_string());
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

                // Always send on drag stop/focus loss, and also on Enter/step-button changes.
                let commit_now = (response.changed()
                    && !response.dragged()
                    && ui.input(|i| i.pointer.any_released() || i.key_pressed(egui::Key::Enter)))
                    || response.drag_stopped()
                    || response.lost_focus();

                if commit_now && value_changed {
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

            ui.horizontal_wrapped(|ui| {
                ui.label(&desc.name);
                let response = ui.text_edit_singleline(buffer);

                if response.lost_focus() && buffer != &param.current_value {
                    pending_update = Some(format!("\"{}\"", buffer));
                }
            });
        }
        // Fallback
        else {
            ui.horizontal_wrapped(|ui| {
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
        // Cancel existing subscription and stop server-side stream (non-blocking).
        // The streaming task's cleanup checks stream_generation to avoid killing the new stream.
        if let Some(sub) = self.subscription.take() {
            let cancel_tx = sub.cancel_tx.clone();
            let mut client = client.clone();
            let old_device_id = sub.device_id.clone();
            tracing::info!(
                old_device = %old_device_id,
                new_device = %device_id,
                "Cancelling existing stream before starting new one"
            );
            // Non-blocking cancellation: fire-and-forget the cancel signal and
            // stop_stream. The streaming task's cleanup checks stream_generation
            // to avoid killing the new stream.
            let new_device_id = device_id.to_string();
            runtime.spawn(async move {
                let _ = cancel_tx.send(()).await;
                // Skip server-side stop when reconnecting to the same device —
                // otherwise this background stop could kill the newly started stream.
                if old_device_id != new_device_id {
                    if let Err(e) = client.stop_stream(&old_device_id).await {
                        tracing::debug!(
                            device = %old_device_id,
                            error = %e,
                            "Error stopping old stream (may already be stopped)"
                        );
                    }
                }
            });
        }

        self.device_id = Some(device_id.to_string());
        self.mark_echelle_run_engine_sync_dirty();
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
        let generation = self.stream_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let stream_gen = self.stream_generation.clone();

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
                quality = ?stream_quality,
                "Frame streaming started - entering receive loop"
            );

            let mut frames_received = 0u64;
            let mut frames_dropped = 0u64;
            // Reusable decompression buffer — avoids per-frame Vec allocation
            let mut decompress_buf = Vec::new();

            // Timeout for stream inactivity (30s) to prevent hanging on network faults (bd-7rk0)
            const STREAM_TIMEOUT: Duration = Duration::from_secs(30);

            // Track why the loop exited for debugging
            let exit_reason: &str;

            loop {
                tokio::select! {
                    _ = cancel_rx.recv() => {
                        tracing::info!(
                            device_id = %device_id_clone,
                            frames_received = frames_received,
                            "Frame stream cancelled by user/system"
                        );
                        exit_reason = "cancelled";
                        break;
                    }
                    () = crate::runtime::sleep(STREAM_TIMEOUT) => {
                        tracing::warn!(
                            device_id = %device_id_clone,
                            timeout_secs = STREAM_TIMEOUT.as_secs(),
                            frames_received = frames_received,
                            "Frame stream timeout - no frames received in timeout period"
                        );
                        let _ = action_tx.send(ImageViewerAction::Error(format!(
                            "Frame stream timeout (no frames for {}s)", STREAM_TIMEOUT.as_secs()
                        )));
                        exit_reason = "timeout";
                        break;
                    }
                    frame_result = stream.next() => {
                        match frame_result {
                            Some(Ok(mut frame_data)) => {
                                frames_received += 1;

                                // Log EVERY frame for the first 10 frames to debug early disconnect
                                if frames_received <= 10 {
                                    tracing::info!(
                                        device_id = %device_id_clone,
                                        frame = frames_received,
                                        frame_number = frame_data.frame_number,
                                        bytes = frame_data.data.len(),
                                        width = frame_data.width,
                                        height = frame_data.height,
                                        compressed = frame_data.compression != 0,
                                        "Received frame from gRPC (early frame debug)"
                                    );
                                }

                                // Decompress frame if compressed (bd-7rk0: gRPC improvements)
                                // Uses buffer reuse to avoid per-frame allocation
                                if let Err(e) = decompress_frame_into(&mut frame_data, &mut decompress_buf) {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        frame = frames_received,
                                        error = %e,
                                        "Frame decompression failed, skipping frame"
                                    );
                                    continue;
                                }

                                if frames_received > 10 && frames_received.is_multiple_of(30) {
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
                                    Ok(()) => {
                                        if frames_received <= 10 {
                                            tracing::info!(
                                                device_id = %device_id_clone,
                                                frame = frames_received,
                                                "Frame queued to UI successfully"
                                            );
                                        }
                                    }
                                    Err(mpsc::TrySendError::Full(_)) => {
                                        frames_dropped += 1;
                                        if frames_dropped.is_multiple_of(10) {
                                            tracing::warn!(
                                                device_id = %device_id_clone,
                                                dropped = frames_dropped,
                                                "Frame dropped - UI queue full (slow render loop?)"
                                            );
                                        }
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => {
                                        // Receiver dropped - this shouldn't happen during normal operation
                                        tracing::error!(
                                            device_id = %device_id_clone,
                                            frames_received = frames_received,
                                            "Frame receiver disconnected unexpectedly - UI channel closed"
                                        );
                                        exit_reason = "receiver_disconnected";
                                        break;
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                // Log detailed error info
                                tracing::error!(
                                    device_id = %device_id_clone,
                                    frames_received = frames_received,
                                    error = %e,
                                    error_debug = ?e,
                                    "Frame stream error from gRPC"
                                );
                                let _ = action_tx.send(ImageViewerAction::Error(format!(
                                    "Frame stream error: {}", e
                                )));
                                exit_reason = "grpc_error";
                                break;
                            }
                            None => {
                                // Stream ended normally (server closed)
                                tracing::warn!(
                                    device_id = %device_id_clone,
                                    frames_received = frames_received,
                                    "Frame stream ended - server closed connection"
                                );
                                let _ = action_tx.send(ImageViewerAction::Error(format!(
                                    "Frame stream from {} ended unexpectedly", device_id_clone
                                )));
                                exit_reason = "stream_ended";
                                break;
                            }
                        }
                    }
                }
            }

            tracing::info!(
                device_id = %device_id_clone,
                exit_reason = exit_reason,
                frames_received = frames_received,
                frames_dropped = frames_dropped,
                "Frame stream loop exited"
            );

            // Cleanup: Only stop the server-side stream if this task is still the current generation.
            // If a newer stream has started (generation changed), the new stream's cancellation or
            // its own cleanup will handle stopping.
            if stream_gen.load(Ordering::Relaxed) == generation {
                let _ = client.stop_stream(&device_id_clone).await;
            } else {
                tracing::debug!(
                    device_id = %device_id_clone,
                    task_generation = generation,
                    "Skipping stop_stream - superseded by newer stream"
                );
            }
        });

        self.subscription = Some(FrameStreamSubscription {
            cancel_tx,
            device_id: device_id.to_string(),
        });
    }

    /// Stop streaming and notify server to stop hardware capture
    pub fn stop_stream(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Only bump generation when we can issue stop_stream ourselves.
        // When client is None, let the streaming task's cleanup handle stopping.
        if client.is_some() {
            self.stream_generation.fetch_add(1, Ordering::Relaxed);
        }
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
            .is_none_or(|t| t.elapsed().as_millis() > 500);
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

        // bd-fwyp: Poll for completed echelle extractions from background thread
        self.poll_echelle_results();
        self.apply_pending_echelle();

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
    fn process_frame(&mut self, _ctx: &egui::Context, mut frame: FrameUpdate) {
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
        if frame.frame_number.is_multiple_of(30) {
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
        self.last_frame_timestamp_ns = frame.timestamp_ns;
        self.error = None;

        // bd-7rk0: Update stream metrics from server
        if frame.metrics.is_some() {
            self.stream_metrics = frame.metrics.take();
        }

        // bd-12qt: Update connection state when receiving frames
        if self.connection_state != ConnectionState::Connected {
            self.connection_state = ConnectionState::Connected;
            self.retry_count = 0;
            self.status = Some("Connected".to_string());
        } else if self.status.as_deref() == Some("Connected") {
            // Only clear the "Connected" status once steady-state is reached;
            // preserve other status messages (e.g., recording, saved).
            self.status = None;
        }

        // Store frame data for ROI statistics
        self.last_frame_data = Some(frame.data.clone());

        // Update ROI statistics if we have an active ROI
        self.roi_selector.update_statistics(
            &frame.data,
            frame.width,
            frame.height,
            frame.bit_depth,
        );

        // Compute pixel statistics when panel is visible (bd-li4i)
        if self.show_pixel_stats {
            self.pixel_statistics = Some(compute_pixel_statistics(&frame.data, frame.bit_depth));
        }

        // Update histogram
        self.histogram
            .from_frame_data(&frame.data, frame.width, frame.height, frame.bit_depth);

        // bd-fwyp: Submit for background echelle extraction (decimated)
        self.submit_for_echelle_extraction(&frame);

        // bd-07j1: Update colorbar range from frame data
        let bit_max = match frame.bit_depth {
            8 => 255.0,
            12 => 4095.0,
            16 => 65535.0,
            _ => 65535.0,
        };
        self.colorbar.min_value = 0.0;
        self.colorbar.max_value = bit_max;

        // bd-xifj: Submit frame for background RGBA conversion to prevent UI freezes
        // The converted RGBA will be applied to texture when polled in drain_updates
        let _submitted = self.submit_for_rgba_conversion(&frame);
        // Note: If submission fails (queue full), frame is dropped which is acceptable
        // under high load - we'll display the next successful frame
    }

    /// Render the image viewer panel
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn ui(&mut self, ui: &mut egui::Ui, mut client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Poll for async action results
        self.poll_actions();
        self.poll_param_results(ui.ctx());
        self.poll_echelle_profile_cache();
        self.sync_echelle_profile_to_run_engine(client.as_deref_mut(), runtime);

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
        let should_refresh = self.last_refresh.is_none_or(|t| t.elapsed().as_secs() > 30);

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
                                if ui.selectable_label(is_selected, cam_id).clicked()
                                    && self.device_id.as_deref() != Some(cam_id.as_str())
                                {
                                    self.device_id = Some(cam_id.clone());
                                    self.mark_echelle_run_engine_sync_dirty();
                                    self.camera_params.clear();
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
                } else if self.device_id.is_some()
                    && ui
                        .button(format!("{} Start", icons::action::START))
                        .on_hover_text("Start streaming")
                        .clicked()
                {
                    if let Some(device_id) = &self.device_id {
                        start_stream_device = Some(device_id.clone());
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
                        if is_streaming
                            && ui
                                .button(icons::action::RECORD)
                                .on_hover_text("Start recording frames to HDF5")
                                .clicked()
                        {
                            start_recording = true;
                        }
                    }
                    RecordingState::Recording => {
                        // Pulsing recording indicator
                        let time = ui.ctx().input(|i| i.time);
                        #[allow(clippy::cast_possible_truncation)]
                        let pulse = ((time * 2.0).sin() * 0.5 + 0.5) as f32;
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
                        ui.selectable_value(
                            &mut self.stream_quality,
                            StreamQuality::Fast,
                            "Fast (4x)",
                        );
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

                // bd-07j1: Colorbar toggle
                if ui
                    .selectable_label(
                        self.show_colorbar,
                        if self.show_colorbar {
                            "Bar [ON]"
                        } else {
                            "Bar"
                        },
                    )
                    .on_hover_text("Show interactive colorbar")
                    .clicked()
                {
                    self.show_colorbar = !self.show_colorbar;
                }

                ui.separator();

                // === Contrast Enhancement (bd-j6xm) ===
                ui.label("Contrast:");
                egui::ComboBox::from_id_salt("contrast_mode_selector")
                    .width(100.0)
                    .selected_text(self.contrast_mode.label())
                    .show_ui(ui, |ui| {
                        for &mode in ContrastMode::all() {
                            ui.selectable_value(&mut self.contrast_mode, mode, mode.label());
                        }
                    });

                // Show controls based on mode
                match self.contrast_mode {
                    ContrastMode::Manual => {
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
                    }
                    ContrastMode::AutoPercentile => {
                        // Show percentile controls
                        ui.add(
                            egui::DragValue::new(&mut self.percentile_low)
                                .speed(0.1)
                                .range(0.0..=100.0)
                                .prefix("Low: ")
                                .suffix("%")
                                .max_decimals(1),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.percentile_high)
                                .speed(0.1)
                                .range(0.0..=100.0)
                                .prefix("High: ")
                                .suffix("%")
                                .max_decimals(1),
                        );
                    }
                    ContrastMode::AutoSimple | ContrastMode::HistogramEq | ContrastMode::Clahe => {
                        // Show computed min/max from last frame
                        ui.weak(format!(
                            "{:.0}%-{:.0}%",
                            self.display_min * 100.0,
                            self.display_max * 100.0
                        ));
                    }
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

                // ROI mode selector (Rectangle/Polygon)
                use crate::widgets::roi_selector::RoiMode;
                if roi_selected {
                    let mode_label = match self.roi_selector.mode {
                        RoiMode::Rectangle => "□",
                        RoiMode::Polygon => "⬡",
                    };
                    if ui
                        .button(mode_label)
                        .on_hover_text("Switch ROI mode (Rectangle/Polygon)")
                        .clicked()
                    {
                        self.roi_selector.mode = match self.roi_selector.mode {
                            RoiMode::Rectangle => RoiMode::Polygon,
                            RoiMode::Polygon => RoiMode::Rectangle,
                        };
                    }
                }

                if self.roi_selector.roi().is_some()
                    && ui
                        .button(icons::action::DELETE)
                        .on_hover_text("Clear ROI")
                        .clicked()
                {
                    self.roi_selector.clear();
                }

                if !self.roi_selector.rois().is_empty()
                    && ui
                        .button("Clear All")
                        .on_hover_text("Clear all ROIs")
                        .clicked()
                {
                    self.roi_selector.clear_all();
                }

                if ui
                    .add_enabled(self.device_id.is_some(), egui::Button::new("Clear HW ROI"))
                    .on_hover_text(
                        "Reset camera acquisition ROI to full sensor (requires stream stopped)",
                    )
                    .clicked()
                {
                    self.queue_clear_hardware_roi();
                }

                ui.separator();

                // === Crosshair Toggle (bd-pgcb) ===
                if ui
                    .selectable_label(
                        self.crosshair_enabled,
                        if self.crosshair_enabled {
                            "⊕ [ON]"
                        } else {
                            "⊕"
                        },
                    )
                    .on_hover_text("Toggle crosshair cursor\nClick to lock position")
                    .clicked()
                {
                    self.crosshair_enabled = !self.crosshair_enabled;
                    if !self.crosshair_enabled {
                        self.crosshair_locked_pos = None;
                    }
                }

                ui.separator();

                ui.checkbox(&mut self.show_roi_panel, "Stats");
                ui.checkbox(&mut self.show_pixel_stats, "Px Stats");
                ui.checkbox(&mut self.show_controls, "Controls");
                ui.checkbox(&mut self.show_metadata_overlay, "Metadata Overlay");
                ui.checkbox(&mut self.show_scale_bar, "Scale Bar");

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
            if !updates.is_empty() {
                tracing::debug!(count = updates.len(), "flushing pending_param_updates");
            }
            for (dev, name, val) in &updates {
                tracing::debug!(device_id = %dev, param = %name, value = %val, "flushing pending param update");
                self.set_camera_parameter(client_val, runtime, dev, name, val);
            }

            Some(client_val)
        } else {
            // bd-aruo.4: Show per-parameter error when updates are dropped
            if !self.pending_param_updates.is_empty() {
                tracing::warn!(
                    count = self.pending_param_updates.len(),
                    "dropping pending_param_updates — no gRPC client connected"
                );
                for (dev, name, _val) in &self.pending_param_updates {
                    self.param_errors.insert(
                        (dev.clone(), name.clone()),
                        "Not connected — change not applied".to_string(),
                    );
                }
            }
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
                        .is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(500));
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
        let has_pixel_stats = self.show_pixel_stats;
        // Always show the echelle panel so the calibration workspace can be used
        // to create/load the first profile before any preview exists.
        let has_echelle_panel = true;

        let stats_panel_width = if has_roi_panel
            || has_histogram_panel
            || has_controls_panel
            || has_echelle_panel
            || has_pixel_stats
        {
            if has_controls_panel || has_echelle_panel {
                320.0
            } else {
                200.0
            }
        } else {
            0.0
        };

        // Side panel for stats/controls (fixed width, drawn first so remainder goes to image)
        if stats_panel_width > 0.0 {
            egui::SidePanel::right("image_viewer_stats_panel")
                .exact_width(stats_panel_width)
                .resizable(false)
                .show_inside(ui, |ui| {
                    self.render_stats_side_panel(
                        ui,
                        has_controls_panel,
                        has_roi_panel,
                        has_histogram_panel,
                        has_echelle_panel,
                        has_pixel_stats,
                    );
                });
        }

        // Image area gets all remaining space via CentralPanel
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                let available_size = ui.available_size();

                if let Some(texture) = &self.texture {
                    // bd-07j1: Reserve space for colorbar if enabled
                    let colorbar_width = if self.show_colorbar { 60.0 } else { 0.0 };
                    let image_available =
                        egui::vec2(available_size.x - colorbar_width, available_size.y);

                    // Calculate fit zoom if needed - continuously fit when auto_fit is enabled
                    if self.auto_fit && self.width > 0 && self.height > 0 {
                        #[allow(clippy::cast_precision_loss)]
                        let scale_x = image_available.x / self.width as f32;
                        #[allow(clippy::cast_precision_loss)]
                        let scale_y = image_available.y / self.height as f32;
                        // Allow upscaling to fill available space (remove .min(1.0) cap)
                        self.zoom = scale_x.min(scale_y);
                        self.pan = egui::Vec2::ZERO;
                        // Keep auto_fit true for continuous fitting as window resizes
                    }

                    #[allow(clippy::cast_precision_loss)]
                    let image_size = egui::vec2(
                        self.width as f32 * self.zoom,
                        self.height as f32 * self.zoom,
                    );

                    // Extract crosshair state for use in closure (bd-pgcb)
                    let crosshair_enabled = self.crosshair_enabled;
                    let crosshair_locked_pos = self.crosshair_locked_pos;
                    let width = self.width;
                    let height = self.height;
                    let bit_depth = self.bit_depth;
                    let zoom = self.zoom;
                    let pixel_scale_x = self.pixel_scale_x;
                    let pixel_scale_y = self.pixel_scale_y;
                    let scale_unit = self.scale_unit.clone();
                    let last_frame_data = self.last_frame_data.clone();
                    let roi_selection_mode = self.roi_selector.selection_mode;

                    // Extract metadata overlay state for use in closure (bd-6h1c)
                    let show_metadata_overlay = self.show_metadata_overlay;
                    let overlay_frame_count = self.frame_count;
                    let overlay_fps = self.fps_counter.fps();
                    let overlay_timestamp_ns = self.last_frame_timestamp_ns;

                    // Extract scale bar state for use in closure (bd-0tcg)
                    let show_scale_bar = self.show_scale_bar;
                    let scale_bar_pixel_scale_x = self.pixel_scale_x;
                    let scale_bar_unit = self.scale_unit.clone();

                    let echelle_trace_overlay_paths = self.build_echelle_trace_overlay_paths();
                    let echelle_trace_overlay_selected_relative = self
                        .echelle_cal_ui
                        .editor_profile
                        .as_ref()
                        .and_then(|p| p.orders.get(self.echelle_cal_ui.selected_order_edit_idx))
                        .map(|o| o.relative_index);
                    let echelle_hover_marker = self.echelle_plot_hover_link.and_then(|link| {
                        let profile = self.echelle_profile_cache.profile()?;
                        let order = profile
                            .orders
                            .iter()
                            .find(|o| o.enabled && o.relative_index == link.relative_index)?;
                        let (x, y) =
                            order_sample_image_position(profile, order, link.sample_index)?;
                        Some((
                            x,
                            y,
                            format!("mvp λ={:.4}, f={:.1}", link.wavelength, link.flux),
                        ))
                    });

                    // Track crosshair lock changes to apply after closure
                    let mut crosshair_lock_action: Option<Option<(i32, i32)>> = None;

                    // StripBuilder for full-height horizontal split: image + colorbar
                    StripBuilder::new(ui)
                        .size(Size::remainder()) // image column
                        .size(Size::exact(colorbar_width)) // colorbar column
                        .horizontal(|mut strip| {
                            strip.cell(|ui| {
                                // Scrollable/pannable area for image
                                egui::ScrollArea::both()
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                    )
                                    .id_salt("image_scroll")
                                    .show(ui, |ui| {
                                        let (rect, response) = ui.allocate_exact_size(
                                            image_available.max(image_size),
                                            egui::Sense::click_and_drag(),
                                        );

                                        // Calculate image offset (centered)
                                        let offset =
                                            (image_available - image_size) / 2.0 + self.pan;
                                        let image_rect = egui::Rect::from_min_size(
                                            rect.min + offset,
                                            image_size,
                                        );

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
                                                        roi.clone(),
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
                                                self.auto_fit = false;
                                                self.pan += response.drag_delta();
                                            }
                                        }

                                        // Handle zoom with scroll wheel (always active)
                                        if response.hovered() {
                                            let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
                                            if scroll_delta != 0.0 {
                                                let zoom_factor = 1.0 + scroll_delta * 0.001;
                                                self.zoom =
                                                    (self.zoom * zoom_factor).clamp(0.1, 10.0);
                                                self.auto_fit = false;
                                            }
                                        }

                                        // Draw the image
                                        ui.painter().image(
                                            texture.id(),
                                            image_rect,
                                            egui::Rect::from_min_max(
                                                egui::pos2(0.0, 0.0),
                                                egui::pos2(1.0, 1.0),
                                            ),
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

                                        // bd-6h1c: Draw metadata overlay on the image
                                        if show_metadata_overlay && overlay_frame_count > 0 {
                                            let painter = ui.painter();
                                            let overlay_padding = 8.0_f32;
                                            let overlay_pos = egui::pos2(
                                                image_rect.min.x + overlay_padding,
                                                image_rect.min.y + overlay_padding,
                                            );

                                            // Build overlay text lines
                                            let mut lines = Vec::with_capacity(3);
                                            lines.push(format!("Frame: {}", overlay_frame_count));
                                            lines.push(format!("FPS: {:.1}", overlay_fps));
                                            if overlay_timestamp_ns > 0 {
                                                let secs = overlay_timestamp_ns / 1_000_000_000;
                                                let subsec_ms = (overlay_timestamp_ns
                                                    % 1_000_000_000)
                                                    / 1_000_000;
                                                let h = secs / 3600;
                                                let m = (secs % 3600) / 60;
                                                let s = secs % 60;
                                                lines.push(format!(
                                                    "T: {:02}:{:02}:{:02}.{:03}",
                                                    h, m, s, subsec_ms
                                                ));
                                            }

                                            let text = lines.join("\n");
                                            let text_color = egui::Color32::WHITE;
                                            let galley = painter.layout_no_wrap(
                                                text,
                                                egui::FontId::monospace(12.0),
                                                text_color,
                                            );
                                            let bg_rect = egui::Rect::from_min_size(
                                                overlay_pos,
                                                galley.size() + egui::vec2(8.0, 8.0),
                                            );
                                            painter.rect_filled(
                                                bg_rect,
                                                4.0,
                                                egui::Color32::from_black_alpha(160),
                                            );
                                            painter.galley(
                                                overlay_pos + egui::vec2(4.0, 4.0),
                                                galley,
                                                text_color,
                                            );
                                        }

                                        // bd-0tcg: Draw scale bar overlay on the image (bottom-left)
                                        if show_scale_bar && width > 0 && height > 0 {
                                            let painter = ui.painter();
                                            let padding = 12.0_f32;
                                            let bar_height = 4.0_f32;
                                            let bar_y = image_rect.max.y - padding - bar_height;

                                            if let Some(um_per_px) = scale_bar_pixel_scale_x {
                                                // Calibrated: compute a "nice" bar length
                                                #[allow(clippy::cast_precision_loss)]
                                                let image_width_um = f64::from(width) * um_per_px;
                                                let target_um = image_width_um * 0.2; // ~20% of image

                                                // Pick the nearest "nice" value from a fixed set
                                                let nice_values: &[f64] = &[
                                                    0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0,
                                                    100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0,
                                                ];
                                                let bar_um = nice_values
                                                    .iter()
                                                    .copied()
                                                    .min_by(|a, b| {
                                                        let da = (a - target_um).abs();
                                                        let db = (b - target_um).abs();
                                                        da.partial_cmp(&db)
                                                            .unwrap_or(std::cmp::Ordering::Equal)
                                                    })
                                                    .unwrap_or(100.0);

                                                // Convert bar length from physical units to screen pixels
                                                let bar_pixels = bar_um / um_per_px; // image pixels
                                                #[allow(clippy::cast_possible_truncation)]
                                                let bar_screen_width = (bar_pixels as f32) * zoom;

                                                let bar_x = image_rect.min.x + padding;

                                                // Draw black outline behind white bar for contrast
                                                let outline_rect = egui::Rect::from_min_size(
                                                    egui::pos2(bar_x - 1.0, bar_y - 1.0),
                                                    egui::vec2(
                                                        bar_screen_width + 2.0,
                                                        bar_height + 2.0,
                                                    ),
                                                );
                                                painter.rect_filled(
                                                    outline_rect,
                                                    0.0,
                                                    egui::Color32::BLACK,
                                                );

                                                // Draw white bar
                                                let bar_rect = egui::Rect::from_min_size(
                                                    egui::pos2(bar_x, bar_y),
                                                    egui::vec2(bar_screen_width, bar_height),
                                                );
                                                painter.rect_filled(
                                                    bar_rect,
                                                    0.0,
                                                    egui::Color32::WHITE,
                                                );

                                                // Format label: use integer if whole number, else one decimal
                                                let label = if bar_um.fract() < f64::EPSILON {
                                                    #[allow(clippy::cast_possible_truncation)]
                                                    let v = bar_um as u64;
                                                    format!("{} {}", v, &scale_bar_unit)
                                                } else {
                                                    format!("{:.1} {}", bar_um, &scale_bar_unit)
                                                };

                                                let label_pos = egui::pos2(
                                                    bar_x + bar_screen_width / 2.0,
                                                    bar_y - 3.0,
                                                );

                                                // Draw label with black shadow for readability
                                                for dx in [-1.0_f32, 0.0, 1.0] {
                                                    for dy in [-1.0_f32, 0.0, 1.0] {
                                                        if dx != 0.0 || dy != 0.0 {
                                                            painter.text(
                                                                label_pos + egui::vec2(dx, dy),
                                                                egui::Align2::CENTER_BOTTOM,
                                                                &label,
                                                                egui::FontId::proportional(12.0),
                                                                egui::Color32::BLACK,
                                                            );
                                                        }
                                                    }
                                                }
                                                painter.text(
                                                    label_pos,
                                                    egui::Align2::CENTER_BOTTOM,
                                                    &label,
                                                    egui::FontId::proportional(12.0),
                                                    egui::Color32::WHITE,
                                                );
                                            } else {
                                                // Uncalibrated: show warning text at bottom-left
                                                let warn_pos =
                                                    egui::pos2(image_rect.min.x + padding, bar_y);
                                                let warn_text = "Scale bar: uncalibrated";
                                                let warn_galley = painter.layout_no_wrap(
                                                    warn_text.to_string(),
                                                    egui::FontId::proportional(11.0),
                                                    egui::Color32::from_rgb(255, 200, 80),
                                                );
                                                let warn_bg = egui::Rect::from_min_size(
                                                    warn_pos
                                                        - egui::vec2(
                                                            0.0,
                                                            warn_galley.size().y + 4.0,
                                                        ),
                                                    warn_galley.size() + egui::vec2(8.0, 4.0),
                                                );
                                                painter.rect_filled(
                                                    warn_bg,
                                                    4.0,
                                                    egui::Color32::from_black_alpha(180),
                                                );
                                                painter.galley(
                                                    warn_pos
                                                        - egui::vec2(
                                                            -4.0,
                                                            warn_galley.size().y + 2.0,
                                                        ),
                                                    warn_galley,
                                                    egui::Color32::from_rgb(255, 200, 80),
                                                );
                                            }
                                        }

                                        // Draw histogram overlay if positioned on image
                                        if self.histogram_position.is_overlay() {
                                            let hist_size = egui::vec2(180.0, 80.0);
                                            let hist_rect = self
                                                .histogram_position
                                                .overlay_rect(image_rect, hist_size);

                                            // Create a child UI at the overlay position
                                            let mut hist_ui = ui.new_child(
                                                egui::UiBuilder::new().max_rect(hist_rect).layout(
                                                    egui::Layout::left_to_right(egui::Align::Min),
                                                ),
                                            );
                                            self.histogram.show_overlay(&mut hist_ui, hist_size);
                                        }

                                        if !echelle_trace_overlay_paths.is_empty() {
                                            let painter = ui.painter();
                                            for (relative_index, path) in
                                                &echelle_trace_overlay_paths
                                            {
                                                let color = if Some(*relative_index)
                                                    == echelle_trace_overlay_selected_relative
                                                {
                                                    egui::Color32::from_rgb(80, 220, 120)
                                                } else {
                                                    egui::Color32::from_rgba_unmultiplied(
                                                        100, 180, 255, 180,
                                                    )
                                                };
                                                let stroke = egui::Stroke::new(
                                                    if Some(*relative_index)
                                                        == echelle_trace_overlay_selected_relative
                                                    {
                                                        2.0
                                                    } else {
                                                        1.0
                                                    },
                                                    color,
                                                );
                                                for segment in path.windows(2) {
                                                    let (x0, y0) = segment[0];
                                                    let (x1, y1) = segment[1];
                                                    let p0 = egui::pos2(
                                                        rect.min.x + offset.x + x0 * zoom,
                                                        rect.min.y + offset.y + y0 * zoom,
                                                    );
                                                    let p1 = egui::pos2(
                                                        rect.min.x + offset.x + x1 * zoom,
                                                        rect.min.y + offset.y + y1 * zoom,
                                                    );
                                                    if image_rect.contains(p0)
                                                        || image_rect.contains(p1)
                                                    {
                                                        painter.line_segment([p0, p1], stroke);
                                                    }
                                                }
                                                if let Some((x, y)) = path.first().copied() {
                                                    let p = egui::pos2(
                                                        rect.min.x + offset.x + x * zoom,
                                                        rect.min.y + offset.y + y * zoom,
                                                    );
                                                    if image_rect.contains(p) {
                                                        painter.text(
                                                            p + egui::vec2(6.0, 6.0),
                                                            egui::Align2::LEFT_TOP,
                                                            format!("rel {}", relative_index),
                                                            egui::FontId::monospace(10.0),
                                                            color,
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        if let Some((px, py, label)) = &echelle_hover_marker {
                                            let marker_x = rect.min.x + offset.x + *px * zoom;
                                            let marker_y = rect.min.y + offset.y + *py * zoom;
                                            let marker_pos = egui::pos2(marker_x, marker_y);
                                            if image_rect.contains(marker_pos) {
                                                let painter = ui.painter();
                                                let color = egui::Color32::from_rgb(255, 120, 0);
                                                painter.circle_stroke(
                                                    marker_pos,
                                                    (4.0 * zoom.clamp(0.5, 2.0)).max(4.0),
                                                    egui::Stroke::new(2.0, color),
                                                );
                                                painter.circle_filled(marker_pos, 2.0, color);
                                                painter.text(
                                                    marker_pos + egui::vec2(8.0, -8.0),
                                                    egui::Align2::LEFT_BOTTOM,
                                                    label,
                                                    egui::FontId::monospace(11.0),
                                                    color,
                                                );
                                            }
                                        }

                                        // Crosshair cursor with pixel readout (bd-pgcb)
                                        if crosshair_enabled {
                                            // Determine crosshair position (locked or hover)
                                            let crosshair_pixel_pos = if let Some(locked_pos) =
                                                crosshair_locked_pos
                                            {
                                                Some(locked_pos)
                                            } else if let Some(hover_pos) = response.hover_pos() {
                                                let image_pos = hover_pos - rect.min - offset;
                                                #[allow(clippy::cast_possible_truncation)]
                                                let pixel_x = (image_pos.x / zoom) as i32;
                                                #[allow(
                                                    clippy::cast_possible_truncation,
                                                    clippy::cast_possible_wrap
                                                )]
                                                let pixel_y = (image_pos.y / zoom) as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let w_i32 = width as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let h_i32 = height as i32;
                                                if pixel_x >= 0
                                                    && pixel_x < w_i32
                                                    && pixel_y >= 0
                                                    && pixel_y < h_i32
                                                {
                                                    Some((pixel_x, pixel_y))
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            };

                                            // Handle click to lock/unlock crosshair (defer mutation)
                                            if response.clicked() && !roi_selection_mode {
                                                if let Some(hover_pos) =
                                                    response.interact_pointer_pos()
                                                {
                                                    let image_pos = hover_pos - rect.min - offset;
                                                    #[allow(clippy::cast_possible_truncation)]
                                                    let pixel_x = (image_pos.x / zoom) as i32;
                                                    #[allow(
                                                        clippy::cast_possible_truncation,
                                                        clippy::cast_possible_wrap
                                                    )]
                                                    let pixel_y = (image_pos.y / zoom) as i32;
                                                    #[allow(clippy::cast_possible_wrap)]
                                                    let w_i32 = width as i32;
                                                    #[allow(clippy::cast_possible_wrap)]
                                                    let h_i32 = height as i32;
                                                    if pixel_x >= 0
                                                        && pixel_x < w_i32
                                                        && pixel_y >= 0
                                                        && pixel_y < h_i32
                                                    {
                                                        // Toggle lock: if already locked at this position, unlock
                                                        if crosshair_locked_pos
                                                            == Some((pixel_x, pixel_y))
                                                        {
                                                            crosshair_lock_action = Some(None);
                                                        } else {
                                                            crosshair_lock_action =
                                                                Some(Some((pixel_x, pixel_y)));
                                                        }
                                                    }
                                                }
                                            }

                                            // Draw crosshair and readout if position is valid
                                            if let Some((pixel_x, pixel_y)) = crosshair_pixel_pos {
                                                // Convert pixel coordinates to screen coordinates
                                                #[allow(clippy::cast_precision_loss)]
                                                let screen_x = rect.min.x
                                                    + offset.x
                                                    + (pixel_x as f32 + 0.5) * zoom;
                                                #[allow(clippy::cast_precision_loss)]
                                                let screen_y = rect.min.y
                                                    + offset.y
                                                    + (pixel_y as f32 + 0.5) * zoom;
                                                let crosshair_pos = egui::pos2(screen_x, screen_y);

                                                let painter = ui.painter();
                                                let crosshair_color =
                                                    if crosshair_locked_pos.is_some() {
                                                        egui::Color32::from_rgb(255, 200, 0)
                                                    } else {
                                                        egui::Color32::from_rgb(0, 255, 0)
                                                    };
                                                let stroke =
                                                    egui::Stroke::new(1.5, crosshair_color);

                                                // Draw crosshair lines
                                                let line_length = 15.0;
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x - line_length,
                                                            crosshair_pos.y,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x - 3.0,
                                                            crosshair_pos.y,
                                                        ),
                                                    ],
                                                    stroke,
                                                );
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x + 3.0,
                                                            crosshair_pos.y,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x + line_length,
                                                            crosshair_pos.y,
                                                        ),
                                                    ],
                                                    stroke,
                                                );
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y - line_length,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y - 3.0,
                                                        ),
                                                    ],
                                                    stroke,
                                                );
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y + 3.0,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y + line_length,
                                                        ),
                                                    ],
                                                    stroke,
                                                );

                                                // Draw center dot
                                                painter.circle_filled(
                                                    crosshair_pos,
                                                    2.0,
                                                    crosshair_color,
                                                );

                                                // Get pixel intensity value
                                                let pixel_value =
                                                    if let Some(frame_data) = &last_frame_data {
                                                        get_pixel_value_inline(
                                                            frame_data,
                                                            pixel_x as u32,
                                                            pixel_y as u32,
                                                            width,
                                                            height,
                                                            bit_depth,
                                                        )
                                                    } else {
                                                        None
                                                    };

                                                // Build readout text
                                                let mut readout_lines = Vec::new();
                                                readout_lines.push(format!(
                                                    "X: {} px, Y: {} px",
                                                    pixel_x, pixel_y
                                                ));

                                                // Physical coordinates if calibrated
                                                if let (Some(scale_x), Some(scale_y)) =
                                                    (pixel_scale_x, pixel_scale_y)
                                                {
                                                    let phys_x = f64::from(pixel_x) * scale_x;
                                                    let phys_y = f64::from(pixel_y) * scale_y;
                                                    readout_lines.push(format!(
                                                        "X: {:.2} {}, Y: {:.2} {}",
                                                        phys_x, &scale_unit, phys_y, &scale_unit
                                                    ));
                                                }

                                                // Pixel intensity
                                                if let Some(value) = pixel_value {
                                                    readout_lines
                                                        .push(format!("Intensity: {}", value));
                                                }

                                                // Draw readout panel (top-left corner of image)
                                                let panel_padding = 8.0;
                                                let panel_pos = egui::pos2(
                                                    image_rect.min.x + panel_padding,
                                                    image_rect.min.y + panel_padding,
                                                );
                                                let text_galley = painter.layout_no_wrap(
                                                    readout_lines.join("\n"),
                                                    egui::FontId::monospace(12.0),
                                                    crosshair_color,
                                                );
                                                let panel_rect = egui::Rect::from_min_size(
                                                    panel_pos,
                                                    text_galley.size() + egui::vec2(8.0, 8.0),
                                                );
                                                painter.rect_filled(
                                                    panel_rect,
                                                    4.0,
                                                    egui::Color32::from_black_alpha(180),
                                                );
                                                painter.galley(
                                                    panel_pos + egui::vec2(4.0, 4.0),
                                                    text_galley,
                                                    crosshair_color,
                                                );
                                            }
                                        } else {
                                            // Simple hover text when crosshair is disabled (bd-07j1)
                                            if let Some(pos) = response.hover_pos() {
                                                let image_pos = pos - rect.min - offset;
                                                #[allow(clippy::cast_possible_truncation)]
                                                let pixel_x = (image_pos.x / self.zoom) as i32;
                                                #[allow(
                                                    clippy::cast_possible_truncation,
                                                    clippy::cast_possible_wrap
                                                )]
                                                let pixel_y = (image_pos.y / self.zoom) as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let w_i32 = self.width as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let h_i32 = self.height as i32;
                                                if pixel_x >= 0
                                                    && pixel_x < w_i32
                                                    && pixel_y >= 0
                                                    && pixel_y < h_i32
                                                {
                                                    // Build hover text with pixel and optional physical coordinates
                                                    let hover_text =
                                                        if let (Some(scale_x), Some(scale_y)) =
                                                            (self.pixel_scale_x, self.pixel_scale_y)
                                                        {
                                                            let phys_x =
                                                                f64::from(pixel_x) * scale_x;
                                                            let phys_y =
                                                                f64::from(pixel_y) * scale_y;
                                                            format!(
                                                            "Pixel: ({}, {}) | {:.2} {} x {:.2} {}",
                                                            pixel_x,
                                                            pixel_y,
                                                            phys_x,
                                                            &self.scale_unit,
                                                            phys_y,
                                                            &self.scale_unit
                                                        )
                                                        } else {
                                                            format!(
                                                                "Pixel: ({}, {})",
                                                                pixel_x, pixel_y
                                                            )
                                                        };
                                                    response.on_hover_text(hover_text);
                                                }
                                            }
                                        }
                                    });
                            });
                            strip.cell(|ui| {
                                // bd-07j1: Colorbar widget
                                if self.show_colorbar {
                                    ui.add_space(4.0);
                                    let colorbar_size =
                                        egui::vec2(40.0, ui.available_height() - 20.0);
                                    if self.colorbar.show(ui, &self.colormap, colorbar_size) {
                                        // Midpoint changed - request repaint to update image
                                        ui.ctx().request_repaint();
                                    }
                                }
                            });
                        });

                    // Apply crosshair lock changes after closure (bd-pgcb)
                    if let Some(action) = crosshair_lock_action {
                        self.crosshair_locked_pos = action;
                    }
                } else {
                    // No image - show placeholder
                    ui.centered_and_justified(|ui| {
                        ui.label("No image. Select a camera device and start streaming.");
                    });
                }
            });
    }

    /// Queue a parameter update to reset hardware ROI to the full sensor.
    fn queue_clear_hardware_roi(&mut self) {
        if let Some(dev_id) = self.device_id.clone() {
            if self.subscription.is_some() {
                self.param_errors.insert(
                    (dev_id, "acquisition.roi".to_string()),
                    "Stop streaming before clearing hardware ROI".to_string(),
                );
            } else if let Some((full_w, full_h)) = self.camera_full_frame_dims.get(&dev_id).copied()
            {
                let roi_json = serde_json::json!({
                    "type": "rectangle",
                    "x": 0,
                    "y": 0,
                    "width": full_w,
                    "height": full_h
                });
                self.pending_param_updates.push((
                    dev_id.clone(),
                    "acquisition.roi".to_string(),
                    roi_json.to_string(),
                ));
                self.param_errors
                    .remove(&(dev_id, "acquisition.roi".to_string()));
            } else {
                self.param_errors.insert(
                    (dev_id, "acquisition.roi".to_string()),
                    "Unknown full-frame size; refresh camera list and retry".to_string(),
                );
            }
        }
    }

    /// Render the stats/controls side panel content (Camera Settings, ROI, Histogram, Calibration)
    fn render_stats_side_panel(
        &mut self,
        ui: &mut egui::Ui,
        has_controls_panel: bool,
        has_roi_panel: bool,
        has_histogram_panel: bool,
        has_echelle_panel: bool,
        has_pixel_stats: bool,
    ) {
        ui.set_max_width(ui.available_width());
        egui::ScrollArea::vertical()
            .scroll([false, true])
            .auto_shrink([true, false])
            .id_salt("side_panel_scroll")
            .show(ui, |ui| {
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

                if has_echelle_panel {
                    layout::card_frame(ui).show(ui, |ui| {
                        egui::CollapsingHeader::new("Echelle Spectrum (MVP Preview)")
                            .default_open(true)
                            .show(ui, |ui| {
                                self.render_echelle_preview_panel(ui);
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
                                ui.horizontal(|ui| {
                                    if ui
                                        .button("Apply as Hardware ROI")
                                        .on_hover_text(
                                            "Update camera acquisition ROI (requires stream stopped)",
                                        )
                                        .clicked()
                                    {
                                        if self.subscription.is_some() {
                                            if let Some(dev_id) = self.device_id.clone() {
                                                self.param_errors.insert(
                                                    (dev_id, "acquisition.roi".to_string()),
                                                    "Stop streaming before applying hardware ROI"
                                                        .to_string(),
                                                );
                                            }
                                        } else if let Some(roi) = self.roi_selector.roi() {
                                            if let Some(dev_id) = self.device_id.clone() {
                                                use crate::widgets::roi_selector::RoiShape;
                                                let roi_json = match roi {
                                                    RoiShape::Rectangle {
                                                        x,
                                                        y,
                                                        width,
                                                        height,
                                                    } => {
                                                        serde_json::json!({
                                                            "type": "rectangle",
                                                            "x": x,
                                                            "y": y,
                                                            "width": width,
                                                            "height": height
                                                        })
                                                    }
                                                    RoiShape::Polygon { .. } => {
                                                        // For hardware ROI, convert polygon to bounding box
                                                        let (min_x, min_y, max_x, max_y) =
                                                            roi.bounding_box();
                                                        serde_json::json!({
                                                            "type": "rectangle",
                                                            "x": min_x,
                                                            "y": min_y,
                                                            "width": max_x.saturating_sub(min_x),
                                                            "height": max_y.saturating_sub(min_y)
                                                        })
                                                    }
                                                };
                                                self.pending_param_updates.push((
                                                    dev_id,
                                                    "acquisition.roi".to_string(),
                                                    roi_json.to_string(),
                                                ));
                                            }
                                        }
                                    }

                                    if ui
                                        .button("Clear Hardware ROI")
                                        .on_hover_text(
                                            "Reset hardware ROI to full sensor (requires stream stopped)",
                                        )
                                        .clicked()
                                    {
                                        self.queue_clear_hardware_roi();
                                    }
                                });
                            });
                    });
                    ui.add_space(layout::SECTION_SPACING);
                }

                if has_pixel_stats {
                    layout::card_frame(ui).show(ui, |ui| {
                        egui::CollapsingHeader::new("Pixel Statistics")
                            .default_open(true)
                            .show(ui, |ui| {
                                if let Some(stats) = &self.pixel_statistics {
                                    egui::Grid::new("pixel_stats_grid")
                                        .num_columns(2)
                                        .spacing([8.0, 2.0])
                                        .show(ui, |ui| {
                                            ui.label("Count:");
                                            ui.label(format!("{}", stats.count));
                                            ui.end_row();

                                            ui.label("Min:");
                                            ui.label(format!("{:.1}", stats.min));
                                            ui.end_row();

                                            ui.label("Max:");
                                            ui.label(format!("{:.1}", stats.max));
                                            ui.end_row();

                                            ui.label("Mean:");
                                            ui.label(format!("{:.2}", stats.mean));
                                            ui.end_row();

                                            ui.label("Std Dev:");
                                            ui.label(format!("{:.2}", stats.std_dev));
                                            ui.end_row();

                                            ui.label("Median:");
                                            ui.label(format!("{:.1}", stats.median));
                                            ui.end_row();

                                            ui.label("Sum:");
                                            ui.label(format!("{:.0}", stats.sum));
                                            ui.end_row();
                                        });

                                    ui.separator();
                                    ui.label("Percentiles");
                                    egui::Grid::new("pixel_stats_percentiles_grid")
                                        .num_columns(2)
                                        .spacing([8.0, 2.0])
                                        .show(ui, |ui| {
                                            ui.label("P1:");
                                            ui.label(format!("{:.1}", stats.p1));
                                            ui.end_row();

                                            ui.label("P5:");
                                            ui.label(format!("{:.1}", stats.p5));
                                            ui.end_row();

                                            ui.label("P25 (Q1):");
                                            ui.label(format!("{:.1}", stats.p25));
                                            ui.end_row();

                                            ui.label("P50:");
                                            ui.label(format!("{:.1}", stats.p50));
                                            ui.end_row();

                                            ui.label("P75 (Q3):");
                                            ui.label(format!("{:.1}", stats.p75));
                                            ui.end_row();

                                            ui.label("P95:");
                                            ui.label(format!("{:.1}", stats.p95));
                                            ui.end_row();

                                            ui.label("P99:");
                                            ui.label(format!("{:.1}", stats.p99));
                                            ui.end_row();
                                        });

                                    ui.add_space(4.0);
                                    if ui
                                        .button("Copy to Clipboard")
                                        .on_hover_text(
                                            "Copy pixel statistics as formatted text",
                                        )
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(stats.to_clipboard_text());
                                    }
                                } else {
                                    ui.label("No frame data available");
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

                        // Physical coordinate calibration UI (bd-4088.6)
                        egui::CollapsingHeader::new("Calibration")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label("Pixel to Physical Unit Conversion");
                                ui.separator();

                                ui.horizontal(|ui| {
                                    ui.label("X Scale:");
                                    let mut scale_x_str = self
                                        .pixel_scale_x
                                        .map(|v| format!("{:.4}", v))
                                        .unwrap_or_default();
                                    if ui.text_edit_singleline(&mut scale_x_str).changed() {
                                        self.pixel_scale_x = scale_x_str.parse().ok();
                                    }
                                    ui.label("units/pixel");
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Y Scale:");
                                    let mut scale_y_str = self
                                        .pixel_scale_y
                                        .map(|v| format!("{:.4}", v))
                                        .unwrap_or_default();
                                    if ui.text_edit_singleline(&mut scale_y_str).changed() {
                                        self.pixel_scale_y = scale_y_str.parse().ok();
                                    }
                                    ui.label("units/pixel");
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Unit:");
                                    egui::ComboBox::from_id_salt("scale_unit")
                                        .selected_text(&self.scale_unit)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.scale_unit,
                                                "µm".to_string(),
                                                "µm",
                                            );
                                            ui.selectable_value(
                                                &mut self.scale_unit,
                                                "mm".to_string(),
                                                "mm",
                                            );
                                            ui.selectable_value(
                                                &mut self.scale_unit,
                                                "nm".to_string(),
                                                "nm",
                                            );
                                        });
                                });

                                if ui.button("Clear Calibration").clicked() {
                                    self.pixel_scale_x = None;
                                    self.pixel_scale_y = None;
                                }
                            });
                    });
                }
            });
    }

    #[allow(clippy::cast_precision_loss)]
    fn render_echelle_preview_panel(&mut self, ui: &mut egui::Ui) {
        self.render_echelle_calibration_workspace(ui);
        ui.separator();

        if self.echelle_profile_cache.profile().is_none() {
            ui.weak("No active echelle calibration profile loaded for live extraction preview.");
            ui.weak("Use the calibration workspace above to load/save+activate a profile path.");
            if self.echelle_preview.is_none() {
                return;
            }
        }

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.echelle_extraction_enabled, "Enabled");
            ui.add(
                egui::DragValue::new(&mut self.echelle_extract_every_n_frames)
                    .range(1..=120)
                    .prefix("Every ")
                    .suffix(" frames"),
            );
            ui.checkbox(&mut self.echelle_show_merged_plot, "Merged");
            ui.separator();
            ui.label("X:");
            ui.selectable_value(
                &mut self.echelle_plot_x_axis_mode,
                EchellePlotXAxisMode::Wavelength,
                "Wavelength",
            );
            ui.selectable_value(
                &mut self.echelle_plot_x_axis_mode,
                EchellePlotXAxisMode::SampleIndex,
                "Sample",
            );
            ui.separator();
            ui.add(
                egui::DragValue::new(&mut self.echelle_plot_smoothing_window)
                    .range(1..=25)
                    .prefix("Smooth ")
                    .suffix(" px"),
            );
        });

        if let Some(profile) = self.echelle_profile_cache.profile() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.small("Calibration Profile");
                ui.small(format!(
                    "{}{}",
                    profile.display_name,
                    profile
                        .profile_id
                        .as_ref()
                        .map(|id| format!(" ({id})"))
                        .unwrap_or_default()
                ));
                ui.small(format!(
                    "Schema {}.{}.{} | Frame {}x{} | ROI ({}, {}) | Bin {}x{}",
                    profile.schema_version.major,
                    profile.schema_version.minor,
                    profile.schema_version.patch,
                    profile.compatibility.frame_width,
                    profile.compatibility.frame_height,
                    profile.compatibility.roi_x,
                    profile.compatibility.roi_y,
                    profile.compatibility.binning_x,
                    profile.compatibility.binning_y
                ));
                ui.small(format!(
                    "Created by {}{} at {}",
                    profile.provenance.creator_tool,
                    profile
                        .provenance
                        .creator_version
                        .as_ref()
                        .map(|v| format!("@{v}"))
                        .unwrap_or_default(),
                    profile.provenance.created_at_utc
                ));
                if let Some(path) = self.echelle_profile_cache.path() {
                    ui.small(format!("Path: {}", path.display()));
                }
            });
        }

        if let Some(err) = &self.echelle_preview_error {
            ui.colored_label(colors::ERROR, err);
        }

        let Some(preview) = &self.echelle_preview else {
            ui.weak("Waiting for extracted spectrum preview...");
            return;
        };

        ui.small(format!(
            "Profile: {} | Frame {} | Orders: {}",
            preview.profile_display_name,
            preview.frame_number,
            preview.orders.len()
        ));

        if !preview.orders.is_empty() {
            let selected_text = preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| match o.physical_order_number {
                    Some(m) => format!("Order {} (m={})", o.relative_index, m),
                    None => format!("Order {}", o.relative_index),
                })
                .unwrap_or_else(|| "Order 0".to_string());

            egui::ComboBox::from_id_salt("echelle_order_plot_selector")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (idx, order) in preview.orders.iter().enumerate() {
                        let label = match order.physical_order_number {
                            Some(m) => format!("Order {} (m={})", order.relative_index, m),
                            None => format!("Order {}", order.relative_index),
                        };
                        ui.selectable_value(&mut self.echelle_selected_order_plot, idx, label);
                    }
                });
        }

        let mut plot_label = "Order";
        let (mut xs, ys, unit) = if self.echelle_show_merged_plot {
            if let Some(merged) = &preview.merged {
                plot_label = "Merged";
                (
                    merged.wavelengths.clone(),
                    &merged.flux,
                    merged.wavelength_unit.as_str(),
                )
            } else if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
                (
                    order.wavelengths.clone(),
                    &order.flux,
                    order.wavelength_unit.as_str(),
                )
            } else {
                ui.weak("No order spectra available.");
                return;
            }
        } else if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
            (
                order.wavelengths.clone(),
                &order.flux,
                order.wavelength_unit.as_str(),
            )
        } else {
            ui.weak("No order spectra available.");
            return;
        };

        if xs.is_empty() || ys.is_empty() {
            ui.weak("Extracted spectrum is empty.");
            return;
        }

        if self.echelle_plot_x_axis_mode == EchellePlotXAxisMode::SampleIndex {
            xs = (0..ys.len()).map(|i| i as f64).collect();
        }
        let display_flux = if self.echelle_plot_smoothing_window > 1 {
            smooth_display_series(ys, self.echelle_plot_smoothing_window as usize)
        } else {
            ys.clone()
        };

        let points: Vec<[f64; 2]> = xs
            .iter()
            .copied()
            .zip(display_flux.iter().copied())
            .map(|(x, y)| [x, y])
            .collect();
        let line = Line::new(format!("{plot_label} flux"), PlotPoints::new(points));
        let selected_order_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview.orders.get(self.echelle_selected_order_plot)
        };
        let wavelength_lookup_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| o.wavelengths.as_slice())
        };
        let flux_lookup_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| o.flux.as_slice())
        };
        let mut hover_link = None;

        Plot::new("image_viewer_echelle_preview_plot")
            .height(180.0)
            .allow_scroll(false)
            .allow_drag(true)
            .allow_zoom(true)
            .x_axis_label(match self.echelle_plot_x_axis_mode {
                EchellePlotXAxisMode::Wavelength => unit,
                EchellePlotXAxisMode::SampleIndex => "sample",
            })
            .y_axis_label("counts")
            .show(ui, |plot_ui| {
                plot_ui.line(line);
                if let (Some(order), Some(w_lookup), Some(f_lookup)) = (
                    selected_order_for_hover,
                    wavelength_lookup_for_hover,
                    flux_lookup_for_hover,
                ) {
                    if let Some(pointer) = plot_ui.pointer_coordinate() {
                        let idx = match self.echelle_plot_x_axis_mode {
                            EchellePlotXAxisMode::SampleIndex => {
                                #[allow(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_possible_wrap,
                                    clippy::cast_sign_loss
                                )]
                                let idx = pointer.x.round() as isize;
                                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                                let clamped = idx
                                    .clamp(0, (f_lookup.len().saturating_sub(1)) as isize)
                                    as usize;
                                clamped
                            }
                            EchellePlotXAxisMode::Wavelength => nearest_index_by_x(&xs, pointer.x),
                        };
                        if idx < w_lookup.len() && idx < f_lookup.len() {
                            hover_link = Some(EchellePlotHoverLink {
                                relative_index: order.relative_index,
                                sample_index: idx,
                                wavelength: w_lookup[idx],
                                flux: f_lookup[idx],
                            });
                        }
                    }
                }
            });
        self.echelle_plot_hover_link = hover_link;

        if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
            #[allow(clippy::cast_precision_loss)]
            let mean_valid = if order.valid_fraction.is_empty() {
                0.0
            } else {
                order
                    .valid_fraction
                    .iter()
                    .map(|&v| f64::from(v))
                    .sum::<f64>()
                    / order.valid_fraction.len() as f64
            };
            ui.small(format!(
                "Coverage: {}/{} samples | mean valid frac: {:.2} | saturated samples: {}",
                order.covered_samples, order.total_samples, mean_valid, order.saturated_samples
            ));
            if let Some(link) = self.echelle_plot_hover_link {
                if link.relative_index == order.relative_index {
                    ui.small(format!(
                        "Hover: sample {} | λ={:.6} {} | flux={:.2}",
                        link.sample_index, link.wavelength, order.wavelength_unit, link.flux
                    ));
                }
            }
        }

        egui::CollapsingHeader::new("Diagnostics")
            .default_open(false)
            .show(ui, |ui| {
                ui.small(format!("extract runs: {}", self.echelle_extract_runs));
                ui.small(format!("extract errors: {}", self.echelle_extract_errors));
                ui.small(format!(
                    "decimation skips: {}",
                    self.echelle_extract_skipped_frames
                ));
                if let Some(ms) = self.echelle_last_extract_ms {
                    ui.small(format!("last extract: {:.2} ms", ms));
                }
                ui.small(format!(
                    "preview spectra objects: {}",
                    self.echelle_preview_measurements.len()
                ));
            });
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
        // Eagerly load camera parameters so settings panel populates on device selection
        // rather than requiring the user to start a stream first.
        self.load_camera_params(client, runtime, device_id);
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

fn smooth_display_series(values: &[f64], window: usize) -> Vec<f64> {
    if values.is_empty() || window <= 1 {
        return values.to_vec();
    }
    let radius = window / 2;
    let mut out = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        let start = i.saturating_sub(radius);
        let end = (i + radius + 1).min(values.len());
        let sum: f64 = values[start..end].iter().sum();
        #[allow(clippy::cast_precision_loss)]
        let avg = sum / (end - start) as f64;
        out.push(avg);
    }
    out
}

fn nearest_index_by_x(xs: &[f64], target: f64) -> usize {
    let mut best_idx = 0usize;
    let mut best_dist = f64::INFINITY;
    for (i, &x) in xs.iter().enumerate() {
        let d = (x - target).abs();
        if d < best_dist {
            best_dist = d;
            best_idx = i;
        }
    }
    best_idx
}

fn compute_wavelength_fit_residuals_for_order(
    order: &EchelleOrderCalibration,
    points: &[EchelleCalibrationPointUi],
) -> Vec<(f64, f64)> {
    let mut residuals = Vec::new();
    for point in points
        .iter()
        .filter(|p| p.enabled && p.order_relative_index == order.relative_index)
    {
        let predicted = match &order.wavelength {
            EchelleWavelengthModel::Polynomial {
                basis,
                coefficients,
                domain_start,
                domain_end,
                ..
            } => eval_polynomial_for_ui(
                *basis,
                coefficients,
                *domain_start,
                *domain_end,
                point.x_sample,
            ),
            EchelleWavelengthModel::Sampled { wavelengths, .. } => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let idx = point.x_sample.round().clamp(0.0, f64::MAX) as usize;
                wavelengths.get(idx).copied()
            }
        };
        if let Some(predicted) = predicted {
            residuals.push((point.x_sample, point.wavelength - predicted));
        }
    }
    residuals
}

fn fit_wavelength_model_for_order_from_points(
    order: &mut EchelleOrderCalibration,
    points: &[EchelleCalibrationPointUi],
) -> Result<String, String> {
    let enabled_points: Vec<&EchelleCalibrationPointUi> = points
        .iter()
        .filter(|p| p.enabled && p.order_relative_index == order.relative_index)
        .collect();
    if enabled_points.len() < 2 {
        return Err(format!(
            "Need at least 2 enabled calibration points for order {}",
            order.relative_index
        ));
    }

    match &mut order.wavelength {
        EchelleWavelengthModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
            unit,
        } => {
            let degree = coefficients.len().saturating_sub(1);
            if enabled_points.len() < coefficients.len() {
                return Err(format!(
                    "Need at least {} enabled points to fit degree {} polynomial for order {}",
                    coefficients.len(),
                    degree,
                    order.relative_index
                ));
            }
            let xs: Vec<f64> = enabled_points.iter().map(|p| p.x_sample).collect();
            let ys: Vec<f64> = enabled_points.iter().map(|p| p.wavelength).collect();
            let fitted = fit_polynomial_basis_least_squares_ui(
                *basis,
                degree,
                *domain_start,
                *domain_end,
                &xs,
                &ys,
            )?;
            *coefficients = fitted;
            Ok(format!(
                "Fitted order {} wavelength polynomial (degree {}, {} points, unit {})",
                order.relative_index,
                degree,
                enabled_points.len(),
                unit
            ))
        }
        EchelleWavelengthModel::Sampled { .. } => Err(format!(
            "Selected order {} uses sampled wavelengths; sampled refit UI not implemented yet",
            order.relative_index
        )),
    }
}

fn eval_polynomial_for_ui(
    basis: PolynomialBasis,
    coefficients: &[f64],
    domain_start: f64,
    domain_end: f64,
    x: f64,
) -> Option<f64> {
    if coefficients.is_empty()
        || !x.is_finite()
        || !domain_start.is_finite()
        || !domain_end.is_finite()
        || domain_start >= domain_end
    {
        return None;
    }
    let value = match basis {
        PolynomialBasis::Monomial => {
            let mut acc = 0.0f64;
            for &c in coefficients.iter().rev() {
                acc = acc * x + c;
            }
            acc
        }
        PolynomialBasis::Chebyshev => {
            let t = ((2.0 * (x - domain_start)) / (domain_end - domain_start)) - 1.0;
            if coefficients.len() == 1 {
                coefficients[0]
            } else {
                let mut t0 = 1.0f64;
                let mut t1 = t;
                let mut acc = coefficients[0] * t0 + coefficients[1] * t1;
                for &c in coefficients.iter().skip(2) {
                    let tn = 2.0 * t * t1 - t0;
                    acc += c * tn;
                    t0 = t1;
                    t1 = tn;
                }
                acc
            }
        }
    };
    value.is_finite().then_some(value)
}

fn fit_polynomial_basis_least_squares_ui(
    basis: PolynomialBasis,
    degree: usize,
    domain_start: f64,
    domain_end: f64,
    xs: &[f64],
    ys: &[f64],
) -> Result<Vec<f64>, String> {
    if xs.len() != ys.len() || xs.is_empty() {
        return Err("xs/ys must be non-empty and same length".to_string());
    }
    let n_coeff = degree + 1;
    let mut ata = vec![vec![0.0f64; n_coeff]; n_coeff];
    let mut aty = vec![0.0f64; n_coeff];

    for (&x, &y) in xs.iter().zip(ys) {
        let terms = basis_terms_for_ui(basis, degree, domain_start, domain_end, x)
            .ok_or_else(|| "invalid polynomial basis/domain/input while fitting".to_string())?;
        for i in 0..n_coeff {
            aty[i] += terms[i] * y;
            for j in 0..n_coeff {
                ata[i][j] += terms[i] * terms[j];
            }
        }
    }

    solve_linear_system_gaussian(ata, aty)
        .ok_or_else(|| "least-squares solve failed (singular/ill-conditioned matrix)".to_string())
}

fn basis_terms_for_ui(
    basis: PolynomialBasis,
    degree: usize,
    domain_start: f64,
    domain_end: f64,
    x: f64,
) -> Option<Vec<f64>> {
    if !x.is_finite()
        || !domain_start.is_finite()
        || !domain_end.is_finite()
        || domain_start >= domain_end
    {
        return None;
    }
    let mut out = vec![0.0; degree + 1];
    match basis {
        PolynomialBasis::Monomial => {
            let mut p = 1.0;
            for term in &mut out {
                *term = p;
                p *= x;
            }
        }
        PolynomialBasis::Chebyshev => {
            let t = ((2.0 * (x - domain_start)) / (domain_end - domain_start)) - 1.0;
            out[0] = 1.0;
            if degree >= 1 {
                out[1] = t;
            }
            for n in 2..=degree {
                out[n] = 2.0 * t * out[n - 1] - out[n - 2];
            }
        }
    }
    Some(out)
}

#[allow(clippy::needless_range_loop)]
fn solve_linear_system_gaussian(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = a.len();
    if n == 0 || b.len() != n || a.iter().any(|row| row.len() != n) {
        return None;
    }

    for col in 0..n {
        let mut pivot = col;
        let mut best = a[col][col].abs();
        for row in (col + 1)..n {
            let v = a[row][col].abs();
            if v > best {
                best = v;
                pivot = row;
            }
        }
        if best <= 1e-12 || !best.is_finite() {
            return None;
        }
        if pivot != col {
            a.swap(pivot, col);
            b.swap(pivot, col);
        }

        let pivot_val = a[col][col];
        for j in col..n {
            a[col][j] /= pivot_val;
        }
        b[col] /= pivot_val;

        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            if factor == 0.0 {
                continue;
            }
            for j in col..n {
                a[row][j] -= factor * a[col][j];
            }
            b[row] -= factor * b[col];
        }
    }

    if b.iter().all(|v| v.is_finite()) {
        Some(b)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
#[allow(clippy::cast_possible_wrap)]
fn detect_cross_dispersion_peaks_from_frame(
    data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    dispersion_axis: DetectorAxis,
    min_separation_px: u32,
    max_peaks: usize,
    threshold_fraction: f64,
) -> Result<Vec<f64>, String> {
    if width == 0 || height == 0 {
        return Err("Frame dimensions must be non-zero".to_string());
    }
    if max_peaks == 0 {
        return Ok(Vec::new());
    }

    let cross_len = match dispersion_axis {
        DetectorAxis::X => height as usize,
        DetectorAxis::Y => width as usize,
    };
    let disp_len = match dispersion_axis {
        DetectorAxis::X => width as usize,
        DetectorAxis::Y => height as usize,
    };
    let mut profile = vec![0.0f64; cross_len];

    for cross in 0..cross_len {
        #[allow(clippy::cast_precision_loss)]
        let mut sum = 0.0f64;
        for disp in 0..disp_len {
            #[allow(clippy::cast_possible_truncation)]
            let (x, y) = match dispersion_axis {
                DetectorAxis::X => (disp as u32, cross as u32),
                DetectorAxis::Y => (cross as u32, disp as u32),
            };
            let px =
                get_pixel_value_inline(data, x, y, width, height, bit_depth).ok_or_else(|| {
                    "Failed to read pixel while auto-detecting trace seeds".to_string()
                })?;
            sum += f64::from(px);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = sum / disp_len.max(1) as f64;
        profile[cross] = avg;
    }

    let mut sorted = profile.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let max_value = profile
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(median);
    let threshold = median + (max_value - median) * threshold_fraction.clamp(0.0, 1.0);

    let mut candidates: Vec<(usize, f64)> = Vec::new();
    for i in 1..profile.len().saturating_sub(1) {
        let v = profile[i];
        if v >= threshold && v > profile[i - 1] && v >= profile[i + 1] {
            candidates.push((i, v));
        }
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));

    #[allow(clippy::cast_possible_wrap)]
    let min_sep = min_separation_px as isize;
    #[allow(clippy::cast_precision_loss)]
    let mut selected: Vec<usize> = Vec::new();
    for (idx, _v) in candidates {
        if selected
            .iter()
            .all(|&keep| (keep as isize - idx as isize).abs() >= min_sep)
        {
            selected.push(idx);
            if selected.len() >= max_peaks {
                break;
            }
        }
    }
    selected.sort_unstable();

    #[allow(clippy::cast_precision_loss)]
    let result: Vec<f64> = selected.into_iter().map(|i| i as f64).collect();
    Ok(result)
}

#[cfg(test)]
mod echelle_calibration_ui_helper_tests {
    use super::*;

    #[test]
    fn eval_polynomial_for_ui_monomial_and_chebyshev() {
        let mono = eval_polynomial_for_ui(PolynomialBasis::Monomial, &[2.0, 3.0], 0.0, 10.0, 4.0);
        assert_eq!(mono, Some(14.0));

        let cheb = eval_polynomial_for_ui(PolynomialBasis::Chebyshev, &[5.0, 2.0], 0.0, 10.0, 10.0);
        // At x=10 => normalized t=1, so 5 + 2*T1 = 7
        assert_eq!(cheb, Some(7.0));
    }

    #[test]
    fn compute_wavelength_fit_residuals_filters_by_order_and_enabled_flag() {
        let order = EchelleOrderCalibration {
            relative_index: 3,
            physical_order_number: Some(77),
            sample_start: 0,
            sample_end: 100,
            trace: EchelleTraceModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![10.0],
                domain_start: 0.0,
                domain_end: 101.0,
            },
            wavelength: EchelleWavelengthModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![500.0, 0.1],
                domain_start: 0.0,
                domain_end: 101.0,
                unit: "nm".to_string(),
            },
            aperture_half_width_px: Some(4.0),
            enabled: true,
            notes: None,
        };

        let points = vec![
            EchelleCalibrationPointUi {
                enabled: true,
                order_relative_index: 3,
                x_sample: 10.0,
                y_pixel: 0.0,
                wavelength: 501.2, // predicted 501.0 => residual 0.2
                note: String::new(),
            },
            EchelleCalibrationPointUi {
                enabled: false,
                order_relative_index: 3,
                x_sample: 20.0,
                y_pixel: 0.0,
                wavelength: 503.0,
                note: String::new(),
            },
            EchelleCalibrationPointUi {
                enabled: true,
                order_relative_index: 4,
                x_sample: 10.0,
                y_pixel: 0.0,
                wavelength: 999.0,
                note: String::new(),
            },
        ];

        let residuals = compute_wavelength_fit_residuals_for_order(&order, &points);
        assert_eq!(residuals.len(), 1);
        assert!((residuals[0].0 - 10.0).abs() < 1e-9);
        assert!((residuals[0].1 - 0.2).abs() < 1e-9);
    }

    #[test]
    fn fit_polynomial_basis_least_squares_ui_recovers_linear_coefficients() {
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0];
        let ys: Vec<f64> = xs.iter().map(|x| 10.0 + 0.5 * x).collect();
        let coeffs =
            fit_polynomial_basis_least_squares_ui(PolynomialBasis::Monomial, 1, 0.0, 5.0, &xs, &ys)
                .unwrap();
        assert_eq!(coeffs.len(), 2);
        assert!((coeffs[0] - 10.0).abs() < 1e-9);
        assert!((coeffs[1] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn detect_cross_dispersion_peaks_from_frame_finds_bright_rows() {
        let width = 8u32;
        let height = 12u32;
        let mut data = vec![1u8; (width * height) as usize];
        for x in 0..width {
            data[(3 * width + x) as usize] = 50;
            data[(8 * width + x) as usize] = 80;
        }
        let peaks = detect_cross_dispersion_peaks_from_frame(
            &data,
            width,
            height,
            8,
            DetectorAxis::X,
            2,
            8,
            0.2,
        )
        .unwrap();
        assert_eq!(peaks, vec![3.0, 8.0]);
    }
}

// Unit tests for image_viewer.rs functions
//
// Tests cover:
// - Pixel value extraction (get_pixel_value_inline)
// - Frame conversion (convert_frame_to_rgba_into)
// - Min/max computation for auto-contrast
// - Percentile-based contrast
// - Histogram operations
// - Colormap and scale mode application
// - Edge cases and boundary conditions

#[cfg(test)]
mod pixel_value_tests {
    use super::*;

    #[test]
    fn test_get_pixel_value_8bit() {
        let data = vec![10u8, 20, 30, 40, 50, 60, 70, 80, 90];
        let width = 3;
        let height = 3;

        // Test valid positions
        assert_eq!(
            get_pixel_value_inline(&data, 0, 0, width, height, 8),
            Some(10)
        );
        assert_eq!(
            get_pixel_value_inline(&data, 1, 0, width, height, 8),
            Some(20)
        );
        assert_eq!(
            get_pixel_value_inline(&data, 2, 2, width, height, 8),
            Some(90)
        );

        // Test out of bounds
        assert_eq!(get_pixel_value_inline(&data, 3, 0, width, height, 8), None);
        assert_eq!(get_pixel_value_inline(&data, 0, 3, width, height, 8), None);
    }

    #[test]
    fn test_get_pixel_value_16bit() {
        // Little-endian 16-bit data: [0x0100, 0x0200, 0x0300, 0x0400]
        let data = vec![0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04];
        let width = 2;
        let height = 2;

        assert_eq!(
            get_pixel_value_inline(&data, 0, 0, width, height, 16),
            Some(0x0100)
        );
        assert_eq!(
            get_pixel_value_inline(&data, 1, 0, width, height, 16),
            Some(0x0200)
        );
        assert_eq!(
            get_pixel_value_inline(&data, 0, 1, width, height, 16),
            Some(0x0300)
        );
        assert_eq!(
            get_pixel_value_inline(&data, 1, 1, width, height, 16),
            Some(0x0400)
        );

        // Out of bounds
        assert_eq!(get_pixel_value_inline(&data, 2, 0, width, height, 16), None);
    }

    #[test]
    fn test_get_pixel_value_edge_cases() {
        let data = vec![42u8];
        // Single pixel
        assert_eq!(get_pixel_value_inline(&data, 0, 0, 1, 1, 8), Some(42));
        assert_eq!(get_pixel_value_inline(&data, 1, 0, 1, 1, 8), None);

        // Empty data
        let empty: Vec<u8> = vec![];
        assert_eq!(get_pixel_value_inline(&empty, 0, 0, 0, 0, 8), None);

        // Invalid bit depth
        assert_eq!(get_pixel_value_inline(&data, 0, 0, 1, 1, 32), None);
    }
}

#[cfg(test)]
mod minmax_tests {
    use super::*;

    #[test]
    fn test_compute_minmax_8bit() {
        let data = vec![10u8, 50, 100, 200, 255];
        let (min, max) = compute_minmax_from_data(&data, 8, 255.0);

        assert!((min - 10.0 / 255.0).abs() < 0.001);
        assert!((max - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_compute_minmax_16bit() {
        // Create 16-bit data: 100, 1000, 10000, 65535
        let data = vec![
            100u8, 0, // 100
            0xe8, 0x03, // 1000
            0x10, 0x27, // 10000
            0xff, 0xff, // 65535
        ];

        let (min, max) = compute_minmax_from_data(&data, 16, 65535.0);

        assert!((min - 100.0 / 65535.0).abs() < 0.001);
        assert!((max - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_compute_minmax_single_value() {
        let data = vec![128u8; 10];
        let (min, max) = compute_minmax_from_data(&data, 8, 255.0);

        // All same value: min == max, so function returns default (0.0, 1.0)
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);
    }

    #[test]
    fn test_compute_minmax_empty() {
        let data: Vec<u8> = vec![];
        let (min, max) = compute_minmax_from_data(&data, 8, 255.0);

        // Empty data should return default range
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);
    }

    #[test]
    fn test_compute_percentile_minmax() {
        // Create data with outliers: [0, 1, 2, ..., 98, 99, 255]
        let mut data: Vec<u8> = (0..100).collect();
        data.push(255); // Outlier

        // Use 1st and 99th percentile (should exclude the 255)
        let (min, max) = compute_percentile_minmax(&data, 8, 255.0, 1.0, 99.0);

        // Should approximately exclude 0 and 255
        assert!(min > 0.0 / 255.0);
        assert!(max < 255.0 / 255.0);
        assert!(max > 98.0 / 255.0); // Should be around 99
    }

    #[test]
    fn test_compute_percentile_minmax_16bit() {
        // Create 16-bit data
        let mut data = Vec::new();
        for i in 0..100u16 {
            data.extend_from_slice(&i.to_le_bytes());
        }

        let (min, max) = compute_percentile_minmax(&data, 16, 65535.0, 5.0, 95.0);

        // Should exclude lowest and highest 5%
        assert!(min > 0.0);
        assert!(max < 1.0);
    }
}

#[cfg(test)]
mod histogram_tests {
    use super::*;

    #[test]
    fn test_build_histogram_8bit() {
        // Create data with known distribution
        let data = vec![0u8, 0, 128, 128, 128, 255, 255];
        let hist = build_histogram(&data, 8, 256);

        assert_eq!(hist.len(), 256);
        assert_eq!(hist[0], 2); // Two zeros
        assert_eq!(hist[128], 3); // Three 128s
        assert_eq!(hist[255], 2); // Two 255s
    }

    #[test]
    fn test_build_histogram_16bit() {
        // Create 16-bit data
        let mut data = Vec::new();
        data.extend_from_slice(&0u16.to_le_bytes()); // 0
        data.extend_from_slice(&32768u16.to_le_bytes()); // Mid value
        data.extend_from_slice(&65535u16.to_le_bytes()); // Max value

        let hist = build_histogram(&data, 16, 256);

        assert_eq!(hist.len(), 256);
        assert!(hist[0] > 0); // Should have bin for 0
        assert!(hist[255] > 0); // Should have bin for 65535
                                // 32768 * (255/65535) = 127.5, truncates to bin 127
        assert!(hist[127] > 0); // Should have bin for 32768
    }

    #[test]
    fn test_histogram_equalization_lut() {
        // Create a simple histogram with uneven distribution
        let histogram = vec![100u32, 0, 0, 0, 200, 0, 0, 0]; // 300 pixels total
        let lut = compute_histogram_equalization_lut(&histogram, 300);

        assert_eq!(lut.len(), 8);
        // First bin (100 pixels) should map to ~1/3
        assert!(lut[0] < 0.5);
        // Fifth bin (200 pixels) should map to near 1.0
        assert!(lut[4] > 0.5);
    }

    #[test]
    fn test_clahe_lut() {
        let histogram = vec![100u32, 200, 300, 400, 100]; // 1100 pixels
        let lut = compute_clahe_lut(&histogram, 1100, 2.0);

        assert_eq!(lut.len(), 5);
        // CLAHE should produce monotonically increasing LUT
        for i in 1..lut.len() {
            assert!(lut[i] >= lut[i - 1]);
        }
    }
}

#[cfg(test)]
mod colormap_tests {
    use super::*;

    #[test]
    fn test_colormap_grayscale() {
        let colormap = Colormap::Grayscale;

        // Test boundary values
        assert_eq!(colormap.apply(0.0), [0, 0, 0]);
        assert_eq!(colormap.apply(1.0), [255, 255, 255]);

        // Test mid value
        let mid = colormap.apply(0.5);
        assert!((f32::from(mid[0]) - 127.5).abs() < 2.0); // Allow some rounding
        assert_eq!(mid[0], mid[1]);
        assert_eq!(mid[1], mid[2]);
    }

    #[test]
    fn test_colormap_viridis() {
        let colormap = Colormap::Viridis;

        // Viridis should start dark and end yellowish
        let low = colormap.apply(0.0);
        let high = colormap.apply(1.0);

        // Low should be darker (cast to u32 to avoid u8 overflow)
        let low_sum = u32::from(low[0]) + u32::from(low[1]) + u32::from(low[2]);
        let high_sum = u32::from(high[0]) + u32::from(high[1]) + u32::from(high[2]);
        assert!(low_sum < high_sum);
    }

    #[test]
    fn test_colormap_clamping() {
        let colormap = Colormap::Grayscale;

        // Test that values outside 0-1 are clamped
        assert_eq!(colormap.apply(-1.0), [0, 0, 0]);
        assert_eq!(colormap.apply(2.0), [255, 255, 255]);
    }

    #[test]
    fn test_colormap_labels() {
        assert_eq!(Colormap::Grayscale.label(), "Grayscale");
        assert_eq!(Colormap::Viridis.label(), "Viridis");
        assert_eq!(Colormap::Inferno.label(), "Inferno");
        assert_eq!(Colormap::Plasma.label(), "Plasma");
        assert_eq!(Colormap::Magma.label(), "Magma");
    }
}

#[cfg(test)]
mod scale_mode_tests {
    use super::*;

    #[test]
    fn test_scale_mode_linear() {
        let mode = ScaleMode::Linear;

        assert_eq!(mode.apply(0.0), 0.0);
        assert_eq!(mode.apply(0.5), 0.5);
        assert_eq!(mode.apply(1.0), 1.0);
    }

    #[test]
    fn test_scale_mode_sqrt() {
        let mode = ScaleMode::Sqrt;

        assert_eq!(mode.apply(0.0), 0.0);
        assert!((mode.apply(0.25) - 0.5).abs() < 0.01);
        assert_eq!(mode.apply(1.0), 1.0);
    }

    #[test]
    fn test_scale_mode_log() {
        let mode = ScaleMode::Log;

        assert_eq!(mode.apply(0.0), 0.0);
        assert!(mode.apply(0.5) > 0.0);
        assert!(mode.apply(1.0) > mode.apply(0.5));
    }

    #[test]
    fn test_scale_mode_labels() {
        assert_eq!(ScaleMode::Linear.label(), "Linear");
        assert_eq!(ScaleMode::Log.label(), "Log");
        assert_eq!(ScaleMode::Sqrt.label(), "Sqrt");
    }
}

#[cfg(test)]
mod contrast_mode_tests {
    use super::*;

    #[test]
    fn test_contrast_mode_labels() {
        assert_eq!(ContrastMode::Manual.label(), "Manual");
        assert_eq!(ContrastMode::AutoSimple.label(), "Auto (Simple)");
        assert_eq!(ContrastMode::AutoPercentile.label(), "Auto (Percentile)");
        assert_eq!(ContrastMode::HistogramEq.label(), "Histogram Eq");
        assert_eq!(ContrastMode::Clahe.label(), "CLAHE");
    }

    #[test]
    fn test_contrast_mode_all() {
        let modes = ContrastMode::all();
        assert_eq!(modes.len(), 5);
        assert!(modes.contains(&ContrastMode::Manual));
        assert!(modes.contains(&ContrastMode::Clahe));
    }
}

#[cfg(test)]
mod frame_conversion_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_convert_frame_8bit_grayscale() {
        let data = vec![0u8, 127, 255];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 3,
            height: 1,
            bit_depth: 8,
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        let (min, max) = convert_frame_to_rgba_into(&req, &mut buffer);

        // Should return requested min/max for manual mode
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);

        // Buffer should be 3 pixels * 4 channels = 12 bytes
        assert_eq!(buffer.len(), 12);

        // Check pixel values (RGBA)
        assert_eq!(buffer[0], 0); // R of first pixel
        assert_eq!(buffer[1], 0); // G
        assert_eq!(buffer[2], 0); // B
        assert_eq!(buffer[3], 255); // A (always 255)

        // Middle pixel should be ~127
        assert!((i32::from(buffer[4]) - 127).abs() <= 1);

        // Last pixel should be 255
        assert_eq!(buffer[8], 255);
    }

    #[test]
    fn test_convert_frame_16bit() {
        let data = vec![
            0x00, 0x00, // 0
            0xff, 0x7f, // 32767
            0xff, 0xff, // 65535
        ];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 3,
            height: 1,
            bit_depth: 16,
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        convert_frame_to_rgba_into(&req, &mut buffer);

        assert_eq!(buffer.len(), 12);
        assert_eq!(buffer[0], 0); // First pixel black
        assert_eq!(buffer[8], 255); // Last pixel white
    }

    #[test]
    fn test_convert_frame_auto_contrast() {
        // Create data with limited range
        let data = vec![100u8, 150, 200];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 3,
            height: 1,
            bit_depth: 8,
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: true,
            contrast_mode: ContrastMode::AutoSimple,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        let (min, max) = convert_frame_to_rgba_into(&req, &mut buffer);

        // Should compute actual min/max
        assert!(min > 0.0);
        assert!(max < 1.0);
    }

    #[test]
    fn test_convert_frame_zero_dimensions() {
        let data = vec![];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 0,
            height: 0,
            bit_depth: 8,
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        let (min, max) = convert_frame_to_rgba_into(&req, &mut buffer);

        // Should handle gracefully
        assert_eq!(buffer.len(), 0);
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);
    }

    #[test]
    fn test_convert_frame_with_colormap() {
        let data = vec![0u8, 127, 255];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 3,
            height: 1,
            bit_depth: 8,
            frame_number: 0,
            colormap: Colormap::Viridis,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        convert_frame_to_rgba_into(&req, &mut buffer);

        // Viridis should have different R, G, B values (not grayscale)
        assert!(buffer[0] != buffer[1] || buffer[1] != buffer[2]);
    }

    #[test]
    fn test_convert_frame_buffer_reuse() {
        let data = vec![128u8; 100];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 10,
            height: 10,
            bit_depth: 8,
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::with_capacity(400);
        convert_frame_to_rgba_into(&req, &mut buffer);

        let capacity1 = buffer.capacity();
        assert_eq!(buffer.len(), 400); // 100 pixels * 4 channels

        // Convert again with same size - capacity should not change
        convert_frame_to_rgba_into(&req, &mut buffer);
        assert_eq!(buffer.capacity(), capacity1);
    }
}

#[cfg(test)]
mod helper_function_tests {
    use super::*;

    #[test]
    fn test_stream_quality_label() {
        assert_eq!(stream_quality_label(StreamQuality::Full), "Full");
        assert_eq!(stream_quality_label(StreamQuality::Preview), "Preview (2x)");
        assert_eq!(stream_quality_label(StreamQuality::Fast), "Fast (4x)");
    }

    #[test]
    fn test_image_viewer_default_quality_and_histogram() {
        let panel = ImageViewerPanel::default();
        assert_eq!(panel.stream_quality, StreamQuality::Fast);
        assert_eq!(panel.histogram_position, HistogramPosition::SidePanel);
    }
}

#[cfg(test)]
mod edge_case_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_oversized_frame_protection() {
        // Test that the conversion protects against integer overflow
        let data = vec![0u8; 1000];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: u32::MAX / 2,
            height: u32::MAX / 2,
            bit_depth: 8,
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        let (min, max) = convert_frame_to_rgba_into(&req, &mut buffer);

        // Should handle gracefully without panic
        assert_eq!(buffer.len(), 0);
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);
    }

    #[test]
    fn test_single_pixel_frame() {
        let data = vec![200u8];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 1,
            height: 1,
            bit_depth: 8,
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        convert_frame_to_rgba_into(&req, &mut buffer);

        assert_eq!(buffer.len(), 4); // 1 pixel * 4 channels
        assert_eq!(buffer[3], 255); // Alpha should be 255
    }

    #[test]
    fn test_invalid_bit_depth() {
        let data = vec![100u8; 16];
        let req = RgbaConversionRequest {
            data: Arc::new(data),
            width: 4,
            height: 4,
            bit_depth: 32, // Invalid
            frame_number: 0,
            colormap: Colormap::Grayscale,
            scale_mode: ScaleMode::Linear,
            display_min: 0.0,
            display_max: 1.0,
            auto_contrast: false,
            contrast_mode: ContrastMode::Manual,
            percentile_low: 0.1,
            percentile_high: 99.9,
            colorbar_midpoint: 0.5,
        };

        let mut buffer = Vec::new();
        convert_frame_to_rgba_into(&req, &mut buffer);

        // Should produce checkerboard error pattern
        assert_eq!(buffer.len(), 64); // 16 pixels * 4 channels
    }
}
