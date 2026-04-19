//! End-to-end regression test for the echelle pipeline on real hardware fixtures.
//!
//! Runs the full chain on the March-18 leabs-dev DH3P flat + HgAr arc:
//! `trace detect → scatter → arc calibrate → extract → DH3P continuum fit →
//! per-order blaze → variance-weighted merge`. The merged spectrum is asserted
//! against a committed golden HDF5 at `debug/phase5/reference_merged_spectrum.hdf5`.
//!
//! ## Why extract the DH3P flat (not the arc)
//!
//! The Stage-E blaze path (`fit_dh3p_continuum`, `compute_blaze_from_dh3p_flat`,
//! `variance_weighted_merge`) was added by bd-sw760 / PR #603 and is only unit-tested.
//! Extracting the flat itself through the chain exercises all three in one
//! contract: since `flux/B` reduces to `C_lamp(λ)` by construction, any drift in
//! the continuum fit, blaze mask, or merge weighting visibly perturbs the merged
//! output. Perf refactors in bd-g22gu.1 (Chebyshev caching, rayon per-order,
//! min/max filter unify) must leave this fingerprint unchanged.
//!
//! ## Regenerating the golden
//!
//! ```
//! RUST_DAQ_UPDATE_GOLDEN=1 cargo nextest run --profile ci \
//!     -p integration-tests echelle_merged_spectrum_regression \
//!     --features storage_hdf5
//! ```
//!
//! Re-run `debug/phase5/validate_phase_e.py` afterwards and visually compare
//! `phaseE-dh3p-continuum.png` before committing the regenerated golden.
//!
//! bd-g22gu.4

#![cfg(feature = "storage_hdf5")]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use echelle::blaze::{
    Dh3pContinuumConfig, MergeOrderInput, MergedSpectrum, OrderBlaze, compute_blaze_from_dh3p_flat,
    fit_dh3p_continuum, variance_weighted_merge,
};
use echelle::calibration_pipeline::{
    CalibrationPipelineConfig, CalibrationResult, WavelengthSeed,
    run_calibration_pipeline_with_flat,
};
use echelle::rectification::{OrderSpec, RectifyConfig, rectify_order};
use echelle::wavelength_fitting::load_hgar_atlas;
use echelle::{
    EchelleCalibrationProfile, EchelleFrameCompatibility, EchelleOrientation, EchelleTraceModel,
    EchelleWavelengthModel, PolynomialBasis,
};
use serde::Deserialize;

// ─── Fixture locations ──────────────────────────────────────────────────────

const FIXTURE_ARC_REL: &str = "debug/phase5/flats/hgar_arc_5s_g500_acc10.tiff";
const FIXTURE_FLAT_REL: &str = "debug/phase5/flats/dh3p_flat_5s_g2000_acc10.tiff";
const FIXTURE_CONFIG_REL: &str = "config/calibration/mechelle_5000.toml";
const GOLDEN_REL: &str = "debug/phase5/reference_merged_spectrum.hdf5";

// ─── Pipeline constants ──────────────────────────────────────────────────────

/// Merged-spectrum bin spacing. 0.01 nm matches the typical ME5000 per-pixel
/// dispersion (~0.05–0.3 nm/px across orders) divided by ~5, giving several
/// contributing pixels per bin without over-smoothing.
const GRID_SPACING_NM: f64 = 0.01;

/// Per-order blaze usability threshold: pixels with `B_i(x) < 0.15 × B_i_peak`
/// are masked (SOTA §Absolute Thresholding; same default as blaze.rs callers).
const BLAZE_THRESHOLD_FRAC: f64 = 0.15;

/// In-illumination pixel selector: keep aperture-summed samples above
/// `10 × p10(F)`. Matches `debug/phase5/validate_phase_e.py:156-157` and
/// is the filter required by `fit_dh3p_continuum`'s documented precondition
/// (crates/echelle/src/blaze.rs:166-187).
const ILLUM_FLUX_MULT: f64 = 10.0;

