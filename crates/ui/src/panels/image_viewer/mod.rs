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
mod controls;
mod echelle_calibration;
mod echelle_extraction;
mod echelle_profile_cache;
#[cfg(not(target_arch = "wasm32"))]
mod echelle_sidecar;
mod echelle_spectrum;
mod processing;
mod side_panel;
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
use common::echelle_wavelength_fitting::{
    detect_arc_lines, fit_order_wavelength, load_hgar_atlas, match_lines_to_atlas, ArcDetectConfig,
    ArcLine, OrderWlSolution, WlFitConfig,
};
use protocol::compression::decompress_frame_into;
use protocol::daq::StreamQuality;
use serde::{Deserialize, Serialize};

/// View mode for the central area: 2D echellogram, 1D spectrum, or split (bd-alxb).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SpectrumViewMode {
    #[default]
    Echellogram,
    Spectrum,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum EchellePlotXAxisMode {
    #[default]
    Wavelength,
    SampleIndex,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EchellePlotHoverLink {
    pub(super) relative_index: u32,
    pub(super) sample_index: usize,
    pub(super) wavelength: f64,
    pub(super) flux: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum EchelleCalibrationTab {
    #[default]
    Profile,
    Trace,
    LinePoints,
    WavelengthFit,
    BlazeFlat,
    MechelleNotes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct EchelleCalibrationPointUi {
    pub(super) enabled: bool,
    pub(super) order_relative_index: u32,
    pub(super) x_sample: f64,
    pub(super) y_pixel: f64,
    pub(super) wavelength: f64,
    pub(super) note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct EchelleLineListEntryUi {
    pub(super) enabled: bool,
    pub(super) wavelength: f64,
    pub(super) label: String,
}

/// Row in the arc-line match table (bd-a64a).
#[derive(Debug, Clone)]
pub(super) struct ArcLineMatchRow {
    /// Detected line pixel center.
    pub(super) pixel_center: f64,
    /// Detected line SNR (amplitude / noise).
    pub(super) snr: f64,
    /// Detected line FWHM in pixels.
    pub(super) fwhm: f64,
    /// Matched atlas wavelength (nm).
    pub(super) matched_wavelength_nm: f64,
    /// Residual: predicted - atlas (nm). Set after fitting.
    pub(super) residual_nm: f64,
    /// Atlas line species label.
    pub(super) species: String,
    /// Whether this match is included in the fit.
    pub(super) included: bool,
    /// Index into the detected_arc_lines vec.
    pub(super) detected_line_idx: usize,
    /// Index into the atlas vec.
    pub(super) atlas_line_idx: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct EchelleCalibrationUiState {
    pub(super) tab: EchelleCalibrationTab,
    pub(super) editor_profile: Option<EchelleCalibrationProfile>,
    pub(super) editor_dirty: bool,
    pub(super) editor_last_loaded_path: Option<std::path::PathBuf>,
    pub(super) save_as_path_text: String,
    pub(super) points_path_text: String,
    pub(super) line_list_path_text: String,
    pub(super) blaze_export_path_text: String,
    pub(super) calibration_points: Vec<EchelleCalibrationPointUi>,
    pub(super) line_list: Vec<EchelleLineListEntryUi>,
    pub(super) selected_order_edit_idx: usize,
    pub(super) selected_point_idx: usize,
    pub(super) trace_overlay_enabled: bool,
    pub(super) trace_overlay_all_orders: bool,
    pub(super) trace_overlay_sample_step: u32,
    pub(super) trace_overlay_max_orders: u32,
    pub(super) trace_nudge_px: f64,
    pub(super) trace_auto_detect_min_separation_px: u32,
    pub(super) trace_auto_detect_threshold_fraction: f64,
    pub(super) fit_outlier_sigma: f64,
    pub(super) fit_rms_acceptance_px: f64,
    // Arc line detection state (bd-a64a)
    pub(super) arc_detect_config: ArcDetectConfig,
    pub(super) detected_arc_lines: Vec<ArcLine>,
    // Atlas matching state (bd-a64a)
    pub(super) atlas_match_tolerance_nm: f64,
    pub(super) matched_pairs: Vec<ArcLineMatchRow>,
    // Chebyshev fit state (bd-a64a)
    pub(super) wl_fit_config: WlFitConfig,
    pub(super) wl_fit_solution: Option<OrderWlSolution>,
    pub(super) blaze_preview_enabled: bool,
    pub(super) blaze_preview_scale: f64,
    pub(super) status_message: Option<String>,
    pub(super) last_error: Option<String>,
}

impl EchelleCalibrationUiState {
    pub(super) fn with_defaults() -> Self {
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
            arc_detect_config: ArcDetectConfig::default(),
            atlas_match_tolerance_nm: 0.5,
            wl_fit_config: WlFitConfig::default(),
            blaze_preview_enabled: false,
            blaze_preview_scale: 1.0,
            ..Default::default()
        }
    }
}

/// Image Viewer Panel state
pub struct ImageViewerPanel {
    /// Currently selected device ID
    pub(super) device_id: Option<String>,
    /// Current frame dimensions
    pub(super) width: u32,
    pub(super) height: u32,
    /// Current frame bit depth
    pub(super) bit_depth: u32,
    /// Frame counter
    pub(super) frame_count: u64,
    /// Cached texture handle
    pub(super) texture: Option<egui::TextureHandle>,
    /// Current colormap
    pub(super) colormap: Colormap,
    /// Current scale mode
    pub(super) scale_mode: ScaleMode,
    /// Zoom level (1.0 = fit to window)
    pub(super) zoom: f32,
    /// Pan offset
    pub(super) pan: egui::Vec2,
    /// Frame update receiver
    pub(super) frame_rx: Option<FrameUpdateReceiver>,
    /// Frame update sender (for cloning to async tasks)
    pub(super) frame_tx: Option<FrameUpdateSender>,
    /// Active stream subscription
    pub(super) subscription: Option<FrameStreamSubscription>,
    /// FPS counter
    pub(super) fps_counter: FpsCounter,
    /// Auto-fit zoom on next frame
    pub(super) auto_fit: bool,
    /// Error message
    pub(super) error: Option<String>,
    /// Status message
    pub(super) status: Option<String>,
    /// Max FPS for streaming (rate limit)
    pub(super) max_fps: u32,
    /// ROI selector state
    pub(super) roi_selector: RoiSelector,
    /// Last frame raw data (for ROI statistics computation)
    pub(super) last_frame_data: Option<Arc<Vec<u8>>>,
    /// Show ROI statistics panel
    pub(super) show_roi_panel: bool,
    /// Show pixel statistics panel (bd-li4i)
    pub(super) show_pixel_stats: bool,
    /// Cached pixel statistics for current frame (bd-li4i)
    pub(super) pixel_statistics: Option<PixelStatistics>,
    /// Histogram for intensity distribution
    pub(super) histogram: Histogram,
    /// Histogram display position
    pub(super) histogram_position: HistogramPosition,
    /// Available camera devices
    pub(super) available_cameras: Vec<String>,
    /// Full sensor dimensions by camera ID (from device metadata)
    pub(super) camera_full_frame_dims: std::collections::HashMap<String, (u32, u32)>,
    /// Display minimum (0.0-1.0 normalized) - pixels at or below this are black
    pub(super) display_min: f32,
    /// Display maximum (0.0-1.0 normalized) - pixels at or above this are white
    pub(super) display_max: f32,
    /// Auto-contrast mode - automatically compute min/max from frame data (deprecated, use contrast_mode)
    pub(super) auto_contrast: bool,
    /// Contrast enhancement mode (bd-j6xm)
    pub(super) contrast_mode: ContrastMode,
    /// Low percentile for auto-percentile mode (0.0-100.0) (bd-j6xm)
    pub(super) percentile_low: f32,
    /// High percentile for auto-percentile mode (0.0-100.0) (bd-j6xm)
    pub(super) percentile_high: f32,
    /// Async action receiver
    pub(super) action_rx: std::sync::mpsc::Receiver<ImageViewerAction>,
    /// Async action sender
    pub(super) action_tx: std::sync::mpsc::Sender<ImageViewerAction>,
    /// Last refresh time
    pub(super) last_refresh: Option<Instant>,
    /// Stream generation counter — incremented on each start_stream() call.
    /// Used by streaming tasks to detect if they've been superseded, preventing
    /// stale tasks from calling stop_stream() and killing a newer stream.
    pub(super) stream_generation: Arc<AtomicU64>,

    // -- Camera Control Fields --
    /// Camera parameters (cached)
    pub(super) camera_params: Vec<ParameterCache>,
    /// Parameter edit buffers (device_id, param_name) -> value
    pub(super) param_edit_buffers: std::collections::HashMap<(String, String), String>,
    /// Parameter errors (device_id, param_name) -> error
    pub(super) param_errors: std::collections::HashMap<(String, String), String>,
    /// Show controls side panel
    pub(super) show_controls: bool,
    /// Receiver for parameter load results
    pub(super) param_load_rx: Option<mpsc::Receiver<ParamLoadResult>>,
    /// Sender for parameter set results (persistent, cloned per request)
    pub(super) param_set_tx: mpsc::Sender<ParamSetResult>,
    /// Receiver for parameter set results
    pub(super) param_set_rx: mpsc::Receiver<ParamSetResult>,
    /// Parameters currently being set
    pub(super) setting_params: std::collections::HashSet<(String, String)>,
    /// Pending parameter updates to execute
    pub(super) pending_param_updates: Vec<(String, String, String)>,
    /// Device ID currently loading parameters
    pub(super) loading_params_device: Option<String>,
    /// Live exposure preview mode (updates during drag)
    pub(super) live_exposure: bool,
    /// Last time exposure was sent (for debounce)
    pub(super) exposure_last_sent: Option<Instant>,

    // -- Connection Resilience Fields (bd-12qt) --
    /// Connection state for the current device
    pub(super) connection_state: ConnectionState,
    /// Number of consecutive connection failures
    pub(super) retry_count: u32,
    /// Time of last disconnect (for auto-retry backoff)
    pub(super) last_disconnect: Option<Instant>,
    /// Enable automatic reconnection attempts
    pub(super) auto_reconnect: bool,

    // -- Stream Metrics (bd-7rk0: gRPC improvements) --
    /// Latest streaming metrics from server
    pub(super) stream_metrics: Option<StreamMetrics>,

    // -- Physical Coordinate Calibration (bd-4088.6) --
    /// Pixel to physical unit calibration in X direction (units per pixel)
    pub(super) pixel_scale_x: Option<f64>,
    /// Pixel to physical unit calibration in Y direction (units per pixel)
    pub(super) pixel_scale_y: Option<f64>,
    /// Physical unit label (e.g., "µm", "mm")
    pub(super) scale_unit: String,

    // -- Recording Fields (bd-3pdi.5.3) --
    /// Current recording state
    pub(super) recording_state: RecordingState,
    /// Recording name input
    pub(super) recording_name: String,
    /// Current output path (when recording)
    pub(super) recording_output_path: Option<String>,
    /// Recording status from server
    pub(super) recording_status: Option<protocol::daq::RecordingStatus>,
    /// Last recording status poll time
    pub(super) last_recording_poll: Option<Instant>,

    // -- Stream Quality Settings --
    /// Stream quality level for server-side downsampling
    pub(super) stream_quality: StreamQuality,

    // -- Background RGBA Conversion (bd-xifj: move CPU work off UI thread) --
    /// Receiver for completed RGBA conversions from background thread
    pub(super) rgba_rx: Option<std::sync::mpsc::Receiver<RgbaConversionResult>>,
    /// Sender for RGBA conversion requests (cloned to background thread)
    pub(super) rgba_request_tx: Option<std::sync::mpsc::SyncSender<RgbaConversionRequest>>,
    /// Pending RGBA data ready to be applied to texture
    pub(super) pending_rgba: Option<RgbaConversionResult>,
    /// Sender to recycle used buffers back to the converter thread (bd-wdx3)
    pub(super) rgba_recycle_tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    /// True when thread spawn failed (e.g., WASM); skip retry and convert synchronously
    pub(super) rgba_sync_mode: bool,

    // -- Background Echelle Extraction (bd-fwyp: move extraction off UI thread) --
    /// Receiver for completed echelle extractions from background thread
    pub(super) echelle_extract_rx: Option<std::sync::mpsc::Receiver<EchelleExtractionResult>>,
    /// Sender for echelle extraction requests
    pub(super) echelle_extract_tx: Option<std::sync::mpsc::SyncSender<EchelleExtractionRequest>>,
    /// Pending echelle extraction result ready to be applied
    pub(super) pending_echelle: Option<EchelleExtractionResult>,
    /// True when thread spawn failed (e.g., WASM); extract synchronously
    pub(super) echelle_sync_mode: bool,

    // -- Crosshair Feature (bd-pgcb) --
    /// Enable crosshair cursor display
    pub(super) crosshair_enabled: bool,
    /// Locked crosshair position (pixel coordinates)
    pub(super) crosshair_locked_pos: Option<(i32, i32)>,

    // -- Interactive Colorbar (bd-07j1) --
    /// Interactive colorbar widget for midpoint adjustment
    pub(super) colorbar: crate::widgets::Colorbar,
    /// Show colorbar in the image viewer
    pub(super) show_colorbar: bool,

    // -- Metadata Overlay (bd-6h1c) --
    /// Show acquisition metadata overlay on the image
    pub(super) show_metadata_overlay: bool,

    // -- Scale Bar Overlay (bd-0tcg) --
    /// Show scale bar overlay on the image
    pub(super) show_scale_bar: bool,
    /// Last frame timestamp in nanoseconds (for overlay display)
    pub(super) last_frame_timestamp_ns: u64,

    // -- Echelle Calibration Profile Cache (bd-2kla.2.4) --
    /// Optional cached echelle calibration profile with hot-reload-safe semantics.
    pub(super) echelle_profile_cache: EchelleProfileCache,
    /// Last extracted echelle spectrum preview (MVP local extraction path).
    pub(super) echelle_preview: Option<EchelleExtractionPreview>,
    /// Most recent extraction error (kept separate from general panel errors).
    pub(super) echelle_preview_error: Option<String>,
    /// Extract every Nth frame to bound CPU cost on UI path.
    pub(super) echelle_extract_every_n_frames: u32,
    /// Toggle echelle extraction preview while profile is loaded.
    pub(super) echelle_extraction_enabled: bool,
    /// Selected order index for side-panel plot (0 = first extracted order).
    pub(super) echelle_selected_order_plot: usize,
    /// Show merged wavelength-sorted preview when available.
    pub(super) echelle_show_merged_plot: bool,
    /// Reusable scratch buffer for 12/16-bit decode fallback path (allocation control).
    pub(super) echelle_decode_scratch_u16: Vec<u16>,
    /// Debug/export hook: latest preview spectra materialized as Measurement::Spectrum values.
    pub(super) echelle_preview_measurements: Vec<Measurement>,
    /// Display-only moving-average smoothing for the spectrum preview plot.
    pub(super) echelle_plot_smoothing_window: u32,
    /// X-axis display mode for the spectrum preview.
    pub(super) echelle_plot_x_axis_mode: EchellePlotXAxisMode,
    /// Developer diagnostics counters for the local extractor.
    pub(super) echelle_extract_runs: u64,
    pub(super) echelle_extract_errors: u64,
    pub(super) echelle_extract_skipped_frames: u64,
    pub(super) echelle_last_extract_ms: Option<f64>,
    /// Hover cross-link from spectrum plot to image sample marker.
    pub(super) echelle_plot_hover_link: Option<EchellePlotHoverLink>,
    /// Calibration authoring workspace state (bd-2kla.8 scaffolding).
    pub(super) echelle_cal_ui: EchelleCalibrationUiState,
    /// True when the active echelle profile snapshot should be resynced into RunEngine state.
    pub(super) echelle_run_engine_sync_dirty: bool,
    /// True while an async echelle snapshot sync request is in flight.
    pub(super) echelle_run_engine_sync_in_flight: bool,

    // -- Spectrum View Mode (bd-alxb) --
    /// Current view mode: 2D echellogram, 1D spectrum, or split view.
    pub(super) spectrum_view_mode: SpectrumViewMode,
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

            // Spectrum view mode (bd-alxb)
            spectrum_view_mode: SpectrumViewMode::default(),
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

                // === Spectrum View Mode (bd-alxb) ===
                // Show when any echelle context exists: active profile, extraction preview,
                // or editor draft (which auto-creates in WASM where filesystem isn't available).
                if self.echelle_profile_cache.profile().is_some()
                    || self.echelle_preview.is_some()
                    || self.echelle_cal_ui.editor_profile.is_some()
                {
                    ui.separator();
                    ui.label("View:");
                    ui.selectable_value(
                        &mut self.spectrum_view_mode,
                        SpectrumViewMode::Echellogram,
                        "2D",
                    )
                    .on_hover_text("2D echellogram");
                    ui.selectable_value(
                        &mut self.spectrum_view_mode,
                        SpectrumViewMode::Spectrum,
                        "1D",
                    )
                    .on_hover_text("1D spectrum (full width)");
                    ui.selectable_value(
                        &mut self.spectrum_view_mode,
                        SpectrumViewMode::Split,
                        "Split",
                    )
                    .on_hover_text("Split: 2D echellogram + 1D spectrum");
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
                // bd-alxb: Spectrum-only mode — full-width spectrum plot, skip image
                if self.spectrum_view_mode == SpectrumViewMode::Spectrum {
                    self.render_spectrum_plot_area(ui, true);
                    return;
                }

                // bd-alxb: Split mode — reserve bottom for spectrum plot
                if self.spectrum_view_mode == SpectrumViewMode::Split {
                    egui::TopBottomPanel::bottom("spectrum_split_panel")
                        .resizable(true)
                        .default_height(200.0)
                        .min_height(100.0)
                        .show_inside(ui, |ui| {
                            self.render_spectrum_plot_area(ui, true);
                        });
                }

                // Remaining space: 2D echellogram (Echellogram mode uses full area,
                // Split mode uses whatever's left after the bottom panel)
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
