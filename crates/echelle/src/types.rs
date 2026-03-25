//! Echelle calibration profile types and validation.
//!
//! This module defines the canonical, versioned calibration profile format used
//! by rust-daq for echellegram-to-spectrum extraction workflows.

use crate::scattered_light::{estimate_scatter_map_fast_f32, TraceInfo};
use chrono::{DateTime, Utc};
use common::core::Measurement;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

pub const ECHELLE_PROFILE_SCHEMA_MAJOR: u32 = 1;

/// Minimum blaze curve value to prevent excessive amplification at order edges.
/// Values below this floor are clamped before dividing science flux.
const BLAZE_FLOOR: f64 = 0.1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchelleSchemaVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl EchelleSchemaVersion {
    pub const fn v1() -> Self {
        Self {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }

    pub fn is_supported_for_read(&self) -> bool {
        self.major == ECHELLE_PROFILE_SCHEMA_MAJOR
    }
}

impl Default for EchelleSchemaVersion {
    fn default() -> Self {
        Self::v1()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorAxis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Positive,
    Negative,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchelleOrientation {
    pub dispersion_axis: DetectorAxis,
    pub cross_dispersion_axis: DetectorAxis,
    pub order_number_increase_direction: AxisDirection,
    pub wavelength_increase_with_dispersion_positive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchelleFrameCompatibility {
    pub sensor_width: u32,
    pub sensor_height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    #[serde(default)]
    pub roi_x: u32,
    #[serde(default)]
    pub roi_y: u32,
    #[serde(default = "default_one")]
    pub binning_x: u32,
    #[serde(default = "default_one")]
    pub binning_y: u32,
    #[serde(default)]
    pub bit_depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchelleBackgroundConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_inter_order_gap_px")]
    pub inter_order_gap_min_px: u32,
    #[serde(default = "default_baseline_window_px")]
    pub baseline_window_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EchelleSummationMode {
    OrderCenterPixel,
    SimpleSum,
    SqrtWeightedSum,
    Optimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchelleExtractionConfig {
    pub summation_mode: EchelleSummationMode,
    pub default_aperture_half_width_px: f64,
    #[serde(default)]
    pub background: Option<EchelleBackgroundConfig>,
    /// Live scattered-light subtraction applied before per-order extraction.
    /// Disabled by default (`None`). When enabled, a coarse-grid CERES method
    /// estimates the inter-order background and subtracts it from the decoded
    /// frame in-place before extraction.
    #[serde(default)]
    pub scattered_light: Option<ScatteredLightLiveConfig>,
}

/// Configuration for live (per-frame) scattered light subtraction.
///
/// Uses a coarse grid of inter-order pixel medians with bilinear
/// interpolation — fast enough for ~1 FPS at 2560×2160.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatteredLightLiveConfig {
    /// Column stride for coarse sampling (default: 8).
    #[serde(default = "default_col_stride")]
    pub col_stride: u32,
    /// Row stride for coarse sampling (default: 4).
    #[serde(default = "default_row_stride")]
    pub row_stride: u32,
}

impl Default for ScatteredLightLiveConfig {
    fn default() -> Self {
        Self {
            col_stride: default_col_stride(),
            row_stride: default_row_stride(),
        }
    }
}

const fn default_col_stride() -> u32 {
    8
}
const fn default_row_stride() -> u32 {
    4
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolynomialBasis {
    Monomial,
    Chebyshev,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EchelleTraceModel {
    Polynomial {
        basis: PolynomialBasis,
        coefficients: Vec<f64>,
        domain_start: f64,
        domain_end: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EchelleWavelengthModel {
    Polynomial {
        basis: PolynomialBasis,
        coefficients: Vec<f64>,
        domain_start: f64,
        domain_end: f64,
        unit: String,
    },
    Sampled {
        wavelengths: Vec<f64>,
        unit: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchelleOrderCalibration {
    pub relative_index: u32,
    #[serde(default)]
    pub physical_order_number: Option<i32>,
    pub sample_start: u32,
    pub sample_end: u32, // inclusive
    pub trace: EchelleTraceModel,
    pub wavelength: EchelleWavelengthModel,
    #[serde(default)]
    pub aperture_half_width_px: Option<f64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchelleArtifactRef {
    pub path: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EchelleCorrections {
    #[serde(default)]
    pub blaze: Option<EchelleArtifactRef>,
    #[serde(default)]
    pub flat_field: Option<EchelleArtifactRef>,
    #[serde(default)]
    pub bad_pixel_mask: Option<EchelleArtifactRef>,
    #[serde(default)]
    pub excluded_regions: Vec<PixelRegion>,
    /// Loaded per-order blaze curves (one `Vec<f64>` per order, indexed by
    /// position in `EchelleCalibrationProfile::orders`). Each element is a
    /// normalised blaze value in `[BLAZE_FLOOR, 1.0]`. Science flux is
    /// divided by this curve during extraction to remove the grating
    /// efficiency envelope.
    #[serde(default)]
    pub blaze_curves: Option<Vec<Vec<f64>>>,
}

/// Loaded bad-pixel mask: a 2D boolean grid where `true` marks a bad pixel
/// that should be excluded from all extraction sums and profile estimates.
///
/// Construct via [`BadPixelMask::from_raw_bytes`] (one byte per pixel, non-zero = bad)
/// or [`BadPixelMask::new`] for programmatic use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadPixelMask {
    width: u32,
    height: u32,
    /// Row-major mask: `mask[y * width + x]`.  `true` = bad pixel.
    mask: Vec<bool>,
}

impl BadPixelMask {
    /// Build a mask from a pre-computed boolean vector (row-major, `width * height` elements).
    ///
    /// Returns `Err` if the length does not match `width * height`.
    pub fn new(width: u32, height: u32, mask: Vec<bool>) -> Result<Self, String> {
        let expected = (width as usize).saturating_mul(height as usize);
        if mask.len() != expected {
            return Err(format!(
                "bad-pixel mask length {} does not match {}x{} = {}",
                mask.len(),
                width,
                height,
                expected
            ));
        }
        Ok(Self {
            width,
            height,
            mask,
        })
    }

    /// Load from raw bytes: one byte per pixel, row-major.  Non-zero = bad pixel.
    pub fn from_raw_bytes(width: u32, height: u32, data: &[u8]) -> Result<Self, String> {
        let expected = (width as usize).saturating_mul(height as usize);
        if data.len() != expected {
            return Err(format!(
                "raw mask data length {} does not match {}x{} = {}",
                data.len(),
                width,
                height,
                expected
            ));
        }
        let mask = data.iter().map(|&b| b != 0).collect();
        Ok(Self {
            width,
            height,
            mask,
        })
    }

    /// Returns `true` if the pixel at `(x, y)` is masked (bad).
    #[inline]
    pub fn is_masked(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.mask[(y * self.width + x) as usize]
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Number of pixels flagged as bad.
    pub fn bad_count(&self) -> usize {
        self.mask.iter().filter(|&&b| b).count()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchelleProvenance {
    pub creator_tool: String,
    #[serde(default)]
    pub creator_version: Option<String>,
    pub created_at_utc: DateTime<Utc>,
    #[serde(default)]
    pub source_frame_ids: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchelleCalibrationProfile {
    #[serde(default)]
    pub schema_version: EchelleSchemaVersion,
    #[serde(default)]
    pub profile_id: Option<String>,
    pub display_name: String,
    pub compatibility: EchelleFrameCompatibility,
    pub orientation: EchelleOrientation,
    pub extraction: EchelleExtractionConfig,
    pub orders: Vec<EchelleOrderCalibration>,
    #[serde(default)]
    pub corrections: EchelleCorrections,
    pub provenance: EchelleProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EchelleFrameContext {
    pub width: u32,
    pub height: u32,
    pub roi_x: Option<u32>,
    pub roi_y: Option<u32>,
    pub binning_x: Option<u32>,
    pub binning_y: Option<u32>,
    pub bit_depth: Option<u32>,
}

/// Structured result from frame/profile compatibility checking.
///
/// Replaces the old binary pass/fail `validate_for_frame()` with physically
/// meaningful compatibility states. Callers can decide whether to proceed,
/// warn, or reject based on the specific mismatch category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameCompatibility {
    /// Profile and frame match on all checked fields.
    Compatible,
    /// Minor mismatches that still allow extraction to proceed.
    /// This is used when the frame and profile agree on geometry
    /// (sensor dimensions, ROI, binning) but differ in other properties,
    /// such as bit depth. The caller should surface the warnings to the
    /// user but may proceed with extraction.
    UsableWithWarnings { warnings: Vec<CompatibilityWarning> },
    /// Profile cannot be meaningfully applied to this frame.
    Incompatible { reasons: Vec<CompatibilityWarning> },
}

impl FrameCompatibility {
    /// Returns `true` if extraction can proceed (compatible or usable with warnings).
    pub fn is_usable(&self) -> bool {
        !matches!(self, Self::Incompatible { .. })
    }

    /// Returns all warnings/reasons as a flat list.
    pub fn diagnostics(&self) -> &[CompatibilityWarning] {
        match self {
            Self::Compatible => &[],
            Self::UsableWithWarnings { warnings } | Self::Incompatible { reasons: warnings } => {
                warnings
            }
        }
    }
}

/// Individual compatibility diagnostic with enough context for the UI
/// to present physically meaningful feedback to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityWarning {
    /// Frame dimensions differ from the profile's calibrated dimensions.
    /// This is incompatible because trace positions and sample ranges
    /// are defined in the profile's frame coordinate space.
    FrameSizeMismatch {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    /// ROI origin differs. Trace positions are relative to sensor origin,
    /// so a different ROI shifts the extraction window.
    RoiMismatch {
        axis: &'static str,
        expected: u32,
        actual: u32,
    },
    /// Binning factor differs. This changes effective pixel scale and
    /// invalidates trace polynomial coefficients.
    BinningMismatch {
        axis: &'static str,
        expected: u32,
        actual: u32,
    },
    /// Bit depth differs. Extraction still works but saturation thresholds
    /// and noise models may be wrong.
    BitDepthMismatch { expected: u32, actual: u32 },
}

impl std::fmt::Display for CompatibilityWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FrameSizeMismatch {
                expected_width,
                expected_height,
                actual_width,
                actual_height,
            } => write!(
                f,
                "frame size mismatch: profile expects {}x{}, got {}x{}",
                expected_width, expected_height, actual_width, actual_height
            ),
            Self::RoiMismatch {
                axis,
                expected,
                actual,
            } => write!(
                f,
                "ROI {} mismatch: profile expects {}, got {}",
                axis, expected, actual
            ),
            Self::BinningMismatch {
                axis,
                expected,
                actual,
            } => write!(
                f,
                "binning {} mismatch: profile expects {}, got {}",
                axis, expected, actual
            ),
            Self::BitDepthMismatch { expected, actual } => write!(
                f,
                "bit depth mismatch: profile expects {}-bit, got {}-bit",
                expected, actual
            ),
        }
    }
}

#[derive(Debug, Error)]
pub enum EchelleProfileError {
    #[error("I/O error reading profile {path}: {source}")]
    IoRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("I/O error writing profile {path}: {source}")]
    IoWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported echelle profile extension for {path}; expected .toml or .json")]
    UnsupportedExtension { path: String },
    #[error("failed to parse TOML profile {path}: {source}")]
    TomlParse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to parse JSON profile {path}: {source}")]
    JsonParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize TOML profile {path}: {source}")]
    TomlSerialize {
        path: String,
        #[source]
        source: toml::ser::Error,
    },
    #[error("failed to serialize JSON profile {path}: {source}")]
    JsonSerialize {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("profile validation failed: {0}")]
    Validation(String),
}

impl EchelleCalibrationProfile {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, EchelleProfileError> {
        let path = path.as_ref();
        let path_str = path.display().to_string();
        let content = fs::read_to_string(path).map_err(|source| EchelleProfileError::IoRead {
            path: path_str.clone(),
            source,
        })?;

        let profile = match detect_profile_format(path)? {
            EchelleProfileFormat::Toml => toml::from_str::<Self>(&content).map_err(|source| {
                EchelleProfileError::TomlParse {
                    path: path_str.clone(),
                    source,
                }
            })?,
            EchelleProfileFormat::Json => {
                serde_json::from_str::<Self>(&content).map_err(|source| {
                    EchelleProfileError::JsonParse {
                        path: path_str.clone(),
                        source,
                    }
                })?
            }
        };

        // If older v1 minor/patch is loaded, preserve exact version while still validating.
        profile.validate()?;
        Ok(profile)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), EchelleProfileError> {
        let path = path.as_ref();
        let path_str = path.display().to_string();
        self.validate()?;
        let serialized = match detect_profile_format(path)? {
            EchelleProfileFormat::Toml => toml::to_string_pretty(self).map_err(|source| {
                EchelleProfileError::TomlSerialize {
                    path: path_str.clone(),
                    source,
                }
            })?,
            EchelleProfileFormat::Json => serde_json::to_string_pretty(self).map_err(|source| {
                EchelleProfileError::JsonSerialize {
                    path: path_str.clone(),
                    source,
                }
            })?,
        };

        fs::write(path, serialized).map_err(|source| EchelleProfileError::IoWrite {
            path: path_str,
            source,
        })
    }

    /// Serialize this profile to pretty-printed TOML in memory (no I/O).
    ///
    /// Used by WASM / gRPC save paths that write via the daemon instead of `save_to_path`.
    pub fn serialize_to_toml_string(&self) -> Result<String, EchelleProfileError> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(|source| EchelleProfileError::TomlSerialize {
            path: "<memory>".to_string(),
            source,
        })
    }

    pub fn validate(&self) -> Result<(), EchelleProfileError> {
        if !self.schema_version.is_supported_for_read() {
            return Err(EchelleProfileError::Validation(format!(
                "unsupported schema version {}.{}.{} (supported major = {})",
                self.schema_version.major,
                self.schema_version.minor,
                self.schema_version.patch,
                ECHELLE_PROFILE_SCHEMA_MAJOR
            )));
        }

        if self.display_name.trim().is_empty() {
            return Err(invalid("display_name must not be empty"));
        }
        if self.provenance.creator_tool.trim().is_empty() {
            return Err(invalid("provenance.creator_tool must not be empty"));
        }

        self.validate_compatibility()?;
        self.validate_orientation()?;
        self.validate_extraction()?;
        self.validate_corrections()?;
        self.validate_orders()?;
        Ok(())
    }

    /// Check structured compatibility between this profile and a frame.
    ///
    /// Returns a [`FrameCompatibility`] that distinguishes:
    /// - `Compatible`: all checked fields match
    /// - `UsableWithWarnings`: extraction can proceed but results may be degraded
    ///   (e.g., bit depth differs — saturation threshold will be wrong)
    /// - `Incompatible`: extraction would produce physically meaningless results
    ///   (e.g., frame dimensions or binning differ from calibration)
    ///
    /// This does NOT call `validate()` — the profile is assumed to be structurally
    /// valid. Call `validate()` separately if loading from untrusted input.
    pub fn check_frame_compatibility(&self, frame: &EchelleFrameContext) -> FrameCompatibility {
        let c = &self.compatibility;
        let mut hard_mismatches = Vec::new();
        let mut soft_mismatches = Vec::new();

        // Frame dimensions: incompatible if different, because trace positions
        // and sample ranges are defined in the profile's frame coordinate space.
        if frame.width != c.frame_width || frame.height != c.frame_height {
            hard_mismatches.push(CompatibilityWarning::FrameSizeMismatch {
                expected_width: c.frame_width,
                expected_height: c.frame_height,
                actual_width: frame.width,
                actual_height: frame.height,
            });
        }

        // ROI: incompatible if different, because trace polynomials encode
        // sensor-coordinate positions that depend on the ROI origin.
        if let Some(roi_x) = frame.roi_x {
            if roi_x != c.roi_x {
                hard_mismatches.push(CompatibilityWarning::RoiMismatch {
                    axis: "X",
                    expected: c.roi_x,
                    actual: roi_x,
                });
            }
        }
        if let Some(roi_y) = frame.roi_y {
            if roi_y != c.roi_y {
                hard_mismatches.push(CompatibilityWarning::RoiMismatch {
                    axis: "Y",
                    expected: c.roi_y,
                    actual: roi_y,
                });
            }
        }

        // Binning: incompatible if different, because it changes effective
        // pixel scale and invalidates trace polynomial coefficients.
        if let Some(bx) = frame.binning_x {
            if bx != c.binning_x {
                hard_mismatches.push(CompatibilityWarning::BinningMismatch {
                    axis: "X",
                    expected: c.binning_x,
                    actual: bx,
                });
            }
        }
        if let Some(by) = frame.binning_y {
            if by != c.binning_y {
                hard_mismatches.push(CompatibilityWarning::BinningMismatch {
                    axis: "Y",
                    expected: c.binning_y,
                    actual: by,
                });
            }
        }

        // Bit depth: usable with warning. Extraction math is the same but
        // saturation detection thresholds will be wrong.
        if let (Some(expected), Some(actual)) = (c.bit_depth, frame.bit_depth) {
            if expected != actual {
                soft_mismatches.push(CompatibilityWarning::BitDepthMismatch { expected, actual });
            }
        }

        if !hard_mismatches.is_empty() {
            FrameCompatibility::Incompatible {
                reasons: hard_mismatches,
            }
        } else if !soft_mismatches.is_empty() {
            FrameCompatibility::UsableWithWarnings {
                warnings: soft_mismatches,
            }
        } else {
            FrameCompatibility::Compatible
        }
    }

    /// Strict pass/fail validation for frame compatibility.
    ///
    /// Delegates to `check_frame_compatibility` and rejects anything
    /// that is not fully `Compatible`. Also validates the profile itself.
    ///
    /// Prefer `check_frame_compatibility` when the caller can handle
    /// warnings (e.g., UI preview can show "bit depth mismatch" without
    /// blocking extraction).
    pub fn validate_for_frame(
        &self,
        frame: EchelleFrameContext,
    ) -> Result<(), EchelleProfileError> {
        self.validate()?;

        let compat = self.check_frame_compatibility(&frame);
        match compat {
            FrameCompatibility::Compatible => Ok(()),
            FrameCompatibility::UsableWithWarnings { warnings } => {
                // Strict mode: treat warnings as errors
                let msgs: Vec<String> = warnings.iter().map(ToString::to_string).collect();
                Err(invalid(msgs.join("; ")))
            }
            FrameCompatibility::Incompatible { reasons } => {
                let msgs: Vec<String> = reasons.iter().map(ToString::to_string).collect();
                Err(invalid(msgs.join("; ")))
            }
        }
    }

    fn validate_compatibility(&self) -> Result<(), EchelleProfileError> {
        let c = &self.compatibility;
        if c.sensor_width == 0 || c.sensor_height == 0 {
            return Err(invalid("sensor dimensions must be > 0"));
        }
        if c.frame_width == 0 || c.frame_height == 0 {
            return Err(invalid("frame dimensions must be > 0"));
        }
        if c.binning_x == 0 || c.binning_y == 0 {
            return Err(invalid("binning values must be >= 1"));
        }
        if c.roi_x.saturating_add(c.frame_width) > c.sensor_width
            || c.roi_y.saturating_add(c.frame_height) > c.sensor_height
        {
            return Err(invalid(format!(
                "ROI/frame extent exceeds sensor bounds: roi=({}, {}), frame={}x{}, sensor={}x{}",
                c.roi_x, c.roi_y, c.frame_width, c.frame_height, c.sensor_width, c.sensor_height
            )));
        }
        Ok(())
    }

    fn validate_orientation(&self) -> Result<(), EchelleProfileError> {
        if self.orientation.dispersion_axis == self.orientation.cross_dispersion_axis {
            return Err(invalid(
                "orientation.dispersion_axis and cross_dispersion_axis must differ",
            ));
        }
        Ok(())
    }

    fn validate_extraction(&self) -> Result<(), EchelleProfileError> {
        if !self.extraction.default_aperture_half_width_px.is_finite()
            || self.extraction.default_aperture_half_width_px <= 0.0
        {
            return Err(invalid(
                "extraction.default_aperture_half_width_px must be finite and > 0",
            ));
        }
        if let Some(bg) = &self.extraction.background {
            if bg.inter_order_gap_min_px == 0 {
                return Err(invalid("background.inter_order_gap_min_px must be > 0"));
            }
            if bg.baseline_window_px == 0 {
                return Err(invalid("background.baseline_window_px must be > 0"));
            }
        }
        Ok(())
    }

    fn validate_corrections(&self) -> Result<(), EchelleProfileError> {
        for region in &self.corrections.excluded_regions {
            if region.width == 0 || region.height == 0 {
                return Err(invalid("excluded region width/height must be > 0"));
            }
        }
        Ok(())
    }

    fn validate_orders(&self) -> Result<(), EchelleProfileError> {
        if self.orders.is_empty() {
            return Err(invalid("orders must not be empty"));
        }
        let mut seen_relative = HashSet::new();
        let mut seen_physical = HashSet::new();
        let dispersion_len = match self.orientation.dispersion_axis {
            DetectorAxis::X => self.compatibility.frame_width,
            DetectorAxis::Y => self.compatibility.frame_height,
        };

        for order in &self.orders {
            if !seen_relative.insert(order.relative_index) {
                return Err(invalid(format!(
                    "duplicate order relative_index {}",
                    order.relative_index
                )));
            }
            if let Some(m) = order.physical_order_number {
                if !seen_physical.insert(m) {
                    return Err(invalid(format!("duplicate physical_order_number {}", m)));
                }
            }
            if order.sample_start > order.sample_end {
                return Err(invalid(format!(
                    "order {} sample_start > sample_end",
                    order.relative_index
                )));
            }
            if order.sample_end >= dispersion_len {
                return Err(invalid(format!(
                    "order {} sample range [{}..={}] exceeds dispersion length {}",
                    order.relative_index, order.sample_start, order.sample_end, dispersion_len
                )));
            }
            if let Some(half_width) = order.aperture_half_width_px {
                if !half_width.is_finite() || half_width <= 0.0 {
                    return Err(invalid(format!(
                        "order {} aperture_half_width_px must be finite and > 0",
                        order.relative_index
                    )));
                }
            }
            validate_trace_model(
                &order.trace,
                order.relative_index,
                order.sample_start,
                order.sample_end,
            )?;
            validate_wavelength_model(
                &order.wavelength,
                order.relative_index,
                order.sample_start,
                order.sample_end,
            )?;
        }

        Ok(())
    }
}

fn validate_trace_model(
    trace: &EchelleTraceModel,
    relative_index: u32,
    sample_start: u32,
    sample_end: u32,
) -> Result<(), EchelleProfileError> {
    match trace {
        EchelleTraceModel::Polynomial {
            coefficients,
            domain_start,
            domain_end,
            ..
        } => {
            if coefficients.is_empty() {
                return Err(invalid(format!(
                    "order {} trace polynomial coefficients must not be empty",
                    relative_index
                )));
            }
            if !all_finite(coefficients) {
                return Err(invalid(format!(
                    "order {} trace polynomial coefficients must be finite",
                    relative_index
                )));
            }
            if !domain_start.is_finite() || !domain_end.is_finite() || domain_start >= domain_end {
                return Err(invalid(format!(
                    "order {} trace domain must be finite and increasing",
                    relative_index
                )));
            }
            if *domain_start > f64::from(sample_start) || *domain_end < f64::from(sample_end) {
                return Err(invalid(format!(
                    "order {} trace domain [{}, {}] does not cover sample range [{}..={}]",
                    relative_index, domain_start, domain_end, sample_start, sample_end
                )));
            }
        }
    }
    Ok(())
}

fn validate_wavelength_model(
    wavelength: &EchelleWavelengthModel,
    relative_index: u32,
    sample_start: u32,
    sample_end: u32,
) -> Result<(), EchelleProfileError> {
    let sample_count = (sample_end - sample_start + 1) as usize;
    match wavelength {
        EchelleWavelengthModel::Polynomial {
            coefficients,
            domain_start,
            domain_end,
            unit,
            ..
        } => {
            if unit.trim().is_empty() {
                return Err(invalid(format!(
                    "order {} wavelength polynomial unit must not be empty",
                    relative_index
                )));
            }
            if coefficients.is_empty() {
                return Err(invalid(format!(
                    "order {} wavelength polynomial coefficients must not be empty",
                    relative_index
                )));
            }
            if !all_finite(coefficients) {
                return Err(invalid(format!(
                    "order {} wavelength polynomial coefficients must be finite",
                    relative_index
                )));
            }
            if !domain_start.is_finite() || !domain_end.is_finite() || domain_start >= domain_end {
                return Err(invalid(format!(
                    "order {} wavelength domain must be finite and increasing",
                    relative_index
                )));
            }
            if *domain_start > f64::from(sample_start) || *domain_end < f64::from(sample_end) {
                return Err(invalid(format!(
                    "order {} wavelength domain [{}, {}] does not cover sample range [{}..={}]",
                    relative_index, domain_start, domain_end, sample_start, sample_end
                )));
            }
        }
        EchelleWavelengthModel::Sampled { wavelengths, unit } => {
            if unit.trim().is_empty() {
                return Err(invalid(format!(
                    "order {} sampled wavelength unit must not be empty",
                    relative_index
                )));
            }
            if wavelengths.len() != sample_count {
                return Err(invalid(format!(
                    "order {} sampled wavelength length {} != sample count {}",
                    relative_index,
                    wavelengths.len(),
                    sample_count
                )));
            }
            if !all_finite(wavelengths) {
                return Err(invalid(format!(
                    "order {} sampled wavelengths must be finite",
                    relative_index
                )));
            }
            if is_constant_or_nan(wavelengths) {
                return Err(invalid(format!(
                    "order {} sampled wavelengths must vary",
                    relative_index
                )));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EchelleProfileFormat {
    Toml,
    Json,
}

fn detect_profile_format(path: &Path) -> Result<EchelleProfileFormat, EchelleProfileError> {
    match path.extension().and_then(|s| s.to_str()) {
        Some("toml") => Ok(EchelleProfileFormat::Toml),
        Some("json") => Ok(EchelleProfileFormat::Json),
        _ => Err(EchelleProfileError::UnsupportedExtension {
            path: path.display().to_string(),
        }),
    }
}

fn all_finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

fn is_constant_or_nan(values: &[f64]) -> bool {
    if values.len() < 2 {
        return true;
    }
    let first = values[0];
    values.iter().all(|v| (*v - first).abs() < f64::EPSILON)
}

fn invalid(msg: impl Into<String>) -> EchelleProfileError {
    EchelleProfileError::Validation(msg.into())
}

const fn default_one() -> u32 {
    1
}

const fn default_true() -> bool {
    true
}

const fn default_inter_order_gap_px() -> u32 {
    4
}

const fn default_baseline_window_px() -> u32 {
    11
}

#[derive(Debug, Clone, PartialEq)]
pub struct EchelleOrderPreview {
    pub relative_index: u32,
    pub physical_order_number: Option<i32>,
    pub wavelength_unit: String,
    pub wavelengths: Vec<f64>,
    pub flux: Vec<f64>,
    pub valid_fraction: Vec<f32>,
    pub saturated: Vec<bool>,
    pub total_samples: u32,
    pub covered_samples: u32,
    pub saturated_samples: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EchelleMergedPreview {
    pub wavelength_unit: String,
    pub wavelengths: Vec<f64>,
    pub flux: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EchelleExtractionPreview {
    pub frame_number: u64,
    pub profile_display_name: String,
    pub profile_id: Option<String>,
    pub orders: Vec<EchelleOrderPreview>,
    pub merged: Option<EchelleMergedPreview>,
}

impl EchelleExtractionPreview {
    pub fn to_measurements(&self) -> Vec<Measurement> {
        self.to_measurements_with_context(true, None)
    }

    pub fn to_measurements_with_context(
        &self,
        preview: bool,
        device_id: Option<&str>,
    ) -> Vec<Measurement> {
        let mut measurements =
            Vec::with_capacity(self.orders.len() + usize::from(self.merged.is_some()));
        let timestamp = Utc::now();

        for order in &self.orders {
            #[allow(clippy::cast_precision_loss)]
            // Diagnostic metadata is stored as f64 for downstream JSON consumers.
            let mean_valid_fraction = if order.valid_fraction.is_empty() {
                0.0
            } else {
                order
                    .valid_fraction
                    .iter()
                    .map(|&v| f64::from(v))
                    .sum::<f64>()
                    / order.valid_fraction.len() as f64
            };

            let mut metadata = json!({
                "kind": "echelle_order",
                "relative_index": order.relative_index,
                "physical_order_number": order.physical_order_number,
                "covered_samples": order.covered_samples,
                "total_samples": order.total_samples,
                "saturated_samples": order.saturated_samples,
                "mean_valid_fraction": mean_valid_fraction,
                "profile_id": self.profile_id,
                "profile_display_name": self.profile_display_name,
                "frame_number": self.frame_number,
                "preview": preview
            });
            if let Some(device_id) = device_id {
                insert_device_id_metadata(&mut metadata, device_id);
            }

            measurements.push(Measurement::Spectrum {
                name: format!("echelle_order_{}", order.relative_index),
                frequencies: order.wavelengths.clone(),
                amplitudes: order.flux.clone(),
                frequency_unit: Some(order.wavelength_unit.clone()),
                amplitude_unit: Some("counts".to_string()),
                metadata: Some(metadata),
                timestamp,
            });
        }

        if let Some(merged) = &self.merged {
            let mut metadata = json!({
                "kind": "echelle_merged",
                "profile_id": self.profile_id,
                "profile_display_name": self.profile_display_name,
                "frame_number": self.frame_number,
                "preview": preview
            });
            if let Some(device_id) = device_id {
                insert_device_id_metadata(&mut metadata, device_id);
            }

            measurements.push(Measurement::Spectrum {
                name: if preview {
                    "echelle_merged_preview".to_string()
                } else {
                    "echelle_merged".to_string()
                },
                frequencies: merged.wavelengths.clone(),
                amplitudes: merged.flux.clone(),
                frequency_unit: Some(merged.wavelength_unit.clone()),
                amplitude_unit: Some("counts".to_string()),
                metadata: Some(metadata),
                timestamp,
            });
        }

        measurements
    }
}

fn insert_device_id_metadata(metadata: &mut serde_json::Value, device_id: &str) {
    if let Some(object) = metadata.as_object_mut() {
        object.insert("device_id".to_string(), device_id.into());
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedIntensityFrame<'a> {
    width: u32,
    height: u32,
    pixels: DecodedPixels<'a>,
}

#[derive(Debug, Clone, PartialEq)]
enum DecodedPixels<'a> {
    U8(&'a [u8]),
    U16Borrowed(&'a [u16]),
    U16Owned(Vec<u16>),
}

impl<'a> DecodedIntensityFrame<'a> {
    pub fn decode(
        frame_data: &'a [u8],
        width: u32,
        height: u32,
        bit_depth: u32,
        u16_scratch: Option<&'a mut Vec<u16>>,
    ) -> Result<Self, String> {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let pixels = match bit_depth {
            8 => {
                if frame_data.len() < pixel_count {
                    return Err(format!(
                        "8-bit frame buffer too small: expected at least {} bytes, got {}",
                        pixel_count,
                        frame_data.len()
                    ));
                }
                DecodedPixels::U8(&frame_data[..pixel_count])
            }
            12 | 16 => {
                let expected_bytes = pixel_count.saturating_mul(2);
                if frame_data.len() < expected_bytes {
                    return Err(format!(
                        "{}-bit frame buffer too small: expected at least {} bytes, got {}",
                        bit_depth,
                        expected_bytes,
                        frame_data.len()
                    ));
                }
                #[allow(unsafe_code)]
                // The zero-copy fast path is intentional; misaligned buffers fall back to owned decoding below.
                // SAFETY: `align_to::<u16>()` only creates a borrowed view when the byte slice is
                // properly aligned and complete; otherwise we detect the misalignment/trailing bytes
                // via `prefix`/`suffix` and fall back to an owned little-endian decode below.
                let (prefix, u16s, suffix) = unsafe { frame_data.align_to::<u16>() };
                if prefix.is_empty() && suffix.is_empty() && u16s.len() >= pixel_count {
                    DecodedPixels::U16Borrowed(&u16s[..pixel_count])
                } else if let Some(scratch) = u16_scratch {
                    scratch.clear();
                    scratch.reserve(pixel_count);
                    for chunk in frame_data[..expected_bytes].chunks_exact(2) {
                        scratch.push(u16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                    DecodedPixels::U16Borrowed(&scratch[..pixel_count])
                } else {
                    let mut decoded = Vec::with_capacity(pixel_count);
                    for chunk in frame_data[..expected_bytes].chunks_exact(2) {
                        decoded.push(u16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                    DecodedPixels::U16Owned(decoded)
                }
            }
            _ => {
                return Err(format!(
                    "unsupported bit depth {} for echelle extraction (expected 8/12/16)",
                    bit_depth
                ));
            }
        };

        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = (y * self.width + x) as usize;
        match &self.pixels {
            DecodedPixels::U8(pixels) => pixels.get(idx).copied().map(u32::from),
            DecodedPixels::U16Borrowed(pixels) => pixels.get(idx).copied().map(u32::from),
            DecodedPixels::U16Owned(pixels) => pixels.get(idx).copied().map(u32::from),
        }
    }

    /// Convert the full frame to a row-major `f32` array.
    ///
    /// Used by the scattered-light fast path which needs a contiguous f32
    /// buffer for inter-order pixel sampling.
    fn to_f32_vec(&self) -> Vec<f32> {
        let n = self.width as usize * self.height as usize;
        match &self.pixels {
            DecodedPixels::U8(px) => px.iter().map(|&v| f32::from(v)).collect(),
            DecodedPixels::U16Borrowed(px) => px[..n].iter().map(|&v| f32::from(v)).collect(),
            DecodedPixels::U16Owned(px) => px[..n].iter().map(|&v| f32::from(v)).collect(),
        }
    }
}

pub fn extract_preview(
    profile: &EchelleCalibrationProfile,
    frame_data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
) -> Result<EchelleExtractionPreview, String> {
    extract_preview_masked(
        profile,
        frame_data,
        width,
        height,
        bit_depth,
        frame_number,
        None,
    )
}

pub fn extract_preview_masked(
    profile: &EchelleCalibrationProfile,
    frame_data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
    mask: Option<&BadPixelMask>,
) -> Result<EchelleExtractionPreview, String> {
    let compat = profile.check_frame_compatibility(&EchelleFrameContext {
        width,
        height,
        bit_depth: Some(bit_depth),
        ..Default::default()
    });
    if !compat.is_usable() {
        let msgs: Vec<String> = compat
            .diagnostics()
            .iter()
            .map(ToString::to_string)
            .collect();
        return Err(msgs.join("; "));
    }

    let decoded = DecodedIntensityFrame::decode(frame_data, width, height, bit_depth, None)?;
    extract_preview_with_scratch_inner(profile, &decoded, bit_depth, frame_number, mask)
}

pub fn extract_preview_with_u16_scratch(
    profile: &EchelleCalibrationProfile,
    frame_data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
    u16_scratch: &mut Vec<u16>,
) -> Result<EchelleExtractionPreview, String> {
    extract_preview_with_u16_scratch_masked(
        profile,
        frame_data,
        width,
        height,
        bit_depth,
        frame_number,
        u16_scratch,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn extract_preview_with_u16_scratch_masked(
    profile: &EchelleCalibrationProfile,
    frame_data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
    u16_scratch: &mut Vec<u16>,
    mask: Option<&BadPixelMask>,
) -> Result<EchelleExtractionPreview, String> {
    let compat = profile.check_frame_compatibility(&EchelleFrameContext {
        width,
        height,
        bit_depth: Some(bit_depth),
        ..Default::default()
    });
    if !compat.is_usable() {
        let msgs: Vec<String> = compat
            .diagnostics()
            .iter()
            .map(ToString::to_string)
            .collect();
        return Err(msgs.join("; "));
    }

    let decoded =
        DecodedIntensityFrame::decode(frame_data, width, height, bit_depth, Some(u16_scratch))?;
    extract_preview_with_scratch_inner(profile, &decoded, bit_depth, frame_number, mask)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)] // Overlay coordinates intentionally quantize calibrated f64 positions into UI-friendly f32 pixels.
pub fn order_sample_image_position(
    profile: &EchelleCalibrationProfile,
    order: &EchelleOrderCalibration,
    sample_index: usize,
) -> Option<(f32, f32)> {
    #[allow(clippy::cast_possible_truncation)]
    // Sample indices are bounded by the validated order range.
    let sample_local = order.sample_start.checked_add(sample_index as u32)?;
    if sample_local > order.sample_end {
        return None;
    }

    let roi_disp_offset = match profile.orientation.dispersion_axis {
        DetectorAxis::X => f64::from(profile.compatibility.roi_x),
        DetectorAxis::Y => f64::from(profile.compatibility.roi_y),
    };
    let roi_cross_offset = match profile.orientation.cross_dispersion_axis {
        DetectorAxis::X => f64::from(profile.compatibility.roi_x),
        DetectorAxis::Y => f64::from(profile.compatibility.roi_y),
    };
    let sample_sensor = f64::from(sample_local) + roi_disp_offset;
    let center_sensor = eval_trace(&order.trace, sample_sensor).ok()?;
    let center_local = center_sensor - roi_cross_offset;

    Some(match profile.orientation.dispersion_axis {
        DetectorAxis::X => (sample_local as f32 + 0.5, center_local as f32 + 0.5),
        DetectorAxis::Y => (center_local as f32 + 0.5, sample_local as f32 + 0.5),
    })
}

fn extract_preview_with_scratch_inner(
    profile: &EchelleCalibrationProfile,
    decoded: &DecodedIntensityFrame<'_>,
    bit_depth: u32,
    frame_number: u64,
    mask: Option<&BadPixelMask>,
) -> Result<EchelleExtractionPreview, String> {
    let saturation_threshold = bit_depth_max(bit_depth);
    let blaze_curves = profile.corrections.blaze_curves.as_ref();

    // Scattered light: compute a full-resolution correction map if enabled.
    let scatter_map: Option<Vec<f32>> =
        profile
            .extraction
            .scattered_light
            .as_ref()
            .map(|sl_config| {
                let f32_frame = decoded.to_f32_vec();
                // NOTE: The scattered-light estimator evaluates trace models
                // using frame-local pixel coordinates. When ROI offset is
                // non-zero, trace polynomials (defined in sensor coordinates)
                // would be evaluated at the wrong position. For now this is
                // correct because the iStar forces full-frame AOI (roi=0).
                // A proper fix for non-zero ROI would pass offsets into the
                // estimator to map frame-local → sensor coordinates.
                let traces: Vec<TraceInfo<'_>> = profile
                    .orders
                    .iter()
                    .filter(|o| o.enabled)
                    .map(|o| TraceInfo {
                        trace: &o.trace,
                        disp_start: o.sample_start,
                        disp_end: o.sample_end,
                    })
                    .collect();
                estimate_scatter_map_fast_f32(
                    &f32_frame,
                    decoded.width(),
                    decoded.height(),
                    &traces,
                    profile.extraction.default_aperture_half_width_px,
                    sl_config.col_stride,
                    sl_config.row_stride,
                )
            });
    let scatter_ref = scatter_map.as_deref();

    // Collect (position_in_orders_vec, order) pairs so blaze_curves is
    // indexed by position, not by relative_index (which may not be contiguous).
    let enabled_orders: Vec<(usize, &EchelleOrderCalibration)> = profile
        .orders
        .iter()
        .enumerate()
        .filter(|(_, order)| order.enabled)
        .collect();

    #[cfg(not(target_arch = "wasm32"))]
    let orders: Vec<EchelleOrderPreview> = {
        use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
        enabled_orders
            .par_iter()
            .map(|&(pos, order)| {
                let blaze = blaze_curves.and_then(|c| c.get(pos)).map(Vec::as_slice);
                extract_order(
                    profile,
                    order,
                    decoded,
                    saturation_threshold,
                    mask,
                    blaze,
                    scatter_ref,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    #[cfg(target_arch = "wasm32")]
    let orders: Vec<EchelleOrderPreview> = enabled_orders
        .iter()
        .map(|&(pos, order)| {
            let blaze = blaze_curves.and_then(|c| c.get(pos)).map(Vec::as_slice);
            extract_order(
                profile,
                order,
                decoded,
                saturation_threshold,
                mask,
                blaze,
                scatter_ref,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(EchelleExtractionPreview {
        frame_number,
        profile_display_name: profile.display_name.clone(),
        profile_id: profile.profile_id.clone(),
        merged: build_merged_preview(&orders, profile.corrections.blaze_curves.is_some()),
        orders,
    })
}

#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)] // Extraction windows intentionally use signed pixel offsets around a local centerline.
fn extract_order(
    profile: &EchelleCalibrationProfile,
    order: &EchelleOrderCalibration,
    frame: &DecodedIntensityFrame<'_>,
    saturation_threshold: u32,
    mask: Option<&BadPixelMask>,
    blaze: Option<&[f64]>,
    scatter_map: Option<&[f32]>,
) -> Result<EchelleOrderPreview, String> {
    let half_width = order
        .aperture_half_width_px
        .unwrap_or(profile.extraction.default_aperture_half_width_px);
    #[allow(clippy::cast_possible_truncation)]
    // The configured aperture radius is rounded to an integer pixel span.
    let radius = half_width.max(0.0).floor() as i32;
    #[allow(clippy::cast_sign_loss)] // `planned_span` is derived from a non-negative radius.
    let planned_span = (radius * 2 + 1).max(1) as u32;

    let mut wavelengths = Vec::new();
    let mut flux = Vec::new();
    let mut valid_fraction = Vec::new();
    let mut saturated = Vec::new();
    let mut covered_samples = 0u32;
    let mut saturated_samples = 0u32;

    let roi_disp_offset = match profile.orientation.dispersion_axis {
        DetectorAxis::X => f64::from(profile.compatibility.roi_x),
        DetectorAxis::Y => f64::from(profile.compatibility.roi_y),
    };
    let roi_cross_offset = match profile.orientation.cross_dispersion_axis {
        DetectorAxis::X => f64::from(profile.compatibility.roi_x),
        DetectorAxis::Y => f64::from(profile.compatibility.roi_y),
    };

    for sample_local in order.sample_start..=order.sample_end {
        let sample_sensor = f64::from(sample_local) + roi_disp_offset;
        let center_sensor = eval_trace(&order.trace, sample_sensor).map_err(|error| {
            format!(
                "order {} trace evaluation failed at sample {}: {}",
                order.relative_index, sample_local, error
            )
        })?;
        let center_local = center_sensor - roi_cross_offset;

        let wavelength = eval_wavelength(
            &order.wavelength,
            sample_sensor,
            (sample_local - order.sample_start) as usize,
        )
        .map_err(|error| {
            format!(
                "order {} wavelength evaluation failed at sample {}: {}",
                order.relative_index, sample_local, error
            )
        })?;
        wavelengths.push(wavelength.0);

        #[allow(clippy::cast_possible_truncation)]
        // Trace evaluation is converted back onto the detector's integer pixel grid.
        let center_px = center_local.round() as i32;
        let mut sample_sum = 0.0f64;
        let mut valid = 0u32;
        let mut saturated_sample = false;

        match profile.extraction.summation_mode {
            EchelleSummationMode::OrderCenterPixel => {
                if let Some((x, y)) = map_disp_cross_to_local_xy(
                    profile.orientation.dispersion_axis,
                    sample_local as i32,
                    center_px,
                ) {
                    if x >= 0
                        && y >= 0
                        && (x as u32) < frame.width()
                        && (y as u32) < frame.height()
                        && !is_excluded(profile, mask, x as u32, y as u32)
                    {
                        if let Some(pixel) = frame.get(x as u32, y as u32) {
                            let signal = f64::from(pixel)
                                - scatter_at(scatter_map, x as u32, y as u32, frame.width());
                            sample_sum = signal.max(0.0);
                            valid = 1;
                            saturated_sample = pixel >= saturation_threshold;
                        }
                    }
                }
            }
            // Optimal extraction requires a rectified order and spatial profile
            // (see optimal_extraction.rs). The preview path operates on raw frames,
            // so Optimal falls back to SimpleSum here. Full optimal extraction is
            // available via the rectification → optimal_extract pipeline.
            EchelleSummationMode::SimpleSum | EchelleSummationMode::Optimal => {
                for offset in -radius..=radius {
                    let cross_px = center_px + offset;
                    if let Some((x, y)) = map_disp_cross_to_local_xy(
                        profile.orientation.dispersion_axis,
                        sample_local as i32,
                        cross_px,
                    ) {
                        if x < 0
                            || y < 0
                            || (x as u32) >= frame.width()
                            || (y as u32) >= frame.height()
                            || is_excluded(profile, mask, x as u32, y as u32)
                        {
                            continue;
                        }
                        if let Some(pixel) = frame.get(x as u32, y as u32) {
                            let signal = f64::from(pixel)
                                - scatter_at(scatter_map, x as u32, y as u32, frame.width());
                            sample_sum += signal.max(0.0);
                            valid += 1;
                            saturated_sample |= pixel >= saturation_threshold;
                        }
                    }
                }
            }
            EchelleSummationMode::SqrtWeightedSum => {
                // Inverse-sqrt-variance weighting: weight_i = 1 / sqrt(V_i)
                // where V_i = readnoise² + max(pixel_i, 0) / gain.
                // This downweights noisy edge pixels without requiring a
                // spatial profile fit (intermediate between SimpleSum and Optimal).
                //
                // Uses conservative defaults when detector model is not configured.
                const DEFAULT_READ_NOISE: f64 = 3.0; // electrons
                const DEFAULT_GAIN: f64 = 1.0; // electrons/ADU

                let mut weight_sum = 0.0f64;
                for offset in -radius..=radius {
                    let cross_px = center_px + offset;
                    if let Some((x, y)) = map_disp_cross_to_local_xy(
                        profile.orientation.dispersion_axis,
                        sample_local as i32,
                        cross_px,
                    ) {
                        if x < 0
                            || y < 0
                            || (x as u32) >= frame.width()
                            || (y as u32) >= frame.height()
                            || is_excluded(profile, mask, x as u32, y as u32)
                        {
                            continue;
                        }
                        if let Some(pixel) = frame.get(x as u32, y as u32) {
                            let signal = (f64::from(pixel)
                                - scatter_at(scatter_map, x as u32, y as u32, frame.width()))
                            .max(0.0);
                            // Variance: readnoise² + Poisson term (signal/gain)
                            let variance =
                                DEFAULT_READ_NOISE * DEFAULT_READ_NOISE + signal / DEFAULT_GAIN;
                            let weight = 1.0 / variance.sqrt();
                            sample_sum += signal * weight;
                            weight_sum += weight;
                            valid += 1;
                            saturated_sample |= pixel >= saturation_threshold;
                        }
                    }
                }
                // Normalize by total weight to produce a weighted mean × aperture
                if weight_sum > 0.0 {
                    sample_sum /= weight_sum;
                    sample_sum *= f64::from(valid);
                }
            }
        }

        // Background subtraction applies to all aperture-based modes
        // (SimpleSum, SqrtWeightedSum, Optimal) but not OrderCenterPixel.
        if valid > 0
            && !matches!(
                profile.extraction.summation_mode,
                EchelleSummationMode::OrderCenterPixel
            )
        {
            if let Some(background) = &profile.extraction.background {
                if background.enabled {
                    #[allow(clippy::cast_possible_wrap)]
                    let (background_sum, background_count) = sample_background_sidebands(
                        profile,
                        mask,
                        frame,
                        sample_local as i32,
                        center_px,
                        radius,
                        background.inter_order_gap_min_px as i32,
                        background.baseline_window_px as i32,
                    );
                    if background_count > 0 {
                        let background_mean = background_sum / f64::from(background_count);
                        sample_sum -= background_mean * f64::from(valid);
                        sample_sum = sample_sum.max(0.0); // Clamp to non-negative
                    }
                }
            }
        }

        // Blaze correction: divide by normalised flat-lamp blaze function.
        // Applied after background subtraction so both additive (sky) and
        // multiplicative (blaze) corrections compose correctly.
        if let Some(blaze_curve) = blaze {
            let sample_idx = (sample_local - order.sample_start) as usize;
            if let Some(&bval) = blaze_curve.get(sample_idx) {
                let bval = bval.max(BLAZE_FLOOR);
                sample_sum /= bval;
            }
        }

        if valid > 0 {
            covered_samples += 1;
        }
        if saturated_sample {
            saturated_samples += 1;
        }
        flux.push(sample_sum);
        #[allow(clippy::cast_precision_loss)]
        // Valid fraction is display-oriented quality metadata, not a numerically sensitive science output.
        valid_fraction.push(valid as f32 / planned_span as f32);
        saturated.push(saturated_sample);
    }

    Ok(EchelleOrderPreview {
        relative_index: order.relative_index,
        physical_order_number: order.physical_order_number,
        wavelength_unit: wavelength_unit(&order.wavelength).to_string(),
        wavelengths,
        flux,
        valid_fraction,
        saturated,
        total_samples: order.sample_end - order.sample_start + 1,
        covered_samples,
        saturated_samples,
    })
}

#[allow(clippy::cast_sign_loss, clippy::too_many_arguments)] // Sideband window math walks signed detector offsets but only returns non-negative coverage counts.
fn sample_background_sidebands(
    profile: &EchelleCalibrationProfile,
    mask: Option<&BadPixelMask>,
    frame: &DecodedIntensityFrame<'_>,
    sample_local: i32,
    center_px: i32,
    radius: i32,
    inter_order_gap: i32,
    baseline_window: i32,
) -> (f64, u32) {
    if baseline_window <= 0 {
        return (0.0, 0);
    }

    let mut background_sum = 0.0f64;
    let mut background_count = 0u32;
    let start = radius + inter_order_gap.max(0);
    let end = start + baseline_window - 1;

    for sign in [-1, 1] {
        for offset in start..=end {
            let cross_px = center_px + sign * offset;
            if let Some((x, y)) = map_disp_cross_to_local_xy(
                profile.orientation.dispersion_axis,
                sample_local,
                cross_px,
            ) {
                if x < 0
                    || y < 0
                    || (x as u32) >= frame.width()
                    || (y as u32) >= frame.height()
                    || is_excluded(profile, mask, x as u32, y as u32)
                {
                    continue;
                }
                if let Some(pixel) = frame.get(x as u32, y as u32) {
                    background_sum += f64::from(pixel);
                    background_count += 1;
                }
            }
        }
    }

    (background_sum, background_count)
}

/// Default merge bin width for `build_merged_preview` (nm). Finer than the
/// legacy 0.1 nm grid so Mechelle-class resolving power is not over-smoothed
/// in the GUI merged preview (bd-srh1).
pub const ECHELLE_MERGE_BIN_WIDTH_NM: f64 = 0.05;

/// Echelle grating blaze envelope: sinc²(π·(λ − λ_blaze) / FSR), matching the
/// model used in [`crate::simulation`] and typical spectrograph texts.
#[inline]
fn echelle_blaze_sinc_squared(lambda_nm: f64, lambda_blaze_nm: f64, fsr_nm: f64) -> f64 {
    if !(lambda_nm.is_finite() && lambda_blaze_nm.is_finite() && fsr_nm.is_finite())
        || fsr_nm <= 0.0
    {
        return 1.0;
    }
    let arg = std::f64::consts::PI * (lambda_nm - lambda_blaze_nm) / fsr_nm;
    if arg.abs() < 1e-12 {
        return 1.0;
    }
    let sinc = arg.sin() / arg;
    (sinc * sinc).clamp(1e-12, 1.0)
}

/// Per-order grating constant estimate: median of `m · λ` over samples with
/// `λ ≥ 200` nm (same cut as `build_merged_preview`). Used only for analytic blaze weights.
fn infer_order_grating_constant_nm(order: &EchelleOrderPreview, m: i32) -> Option<f64> {
    let mf = f64::from(m);
    let mut products: Vec<f64> = order
        .wavelengths
        .iter()
        .copied()
        .filter(|&wl| wl.is_finite() && wl >= 200.0)
        .map(|wl| mf * wl)
        .collect();
    if products.is_empty() {
        return None;
    }
    products.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(products[products.len() / 2])
}

/// Merge per-order 1D previews into one wavelength-sorted spectrum.
///
/// When `flux_is_blaze_corrected` is false (no empirical `blaze_curves` applied
/// in [`extract_order`]), overlap regions are combined with **blaze weights**
/// `W ∝ sinc²((λ−λ_blaze)/FSR)` using per-order `λ_blaze = (m·λ)_median / m` and
/// `FSR = (m·λ)_median / m²`. That replaces the old fixed 10% edge trim (bd-srh1).
///
/// When empirical flat-field blaze curves were applied during extraction, flux is
/// already flattened; merge uses uniform weights so we do not double-apply the
/// grating envelope (bd-dgea).
fn build_merged_preview(
    orders: &[EchelleOrderPreview],
    flux_is_blaze_corrected: bool,
) -> Option<EchelleMergedPreview> {
    let first_unit = orders.first()?.wavelength_unit.clone();
    if orders
        .iter()
        .any(|order| order.wavelength_unit != first_unit)
    {
        return None;
    }

    let mut weighted: Vec<(f64, f64, f64)> = Vec::new();
    for order in orders {
        let m = match order.physical_order_number {
            Some(m) if (40..=120).contains(&m) => m,
            _ => continue,
        };
        let n = order.wavelengths.len();
        if n < 2 {
            continue;
        }
        let Some(gc_o) = infer_order_grating_constant_nm(order, m) else {
            continue;
        };
        let mf = f64::from(m);
        let lambda_blaze = gc_o / mf;
        let fsr = gc_o / (mf * mf);

        for i in 0..n {
            let wl = order.wavelengths[i];
            let fl = order.flux[i];
            if !wl.is_finite() || wl < 200.0 || !fl.is_finite() {
                continue;
            }
            let w = if flux_is_blaze_corrected {
                1.0
            } else {
                echelle_blaze_sinc_squared(wl, lambda_blaze, fsr)
            };
            weighted.push((wl, fl, w));
        }
    }

    if weighted.is_empty() {
        return None;
    }

    weighted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let bin_width = ECHELLE_MERGE_BIN_WIDTH_NM;
    let mut binned_wl = Vec::with_capacity(weighted.len());
    let mut binned_flux = Vec::with_capacity(weighted.len());
    let mut bin_start = weighted[0].0;
    let mut bin_w_sum = 0.0;
    let mut bin_fw_sum = 0.0;

    for &(wl, fl, w) in &weighted {
        if wl - bin_start < bin_width {
            bin_w_sum += w;
            bin_fw_sum += fl * w;
        } else {
            if bin_w_sum > 0.0 {
                binned_wl.push(bin_start + bin_width * 0.5);
                binned_flux.push(bin_fw_sum / bin_w_sum);
            }
            bin_start = wl;
            bin_w_sum = w;
            bin_fw_sum = fl * w;
        }
    }
    if bin_w_sum > 0.0 {
        binned_wl.push(bin_start + bin_width * 0.5);
        binned_flux.push(bin_fw_sum / bin_w_sum);
    }

    Some(EchelleMergedPreview {
        wavelength_unit: first_unit,
        wavelengths: binned_wl,
        flux: binned_flux,
    })
}

#[inline]
fn bit_depth_max(bit_depth: u32) -> u32 {
    match bit_depth {
        8 => 255,
        12 => 4095,
        16 => 65535,
        _ => u32::MAX,
    }
}

/// Look up the scatter correction value at pixel (x, y).
/// Returns 0.0 if no scatter map is provided or coordinates are out of range.
#[inline]
fn scatter_at(scatter_map: Option<&[f32]>, x: u32, y: u32, width: u32) -> f64 {
    scatter_map
        .and_then(|m| m.get((y * width + x) as usize).copied())
        .map_or(0.0, f64::from)
}

#[inline]
fn map_disp_cross_to_local_xy(
    dispersion_axis: DetectorAxis,
    dispersion_px: i32,
    cross_px: i32,
) -> Option<(i32, i32)> {
    match dispersion_axis {
        DetectorAxis::X => Some((dispersion_px, cross_px)),
        DetectorAxis::Y => Some((cross_px, dispersion_px)),
    }
}

fn is_excluded(
    profile: &EchelleCalibrationProfile,
    mask: Option<&BadPixelMask>,
    x: u32,
    y: u32,
) -> bool {
    profile
        .corrections
        .excluded_regions
        .iter()
        .any(|region| point_in_region(x, y, region))
        || mask.is_some_and(|m| m.is_masked(x, y))
}

#[inline]
fn point_in_region(x: u32, y: u32, region: &PixelRegion) -> bool {
    x >= region.x
        && x < region.x.saturating_add(region.width)
        && y >= region.y
        && y < region.y.saturating_add(region.height)
}

fn wavelength_unit(wavelength: &EchelleWavelengthModel) -> &str {
    match wavelength {
        EchelleWavelengthModel::Polynomial { unit, .. } => unit,
        EchelleWavelengthModel::Sampled { unit, .. } => unit,
    }
}

fn eval_trace(trace: &EchelleTraceModel, x: f64) -> Result<f64, &'static str> {
    match trace {
        EchelleTraceModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
        } => eval_polynomial(*basis, coefficients, *domain_start, *domain_end, x),
    }
}

fn eval_wavelength(
    wavelength: &EchelleWavelengthModel,
    x: f64,
    sampled_index: usize,
) -> Result<(f64, &str), &'static str> {
    match wavelength {
        EchelleWavelengthModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
            unit,
        } => Ok((
            eval_polynomial(*basis, coefficients, *domain_start, *domain_end, x)?,
            unit.as_str(),
        )),
        EchelleWavelengthModel::Sampled { wavelengths, unit } => wavelengths
            .get(sampled_index)
            .copied()
            .map(|wavelength| (wavelength, unit.as_str()))
            .ok_or("sampled wavelength index out of bounds"),
    }
}

fn eval_polynomial(
    basis: PolynomialBasis,
    coefficients: &[f64],
    domain_start: f64,
    domain_end: f64,
    x: f64,
) -> Result<f64, &'static str> {
    if coefficients.is_empty() {
        return Err("empty coefficient list");
    }
    if !x.is_finite()
        || !domain_start.is_finite()
        || !domain_end.is_finite()
        || domain_start >= domain_end
    {
        return Err("invalid polynomial domain/input");
    }

    Ok(match basis {
        PolynomialBasis::Monomial => {
            let mut accumulator = 0.0f64;
            for &coefficient in coefficients.iter().rev() {
                accumulator = accumulator * x + coefficient;
            }
            accumulator
        }
        PolynomialBasis::Chebyshev => {
            let t = ((2.0 * (x - domain_start)) / (domain_end - domain_start)) - 1.0;
            if coefficients.len() == 1 {
                return Ok(coefficients[0]);
            }
            let mut t0 = 1.0f64;
            let mut t1 = t;
            let mut accumulator = coefficients[0] * t0 + coefficients[1] * t1;
            for &coefficient in coefficients.iter().skip(2) {
                let tn = 2.0 * t * t1 - t0;
                accumulator += coefficient * tn;
                t0 = t1;
                t1 = tn;
            }
            accumulator
        }
    })
}

// ─── Blaze correction ──────────────────────────────────────────────────────

/// Extract per-order blaze curves from a flat-lamp frame.
///
/// For each calibrated order the function:
/// 1. Extracts the flat spectrum using simple-sum mode (no background,
///    no blaze correction, no scattered-light subtraction).
/// 2. Normalises the per-order flat spectrum so that its peak value is 1.0.
/// 3. Clamps values below `BLAZE_FLOOR` to that floor to prevent
///    excessive amplification at order edges.
///
/// Note: this is a simple per-order normalisation; it does not perform
/// lamp SED removal via continuum modelling or median smoothing. Within
/// a single echelle order the lamp SED varies negligibly (~10 nm range).
///
/// Store the result in `profile.corrections.blaze_curves` so subsequent
/// calls to [`extract_preview`] will automatically divide science flux
/// by the blaze function.
pub fn extract_flat_blaze_curves(
    profile: &EchelleCalibrationProfile,
    flat_data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
) -> Result<Vec<Vec<f64>>, String> {
    // Build a throw-away profile copy that always uses SimpleSum without
    // background or blaze corrections – we want the raw flat shape.
    let mut flat_profile = profile.clone();
    flat_profile.corrections.blaze_curves = None;
    flat_profile.extraction.summation_mode = EchelleSummationMode::SimpleSum;
    flat_profile.extraction.background = None;
    flat_profile.extraction.scattered_light = None;

    let preview = extract_preview(&flat_profile, flat_data, width, height, bit_depth, 0)?;

    // Map relative_index → extracted flux for quick lookup.
    let flux_map: std::collections::HashMap<u32, &[f64]> = preview
        .orders
        .iter()
        .map(|o| (o.relative_index, o.flux.as_slice()))
        .collect();

    let mut curves = Vec::with_capacity(profile.orders.len());
    for order in &profile.orders {
        if let Some(flux) = flux_map.get(&order.relative_index) {
            curves.push(compute_blaze_from_flat(flux));
        } else {
            // Disabled or failed order – identity curve (no correction).
            let n = (order.sample_end.saturating_sub(order.sample_start) + 1) as usize;
            curves.push(vec![1.0; n]);
        }
    }
    Ok(curves)
}

/// Derive a normalised blaze curve from raw flat-lamp flux.
///
/// Within a single echelle order the lamp SED varies negligibly (~10 nm
/// range), so the flat spectrum is dominated by the blaze function.
/// Normalising to peak = 1.0 and clamping to [`BLAZE_FLOOR`] is the
/// standard per-order approach used by CERES, PypeIt and others.
fn compute_blaze_from_flat(flat_flux: &[f64]) -> Vec<f64> {
    let n = flat_flux.len();
    if n == 0 {
        return Vec::new();
    }

    let peak = flat_flux
        .iter()
        .copied()
        .filter(|x| x.is_finite() && *x > 0.0)
        .fold(0.0_f64, f64::max);

    if peak <= 0.0 {
        return vec![BLAZE_FLOOR; n];
    }

    flat_flux
        .iter()
        .map(|&f| {
            if f.is_finite() && f > 0.0 {
                (f / peak).max(BLAZE_FLOOR)
            } else {
                BLAZE_FLOOR
            }
        })
        .collect()
}

/// Normalise extracted flat-like 1D flux into a per-order blaze curve for
/// [`EchelleCorrections::blaze_curves`] (same rule as [`extract_flat_blaze_curves`]).
#[must_use]
pub fn blaze_curve_from_flat_flux(flat_flux: &[f64]) -> Vec<f64> {
    compute_blaze_from_flat(flat_flux)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn minimal_profile() -> EchelleCalibrationProfile {
        EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: Some("mechelle-demo".to_string()),
            display_name: "Mechelle Demo V1".to_string(),
            compatibility: EchelleFrameCompatibility {
                sensor_width: 2048,
                sensor_height: 2048,
                frame_width: 1024,
                frame_height: 512,
                roi_x: 128,
                roi_y: 256,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Negative,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: 4.0,
                background: Some(EchelleBackgroundConfig {
                    enabled: true,
                    inter_order_gap_min_px: 5,
                    baseline_window_px: 15,
                }),
                scattered_light: None,
            },
            orders: vec![
                EchelleOrderCalibration {
                    relative_index: 0,
                    physical_order_number: Some(45),
                    sample_start: 0,
                    sample_end: 15,
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![250.0, 0.0, 0.0005],
                        domain_start: 0.0,
                        domain_end: 1023.0,
                    },
                    wavelength: EchelleWavelengthModel::Sampled {
                        wavelengths: (0..16).map(|i| 400.0 + f64::from(i) * 0.02).collect(),
                        unit: "nm".to_string(),
                    },
                    aperture_half_width_px: Some(3.5),
                    enabled: true,
                    notes: None,
                },
                EchelleOrderCalibration {
                    relative_index: 1,
                    physical_order_number: Some(44),
                    sample_start: 10,
                    sample_end: 31,
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Chebyshev,
                        coefficients: vec![300.0, 0.1, -0.02],
                        domain_start: 0.0,
                        domain_end: 1023.0,
                    },
                    wavelength: EchelleWavelengthModel::Polynomial {
                        basis: PolynomialBasis::Chebyshev,
                        coefficients: vec![450.0, 0.03, -1e-6],
                        domain_start: 0.0,
                        domain_end: 1023.0,
                        unit: "nm".to_string(),
                    },
                    aperture_half_width_px: None,
                    enabled: true,
                    notes: Some("reference order".to_string()),
                },
            ],
            corrections: EchelleCorrections {
                blaze: Some(EchelleArtifactRef {
                    path: "calib/blaze_mechelle_demo.npz".to_string(),
                    sha256: Some("deadbeef".repeat(8)),
                    format: Some("npz".to_string()),
                }),
                flat_field: None,
                bad_pixel_mask: None,
                excluded_regions: vec![PixelRegion {
                    x: 10,
                    y: 10,
                    width: 3,
                    height: 4,
                }],
                blaze_curves: None,
            },
            provenance: EchelleProvenance {
                creator_tool: "rust-daq-importer".to_string(),
                creator_version: Some("0.1.0".to_string()),
                created_at_utc: Utc::now(),
                source_frame_ids: vec!["flat_0001".to_string(), "arc_0007".to_string()],
                notes: Some("Generated from test fixture".to_string()),
            },
        }
    }

    #[test]
    fn test_profile_validation_accepts_minimal_valid_profile() {
        let profile = minimal_profile();
        profile.validate().expect("profile should validate");
    }

    #[test]
    fn test_profile_rejects_future_major_schema() {
        let mut profile = minimal_profile();
        profile.schema_version.major = 2;
        let err = profile.validate().unwrap_err();
        assert!(err.to_string().contains("unsupported schema version"));
    }

    #[test]
    fn test_profile_accepts_older_minor_patch_with_same_major() {
        let mut profile = minimal_profile();
        profile.schema_version = EchelleSchemaVersion {
            major: 1,
            minor: 0,
            patch: 0,
        };
        profile.validate().expect("same major should be accepted");
    }

    #[test]
    fn test_profile_rejects_roi_out_of_sensor_bounds() {
        let mut profile = minimal_profile();
        profile.compatibility.roi_x = 2000;
        let err = profile.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("ROI/frame extent exceeds sensor bounds"));
    }

    #[test]
    fn test_profile_rejects_sampled_wavelength_length_mismatch() {
        let mut profile = minimal_profile();
        if let EchelleWavelengthModel::Sampled { wavelengths, .. } =
            &mut profile.orders[0].wavelength
        {
            wavelengths.pop();
        }
        let err = profile.validate().unwrap_err();
        assert!(err.to_string().contains("sampled wavelength length"));
    }

    #[test]
    fn test_validate_for_frame_detects_roi_and_binning_mismatch() {
        let profile = minimal_profile();
        let err = profile
            .validate_for_frame(EchelleFrameContext {
                width: 1024,
                height: 512,
                roi_x: Some(0),
                roi_y: Some(256),
                binning_x: Some(2),
                binning_y: Some(1),
                bit_depth: Some(16),
            })
            .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("ROI X mismatch") || text.contains("binning_x mismatch"));
    }

    #[test]
    fn test_toml_roundtrip_save_load() {
        let profile = minimal_profile();
        let dir = tempdir().unwrap();
        let path = dir.path().join("echelle_profile.toml");
        profile.save_to_path(&path).unwrap();
        let loaded = EchelleCalibrationProfile::load_from_path(&path).unwrap();
        assert_eq!(loaded.display_name, profile.display_name);
        assert_eq!(loaded.orders.len(), 2);
        assert_eq!(loaded.compatibility.frame_width, 1024);
    }

    #[test]
    fn test_json_roundtrip_save_load() {
        let profile = minimal_profile();
        let dir = tempdir().unwrap();
        let path = dir.path().join("echelle_profile.json");
        profile.save_to_path(&path).unwrap();
        let loaded = EchelleCalibrationProfile::load_from_path(&path).unwrap();
        assert_eq!(loaded.orders[0].relative_index, 0);
    }

    #[test]
    fn test_rejects_unknown_profile_extension() {
        let profile = minimal_profile();
        let dir = tempdir().unwrap();
        let path = dir.path().join("echelle_profile.yaml");
        let err = profile.save_to_path(&path).unwrap_err();
        assert!(err
            .to_string()
            .contains("unsupported echelle profile extension"));
    }

    #[test]
    fn test_extract_preview_emits_device_scoped_measurements() {
        let profile = minimal_profile();
        let bytes_per_pixel = 2usize;
        let frame_bytes = (profile.compatibility.frame_width as usize)
            * (profile.compatibility.frame_height as usize)
            * bytes_per_pixel;
        let frame = vec![0u8; frame_bytes];

        let preview = extract_preview(
            &profile,
            &frame,
            profile.compatibility.frame_width,
            profile.compatibility.frame_height,
            profile.compatibility.bit_depth.unwrap(),
            7,
        )
        .expect("minimal profile should extract on a matching blank frame");
        let measurements = preview.to_measurements_with_context(false, Some("istar_camera"));

        assert_eq!(measurements.len(), 3);
        let Measurement::Spectrum { name, metadata, .. } = &measurements[0] else {
            panic!("expected first extracted measurement to be a spectrum");
        };
        assert_eq!(name, "echelle_order_0");

        let metadata = metadata
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .expect("spectrum metadata object");
        assert_eq!(
            metadata
                .get("device_id")
                .and_then(serde_json::Value::as_str),
            Some("istar_camera")
        );
        assert_eq!(
            metadata.get("preview").and_then(serde_json::Value::as_bool),
            Some(false)
        );

        let Measurement::Spectrum { name, .. } = &measurements[2] else {
            panic!("expected merged extracted measurement to be a spectrum");
        };
        assert_eq!(name, "echelle_merged");
    }

    #[test]
    fn test_bad_pixel_mask_construction_and_query() {
        let mask =
            BadPixelMask::from_raw_bytes(4, 3, &[0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]).unwrap();

        assert_eq!(mask.width(), 4);
        assert_eq!(mask.height(), 3);
        assert_eq!(mask.bad_count(), 2);
        assert!(!mask.is_masked(0, 0));
        assert!(mask.is_masked(1, 0));
        assert!(mask.is_masked(3, 1));
        assert!(!mask.is_masked(2, 2));
        // Out-of-bounds returns false (not masked).
        assert!(!mask.is_masked(10, 10));
    }

    #[test]
    fn test_bad_pixel_mask_rejects_wrong_size() {
        let err = BadPixelMask::from_raw_bytes(4, 3, &[0; 10]).unwrap_err();
        assert!(err.contains("does not match"));
    }

    #[test]
    fn test_extraction_excludes_masked_pixels() {
        // 16×5 frame, trace at y=2 (dispersion along X), aperture radius 1 → rows 1,2,3.
        // All signal pixels have value 100. Mask pixel (4, 2) as bad.
        let width = 16u32;
        let height = 5u32;
        let frame_width = width;
        let frame_height = height;
        let bytes_per_pixel = 2usize;
        let frame_bytes = (frame_width as usize) * (frame_height as usize) * bytes_per_pixel;
        let mut frame = vec![0u8; frame_bytes];

        // Fill rows 1..=3 with value 100 (little-endian u16).
        for x in 0..width {
            for y in 1..=3 {
                let idx = ((y * width + x) as usize) * 2;
                frame[idx] = 100;
                frame[idx + 1] = 0;
            }
        }

        let profile = EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: None,
            display_name: "mask-test".to_string(),
            compatibility: EchelleFrameCompatibility {
                sensor_width: width,
                sensor_height: height,
                frame_width,
                frame_height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Positive,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: 1.0,
                background: None,
                scattered_light: None,
            },
            orders: vec![EchelleOrderCalibration {
                relative_index: 0,
                physical_order_number: None,
                sample_start: 0,
                sample_end: width - 1,
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![2.0],
                    domain_start: 0.0,
                    domain_end: f64::from(width - 1),
                },
                wavelength: EchelleWavelengthModel::Sampled {
                    wavelengths: (0..width).map(|i| 500.0 + f64::from(i)).collect(),
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: None,
                enabled: true,
                notes: None,
            }],
            corrections: EchelleCorrections::default(),
            provenance: EchelleProvenance {
                creator_tool: "test".to_string(),
                creator_version: None,
                created_at_utc: Utc::now(),
                source_frame_ids: vec![],
                notes: None,
            },
        };

        // Extraction without mask: all samples get 3 pixels × 100 = 300.
        let no_mask = extract_preview(&profile, &frame, width, height, 16, 1).unwrap();
        assert_eq!(no_mask.orders[0].flux[4], 300.0);

        // Build mask that marks (4, 2) as bad.
        let mut mask_data = vec![0u8; (width * height) as usize];
        mask_data[(2 * width + 4) as usize] = 1; // (x=4, y=2) is bad
        let mask = BadPixelMask::from_raw_bytes(width, height, &mask_data).unwrap();

        // Extraction with mask: sample at x=4 loses center pixel → 2 × 100 = 200.
        let with_mask =
            extract_preview_masked(&profile, &frame, width, height, 16, 1, Some(&mask)).unwrap();
        assert_eq!(with_mask.orders[0].flux[4], 200.0);
        // Sample at x=3 is unaffected → still 300.
        assert_eq!(with_mask.orders[0].flux[3], 300.0);
        // Valid fraction at x=4 should be 2/3.
        let frac = with_mask.orders[0].valid_fraction[4];
        assert!(
            (frac - 2.0 / 3.0).abs() < 1e-5,
            "expected ~0.667, got {frac}"
        );
    }

    // -- FrameCompatibility tests (bd-qe8p.1.2) --

    #[test]
    fn compatible_when_all_fields_match() {
        let profile = minimal_profile();
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            roi_x: Some(128),
            roi_y: Some(256),
            binning_x: Some(1),
            binning_y: Some(1),
            bit_depth: Some(16),
        };
        let result = profile.check_frame_compatibility(&frame);
        assert_eq!(result, FrameCompatibility::Compatible);
        assert!(result.is_usable());
        assert!(result.diagnostics().is_empty());
    }

    #[test]
    fn compatible_when_optional_fields_omitted() {
        let profile = minimal_profile();
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            ..Default::default()
        };
        let result = profile.check_frame_compatibility(&frame);
        assert_eq!(result, FrameCompatibility::Compatible);
    }

    #[test]
    fn incompatible_on_frame_size_mismatch() {
        let profile = minimal_profile();
        let frame = EchelleFrameContext {
            width: 2048,
            height: 2048,
            ..Default::default()
        };
        let result = profile.check_frame_compatibility(&frame);
        assert!(!result.is_usable());
        assert!(matches!(result, FrameCompatibility::Incompatible { .. }));
        assert!(matches!(
            result.diagnostics()[0],
            CompatibilityWarning::FrameSizeMismatch { .. }
        ));
    }

    #[test]
    fn incompatible_on_binning_mismatch() {
        let profile = minimal_profile();
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            binning_x: Some(2),
            ..Default::default()
        };
        let result = profile.check_frame_compatibility(&frame);
        assert!(!result.is_usable());
        assert!(matches!(
            result.diagnostics()[0],
            CompatibilityWarning::BinningMismatch {
                axis: "X",
                expected: 1,
                actual: 2
            }
        ));
    }

    #[test]
    fn usable_with_warning_on_bit_depth_mismatch() {
        let profile = minimal_profile();
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            bit_depth: Some(12),
            ..Default::default()
        };
        let result = profile.check_frame_compatibility(&frame);
        assert!(result.is_usable());
        assert!(matches!(
            result,
            FrameCompatibility::UsableWithWarnings { .. }
        ));
        assert!(matches!(
            result.diagnostics()[0],
            CompatibilityWarning::BitDepthMismatch {
                expected: 16,
                actual: 12
            }
        ));
    }

    #[test]
    fn hard_mismatch_takes_precedence_over_soft() {
        let profile = minimal_profile();
        let frame = EchelleFrameContext {
            width: 512,
            height: 512,
            bit_depth: Some(12),
            ..Default::default()
        };
        let result = profile.check_frame_compatibility(&frame);
        // Frame size mismatch is hard → Incompatible, even though bit depth is soft.
        assert!(!result.is_usable());
        assert!(matches!(result, FrameCompatibility::Incompatible { .. }));
    }

    #[test]
    fn extract_preview_allows_bit_depth_mismatch() {
        // Extraction should proceed when only bit depth differs (usable with warnings).
        let profile = minimal_profile();
        let width = profile.compatibility.frame_width;
        let height = profile.compatibility.frame_height;
        let frame_data = vec![0u8; (width as usize) * (height as usize) * 2];
        // Profile expects 16-bit, we provide 12-bit — should still extract
        let result = extract_preview(&profile, &frame_data, width, height, 12, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn extract_preview_rejects_frame_size_mismatch() {
        let profile = minimal_profile();
        let frame_data = vec![0u8; 100 * 100 * 2];
        let result = extract_preview(&profile, &frame_data, 100, 100, 16, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("frame size mismatch"));
    }

    // ====================================================================
    // Task 1: bd-qe8p.2.2 — Compatibility regression suite
    // ====================================================================

    #[test]
    fn incompatible_on_roi_mismatch() {
        let profile = minimal_profile();
        // Profile has roi_x=128, roi_y=256. Provide different ROI values.
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            roi_x: Some(64),
            roi_y: Some(256),
            binning_x: Some(1),
            binning_y: Some(1),
            bit_depth: Some(16),
        };
        let result = profile.check_frame_compatibility(&frame);
        assert!(!result.is_usable());
        assert!(matches!(result, FrameCompatibility::Incompatible { .. }));
        assert!(matches!(
            result.diagnostics()[0],
            CompatibilityWarning::RoiMismatch {
                axis: "X",
                expected: 128,
                actual: 64,
            }
        ));
    }

    #[test]
    fn multiple_mismatches_all_reported() {
        let profile = minimal_profile();
        // Frame size AND binning differ simultaneously.
        let frame = EchelleFrameContext {
            width: 2048,
            height: 2048,
            binning_x: Some(2),
            binning_y: Some(2),
            ..Default::default()
        };
        let result = profile.check_frame_compatibility(&frame);
        assert!(!result.is_usable());
        let diagnostics = result.diagnostics();
        // Should report at least frame size + binning X + binning Y = 3 issues.
        assert!(
            diagnostics.len() >= 3,
            "expected at least 3 diagnostics, got {}",
            diagnostics.len()
        );
        let has_frame_size = diagnostics
            .iter()
            .any(|d| matches!(d, CompatibilityWarning::FrameSizeMismatch { .. }));
        let has_binning_x = diagnostics
            .iter()
            .any(|d| matches!(d, CompatibilityWarning::BinningMismatch { axis: "X", .. }));
        let has_binning_y = diagnostics
            .iter()
            .any(|d| matches!(d, CompatibilityWarning::BinningMismatch { axis: "Y", .. }));
        assert!(has_frame_size, "missing FrameSizeMismatch diagnostic");
        assert!(has_binning_x, "missing BinningMismatch X diagnostic");
        assert!(has_binning_y, "missing BinningMismatch Y diagnostic");
    }

    #[test]
    fn bit_depth_mismatch_with_matching_geometry_is_usable_with_warnings() {
        let profile = minimal_profile();
        // All geometry matches, only bit depth differs.
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            roi_x: Some(128),
            roi_y: Some(256),
            binning_x: Some(1),
            binning_y: Some(1),
            bit_depth: Some(12),
        };
        let result = profile.check_frame_compatibility(&frame);
        assert!(result.is_usable());
        assert!(matches!(
            result,
            FrameCompatibility::UsableWithWarnings { .. }
        ));
        assert_eq!(result.diagnostics().len(), 1);
        assert!(matches!(
            result.diagnostics()[0],
            CompatibilityWarning::BitDepthMismatch {
                expected: 16,
                actual: 12,
            }
        ));
    }

    #[test]
    fn omitted_optional_fields_are_compatible() {
        let profile = minimal_profile();
        // Only frame dimensions provided; roi, binning, bit_depth all None.
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            roi_x: None,
            roi_y: None,
            binning_x: None,
            binning_y: None,
            bit_depth: None,
        };
        let result = profile.check_frame_compatibility(&frame);
        assert_eq!(result, FrameCompatibility::Compatible);
        assert!(result.diagnostics().is_empty());
    }

    #[test]
    fn validate_for_frame_rejects_usable_with_warnings_in_strict_mode() {
        let profile = minimal_profile();
        // Geometry matches, but bit depth differs → UsableWithWarnings.
        // validate_for_frame is strict: it should reject this.
        let frame = EchelleFrameContext {
            width: 1024,
            height: 512,
            roi_x: Some(128),
            roi_y: Some(256),
            binning_x: Some(1),
            binning_y: Some(1),
            bit_depth: Some(12),
        };
        let err = profile.validate_for_frame(frame).unwrap_err();
        assert!(
            err.to_string().contains("bit depth mismatch"),
            "expected bit depth mismatch error, got: {}",
            err
        );
    }

    // ====================================================================
    // Task 2: bd-qe8p.2.3 — Extraction regression fixtures
    // ====================================================================

    /// Build a small profile with a single order on a compact frame
    /// suitable for extraction tests with controlled pixel values.
    fn small_extraction_profile(mode: EchelleSummationMode) -> EchelleCalibrationProfile {
        let width = 16u32;
        let height = 10u32;
        EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: Some("extraction-test".to_string()),
            display_name: "Extraction Test".to_string(),
            compatibility: EchelleFrameCompatibility {
                sensor_width: width,
                sensor_height: height,
                frame_width: width,
                frame_height: height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Positive,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: mode,
                default_aperture_half_width_px: 1.0,
                background: None,
                scattered_light: None,
            },
            orders: vec![EchelleOrderCalibration {
                relative_index: 0,
                physical_order_number: Some(42),
                sample_start: 0,
                sample_end: width - 1,
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![5.0],
                    domain_start: 0.0,
                    domain_end: f64::from(width - 1),
                },
                wavelength: EchelleWavelengthModel::Sampled {
                    wavelengths: (0..width).map(|i| 400.0 + f64::from(i) * 0.5).collect(),
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: None,
                enabled: true,
                notes: None,
            }],
            corrections: EchelleCorrections::default(),
            provenance: EchelleProvenance {
                creator_tool: "test".to_string(),
                creator_version: None,
                created_at_utc: Utc::now(),
                source_frame_ids: vec![],
                notes: None,
            },
        }
    }

    /// Build a 16-bit LE frame with non-uniform pixel values to distinguish
    /// SimpleSum from `SqrtWeightedSum` extraction.
    fn non_uniform_frame(width: u32, height: u32) -> Vec<u8> {
        let pixel_count = (width as usize) * (height as usize);
        let mut buf = vec![0u8; pixel_count * 2];
        // Fill rows 4..=6 (aperture around trace center y=5) with increasing values.
        for x in 0..width {
            for y in 4..=6u32 {
                // Center pixel (y=5) gets high value, edges get low values.
                let value: u16 = match y {
                    4 => 10,   // low edge
                    5 => 1000, // center
                    6 => 10,   // low edge
                    _ => 0,
                };
                let idx = ((y * width + x) as usize) * 2;
                let le = value.to_le_bytes();
                buf[idx] = le[0];
                buf[idx + 1] = le[1];
            }
        }
        buf
    }

    #[test]
    fn sqrt_weighted_sum_differs_from_simple_sum_on_non_uniform_data() {
        let width = 16u32;
        let height = 10u32;
        let frame = non_uniform_frame(width, height);

        let simple_profile = small_extraction_profile(EchelleSummationMode::SimpleSum);
        let sqrt_profile = small_extraction_profile(EchelleSummationMode::SqrtWeightedSum);

        let simple_preview =
            extract_preview(&simple_profile, &frame, width, height, 16, 0).unwrap();
        let sqrt_preview = extract_preview(&sqrt_profile, &frame, width, height, 16, 0).unwrap();

        // SimpleSum: 10 + 1000 + 10 = 1020 per sample.
        for val in &simple_preview.orders[0].flux {
            assert!(
                (*val - 1020.0).abs() < 1e-6,
                "SimpleSum flux should be 1020, got {}",
                val
            );
        }

        // SqrtWeightedSum uses inverse-sqrt-variance weighting, which produces
        // different values when pixel intensities are non-uniform.
        let sqrt_flux_0 = sqrt_preview.orders[0].flux[0];
        let simple_flux_0 = simple_preview.orders[0].flux[0];
        assert!(
            (sqrt_flux_0 - simple_flux_0).abs() > 1.0,
            "SqrtWeightedSum should produce different flux than SimpleSum \
             on non-uniform data: sqrt={}, simple={}",
            sqrt_flux_0,
            simple_flux_0
        );
    }

    #[test]
    fn extraction_with_bit_depth_mismatch_succeeds() {
        // Profile says 16-bit, but we provide 12-bit data.
        // check_frame_compatibility returns UsableWithWarnings, so extraction proceeds.
        let profile = small_extraction_profile(EchelleSummationMode::SimpleSum);
        let width = profile.compatibility.frame_width;
        let height = profile.compatibility.frame_height;
        let frame = vec![0u8; (width as usize) * (height as usize) * 2];

        let result = extract_preview(&profile, &frame, width, height, 12, 0);
        assert!(
            result.is_ok(),
            "extraction should succeed with bit depth mismatch: {:?}",
            result.err()
        );
    }

    #[test]
    fn extraction_with_frame_size_mismatch_returns_error() {
        let profile = small_extraction_profile(EchelleSummationMode::SimpleSum);
        // Profile expects 16x10, but we provide a 32x20 frame.
        let frame = vec![0u8; 32 * 20 * 2];
        let result = extract_preview(&profile, &frame, 32, 20, 16, 0);
        assert!(result.is_err());
        assert!(
            result.as_ref().unwrap_err().contains("frame size mismatch"),
            "expected frame size mismatch error, got: {}",
            result.unwrap_err()
        );
    }

    #[test]
    fn extraction_preserves_per_order_diagnostics() {
        let width = 16u32;
        let height = 10u32;
        let frame = non_uniform_frame(width, height);

        let profile = small_extraction_profile(EchelleSummationMode::SimpleSum);
        let preview = extract_preview(&profile, &frame, width, height, 16, 0).unwrap();

        let order = &preview.orders[0];
        // All 16 samples should be covered since trace center is y=5 with aperture radius 1.
        assert_eq!(order.total_samples, 16);
        assert_eq!(order.covered_samples, 16);
        // No pixel is at saturation (max u16 = 65535), so no saturated samples.
        assert_eq!(order.saturated_samples, 0);
        // valid_fraction should be 1.0 for each sample (all 3 pixels in aperture are valid).
        for &frac in &order.valid_fraction {
            assert!(
                (frac - 1.0).abs() < 1e-6,
                "expected valid_fraction=1.0, got {}",
                frac
            );
        }
    }

    #[test]
    fn merged_preview_sorts_wavelengths_across_orders() {
        // Build a profile with two orders whose wavelength ranges overlap/interleave.
        let width = 8u32;
        let height = 20u32;
        let profile = EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: None,
            display_name: "merge-sort-test".to_string(),
            compatibility: EchelleFrameCompatibility {
                sensor_width: width,
                sensor_height: height,
                frame_width: width,
                frame_height: height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Positive,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: 1.0,
                background: None,
                scattered_light: None,
            },
            orders: vec![
                EchelleOrderCalibration {
                    relative_index: 0,
                    physical_order_number: Some(50),
                    sample_start: 0,
                    sample_end: width - 1,
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![5.0],
                        domain_start: 0.0,
                        domain_end: f64::from(width - 1),
                    },
                    // Order 0 covers 500-507 nm
                    wavelength: EchelleWavelengthModel::Sampled {
                        wavelengths: (0..width).map(|i| 500.0 + f64::from(i)).collect(),
                        unit: "nm".to_string(),
                    },
                    aperture_half_width_px: None,
                    enabled: true,
                    notes: None,
                },
                EchelleOrderCalibration {
                    relative_index: 1,
                    physical_order_number: Some(49),
                    sample_start: 0,
                    sample_end: width - 1,
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![15.0],
                        domain_start: 0.0,
                        domain_end: f64::from(width - 1),
                    },
                    // Order 1 covers 400-407 nm (lower wavelengths than order 0)
                    wavelength: EchelleWavelengthModel::Sampled {
                        wavelengths: (0..width).map(|i| 400.0 + f64::from(i)).collect(),
                        unit: "nm".to_string(),
                    },
                    aperture_half_width_px: None,
                    enabled: true,
                    notes: None,
                },
            ],
            corrections: EchelleCorrections::default(),
            provenance: EchelleProvenance {
                creator_tool: "test".to_string(),
                creator_version: None,
                created_at_utc: Utc::now(),
                source_frame_ids: vec![],
                notes: None,
            },
        };

        let frame = vec![0u8; (width as usize) * (height as usize) * 2];
        let preview = extract_preview(&profile, &frame, width, height, 16, 0).unwrap();

        let merged = preview
            .merged
            .as_ref()
            .expect("merged preview should be present");
        // Merged wavelengths should be sorted in ascending order.
        for pair in merged.wavelengths.windows(2) {
            assert!(
                pair[0] <= pair[1],
                "merged wavelengths not sorted: {} > {}",
                pair[0],
                pair[1]
            );
        }
        // First wavelength should be from order 1 (400 nm range).
        assert!(
            merged.wavelengths[0] < 410.0,
            "first merged wavelength should be in 400nm range, got {}",
            merged.wavelengths[0]
        );
        // Last wavelength should be from order 0 (500 nm range).
        assert!(
            *merged.wavelengths.last().unwrap() > 490.0,
            "last merged wavelength should be in 500nm range, got {}",
            merged.wavelengths.last().unwrap()
        );
    }

    // ====================================================================
    // Task 3: bd-qe8p.2.1 — WASM activate-path regression (non-WASM)
    // ====================================================================

    #[test]
    fn large_multi_order_profile_roundtrip_and_extraction() {
        // Build a profile with 50+ orders programmatically.
        let width = 200u32;
        let height = 600u32;
        let num_orders = 55u32;

        let orders: Vec<EchelleOrderCalibration> = (0..num_orders)
            .map(|i| {
                // Space trace centers evenly across the cross-dispersion axis.
                let trace_center = 10.0 + f64::from(i) * 10.0;
                EchelleOrderCalibration {
                    relative_index: i,
                    #[allow(clippy::cast_possible_wrap)]
                    physical_order_number: Some(100 - i as i32),
                    sample_start: 0,
                    sample_end: width - 1,
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![trace_center],
                        domain_start: 0.0,
                        domain_end: f64::from(width - 1),
                    },
                    wavelength: EchelleWavelengthModel::Sampled {
                        wavelengths: (0..width)
                            .map(|s| {
                                // Each order covers a unique wavelength range.
                                let base = 300.0 + f64::from(i) * 10.0;
                                base + f64::from(s) * 0.05
                            })
                            .collect(),
                        unit: "nm".to_string(),
                    },
                    aperture_half_width_px: Some(2.0),
                    enabled: true,
                    notes: None,
                }
            })
            .collect();

        let profile = EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: Some("large-profile-test".to_string()),
            display_name: "Large Multi-Order Test".to_string(),
            compatibility: EchelleFrameCompatibility {
                sensor_width: width,
                sensor_height: height,
                frame_width: width,
                frame_height: height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Positive,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: 3.0,
                background: None,
                scattered_light: None,
            },
            orders,
            corrections: EchelleCorrections::default(),
            provenance: EchelleProvenance {
                creator_tool: "test".to_string(),
                creator_version: Some("1.0.0".to_string()),
                created_at_utc: Utc::now(),
                source_frame_ids: vec!["synth_flat_001".to_string()],
                notes: Some("Programmatically generated 55-order profile".to_string()),
            },
        };

        // Step 1: Validate the profile.
        profile
            .validate()
            .expect("large multi-order profile should validate");

        // Step 2: Serialize to TOML.
        let dir = tempdir().unwrap();
        let toml_path = dir.path().join("large_profile.toml");
        profile
            .save_to_path(&toml_path)
            .expect("should serialize to TOML");

        // Step 3: Deserialize from TOML and re-validate.
        let loaded =
            EchelleCalibrationProfile::load_from_path(&toml_path).expect("should deserialize");
        assert_eq!(loaded.orders.len(), num_orders as usize);
        assert_eq!(loaded.display_name, profile.display_name);
        assert_eq!(loaded.compatibility, profile.compatibility);
        // Verify order data survived the roundtrip.
        for (orig, loaded_order) in profile.orders.iter().zip(loaded.orders.iter()) {
            assert_eq!(orig.relative_index, loaded_order.relative_index);
            assert_eq!(
                orig.physical_order_number,
                loaded_order.physical_order_number
            );
        }

        // Step 4: Extract on a matching synthetic frame.
        let frame = vec![0u8; (width as usize) * (height as usize) * 2];
        let preview = extract_preview(&loaded, &frame, width, height, 16, 42)
            .expect("extraction on matching synthetic frame should succeed");
        assert_eq!(preview.orders.len(), num_orders as usize);
        assert_eq!(preview.frame_number, 42);
        assert!(
            preview.merged.is_some(),
            "merged preview should be present for multi-order extraction"
        );
        // Verify merged wavelengths are sorted.
        let merged = preview.merged.as_ref().unwrap();
        for pair in merged.wavelengths.windows(2) {
            assert!(
                pair[0] <= pair[1],
                "merged wavelengths not sorted: {} > {}",
                pair[0],
                pair[1]
            );
        }
    }

    // -- Wavelength model fidelity tests (bd-qe8p.1.6) --

    #[test]
    fn chebyshev_wavelength_model_roundtrip_preserves_full_precision() {
        // Create a profile with high-degree Chebyshev wavelength models
        // (degree 5) and verify all coefficients survive TOML serialization
        // at full f64 precision.
        #[allow(clippy::excessive_precision)] // test intentionally uses high-precision literals
        let chebyshev_coeffs = vec![
            450.123_456_789_012_3,
            0.098_765_432_109_876,
            -0.000_012_345_678_901,
            0.000_000_001_234_567,
            -1.234_567_890_123_456e-12,
            9.876_543_210_987_654e-16,
        ];
        let trace_coeffs = vec![
            256.789_012_345_678_9,
            -0.001_234_567_890_123,
            0.000_000_567_890_123,
        ];

        let mut profile = minimal_profile();
        profile.orders = vec![EchelleOrderCalibration {
            relative_index: 0,
            physical_order_number: Some(50),
            sample_start: 0,
            sample_end: 15,
            trace: EchelleTraceModel::Polynomial {
                basis: PolynomialBasis::Chebyshev,
                coefficients: trace_coeffs.clone(),
                domain_start: 0.0,
                domain_end: 1023.0,
            },
            wavelength: EchelleWavelengthModel::Polynomial {
                basis: PolynomialBasis::Chebyshev,
                coefficients: chebyshev_coeffs.clone(),
                domain_start: 0.0,
                domain_end: 1023.0,
                unit: "nm".to_string(),
            },
            aperture_half_width_px: Some(4.0),
            enabled: true,
            notes: None,
        }];

        // TOML roundtrip
        let dir = tempdir().unwrap();
        let toml_path = dir.path().join("chebyshev_profile.toml");
        profile.save_to_path(&toml_path).unwrap();
        let loaded_toml = EchelleCalibrationProfile::load_from_path(&toml_path).unwrap();

        // JSON roundtrip
        let json_path = dir.path().join("chebyshev_profile.json");
        profile.save_to_path(&json_path).unwrap();
        let loaded_json = EchelleCalibrationProfile::load_from_path(&json_path).unwrap();

        // Verify wavelength coefficients at full precision
        for (label, loaded) in [("TOML", &loaded_toml), ("JSON", &loaded_json)] {
            let EchelleWavelengthModel::Polynomial {
                basis,
                coefficients,
                ..
            } = &loaded.orders[0].wavelength
            else {
                panic!("{label}: expected Polynomial wavelength model");
            };
            assert_eq!(*basis, PolynomialBasis::Chebyshev, "{label}: basis");
            assert_eq!(
                coefficients.len(),
                chebyshev_coeffs.len(),
                "{label}: coefficient count"
            );
            // Bit-for-bit f64 preservation — serde serializes full precision.
            for (i, (expected, actual)) in
                chebyshev_coeffs.iter().zip(coefficients.iter()).enumerate()
            {
                assert_eq!(
                    *expected, *actual,
                    "{label}: wavelength coeff[{i}] not bit-exact"
                );
            }

            // Verify trace coefficients too
            let EchelleTraceModel::Polynomial {
                basis: tbasis,
                coefficients: tcoeffs,
                ..
            } = &loaded.orders[0].trace;
            assert_eq!(*tbasis, PolynomialBasis::Chebyshev, "{label}: trace basis");
            for (i, (expected, actual)) in trace_coeffs.iter().zip(tcoeffs.iter()).enumerate() {
                assert_eq!(
                    *expected, *actual,
                    "{label}: trace coeff[{i}] not bit-exact"
                );
            }
        }
    }

    // ====================================================================
    // Blaze correction tests
    // ====================================================================

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn compute_blaze_normalises_to_peak_one_and_clamps() {
        // Bell-shaped flat spectrum simulating the grating blaze envelope.
        let n = 100;
        let flat: Vec<f64> = (0..n)
            .map(|i| {
                let x = (i as f64 - 50.0) / 20.0;
                let blaze = (-x * x).exp(); // Gaussian bell
                (blaze * 10000.0).max(1.0) // Ensure positive
            })
            .collect();

        let blaze = compute_blaze_from_flat(&flat);

        assert_eq!(blaze.len(), n);
        // Peak should be 1.0
        let peak = blaze.iter().copied().fold(0.0_f64, f64::max);
        assert!((peak - 1.0).abs() < 1e-6, "peak should be 1.0, got {peak}");
        // All values ≥ BLAZE_FLOOR
        assert!(blaze.iter().all(|&b| b >= BLAZE_FLOOR));
        // Center (i=50) is the bell peak → normalised to 1.0
        assert!(
            blaze[50] > 0.99,
            "center blaze should be ~1.0, got {}",
            blaze[50]
        );
        // Edges are low (bell drops to ~exp(-6.25) ≈ 0.002) → clamped to floor
        assert!(
            (blaze[0] - BLAZE_FLOOR).abs() < 1e-6,
            "far edge should be clamped to BLAZE_FLOOR, got {}",
            blaze[0]
        );
    }

    #[test]
    fn blaze_correction_divides_flux_during_extraction() {
        let width = 16u32;
        let height = 10u32;
        // All aperture pixels set to 1000 → raw flux = 3 × 1000 = 3000 per sample
        // (aperture radius 1 = 3 pixels)
        let frame = non_uniform_frame(width, height);
        let mut profile = small_extraction_profile(EchelleSummationMode::SimpleSum);

        // Extract WITHOUT blaze → raw flux
        let raw = extract_preview(&profile, &frame, width, height, 16, 0).unwrap();
        let raw_flux: Vec<f64> = raw.orders[0].flux.clone();

        // Now set a uniform blaze curve of 0.5 → flux should double
        profile.corrections.blaze_curves = Some(vec![vec![0.5; width as usize]]);

        let corrected = extract_preview(&profile, &frame, width, height, 16, 1).unwrap();
        let corrected_flux = &corrected.orders[0].flux;

        for (i, (&raw_val, &corr_val)) in raw_flux.iter().zip(corrected_flux.iter()).enumerate() {
            if raw_val > 0.0 {
                let ratio = corr_val / raw_val;
                assert!(
                    (ratio - 2.0).abs() < 1e-6,
                    "sample {i}: expected 2× amplification, got ratio {ratio}"
                );
            }
        }
    }

    #[test]
    fn blaze_floor_prevents_extreme_amplification() {
        let width = 16u32;
        let height = 10u32;
        let frame = non_uniform_frame(width, height);
        let mut profile = small_extraction_profile(EchelleSummationMode::SimpleSum);

        // Extract without blaze for reference
        let raw = extract_preview(&profile, &frame, width, height, 16, 0).unwrap();
        let raw_flux: Vec<f64> = raw.orders[0].flux.clone();

        // Blaze curve with a near-zero value → should be clamped to BLAZE_FLOOR
        let mut blaze = vec![1.0; width as usize];
        blaze[8] = 0.001; // Would cause 1000× amplification without floor
        profile.corrections.blaze_curves = Some(vec![blaze]);

        let corrected = extract_preview(&profile, &frame, width, height, 16, 1).unwrap();
        let max_ratio = corrected.orders[0]
            .flux
            .iter()
            .zip(raw_flux.iter())
            .filter(|(_, &r)| r > 0.0)
            .map(|(&c, &r)| c / r)
            .fold(0.0_f64, f64::max);

        // Maximum amplification should be 1/BLAZE_FLOOR = 10×
        assert!(
            max_ratio <= 1.0 / BLAZE_FLOOR + 1e-6,
            "max amplification {max_ratio} exceeds floor limit {}",
            1.0 / BLAZE_FLOOR
        );
    }

    #[test]
    fn extract_flat_blaze_curves_produces_valid_output() {
        let width = 16u32;
        let height = 10u32;
        let profile = small_extraction_profile(EchelleSummationMode::SimpleSum);
        // Use the non_uniform_frame as a "flat" — it has constant signal
        // across the dispersion axis in the aperture.
        let flat_frame = non_uniform_frame(width, height);

        let curves = extract_flat_blaze_curves(&profile, &flat_frame, width, height, 16).unwrap();

        assert_eq!(curves.len(), 1, "one order → one blaze curve");
        assert_eq!(curves[0].len(), width as usize);
        // All values should be in [BLAZE_FLOOR, 1.0]
        for &b in &curves[0] {
            assert!(b >= BLAZE_FLOOR, "blaze {b} < floor");
            assert!(b <= 1.0 + 1e-10, "blaze {b} > 1.0");
        }
        // Peak should be 1.0
        let peak = curves[0].iter().copied().fold(0.0_f64, f64::max);
        assert!((peak - 1.0).abs() < 1e-6, "peak should be 1.0, got {peak}");
    }

    #[test]
    fn blaze_curves_roundtrip_through_serialisation() {
        let mut profile = minimal_profile();
        profile.corrections.blaze_curves = Some(vec![
            vec![0.1, 0.5, 1.0, 0.5, 0.1],
            vec![0.2, 0.8, 1.0, 0.8, 0.2],
        ]);

        let dir = tempdir().unwrap();
        let path = dir.path().join("blaze_test.json");
        profile.save_to_path(&path).unwrap();
        let loaded = EchelleCalibrationProfile::load_from_path(&path).unwrap();

        assert_eq!(
            loaded.corrections.blaze_curves,
            profile.corrections.blaze_curves
        );
    }
}