/// Simple-sum Poisson variance floor. Keeps `σ² > 0` for empty-aperture
/// pixels so the variance-weighted merge does not divide by zero.
const VAR_FLOOR: f64 = 1.0;

/// iStar ICCD read noise in electrons/pixel (per `OptimalExtractionConfig::istar_iccd`
/// at crates/echelle/src/optimal_extraction.rs:88). Squared and multiplied by the
/// aperture pixel count to build a realistic simple-sum variance.
const ISTAR_READ_NOISE_E: f64 = 20.0;

/// Nominal aperture diameter in cross-dispersion pixels (2 × half-width = 8 px).
/// Matches `RectifyConfig::default().aperture_half_width = 4.0`.
const APERTURE_PIXELS: f64 = 9.0;

// ─── Regression tolerances ──────────────────────────────────────────────────

/// Wavelength grid is uniform by construction; any drift implies the global
/// min/max of all order wavelengths shifted, i.e., a real calibration change.
/// Use a tight absolute tolerance (1 pm) rather than 0.01 nm to actually
/// catch drift on a bin-exact comparison.
const WAVELENGTH_TOL_NM: f64 = 1e-6;

/// Flux agreement: 1 % relative, or 0.1 % of the global peak absolute (whichever
/// is larger). The absolute floor prevents noise-floor bins from tripping the
/// relative test when both values are near zero.
const FLUX_REL_TOL: f64 = 0.01;
const FLUX_ABS_TOL_FRAC: f64 = 1e-3;

/// Variance agreement: 10 % relative. Looser than flux because variance is
/// quadratic in the (already 1 %-tolerant) inputs and more sensitive to per-
/// order blaze-mask edge effects.
const VARIANCE_REL_TOL: f64 = 0.10;

/// Coverage agreement: number of contributing orders per bin must match
/// within 1 (one order may shift between adjacent bins under a recompute).
const N_ORDERS_DIFF_TOL: i64 = 1;

// ─── Minimal config mirror ──────────────────────────────────────────────────

/// Subset of `crates/bin/src/calibrate.rs::CalibrateFileConfig` that we parse
/// here to avoid depending on the binary crate from an integration test.
/// If the schema in the bin crate changes, this struct must track it.
#[derive(Debug, Deserialize)]
struct FixtureConfig {
    instrument: InstrumentSection,
    detector: DetectorSection,
    orientation: EchelleOrientation,
    #[serde(default)]
    tuning: TuningSection,
}

#[derive(Debug, Deserialize)]
struct InstrumentSection {
    grating_constant_nm: f64,
    first_physical_order: i32,
    #[serde(default = "default_order_step")]
    order_step: i32,
}

