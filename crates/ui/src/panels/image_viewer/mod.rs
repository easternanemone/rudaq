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
mod frame_handling;
mod processing;
mod rendering;
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
use echelle::wavelength_fitting::{
    detect_arc_lines, fit_order_wavelength, load_hgar_atlas, match_lines_to_atlas,
};
use echelle::{
    AxisDirection, DetectorAxis, EchelleArtifactRef, EchelleCalibrationProfile,
    EchelleExtractionConfig, EchelleFrameCompatibility, EchelleOrderCalibration,
    EchelleOrientation, EchelleProvenance, EchelleSchemaVersion, EchelleSummationMode,
    EchelleTraceModel, EchelleWavelengthModel, PolynomialBasis,
};
use protocol::compression::decompress_frame_into;
use protocol::daq::StreamQuality;

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

use echelle_calibration::*;

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
    pub(in crate::panels::image_viewer) fps_counter: FpsCounter,
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
    pub(in crate::panels::image_viewer) action_rx: std::sync::mpsc::Receiver<ImageViewerAction>,
    /// Async action sender
    pub(in crate::panels::image_viewer) action_tx: std::sync::mpsc::Sender<ImageViewerAction>,
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
    pub(in crate::panels::image_viewer) rgba_rx:
        Option<std::sync::mpsc::Receiver<RgbaConversionResult>>,
    /// Sender for RGBA conversion requests (cloned to background thread)
    pub(in crate::panels::image_viewer) rgba_request_tx:
        Option<std::sync::mpsc::SyncSender<RgbaConversionRequest>>,
    /// Pending RGBA data ready to be applied to texture
    pub(in crate::panels::image_viewer) pending_rgba: Option<RgbaConversionResult>,
    /// Sender to recycle used buffers back to the converter thread (bd-wdx3)
    pub(super) rgba_recycle_tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    /// True when thread spawn failed (e.g., WASM); skip retry and convert synchronously
    pub(super) rgba_sync_mode: bool,

    // -- Background Echelle Extraction (bd-fwyp: move extraction off UI thread) --
    /// Receiver for completed echelle extractions from background thread
    pub(in crate::panels::image_viewer) echelle_extract_rx:
        Option<std::sync::mpsc::Receiver<EchelleExtractionResult>>,
    /// Sender for echelle extraction requests
    pub(in crate::panels::image_viewer) echelle_extract_tx:
        Option<std::sync::mpsc::SyncSender<EchelleExtractionRequest>>,
    /// Pending echelle extraction result ready to be applied
    pub(in crate::panels::image_viewer) pending_echelle: Option<EchelleExtractionResult>,
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
    pub(in crate::panels::image_viewer) echelle_preview: Option<EchelleExtractionPreview>,
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
    pub(in crate::panels::image_viewer) echelle_cal_ui: EchelleCalibrationUiState,
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
