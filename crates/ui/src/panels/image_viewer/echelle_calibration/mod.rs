//! Echelle calibration workspace UI - profile editor, trace fitting, arc line detection, wavelength fitting.

mod pipeline;
mod rendering;

use echelle::wavelength_fitting::{ArcDetectConfig, ArcLine, OrderWlSolution, WlFitConfig};
use echelle::EchelleCalibrationProfile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::panels::image_viewer) enum EchelleCalibrationTab {
    #[default]
    Profile,
    Trace,
    LinePoints,
    WavelengthFit,
    BlazeFlat,
    MechelleNotes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(in crate::panels::image_viewer) struct EchelleCalibrationPointUi {
    pub(in crate::panels::image_viewer) enabled: bool,
    pub(in crate::panels::image_viewer) order_relative_index: u32,
    pub(in crate::panels::image_viewer) x_sample: f64,
    pub(in crate::panels::image_viewer) y_pixel: f64,
    pub(in crate::panels::image_viewer) wavelength: f64,
    pub(in crate::panels::image_viewer) note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(in crate::panels::image_viewer) struct EchelleLineListEntryUi {
    pub(in crate::panels::image_viewer) enabled: bool,
    pub(in crate::panels::image_viewer) wavelength: f64,
    pub(in crate::panels::image_viewer) label: String,
}

/// Row in the arc-line match table (bd-a64a).
#[derive(Debug, Clone)]
pub(in crate::panels::image_viewer) struct ArcLineMatchRow {
    /// Detected line pixel center.
    pub(in crate::panels::image_viewer) pixel_center: f64,
    /// Detected line SNR (amplitude / noise).
    pub(in crate::panels::image_viewer) snr: f64,
    /// Detected line FWHM in pixels.
    pub(in crate::panels::image_viewer) fwhm: f64,
    /// Matched atlas wavelength (nm).
    pub(in crate::panels::image_viewer) matched_wavelength_nm: f64,
    /// Residual: predicted - atlas (nm). Set after fitting.
    pub(in crate::panels::image_viewer) residual_nm: f64,
    /// Atlas line species label.
    pub(in crate::panels::image_viewer) species: String,
    /// Whether this match is included in the fit.
    pub(in crate::panels::image_viewer) included: bool,
    /// Index into the detected_arc_lines vec.
    pub(in crate::panels::image_viewer) detected_line_idx: usize,
    /// Index into the atlas vec.
    pub(in crate::panels::image_viewer) atlas_line_idx: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct EchelleCalibrationUiState {
    pub(in crate::panels::image_viewer) tab: EchelleCalibrationTab,
    pub(in crate::panels::image_viewer) editor_profile: Option<EchelleCalibrationProfile>,
    pub(in crate::panels::image_viewer) editor_dirty: bool,
    pub(in crate::panels::image_viewer) editor_last_loaded_path: Option<std::path::PathBuf>,
    pub(crate) save_as_path_text: String,
    /// Last-used calibration profile paths (daemon-relative or local), newest first (bd-c7of).
    pub(crate) recent_profile_paths: Vec<String>,
    pub(in crate::panels::image_viewer) points_path_text: String,
    pub(in crate::panels::image_viewer) line_list_path_text: String,
    pub(in crate::panels::image_viewer) blaze_export_path_text: String,
    pub(in crate::panels::image_viewer) calibration_points: Vec<EchelleCalibrationPointUi>,
    pub(in crate::panels::image_viewer) line_list: Vec<EchelleLineListEntryUi>,
    pub(in crate::panels::image_viewer) selected_order_edit_idx: usize,
    pub(in crate::panels::image_viewer) selected_point_idx: usize,
    pub(in crate::panels::image_viewer) trace_overlay_enabled: bool,
    pub(in crate::panels::image_viewer) trace_overlay_all_orders: bool,
    pub(in crate::panels::image_viewer) trace_overlay_sample_step: u32,
    pub(in crate::panels::image_viewer) trace_overlay_max_orders: u32,
    pub(in crate::panels::image_viewer) trace_nudge_px: f64,
    pub(in crate::panels::image_viewer) trace_auto_detect_min_separation_px: u32,
    pub(in crate::panels::image_viewer) trace_auto_detect_threshold_fraction: f64,
    pub(in crate::panels::image_viewer) fit_outlier_sigma: f64,
    pub(in crate::panels::image_viewer) fit_rms_acceptance_px: f64,
    // Arc line detection state (bd-a64a)
    pub(in crate::panels::image_viewer) arc_detect_config: ArcDetectConfig,
    pub(in crate::panels::image_viewer) detected_arc_lines: Vec<ArcLine>,
    // Atlas matching state (bd-a64a)
    pub(in crate::panels::image_viewer) atlas_match_tolerance_nm: f64,
    pub(in crate::panels::image_viewer) matched_pairs: Vec<ArcLineMatchRow>,
    // Chebyshev fit state (bd-a64a)
    pub(in crate::panels::image_viewer) wl_fit_config: WlFitConfig,
    pub(in crate::panels::image_viewer) wl_fit_solution: Option<OrderWlSolution>,
    pub(in crate::panels::image_viewer) blaze_preview_enabled: bool,
    pub(in crate::panels::image_viewer) blaze_preview_scale: f64,
    pub(in crate::panels::image_viewer) status_message: Option<String>,
    pub(in crate::panels::image_viewer) last_error: Option<String>,
}

impl EchelleCalibrationUiState {
    pub const RECENT_PROFILE_PATHS_MAX: usize = 5;

    pub(in crate::panels::image_viewer) fn record_recent_profile_path(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        self.recent_profile_paths.retain(|p| p != path);
        self.recent_profile_paths.insert(0, path.to_string());
        self.recent_profile_paths
            .truncate(Self::RECENT_PROFILE_PATHS_MAX);
    }

    pub(in crate::panels::image_viewer) fn with_defaults() -> Self {
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