#[derive(Debug, Deserialize)]
struct DetectorSection {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TuningSection {
    trace_min_snr: f64,
    trace_fwhm: f64,
    trace_min_peak_distance: usize,
    arc_sigdetect: f64,
    wl_poly_degree: usize,
    wl_seed_tolerance_nm: f64,
    min_lines_per_order: usize,
    max_fit_rms_nm: f64,
}

fn default_order_step() -> i32 {
    1
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Workspace-root–relative path. Integration tests run with CWD =
/// `crates/integration-tests/`, so the workspace root is two levels up.
fn repo_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Load a 16-bit TIFF into a row-major `Vec<f32>` (u16 → f32 with no scaling).
/// Panics if dimensions disagree with `expected_w × expected_h`; the test
/// fixtures are size-locked to 2560×2160 so a mismatch is a bug, not a runtime
/// condition we want to handle gracefully.
fn load_tiff_u16_as_f32(path: &Path, expected_w: u32, expected_h: u32) -> Result<Vec<f32>> {
    let img = image::open(path).with_context(|| format!("open {}", path.display()))?;
    let gray = img.into_luma16();
    let (w, h) = gray.dimensions();
    if w != expected_w || h != expected_h {
        return Err(anyhow!(
            "TIFF {} is {w}×{h}, expected {expected_w}×{expected_h}",
            path.display()
        ));
    }
    Ok(gray.pixels().map(|p| f32::from(p[0])).collect())
}

/// Build a production-matching pipeline config from the parsed TOML.
/// Matches `crates/bin/src/calibrate.rs::into_pipeline_config` semantics for
/// the fields the regression test cares about.
fn build_pipeline_config(
    cfg: &FixtureConfig,
    atlas: Vec<echelle::wavelength_fitting::AtlasLine>,
) -> CalibrationPipelineConfig {
    let trace_config = echelle::trace_fitting::TraceFitConfig {
        min_snr: cfg.tuning.trace_min_snr,
        fwhm_gaussian: cfg.tuning.trace_fwhm,
        min_peak_distance: cfg.tuning.trace_min_peak_distance,
        ..Default::default()
    };
    let arc_config = echelle::wavelength_fitting::ArcDetectConfig {
        sigdetect: cfg.tuning.arc_sigdetect,
        ..Default::default()
    };
    let wl_config = echelle::wavelength_fitting::WlFitConfig {
        poly_degree: cfg.tuning.wl_poly_degree,
        seed_tolerance_nm: cfg.tuning.wl_seed_tolerance_nm,
        max_fit_rms_nm: cfg.tuning.max_fit_rms_nm,
        ..Default::default()
    };

    CalibrationPipelineConfig {
        trace_config,
        trace_validation: echelle::trace_validation::TraceValidationConfig::mechelle_5000_istar(),
        arc_config,
        wl_config,
        // ME5000 preset ships enabled=false for pure-emission HgAr (bd-g22gu.3);
        // leaving it in place means Phase B short-circuits on arcs but can
        // still activate on the flat via preliminary trace detection.
        scatter_config: Some(echelle::scattered_light::ScatteredLightConfig::mechelle_5000_istar()),
        rectify_config: RectifyConfig::default(),
        use_optimal_extraction: false,
        optimal_config: echelle::optimal_extraction::OptimalExtractionConfig::istar_iccd(),
        atlas,
        seed: WavelengthSeed::EchelleEquation {
            grating_constant_nm: cfg.instrument.grating_constant_nm,
            first_physical_order: cfg.instrument.first_physical_order,
            order_step: cfg.instrument.order_step,
            n_pixels: cfg.detector.width,
        },
        frame_compat: EchelleFrameCompatibility {
            sensor_width: cfg.detector.width,
            sensor_height: cfg.detector.height,
            frame_width: cfg.detector.width,
            frame_height: cfg.detector.height,
            roi_x: 0,
            roi_y: 0,
            binning_x: 1,
            binning_y: 1,
            bit_depth: Some(16),
        },
        orientation: cfg.orientation.clone(),
        profile_name: "bd-g22gu.4 regression".to_string(),
        min_lines_per_order: cfg.tuning.min_lines_per_order,
        hdr_extra_arc_frames: Vec::new(),
        hdr_merge_tol_px: 1.0,
        hdr_prefer_unsaturated: true,
    }
}

/// Compute the 10th percentile of the strictly-positive entries of `flux`.
/// Used to derive the in-illumination mask threshold.
fn percentile_p10(flux: &[f64]) -> f64 {
    let mut positives: Vec<f64> = flux.iter().copied().filter(|&f| f > 0.0).collect();
    if positives.is_empty() {
        return 0.0;
    }
    positives.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // n ≤ 2560: f64→usize is safe, sign loss impossible (positives.len() > 0 here,
    // and the expression is always non-negative).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let idx = ((positives.len() as f64 - 1.0) * 0.10).round() as usize;
    positives[idx]
}

/// Simple-sum boxcar extraction on a rectified order, matching
/// `crates/echelle/src/calibration_pipeline.rs::simple_sum_extract` exactly.
fn simple_sum_extract(rect: &echelle::rectification::RectifiedOrder) -> Vec<f64> {
    let mut flux = vec![0.0f64; rect.n_dispersion];
    for (col, f) in flux.iter_mut().enumerate() {
        for row in 0..rect.n_cross {
            let idx = row * rect.n_dispersion + col;
            *f += f64::from(rect.data[idx]) * f64::from(rect.mask[idx]);
        }
    }
    flux
}

/// Evaluate a profile order's wavelength model at integer dispersion pixels
/// `[disp_start ..= disp_end]`. Supports both Chebyshev and Monomial polynomial
/// bases and `Sampled` lookups. Returns one wavelength per pixel.
fn eval_order_wavelengths(order: &echelle::EchelleOrderCalibration) -> Vec<f64> {
    let n = (order.sample_end - order.sample_start + 1) as usize;
    let mut wls = Vec::with_capacity(n);
    for px in order.sample_start..=order.sample_end {
        let x = f64::from(px);
        let wl = match &order.wavelength {
            EchelleWavelengthModel::Polynomial {
                basis,
                coefficients,
                domain_start,
                domain_end,
                ..
            } => eval_polynomial(*basis, coefficients, *domain_start, *domain_end, x),
            EchelleWavelengthModel::Sampled { wavelengths, .. } => {
                let idx = (px - order.sample_start) as usize;
                wavelengths.get(idx).copied().unwrap_or(f64::NAN)
            }
        };
        wls.push(wl);
    }
    wls
}

fn eval_polynomial(
    basis: PolynomialBasis,
    coeffs: &[f64],
    domain_start: f64,
    domain_end: f64,
    x: f64,
) -> f64 {
    if coeffs.is_empty() {
        return f64::NAN;
    }
    match basis {
        PolynomialBasis::Monomial => {
            let mut acc = 0.0f64;
            for &c in coeffs.iter().rev() {
                acc = acc * x + c;
            }
            acc
        }
        PolynomialBasis::Chebyshev => {
            let range = domain_end - domain_start;
            if range <= 0.0 {
                return f64::NAN;
            }
            let t = 2.0 * (x - domain_start) / range - 1.0;
            if coeffs.len() == 1 {
                return coeffs[0];
            }
            let mut t0 = 1.0f64;
            let mut t1 = t;
            let mut acc = coeffs[0] * t0 + coeffs[1] * t1;
            for &c in coeffs.iter().skip(2) {
                let tn = 2.0 * t * t1 - t0;
                acc += c * tn;
                t0 = t1;
                t1 = tn;
            }
            acc
        }
    }
}

// ─── Full-chain orchestration ────────────────────────────────────────────────

#[derive(Debug)]
struct ChainOutput {
    merged: MergedSpectrum,
    n_orders_calibrated: usize,
    n_orders_merged: usize,
    overall_rms_nm: f64,
}

/// Per-order intermediate bundle needed to drive variance_weighted_merge.
struct OrderExtraction {
    wavelengths: Vec<f64>,
    flat_flux: Vec<f64>,
    illum_mask: Vec<bool>,
}

fn run_full_chain() -> Result<ChainOutput> {
    let config_path = repo_path(FIXTURE_CONFIG_REL);
    let arc_path = repo_path(FIXTURE_ARC_REL);
    let flat_path = repo_path(FIXTURE_FLAT_REL);

    let cfg_toml = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let cfg: FixtureConfig = toml::from_str(&cfg_toml).context("parse mechelle_5000.toml")?;

    let w = cfg.detector.width;
    let h = cfg.detector.height;
    let arc = load_tiff_u16_as_f32(&arc_path, w, h)?;
    let flat = load_tiff_u16_as_f32(&flat_path, w, h)?;

    let atlas = load_hgar_atlas();
    let pipeline_cfg = build_pipeline_config(&cfg, atlas);

    let cal: CalibrationResult =
        run_calibration_pipeline_with_flat(&arc, &flat, w, h, &pipeline_cfg)
            .map_err(|e| anyhow!("calibration pipeline failed: {e}"))?;

    // Extract flat flux + wavelengths + illumination mask for each calibrated order.
    let extractions =
        extract_calibrated_orders(&flat, w, h, &cal.profile, &pipeline_cfg.rectify_config);
    if extractions.is_empty() {
        return Err(anyhow!(
            "no extractable orders after calibration (n_calibrated={})",
            cal.n_orders_calibrated
        ));
    }

    // Fit the DH3P lamp continuum from in-illumination samples across all orders.
    let illum_samples: Vec<(Vec<f64>, Vec<f64>)> = extractions
        .iter()
        .map(|e| {
            let (wl, fx): (Vec<f64>, Vec<f64>) = e
                .wavelengths
                .iter()
                .zip(e.flat_flux.iter())
                .zip(e.illum_mask.iter())
                .filter_map(|((&w, &f), &m)| if m { Some((w, f)) } else { None })
                .unzip();
            (wl, fx)
        })
        .collect();
    let refs: Vec<(&[f64], &[f64])> = illum_samples
        .iter()
        .map(|(wl, fx)| (wl.as_slice(), fx.as_slice()))
        .collect();
    let continuum = fit_dh3p_continuum(&refs, &Dh3pContinuumConfig::default())
        .ok_or_else(|| anyhow!("fit_dh3p_continuum returned None — continuum fit failed"))?;

    // Per-order blaze + blaze-corrected flux + variance.
    let blaze_outputs: Vec<(OrderBlaze, Vec<f64>, Vec<f64>)> = extractions
        .iter()
        .map(|e| {
            let blaze = compute_blaze_from_dh3p_flat(
                &e.flat_flux,
                &e.wavelengths,
                &continuum,
                BLAZE_THRESHOLD_FRAC,
            );
            let corrected: Vec<f64> = e
                .flat_flux
                .iter()
                .zip(blaze.blaze.iter())
                .map(|(&f, &b)| if b > 0.0 { f / b } else { 0.0 })
                .collect();
            let variance: Vec<f64> = e
                .flat_flux
                .iter()
                .map(|&f| {
                    // Simple-sum Poisson variance with read-noise aperture term:
                    //   σ² = max(F, 0) + N_ap · RN²  (gain = 1)
                    // VAR_FLOOR keeps σ² > 0 for zero-flat pixels so the inverse-
                    // variance weight stays finite.
                    let poisson = f.max(0.0);
                    let read = APERTURE_PIXELS * ISTAR_READ_NOISE_E * ISTAR_READ_NOISE_E;
                    (poisson + read).max(VAR_FLOOR)
                })
                .collect();
            (blaze, corrected, variance)
        })
        .collect();

    let merge_inputs: Vec<MergeOrderInput<'_>> = extractions
        .iter()
        .zip(blaze_outputs.iter())
        .map(|(ex, (blaze, corrected, variance))| MergeOrderInput {
            wavelengths: &ex.wavelengths,
            flux: corrected,
            variance,
            blaze: &blaze.blaze,
            usable_mask: &blaze.usable_mask,
        })
        .collect();

    let merged = variance_weighted_merge(&merge_inputs, GRID_SPACING_NM)
        .ok_or_else(|| anyhow!("variance_weighted_merge returned None"))?;

    Ok(ChainOutput {
        n_orders_merged: extractions.len(),
        merged,
        n_orders_calibrated: cal.n_orders_calibrated,
        overall_rms_nm: cal.overall_rms_nm,
    })
}

/// Rectify + simple-sum-extract each calibrated order from the input frame.
/// Returns one `OrderExtraction` per order that could be successfully rectified;
/// orders that fall off the detector or have degenerate wavelength models are
/// silently skipped (already reflected in `n_orders_calibrated` diagnostics).
fn extract_calibrated_orders(
    frame: &[f32],
    w: u32,
    h: u32,
    profile: &EchelleCalibrationProfile,
    rectify_cfg: &RectifyConfig,
) -> Vec<OrderExtraction> {
    let mut out = Vec::with_capacity(profile.orders.len());
    for order in &profile.orders {
        if !order.enabled {
            continue;
        }
        let trace: &EchelleTraceModel = &order.trace;
        let spec = OrderSpec {
            trace,
            disp_start: order.sample_start,
            disp_end: order.sample_end,
            order_index: order.relative_index,
        };
        let Some(rect) = rectify_order(frame, w, h, &spec, rectify_cfg) else {
            continue;
        };
        let flat_flux = simple_sum_extract(&rect);
        let wavelengths = eval_order_wavelengths(order);
        if wavelengths.len() != flat_flux.len() {
            continue;
        }
        let p10 = percentile_p10(&flat_flux);
        let threshold = ILLUM_FLUX_MULT * p10;
        let illum_mask: Vec<bool> = flat_flux.iter().map(|&f| f > threshold).collect();
        out.push(OrderExtraction {
            wavelengths,
            flat_flux,
            illum_mask,
        });
    }
    out
}

// ─── Golden I/O (HDF5) ──────────────────────────────────────────────────────

fn write_golden(path: &Path, out: &ChainOutput) -> Result<()> {
    use hdf5::File;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;

    write_f64(&file, "wavelength_nm", &out.merged.wavelengths)?;
    write_f64(&file, "flux", &out.merged.flux)?;
    write_f64(&file, "variance", &out.merged.variance)?;
    write_u32(&file, "n_orders_per_bin", &out.merged.n_orders_per_bin)?;

    // Provenance attributes help a future debugger reconstruct golden conditions.
    let attr_n_calib = file.new_attr::<u32>().create("n_orders_calibrated")?;
    attr_n_calib.write_scalar(&u32::try_from(out.n_orders_calibrated).unwrap_or(u32::MAX))?;
    let attr_n_merged = file.new_attr::<u32>().create("n_orders_merged")?;
    attr_n_merged.write_scalar(&u32::try_from(out.n_orders_merged).unwrap_or(u32::MAX))?;
    let attr_rms = file.new_attr::<f64>().create("overall_rms_nm")?;
    attr_rms.write_scalar(&out.overall_rms_nm)?;
    let attr_grid = file.new_attr::<f64>().create("grid_spacing_nm")?;
    attr_grid.write_scalar(&GRID_SPACING_NM)?;
    Ok(())
}

fn write_f64(file: &hdf5::File, name: &str, data: &[f64]) -> Result<()> {
    let ds = file
        .new_dataset::<f64>()
        .deflate(3)
        .shape([data.len()])
        .create(name)
        .with_context(|| format!("create dataset {name}"))?;
    ds.write_raw(data)
        .with_context(|| format!("write {name}"))?;
    Ok(())
}

fn write_u32(file: &hdf5::File, name: &str, data: &[u32]) -> Result<()> {
    let ds = file
        .new_dataset::<u32>()
        .deflate(3)
        .shape([data.len()])
        .create(name)
        .with_context(|| format!("create dataset {name}"))?;
    ds.write_raw(data)
        .with_context(|| format!("write {name}"))?;
    Ok(())
}

fn read_golden(path: &Path) -> Result<MergedSpectrum> {
    use hdf5::File;
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let wavelengths = file.dataset("wavelength_nm")?.read_raw::<f64>()?;
    let flux = file.dataset("flux")?.read_raw::<f64>()?;
    let variance = file.dataset("variance")?.read_raw::<f64>()?;
    let n_orders_per_bin = file.dataset("n_orders_per_bin")?.read_raw::<u32>()?;
    Ok(MergedSpectrum {
        wavelengths,
        flux,
        variance,
        n_orders_per_bin,
    })
}

// ─── Assertions ─────────────────────────────────────────────────────────────

fn assert_merged_matches_golden(actual: &MergedSpectrum, golden: &MergedSpectrum) {
    assert_eq!(
        actual.wavelengths.len(),
        golden.wavelengths.len(),
        "wavelength grid length differs: actual={} golden={}",
        actual.wavelengths.len(),
        golden.wavelengths.len()
    );

    // Global peak of the golden is the scale we compare absolute tolerances
    // against. It's stable because it's a property of the committed golden.
    let peak = golden
        .flux
        .iter()
        .copied()
        .filter(|f| f.is_finite())
        .fold(0.0f64, f64::max);
    let flux_abs_tol = FLUX_ABS_TOL_FRAC * peak;

    let n = actual.wavelengths.len();
    let mut n_wl_bad = 0usize;
    let mut n_flux_bad = 0usize;
    let mut n_var_bad = 0usize;
    let mut n_orders_bad = 0usize;
    let mut worst_flux_rel = 0.0f64;
    let mut worst_wl_diff = 0.0f64;

    for i in 0..n {
        let wl_a = actual.wavelengths[i];
        let wl_g = golden.wavelengths[i];
        let dwl = (wl_a - wl_g).abs();
        worst_wl_diff = worst_wl_diff.max(dwl);
        if dwl > WAVELENGTH_TOL_NM {
            n_wl_bad += 1;
        }

        let f_a = actual.flux[i];
        let f_g = golden.flux[i];
        if f_a.is_finite() && f_g.is_finite() {
            let abs_diff = (f_a - f_g).abs();
            let rel_diff = if f_g.abs() > 0.0 {
                abs_diff / f_g.abs()
            } else {
                0.0
            };
            worst_flux_rel = worst_flux_rel.max(rel_diff);
            if abs_diff > flux_abs_tol && rel_diff > FLUX_REL_TOL {
                n_flux_bad += 1;
            }
        } else if f_a.is_finite() != f_g.is_finite() {
            // One NaN, one finite — always a mismatch.
            n_flux_bad += 1;
        }

        let v_a = actual.variance[i];
        let v_g = golden.variance[i];
        if v_a.is_finite() && v_g.is_finite() && v_g > 0.0 {
            let rel = (v_a - v_g).abs() / v_g;
            if rel > VARIANCE_REL_TOL {
                n_var_bad += 1;
            }
        } else if v_a.is_finite() != v_g.is_finite() {
            n_var_bad += 1;
        }

        let d_orders =
            i64::from(actual.n_orders_per_bin[i]) - i64::from(golden.n_orders_per_bin[i]);
        if d_orders.abs() > N_ORDERS_DIFF_TOL {
            n_orders_bad += 1;
        }
    }

    let total_bad = n_wl_bad + n_flux_bad + n_var_bad + n_orders_bad;
    assert!(
        total_bad == 0,
        "merged spectrum regression: {n_wl_bad} wl bins, {n_flux_bad} flux bins, \
         {n_var_bad} variance bins, {n_orders_bad} n_orders bins out of tolerance \
         (n={n}, worst wl diff={worst_wl_diff:.3e} nm, worst flux rel={worst_flux_rel:.3e}). \
         If the change was intentional, regenerate the golden with \
         RUST_DAQ_UPDATE_GOLDEN=1 and sanity-check against \
         debug/phase5/validate_phase_e.py.",
    );
}

// ─── Test entry point ───────────────────────────────────────────────────────

#[test]
fn echelle_merged_spectrum_regression_dh3p_flat() {
    let out = run_full_chain().expect("full pipeline must succeed on committed fixtures");

    // Sanity floor: a collapse would produce zero orders and the downstream
    // comparison would silently pass on two empty spectra.
    assert!(
        out.n_orders_merged >= 5,
        "only {} orders merged — trace detection or calibration regressed",
        out.n_orders_merged
    );
    assert!(
        out.overall_rms_nm < 1.0,
        "overall calibration RMS {} nm > 1 nm — atlas match regressed",
        out.overall_rms_nm
    );
    assert!(
        !out.merged.flux.is_empty(),
        "merged spectrum is empty — variance_weighted_merge returned a zero-bin grid"
    );

    let golden_path = repo_path(GOLDEN_REL);

    if std::env::var_os("RUST_DAQ_UPDATE_GOLDEN").is_some() {
        write_golden(&golden_path, &out).expect("write golden HDF5");
        eprintln!(
            "updated golden: {} ({} bins, {} orders)",
            golden_path.display(),
            out.merged.wavelengths.len(),
            out.n_orders_merged
        );
        return;
    }

    let golden = read_golden(&golden_path).unwrap_or_else(|e| {
        panic!(
            "failed to read golden at {}: {e}. If this is the first run, create it with \
             RUST_DAQ_UPDATE_GOLDEN=1 cargo nextest run -p integration-tests \
             echelle_merged_spectrum_regression --features storage_hdf5",
            golden_path.display()
        )
    });

    assert_merged_matches_golden(&out.merged, &golden);
}
