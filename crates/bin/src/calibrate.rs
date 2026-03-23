//! CLI `calibrate` subcommand — run the echelle wavelength calibration pipeline.
//!
//! Loads an arc lamp frame from TIFF (or raw), reads instrument parameters from
//! a TOML config, invokes `run_calibration_pipeline()`, and writes the resulting
//! `EchelleCalibrationProfile` as TOML.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

use echelle::calibration_pipeline::{CalibrationPipelineConfig, WavelengthSeed};
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
            use_optimal_extraction: false,
            lamp: default_lamp(),
        }
    }
}

// ─── Config conversion ──────────────────────────────────────────────────────

impl CalibrateFileConfig {
    /// Convert the file config into a full `CalibrationPipelineConfig`.
    fn into_pipeline_config(self) -> CalibrationPipelineConfig {
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

        CalibrationPipelineConfig {
            trace_config,
            trace_validation: Default::default(),
            arc_config,
            wl_config,
            scatter_config: d.scatter_config,
            rectify_config: d.rectify_config,
            optimal_config: d.optimal_config,
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
) -> Result<()> {
    // 1. Load calibration config
    let config_str = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
    let file_config: CalibrateFileConfig = toml::from_str(&config_str)
        .with_context(|| format!("Failed to parse config: {}", config_path.display()))?;

    let detector_width = file_config.detector.width;
    let detector_height = file_config.detector.height;
    let pipeline_config = file_config.into_pipeline_config();

    // 2. Load arc frame
    let f32_pixels = load_frame(&frame_path, detector_width, detector_height).await?;

    let (width, height) =
        if f32_pixels.len() == (detector_width as usize) * (detector_height as usize) {
            (detector_width, detector_height)
        } else {
            return Err(anyhow::anyhow!(
                "Frame pixel count ({}) doesn't match config dimensions {}x{} ({})",
                f32_pixels.len(),
                detector_width,
                detector_height,
                detector_width as usize * detector_height as usize,
            ));
        };

    println!(
        "Loaded {width}x{height} arc frame from {}",
        frame_path.display()
    );

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
