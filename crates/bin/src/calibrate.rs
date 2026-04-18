//! CLI `calibrate` subcommand — run the echelle wavelength calibration pipeline.
//!
//! Loads an arc lamp frame from TIFF (or raw), reads instrument parameters from
//! a TOML config, invokes `run_calibration_pipeline()`, and writes the resulting
//! `EchelleCalibrationProfile` as TOML.
//!
//! Optional `[hdr]` lists extra arc exposures (same geometry as the primary `--frame`);
//! line detections are merged before atlas matching (see `merge_arc_lines_hdr`).

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use echelle::calibration_pipeline::{CalibrationPipelineConfig, CalibrationResult, WavelengthSeed};
use echelle::wavelength_fitting::{load_hg_atlas, load_hgar_atlas};
use echelle::{EchelleFrameCompatibility, EchelleOrientation};

// ─── Config TOML types ──────────────────────────────────────────────────────

/// Top-level calibration config file structure.
#[derive(Debug, Deserialize)]
pub struct CalibrateFileConfig {
    pub instrument: InstrumentSection,
    pub detector: DetectorSection,
    pub orientation: EchelleOrientation,
    #[serde(default)]
    pub tuning: TuningSection,
    /// Optional multi-exposure arc merge for line detection (empty = single-frame path).
    #[serde(default)]
    pub hdr: HdrSection,
}

/// Echelle spectrograph parameters.
#[derive(Debug, Deserialize)]
pub struct InstrumentSection {
    /// Human-readable name (e.g., "Mechelle 5000 HgAr (leabs-dev)")
    pub name: String,
    /// Echelle grating constant in nm (m × λ_center)
    pub grating_constant_nm: f64,
    /// Physical diffraction order number of the first detected order
    pub first_physical_order: i32,
    /// Order step per detected index (typically 1 for increasing Y → increasing m)
    #[serde(default = "default_order_step")]
    pub order_step: i32,
}

/// Detector geometry.
#[derive(Debug, Deserialize)]
pub struct DetectorSection {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_bit_depth")]
    pub bit_depth: u32,
}

/// Optional tuning knobs with sensible defaults.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct TuningSection {
    /// Minimum SNR for order trace detection
    pub trace_min_snr: f64,
    /// Expected FWHM of order traces (pixels)
    pub trace_fwhm: f64,
    /// Minimum peak distance in pixels for trace detection (default: 5)
    pub trace_min_peak_distance: usize,
    /// Minimum SNR for arc line detection
    pub arc_sigdetect: f64,
    /// Polynomial degree for wavelength fit (default: 2; use 1 for sparse data)
    pub wl_poly_degree: usize,
    /// Wavelength tolerance for atlas matching in nm (default: 2.0)
    pub wl_seed_tolerance_nm: f64,
    /// Minimum matched lines required per order
    pub min_lines_per_order: usize,
    /// Maximum acceptable per-order wavelength-fit RMS in nm (bd-0poyt).
    /// Fits whose RMS exceeds this are rejected as likely spurious.
    /// Set to 0.0 to disable the gate.
    pub max_fit_rms_nm: f64,
    /// Use Horne 1986 optimal extraction (vs simple summation)
    pub use_optimal_extraction: bool,
    /// Calibration lamp type: "hg" (pure mercury, e.g. HG-2), "hgar" (mercury-argon).
    /// Default: "hg" for best compatibility with OceanInsight HG-2 lamps.
    #[serde(default = "default_lamp")]
    pub lamp: String,
}

fn default_lamp() -> String {
    "hg".to_string()
}

fn default_order_step() -> i32 {
    1
}
fn default_bit_depth() -> u32 {
    16
}

impl Default for TuningSection {
    fn default() -> Self {
        Self {
            trace_min_snr: 3.0,
            trace_fwhm: 4.0,
            trace_min_peak_distance: 5,
            arc_sigdetect: 5.0,
            wl_poly_degree: 2,
            wl_seed_tolerance_nm: 2.0,
            min_lines_per_order: 3,
            max_fit_rms_nm: 1.0,
            use_optimal_extraction: false,
            lamp: default_lamp(),
        }
    }
}

/// HDR-style multi-exposure arc handling: extra frames merged at line-detection stage.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct HdrSection {
    /// Additional arc lamp files (TIFF/PNG/raw), same width×height as `[detector]`.
    /// Relative paths are resolved from the calibration config file's directory.
    pub extra_arc_paths: Vec<PathBuf>,
    /// Pixel chaining tolerance for `merge_arc_lines_hdr` (default 1.0).
    pub merge_tol_px: f64,
    /// Prefer unsaturated centroids when picking a merged line (default true).
    pub prefer_unsaturated: bool,
}

