//! Echelle calibration profile types and validation.
//!
//! This module defines the canonical, versioned calibration profile format used
//! by rust-daq for echellegram-to-spectrum extraction workflows.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

pub const ECHELLE_PROFILE_SCHEMA_MAJOR: u32 = 1;

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

    pub fn validate_for_frame(
        &self,
        frame: EchelleFrameContext,
    ) -> Result<(), EchelleProfileError> {
        self.validate()?;

        let c = &self.compatibility;
        if frame.width != c.frame_width || frame.height != c.frame_height {
            return Err(invalid(format!(
                "frame size mismatch: profile expects {}x{}, got {}x{}",
                c.frame_width, c.frame_height, frame.width, frame.height
            )));
        }

        if let Some(roi_x) = frame.roi_x {
            if roi_x != c.roi_x {
                return Err(invalid(format!(
                    "ROI X mismatch: profile expects {}, got {}",
                    c.roi_x, roi_x
                )));
            }
        }
        if let Some(roi_y) = frame.roi_y {
            if roi_y != c.roi_y {
                return Err(invalid(format!(
                    "ROI Y mismatch: profile expects {}, got {}",
                    c.roi_y, roi_y
                )));
            }
        }
        if let Some(bx) = frame.binning_x {
            if bx != c.binning_x {
                return Err(invalid(format!(
                    "binning_x mismatch: profile expects {}, got {}",
                    c.binning_x, bx
                )));
            }
        }
        if let Some(by) = frame.binning_y {
            if by != c.binning_y {
                return Err(invalid(format!(
                    "binning_y mismatch: profile expects {}, got {}",
                    c.binning_y, by
                )));
            }
        }
        if let (Some(expected), Some(actual)) = (c.bit_depth, frame.bit_depth) {
            if expected != actual {
                return Err(invalid(format!(
                    "bit_depth mismatch: profile expects {}, got {}",
                    expected, actual
                )));
            }
        }
        Ok(())
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
}