impl Default for HdrSection {
    fn default() -> Self {
        Self {
            extra_arc_paths: Vec::new(),
            merge_tol_px: 1.0,
            prefer_unsaturated: true,
        }
    }
}

fn resolve_config_relative_path(config_path: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(p)
    }
}

// ─── Config conversion ──────────────────────────────────────────────────────

impl CalibrateFileConfig {
    /// Convert the file config into a full `CalibrationPipelineConfig`.
    ///
    /// `hdr_extra_arc_frames` must match `[detector]` pixel count per frame; primary arc is still `--frame`.
    fn into_pipeline_config(
        self,
        hdr_extra_arc_frames: Vec<Vec<f32>>,
    ) -> CalibrationPipelineConfig {
        let d = CalibrationPipelineConfig::default();

        let mut trace_config = d.trace_config;
        trace_config.min_snr = self.tuning.trace_min_snr;
        trace_config.fwhm_gaussian = self.tuning.trace_fwhm;
        trace_config.min_peak_distance = self.tuning.trace_min_peak_distance;

        let mut arc_config = d.arc_config;
        arc_config.sigdetect = self.tuning.arc_sigdetect;

        let mut wl_config = d.wl_config;
        wl_config.poly_degree = self.tuning.wl_poly_degree;
        wl_config.seed_tolerance_nm = self.tuning.wl_seed_tolerance_nm;
        wl_config.max_fit_rms_nm = self.tuning.max_fit_rms_nm;

        // bd-lpgyn: for Mechelle-class instruments, default the trace
        // validation to the FM2 preset that rejects MCP-halo / prism-
        // ghost traces. This is load-bearing for downstream Cauchy Y(m)
        // fit quality — without it the fit sees edge-clipped ghosts and
        // its RMS is dominated by outliers. Users can opt out by setting
        // all trace_validation.* fields explicitly in a custom config.
        // bd-vdfum / Phase B: enable morphological-opening scattered-light
        // subtraction by default for Mechelle-class ICCD frames. MCP halos
        // flood the inter-order gap on HgAr captures (NotebookLM §FM4 —
        // "4500-count baseline anomaly"); the morphological opening
        // squashes those halos before the background surface is fit.
        // Users can override via a custom config that sets scatter_config
        // = None to disable.
        let scatter_config =
            Some(echelle::scattered_light::ScatteredLightConfig::mechelle_5000_istar());
        // bd-lf1bi / Phase D: ICCD variance model. NotebookLM §FM5 —
        // the iStar MCP has an excess-noise factor F=1.6 (vs 1.0 for a
        // standard CCD), and the standard-CCD variance model
        // underestimates noise at the spatial-profile centre by ~2.56×.
        // F=1.6 AND cr_sigma=5.0 (tightened threshold) are baked into
        // `OptimalExtractionConfig::istar_iccd()`. Wiring it here means
        // that whenever optimal extraction runs on a Mechelle frame
        // (currently opt-in via the TOML `use_optimal_extraction` flag)
        // it uses the correct ICCD variance model. For simple-sum
        // boxcar the config is ignored — harmless.
        let optimal_config = echelle::optimal_extraction::OptimalExtractionConfig::istar_iccd();
        CalibrationPipelineConfig {
            trace_config,
            trace_validation: echelle::trace_validation::TraceValidationConfig::mechelle_5000_istar(
            ),
            arc_config,
            wl_config,
            scatter_config,
            rectify_config: d.rectify_config,
            optimal_config,
            use_optimal_extraction: self.tuning.use_optimal_extraction,
            atlas: match self.tuning.lamp.as_str() {
                "hg" | "hg2" | "mercury" => load_hg_atlas(),
                "hgar" | "hg-ar" => load_hgar_atlas(),
                other => {
                    tracing::warn!("Unknown lamp type '{other}', using pure Hg atlas");
                    load_hg_atlas()
                }
            },
            seed: WavelengthSeed::EchelleEquation {
                grating_constant_nm: self.instrument.grating_constant_nm,
                first_physical_order: self.instrument.first_physical_order,
                order_step: self.instrument.order_step,
                n_pixels: self.detector.width,
            },
            frame_compat: EchelleFrameCompatibility {
                sensor_width: self.detector.width,
                sensor_height: self.detector.height,
                frame_width: self.detector.width,
                frame_height: self.detector.height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(self.detector.bit_depth),
            },
            orientation: self.orientation,
            profile_name: self.instrument.name,
            min_lines_per_order: self.tuning.min_lines_per_order,
            hdr_extra_arc_frames: hdr_extra_arc_frames.into_iter().map(Arc::new).collect(),
            hdr_merge_tol_px: self.hdr.merge_tol_px,
            hdr_prefer_unsaturated: self.hdr.prefer_unsaturated,
        }
    }
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// Run the echelle calibration pipeline on an arc lamp frame.
pub async fn handle_calibrate(
    frame_path: PathBuf,
    flat_path: Option<PathBuf>,
    config_path: PathBuf,
    output_path: PathBuf,
    diagnose: bool,
) -> Result<()> {
    // 1. Load calibration config
    let config_str = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
    let file_config: CalibrateFileConfig = toml::from_str(&config_str)
        .with_context(|| format!("Failed to parse config: {}", config_path.display()))?;

    let detector_width = file_config.detector.width;
    let detector_height = file_config.detector.height;
    let expected_pixels = detector_width as usize * detector_height as usize;

    let hdr_resolved_paths: Vec<PathBuf> = file_config
        .hdr
        .extra_arc_paths
        .iter()
        .map(|p| resolve_config_relative_path(&config_path, p))
        .collect();

    // 2. Load arc frame
    let f32_pixels = load_frame(&frame_path, detector_width, detector_height).await?;

    let (width, height) = if f32_pixels.len() == expected_pixels {
        (detector_width, detector_height)
    } else {
        return Err(anyhow::anyhow!(
            "Frame pixel count ({}) doesn't match config dimensions {}x{} ({})",
            f32_pixels.len(),
            detector_width,
            detector_height,
            expected_pixels,
        ));
    };

    println!(
        "Loaded {width}x{height} arc frame from {}",
        frame_path.display()
    );

    let mut hdr_extra_arc_frames: Vec<Vec<f32>> = Vec::with_capacity(hdr_resolved_paths.len());
    for (i, path) in hdr_resolved_paths.iter().enumerate() {
        let px = load_frame(path, detector_width, detector_height)
            .await
            .with_context(|| format!("HDR extra arc frame {i}: {}", path.display()))?;
        if px.len() != expected_pixels {
            return Err(anyhow::anyhow!(
                "HDR extra arc frame {} pixel count ({}) doesn't match config {}x{} ({})",
                path.display(),
                px.len(),
                detector_width,
                detector_height,
                expected_pixels,
            ));
        }
        hdr_extra_arc_frames.push(px);
    }
    if !hdr_extra_arc_frames.is_empty() {
        println!(
            "Loaded {} HDR extra arc frame(s) for merged line detection",
            hdr_extra_arc_frames.len()
        );
    }

    let pipeline_config = file_config.into_pipeline_config(hdr_extra_arc_frames);

    // 3. Optionally load flat frame for trace detection
    let flat_pixels = if let Some(ref flat) = flat_path {
        let fp = load_frame(flat, detector_width, detector_height).await?;
        println!("Loaded {width}x{height} flat frame from {}", flat.display());
        Some(fp)
    } else {
        None
    };

    println!("Running calibration pipeline...");

    // 4. Run pipeline (with or without flat)
    let result = if let Some(ref flat) = flat_pixels {
        echelle::calibration_pipeline::run_calibration_pipeline_with_flat(
            &f32_pixels,
            flat,
            width,
            height,
            &pipeline_config,
        )
    } else {
        echelle::calibration_pipeline::run_calibration_pipeline(
            &f32_pixels,
            width,
            height,
            &pipeline_config,
        )
    }
    .map_err(|e| anyhow::anyhow!("Pipeline failed: {e}"))?;

    // 4. Print diagnostics
    println!();
    println!("=== Calibration Result ===");
    println!("Orders detected:    {}", result.n_orders_detected);
    println!("Orders calibrated:  {}", result.n_orders_calibrated);
    println!("Overall RMS:        {:.4} nm", result.overall_rms_nm);
    println!();

    for diag in &result.per_order_diagnostics {
        let status = if diag.success { " OK " } else { "FAIL" };
        let reason = diag
            .failure_reason
            .as_ref()
            .map(|r| format!(" | {r}"))
            .unwrap_or_default();
        println!(
            "  Order {:3}: {status} | lines: {}/{}/{} (detected/matched/used) | RMS: {:.4} nm{reason}",
            diag.order_index,
            diag.n_lines_detected,
            diag.n_lines_matched,
            diag.n_lines_used,
            diag.rms_nm,
        );
    }

    if diagnose {
        emit_diagnose_report(&result, &pipeline_config.orientation);
    }

    // 5. Serialize and write profile
    let profile_toml = toml::to_string_pretty(&result.profile)
        .context("Failed to serialize calibration profile to TOML")?;
    tokio::fs::write(&output_path, &profile_toml)
        .await
        .with_context(|| format!("Failed to write profile to {}", output_path.display()))?;

    println!();
    println!("Profile written to: {}", output_path.display());

    Ok(())
}

/// Load a frame file into() pixels.
///
/// Supports TIFF/PNG (via image crate) and raw binary (assumes 16-bit LE).
async fn load_frame(path: &PathBuf, expected_width: u32, expected_height: u32) -> Result<Vec<f32>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "raw" | "bin" => {
            // Raw 16-bit LE binary — dimensions come from config
            let raw_bytes = tokio::fs::read(path)
                .await
                .with_context(|| format!("Failed to read raw frame: {}", path.display()))?;

            let expected_bytes = expected_width as usize * expected_height as usize * 2;
            if raw_bytes.len() != expected_bytes {
                return Err(anyhow::anyhow!(
                    "Raw file size {} bytes doesn't match expected {} bytes for {}x{} 16-bit",
                    raw_bytes.len(),
                    expected_bytes,
                    expected_width,
                    expected_height,
                ));
            }

            Ok(raw_bytes
                .chunks_exact(2)
                .map(|c| f32::from(u16::from_le_bytes([c[0], c[1]])))
                .collect())
        }
        _ => {
            // Image format (TIFF, PNG, etc.)
            let path_clone = path.clone();
            // image::open is blocking — run on blocking thread
            let img = tokio::task::spawn_blocking(move || {
                image::open(&path_clone)
                    .with_context(|| format!("Failed to open image: {}", path_clone.display()))
            })
            .await??;

            let gray = img.into_luma16();
            let (w, h) = gray.dimensions();

            if w != expected_width || h != expected_height {
                tracing::warn!(
                    "Frame dimensions {w}x{h} differ from config {expected_width}x{expected_height}"
                );
            }

            Ok(gray.pixels().map(|p| f32::from(p[0])).collect())
        }
    }
}

// ─── Diagnostic report ──────────────────────────────────────────────────────

/// Emit a verbose per-order diagnostic report to stdout.
///
/// Included checks:
///   - Physical order number m (from the assembled profile)
///   - Wavelength samples at 5 pixel fractions across each order
///   - Monotonicity of the wavelength axis (strict increase / strict decrease)
///   - Sign agreement with `orientation.wavelength_increase_with_dispersion_positive`
///   - Grating-constant consistency (stddev / mean of m·λ_center across orders)
///
/// The report is plain text so an operator can `diff` before/after runs.
fn emit_diagnose_report(result: &CalibrationResult, orientation: &EchelleOrientation) {
    use std::collections::HashMap;

    let m_by_idx: HashMap<u32, Option<i32>> = result
        .profile
        .orders
        .iter()
        .map(|o| (o.relative_index, o.physical_order_number))
        .collect();

    let expect_ascending = orientation.wavelength_increase_with_dispersion_positive;
    let sample_fracs = [0.1_f64, 0.25, 0.5, 0.75, 0.9];

    let mut n_inverted = 0usize;
    let mut n_nonmonotonic = 0usize;
    let mut n_out_of_range = 0usize;
    let mut gc_products: Vec<f64> = Vec::new();

    println!();
    println!("=== --diagnose: per-order wavelength dump ===");
    println!(
        "expected sign: {}",
        if expect_ascending {
            "λ INCREASES with pixel (ascending)"
        } else {
            "λ DECREASES with pixel (descending)"
        }
    );

    for diag in &result.per_order_diagnostics {
        let Some(sol) = &diag.wl_solution else {
            continue;
        };
        let m = m_by_idx.get(&diag.order_index).copied().flatten();
        let samples: Vec<(f64, f64)> = sample_fracs
            .iter()
            .map(|f| {
                let px = sol.pixel_min + (sol.pixel_max - sol.pixel_min) * *f;
                (px, sol.eval(px))
            })
            .collect();

        let diffs: Vec<f64> = samples.windows(2).map(|w| w[1].1 - w[0].1).collect();
        let strictly_ascending = diffs.iter().all(|&d| d > 0.0);
        let strictly_descending = diffs.iter().all(|&d| d < 0.0);
        let monotonic = strictly_ascending || strictly_descending;
        let sign_ok = if expect_ascending {
            strictly_ascending
        } else {
            strictly_descending
        };

        let min_wl = samples
            .iter()
            .map(|(_, w)| *w)
            .fold(f64::INFINITY, f64::min);
        let max_wl = samples
            .iter()
            .map(|(_, w)| *w)
            .fold(f64::NEG_INFINITY, f64::max);
        let in_physical_range = min_wl >= 150.0 && max_wl <= 1200.0;

        if !monotonic {
            n_nonmonotonic += 1;
        } else if !sign_ok {
            n_inverted += 1;
        }
        if !in_physical_range {
            n_out_of_range += 1;
        }

        if let Some(m_val) = m {
            let center_px = 0.5 * (sol.pixel_min + sol.pixel_max);
            let center_wl = sol.eval(center_px);
            gc_products.push(f64::from(m_val) * center_wl);
        }

        let flag_mono = if monotonic { "mono" } else { "WAVY" };
        let flag_sign = if sign_ok { "sign" } else { "INVR" };
        let flag_range = if in_physical_range { "rng" } else { "OOR" };
        let m_str = m.map_or_else(|| "m=?  ".to_string(), |v| format!("m={v:3}"));

        println!(
            "  Order {:3} {} [{} {} {}] λ@({:.1},{:.1},{:.1},{:.1},{:.1})nm",
            diag.order_index,
            m_str,
            flag_mono,
            flag_sign,
            flag_range,
            samples[0].1,
            samples[1].1,
            samples[2].1,
            samples[3].1,
            samples[4].1,
        );
    }

    println!();
    println!("=== --diagnose: summary ===");
    println!("orders with non-monotonic axis: {n_nonmonotonic}");
    println!("orders with inverted sign:      {n_inverted}");
    println!("orders outside [150,1200] nm:   {n_out_of_range}");

    if gc_products.len() >= 2 {
        let n = f64::from(u32::try_from(gc_products.len()).expect("order count fits in u32"));
        let mean = gc_products.iter().sum::<f64>() / n;
        let var = gc_products.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / n;
        let stddev = var.sqrt();
        let rel = if mean.abs() > 1e-12 {
            stddev / mean.abs() * 100.0
        } else {
            f64::NAN
        };
        println!(
            "grating constant (m·λ_center): mean={mean:.1} nm  stddev={stddev:.1} nm ({rel:.2}%)"
        );
        if rel > 3.0 {
            println!(
                "  ⚠ GC scatter > 3%: configured grating_constant_nm is likely wrong \
                 (suspected actual: {mean:.0} nm)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_calibrate_toml_with_hdr_defaults() {
        let toml = r#"
[instrument]
name = "Test"
grating_constant_nm = 2800.0
first_physical_order = 10
order_step = 1

[detector]
width = 100
height = 100
bit_depth = 16

[orientation]
dispersion_axis = "x"
cross_dispersion_axis = "y"
order_number_increase_direction = "positive"
wavelength_increase_with_dispersion_positive = true
"#;
        let cfg: CalibrateFileConfig =
            toml::from_str(toml).expect("minimal calibrate TOML should parse");
        assert!(cfg.hdr.extra_arc_paths.is_empty());
        assert!((cfg.hdr.merge_tol_px - 1.0).abs() < f64::EPSILON);
        assert!(cfg.hdr.prefer_unsaturated);
    }

    #[test]
    fn parse_calibrate_toml_hdr_section() {
        let toml = r#"
[instrument]
name = "Test"
grating_constant_nm = 2800.0
first_physical_order = 10
order_step = 1

[detector]
width = 100
height = 100

[orientation]
dispersion_axis = "x"
cross_dispersion_axis = "y"
order_number_increase_direction = "positive"
wavelength_increase_with_dispersion_positive = true

[hdr]
extra_arc_paths = ["short.tif", "/abs/other.tif"]
merge_tol_px = 1.5
prefer_unsaturated = false
"#;
        let cfg: CalibrateFileConfig = toml::from_str(toml).expect("hdr TOML should parse");
        assert_eq!(cfg.hdr.extra_arc_paths.len(), 2);
        assert!((cfg.hdr.merge_tol_px - 1.5).abs() < 1e-9);
        assert!(!cfg.hdr.prefer_unsaturated);
    }
}
