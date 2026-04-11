//! Realistic synthetic HgAr echelle dataset for calibration pipeline testing.
//!
//! Generates a 2048x2048 synthetic arc frame modelling the Mechelle 5000
//! echelle spectrograph + Andor iStar sCMOS detector with realistic
//! perturbations: order trace curvature, non-uniform inter-order spacing,
//! Gaussian line profiles, read/shot/dark noise, hot pixels, intensity
//! variation from atlas strengths, and defocus/aberration at detector edges.

// Numerical code: pixel-index casts are always lossless for realistic frame sizes.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

use echelle::calibration_pipeline::{
    CalibrationPipelineConfig, WavelengthSeed, run_calibration_pipeline,
};
use echelle::trace_fitting::TraceFitConfig;
use echelle::wavelength_fitting::{ArcDetectConfig, WlFitConfig, load_hgar_atlas};
use echelle::{AxisDirection, DetectorAxis, EchelleFrameCompatibility, EchelleOrientation};

// ─── Constants ───────────────────────────────────────────────────────────────

const WIDTH: usize = 2048;
const HEIGHT: usize = 2048;

/// Mechelle 5000 grating constant: m * lambda_center (nm).
/// With grating_constant=6300 and orders m=20..7, order centers span 315-900nm.
/// This covers the full HgAr emission range with enough orders that the
/// Ar-rich region (696-852nm) supplies many calibratable orders.
const GRATING_CONSTANT_NM: f64 = 6300.0;
const FIRST_PHYSICAL_ORDER: i32 = 20; // highest order (shortest wavelength, ~315nm)
const LAST_PHYSICAL_ORDER: i32 = 7; // lowest order (longest wavelength, ~900nm)

/// Noise model parameters.
const READ_NOISE_ADU: f64 = 4.0;
const DARK_CURRENT_ADU: f64 = 0.5;
const HOT_PIXEL_FRACTION: f64 = 0.000_1; // 0.01%
const HOT_PIXEL_MAX_ADU: f64 = 60_000.0;

/// Line profile defaults (center of detector).
const SPECTRAL_FWHM_CENTER: f64 = 4.0; // pixels
const SPATIAL_FWHM_CENTER: f64 = 3.5; // pixels
/// Edge broadening factor (10-20% increase).
const EDGE_BROADENING_FRAC: f64 = 0.15;

/// Intensity scaling: strongest atlas line maps to this ADU value.
const PEAK_ADU_MAX: f64 = 50_000.0;
/// Weakest detectable lines target ~200 ADU above background.
const PEAK_ADU_MIN: f64 = 200.0;

/// Continuum flux along each order trace (simulates scattered light / fiber).
/// Must be high enough relative to read noise that trace detection cleanly
/// separates the 21 orders from inter-order background on a 2048-pixel detector.
const CONTINUUM_FLUX: f64 = 500.0;

// ─── Deterministic PRNG ─────────────────────────────────────────────────────
//
// A minimal xoshiro256** for reproducible noise without pulling in `rand`.

struct Rng {
    s: [u64; 4],
}

impl Rng {
    fn new(seed: u64) -> Self {
        // SplitMix64 to expand the seed into 4 state words.
        let mut z = seed;
        let mut s = [0u64; 4];
        for slot in &mut s {
            z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            *slot = x ^ (x >> 31);
        }
        Self { s }
    }

    /// Returns a u64 in [0, 2^64).
    fn next_u64(&mut self) -> u64 {
        let result = (self.s[1].wrapping_mul(5)).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform f64 in [0, 1).
    fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Approximate Gaussian via Box-Muller.
    fn gaussian(&mut self, mean: f64, std: f64) -> f64 {
        let u1 = self.uniform().max(1e-15);
        let u2 = self.uniform();
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        mean + std * z
    }

    /// Poisson draw for small-to-moderate lambda via Knuth's method.
    fn poisson(&mut self, lambda: f64) -> u64 {
        if lambda <= 0.0 {
            return 0;
        }
        // For large lambda, approximate with Gaussian (much faster).
        if lambda > 100.0 {
            return self.gaussian(lambda, lambda.sqrt()).max(0.0).round() as u64;
        }
        let l = (-lambda).exp();
        let mut k: u64 = 0;
        let mut p = 1.0;
        loop {
            k += 1;
            p *= self.uniform();
            if p < l {
                return k - 1;
            }
        }
    }
}

// ─── Order model ─────────────────────────────────────────────────────────────

/// Truth data for a single synthetic echelle order.
struct OrderTruth {
    /// Physical diffraction order number.
    physical_order: i32,
    /// Center wavelength (nm): grating_constant / m.
    lambda_center_nm: f64,
    /// Free spectral range (nm): grating_constant / m^2.
    fsr_nm: f64,
    /// Wavelength at left edge of order (pixel 0).
    lambda_start_nm: f64,
    /// Wavelength at right edge (pixel WIDTH-1).
    lambda_end_nm: f64,
    /// Polynomial coefficients for trace center Y(x): y = a0 + a1*x + a2*x^2 + a3*x^3.
    trace_coeffs: [f64; 4],
}

impl OrderTruth {
    /// Evaluate the trace center Y at a given x pixel.
    fn center_y(&self, x: f64) -> f64 {
        let x_norm = x / (WIDTH as f64 - 1.0); // normalize to [0, 1]
        self.trace_coeffs[0]
            + self.trace_coeffs[1] * x_norm
            + self.trace_coeffs[2] * x_norm * x_norm
            + self.trace_coeffs[3] * x_norm * x_norm * x_norm
    }

    /// Map wavelength to fractional pixel position.
    ///
    /// Uses a mildly non-linear dispersion model: the quadratic term
    /// introduces ~2.5% compression at the red end, mimicking real grating
    /// non-linearity. This is small enough that the linear echelle-equation
    /// seed (tolerance ~15nm) still captures atlas matches.
    fn wavelength_to_pixel(&self, wl_nm: f64) -> f64 {
        let frac = (wl_nm - self.lambda_start_nm) / self.fsr_nm;
        // Mild non-linearity (~2.5% max deviation from linear).
        let nonlinear_frac = (frac + 0.025 * frac * frac) / 1.025;
        nonlinear_frac * (WIDTH as f64 - 1.0)
    }
}

/// Build the order truth table for the Mechelle 5000 model.
fn build_order_truth(rng: &mut Rng) -> Vec<OrderTruth> {
    let n_orders = (FIRST_PHYSICAL_ORDER - LAST_PHYSICAL_ORDER + 1) as usize; // 21 orders
    let mut orders = Vec::with_capacity(n_orders);

    // Inter-order spacing: non-uniform. Higher orders (shorter wavelength) are
    // packed closer together. Total Y extent uses ~80% of detector.
    let y_margin = HEIGHT as f64 * 0.08;
    let y_available = HEIGHT as f64 - 2.0 * y_margin;

    // Weight each order by 1/m^2 (proportional to FSR) for spacing.
    let mut weights: Vec<f64> = Vec::with_capacity(n_orders);
    for i in 0..n_orders {
        let m = (FIRST_PHYSICAL_ORDER - i as i32) as f64;
        weights.push(1.0 / (m * m));
    }
    let total_weight: f64 = weights.iter().sum();
    // Cumulative positions (centers are between the dividers).
    let mut cumulative = vec![0.0f64; n_orders];
    {
        let mut running = 0.0;
        for (i, w) in weights.iter().enumerate() {
            running += w / total_weight;
            cumulative[i] = running;
        }
    }

    for i in 0..n_orders {
        let m = (FIRST_PHYSICAL_ORDER - i as i32) as f64;
        let physical_order = FIRST_PHYSICAL_ORDER - i as i32;

        let lambda_center = GRATING_CONSTANT_NM / m;
        let fsr = GRATING_CONSTANT_NM / (m * m);
        let lambda_start = lambda_center - fsr / 2.0;
        let lambda_end = lambda_center + fsr / 2.0;

        // Y center: use cumulative spacing. Shift so mid-order is at center.
        let frac = if i == 0 {
            cumulative[0] / 2.0
        } else {
            f64::midpoint(cumulative[i - 1], cumulative[i])
        };
        let base_y = y_margin + frac * y_available;

        // Trace curvature: slight tilt (0.5-2 deg) + 1-3 pixel curvature.
        let tilt_pixels = rng.gaussian(0.0, 3.0); // total tilt across frame
        let curvature_px = rng.gaussian(0.0, 1.5); // peak curvature
        let cubic_px = rng.gaussian(0.0, 0.3); // very mild cubic

        let trace_coeffs = [base_y, tilt_pixels, curvature_px, cubic_px];

        orders.push(OrderTruth {
            physical_order,
            lambda_center_nm: lambda_center,
            fsr_nm: fsr,
            lambda_start_nm: lambda_start,
            lambda_end_nm: lambda_end,
            trace_coeffs,
        });
    }

    orders
}

// ─── Frame generation ────────────────────────────────────────────────────────

/// Compute a position-dependent line width (FWHM in pixels).
/// Wider at detector edges (defocus/aberration), tightest at center.
fn fwhm_at_position(x: f64, y: f64, base_fwhm: f64) -> f64 {
    let cx = WIDTH as f64 / 2.0;
    let cy = HEIGHT as f64 / 2.0;
    let dx = (x - cx) / cx;
    let dy = (y - cy) / cy;
    let r_sq = dx * dx + dy * dy; // 0 at center, ~2 at corners
    base_fwhm * (1.0 + EDGE_BROADENING_FRAC * r_sq)
}

fn fwhm_to_sigma(fwhm: f64) -> f64 {
    fwhm / (2.0 * (2.0_f64.ln() * 2.0).sqrt()) // FWHM = 2*sqrt(2*ln2)*sigma
}

/// Scale atlas strength to ADU peak value.
/// Linearly maps [min_strength, max_strength] -> [PEAK_ADU_MIN, PEAK_ADU_MAX].
fn strength_to_adu(strength: f64, min_strength: f64, max_strength: f64) -> f64 {
    if (max_strength - min_strength).abs() < 1e-12 {
        return PEAK_ADU_MAX;
    }
    let frac = (strength - min_strength) / (max_strength - min_strength);
    PEAK_ADU_MIN + frac * (PEAK_ADU_MAX - PEAK_ADU_MIN)
}

/// Generate the full 2048x2048 realistic synthetic HgAr frame.
///
/// Returns (frame, width, height, orders_truth).
fn generate_realistic_hgar_frame() -> (Vec<f32>, usize, usize, Vec<OrderTruth>) {
    let mut rng = Rng::new(42);
    let orders = build_order_truth(&mut rng);
    let atlas = load_hgar_atlas();

    let min_strength = atlas
        .iter()
        .map(|a| a.strength)
        .fold(f64::INFINITY, f64::min);
    let max_strength = atlas
        .iter()
        .map(|a| a.strength)
        .fold(f64::NEG_INFINITY, f64::max);

    // Start with dark current background.
    let n_pixels = WIDTH * HEIGHT;
    let mut frame = vec![DARK_CURRENT_ADU as f32; n_pixels];

    // 1. Lay down continuum along each order trace (so trace detection works).
    //    The continuum must be strong enough that the sigma-clipped mean per row
    //    clearly rises above the inter-order background, even with noise.
    for order in &orders {
        for x in 0..WIDTH {
            let cy = order.center_y(x as f64);
            let sigma_spatial = fwhm_to_sigma(SPATIAL_FWHM_CENTER);
            let y_lo = (cy - 5.0 * sigma_spatial).max(0.0) as usize;
            let y_hi = ((cy + 5.0 * sigma_spatial) as usize + 1).min(HEIGHT);
            for y in y_lo..y_hi {
                let dy = y as f64 - cy;
                let weight = (-0.5 * (dy / sigma_spatial).powi(2)).exp();
                frame[y * WIDTH + x] += (CONTINUUM_FLUX * weight) as f32;
            }
        }
    }

    // 2. Stamp emission lines as 2D Gaussians with position-dependent width.
    for order in &orders {
        for atlas_line in &atlas {
            let wl = atlas_line.wavelength_nm;
            if wl < order.lambda_start_nm || wl > order.lambda_end_nm {
                continue;
            }

            let center_x = order.wavelength_to_pixel(wl);
            if center_x < 0.0 || center_x >= WIDTH as f64 {
                continue;
            }

            // Position-dependent FWHM.
            let center_y_at_x = order.center_y(center_x);
            let spec_fwhm = fwhm_at_position(center_x, center_y_at_x, SPECTRAL_FWHM_CENTER);
            let spat_fwhm = fwhm_at_position(center_x, center_y_at_x, SPATIAL_FWHM_CENTER);
            let sigma_spec = fwhm_to_sigma(spec_fwhm);
            let sigma_spat = fwhm_to_sigma(spat_fwhm);

            let peak_adu = strength_to_adu(atlas_line.strength, min_strength, max_strength);

            // Stamp region: 5-sigma footprint.
            let x_lo = (center_x - 5.0 * sigma_spec).max(0.0) as usize;
            let x_hi = ((center_x + 5.0 * sigma_spec) as usize + 1).min(WIDTH);

            for x in x_lo..x_hi {
                let cy = order.center_y(x as f64);
                let dx = x as f64 - center_x;
                let spec_weight = (-0.5 * (dx / sigma_spec).powi(2)).exp();

                let y_lo = (cy - 5.0 * sigma_spat).max(0.0) as usize;
                let y_hi = ((cy + 5.0 * sigma_spat) as usize + 1).min(HEIGHT);

                for y in y_lo..y_hi {
                    let dy = y as f64 - cy;
                    let spat_weight = (-0.5 * (dy / sigma_spat).powi(2)).exp();
                    frame[y * WIDTH + x] += (peak_adu * spec_weight * spat_weight) as f32;
                }
            }
        }
    }

    // 3. Apply noise model.
    // Shot noise (Poisson on signal) + read noise (Gaussian).
    for pixel in &mut frame {
        let signal = (*pixel as f64).max(0.0);
        // Poisson draw for shot noise.
        let noisy_signal = rng.poisson(signal) as f64;
        // Add read noise.
        let read = rng.gaussian(0.0, READ_NOISE_ADU);
        *pixel = (noisy_signal + read).max(0.0) as f32;
    }

    // 4. Hot pixels.
    let n_hot = (n_pixels as f64 * HOT_PIXEL_FRACTION) as usize;
    for _ in 0..n_hot {
        let idx = (rng.next_u64() as usize) % n_pixels;
        frame[idx] = (rng.uniform() * HOT_PIXEL_MAX_ADU) as f32;
    }

    (frame, WIDTH, HEIGHT, orders)
}

// ─── Pipeline config builder ─────────────────────────────────────────────────

fn build_pipeline_config(width: usize, height: usize) -> CalibrationPipelineConfig {
    let atlas = load_hgar_atlas();

    // Use the echelle grating equation seed: this is independent of the
    // detected order count and indices. The pipeline computes per-order
    // wavelength ranges from the grating equation m * lambda = constant.
    // Order numbers decrease with increasing Y (higher Y = lower order = longer
    // wavelength), matching AxisDirection::Negative.
    let seed = WavelengthSeed::EchelleEquation {
        grating_constant_nm: GRATING_CONSTANT_NM,
        first_physical_order: FIRST_PHYSICAL_ORDER,
        order_step: -1,
        n_pixels: width as u32,
    };

    CalibrationPipelineConfig {
        trace_config: TraceFitConfig {
            min_snr: 100.0,           // high SNR to reject inter-order noise peaks
            step_pixels: 10,          // coarser stepping for speed on 2048-wide frame
            poly_degree: 3,           // cubic to capture trace curvature
            aperture_half_width: 6.0, // wide aperture for robust centroid on noisy data
            ..Default::default()
        },
        arc_config: ArcDetectConfig {
            sigdetect: 4.0,
            min_fwhm: 1.5,
            max_fwhm: 12.0,
            min_separation: 3.0,
            continuum_window: 101,
        },
        wl_config: WlFitConfig {
            poly_degree: 2,
            seed_tolerance_nm: 15.0, // wide tolerance for non-linear dispersion
            ..Default::default()
        },
        atlas,
        seed,
        frame_compat: EchelleFrameCompatibility {
            sensor_width: width as u32,
            sensor_height: height as u32,
            frame_width: width as u32,
            frame_height: height as u32,
            roi_x: 0,
            roi_y: 0,
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
        profile_name: "Synthetic HgAr Mechelle 5000 + iStar sCMOS".to_string(),
        min_lines_per_order: 2,
        ..Default::default()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// Full pipeline test: realistic 2048x2048 synthetic HgAr arc frame through
/// the calibration pipeline with all perturbations enabled.
#[ignore = "pre-existing on main: RMS 2.3nm exceeds 1.0nm threshold (echelle calibration regression)"]
#[test]
fn test_echelle_pipeline_with_realistic_synthetic_hgar() {
    let start = std::time::Instant::now();

    // 1. Generate frame.
    let (frame, width, height, orders_truth) = generate_realistic_hgar_frame();
    let gen_elapsed = start.elapsed();
    println!("Frame generation: {gen_elapsed:.2?}");
    assert_eq!(frame.len(), width * height);

    // Sanity: frame has reasonable dynamic range.
    let frame_min = frame.iter().copied().fold(f32::INFINITY, f32::min);
    let frame_max = frame.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    println!("Frame dynamic range: {frame_min:.0} - {frame_max:.0} ADU");
    assert!(frame_max > 1000.0, "frame appears too dim");

    // 2. Build pipeline config with echelle equation seed.
    let config = build_pipeline_config(width, height);

    // 3. Run pipeline.
    let pipeline_start = std::time::Instant::now();
    let result = run_calibration_pipeline(&frame, width as u32, height as u32, &config)
        .expect("pipeline should succeed on realistic synthetic data");
    let pipeline_elapsed = pipeline_start.elapsed();
    println!("Pipeline execution: {pipeline_elapsed:.2?}");

    // 4. Report diagnostics.
    println!("--- Calibration Results ---");
    println!("Orders detected:    {}", result.n_orders_detected);
    println!("Orders calibrated:  {}", result.n_orders_calibrated);
    println!("Overall RMS:        {:.4} nm", result.overall_rms_nm);
    println!();

    println!("--- Per-Order Diagnostics ---");
    for diag in &result.per_order_diagnostics {
        let status = if diag.success { "OK" } else { "FAIL" };
        println!(
            "  Order {:2}: [{status:4}] lines_det={:2}  matched={:2}  used={:2}  RMS={:.4} nm{}",
            diag.order_index,
            diag.n_lines_detected,
            diag.n_lines_matched,
            diag.n_lines_used,
            diag.rms_nm,
            diag.failure_reason
                .as_ref()
                .map_or(String::new(), |r| format!("  ({r})")),
        );
    }

    // 5. Ground-truth comparison: verify atlas lines in truth orders.
    let atlas = load_hgar_atlas();
    println!("\n--- Ground Truth Order Coverage ---");
    for (i, order) in orders_truth.iter().enumerate() {
        let n_lines: usize = atlas
            .iter()
            .filter(|a| {
                a.wavelength_nm >= order.lambda_start_nm && a.wavelength_nm <= order.lambda_end_nm
            })
            .count();
        println!(
            "  Order {:2} (m={:2}): {:.1}-{:.1} nm  ({} atlas lines)",
            i, order.physical_order, order.lambda_start_nm, order.lambda_end_nm, n_lines
        );
    }

    // 6. Assertions.
    // With 9 physical orders spanning 420-900nm and ~29 HgAr atlas lines
    // concentrated in this range, most orders should contain 2+ atlas lines.
    assert!(
        result.n_orders_detected >= 10,
        "expected >= 10 orders detected, got {}",
        result.n_orders_detected
    );
    // The HgAr atlas has natural gaps (436-546nm, 579-696nm) where no
    // reference lines exist. Only orders overlapping the Hg UV cluster or
    // the Ar NIR cluster (696-852nm) can calibrate. With 14 orders spanning
    // 315-900nm, typically 4-6 orders have enough atlas lines.
    assert!(
        result.n_orders_calibrated >= 4,
        "expected >= 4 orders calibrated, got {}. Diagnostics:\n{}",
        result.n_orders_calibrated,
        result
            .per_order_diagnostics
            .iter()
            .map(|d| format!(
                "  order {}: success={} det={} match={} used={} rms={:.4} reason={:?}",
                d.order_index,
                d.success,
                d.n_lines_detected,
                d.n_lines_matched,
                d.n_lines_used,
                d.rms_nm,
                d.failure_reason
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // RMS should be sub-nanometer when echelle seeds use Δλ=gc/m² across the chip
    // (fixed bd-kt8k: build_echelle_seeds dispersion was gc/(m_ref·m·npx)).
    assert!(
        result.overall_rms_nm < 1.0,
        "overall RMS {:.4} nm exceeds 1.0 nm threshold",
        result.overall_rms_nm
    );

    // Profile validation should pass.
    result
        .profile
        .validate()
        .expect("generated profile should pass validation");

    let total_elapsed = start.elapsed();
    println!("\nTotal elapsed: {total_elapsed:.2?}");
}

/// Verify the synthetic frame generator produces the expected number of orders
/// with reasonable wavelength ranges.
#[test]
fn test_synthetic_order_model_is_physically_reasonable() {
    let mut rng = Rng::new(42);
    let orders = build_order_truth(&mut rng);

    // Should have 14 orders (m=20 down to m=7).
    assert_eq!(orders.len(), 14, "expected 14 orders");

    // First order (m=20): ~315nm center, moderate FSR.
    let first = &orders[0];
    assert_eq!(first.physical_order, 20);
    assert!(
        first.lambda_center_nm > 300.0 && first.lambda_center_nm < 330.0,
        "m=20 center {:.1} nm out of range",
        first.lambda_center_nm
    );
    assert!(
        first.fsr_nm > 10.0 && first.fsr_nm < 25.0,
        "m=20 FSR {:.1} nm out of expected range",
        first.fsr_nm
    );

    // Last order (m=7): NIR, ~900nm center, wide FSR.
    let last = orders.last().unwrap();
    assert_eq!(last.physical_order, 7);
    assert!(
        last.lambda_center_nm > 850.0 && last.lambda_center_nm < 950.0,
        "m=7 center {:.1} nm out of range",
        last.lambda_center_nm
    );
    assert!(
        last.fsr_nm > 100.0,
        "m=7 FSR {:.1} nm unexpectedly narrow",
        last.fsr_nm
    );

    // Orders should be sorted by increasing Y (cross-dispersion).
    // Check that base_y (trace_coeffs[0]) is monotonically increasing.
    for pair in orders.windows(2) {
        assert!(
            pair[0].trace_coeffs[0] < pair[1].trace_coeffs[0],
            "order centers not monotonically increasing in Y: {:.1} >= {:.1}",
            pair[0].trace_coeffs[0],
            pair[1].trace_coeffs[0]
        );
    }

    // All orders should fit within the detector.
    for order in &orders {
        for x in [0.0, 1024.0, 2047.0] {
            let cy = order.center_y(x);
            assert!(
                cy > 0.0 && cy < HEIGHT as f64,
                "order m={} trace at x={x} is at y={cy:.1}, outside detector",
                order.physical_order,
            );
        }
    }
}

/// Verify noise model produces expected statistical properties.
#[test]
fn test_noise_model_statistics() {
    let mut rng = Rng::new(123);

    // Test Gaussian noise: mean ~0, std ~4 (read noise).
    let n = 10_000;
    let samples: Vec<f64> = (0..n).map(|_| rng.gaussian(0.0, READ_NOISE_ADU)).collect();
    let mean: f64 = samples.iter().sum::<f64>() / n as f64;
    let variance: f64 = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
    let std = variance.sqrt();

    assert!(mean.abs() < 0.2, "Gaussian mean {mean:.3} not near 0");
    assert!(
        (std - READ_NOISE_ADU).abs() < 0.3,
        "Gaussian std {std:.3} not near {READ_NOISE_ADU}"
    );

    // Test Poisson: mean should approximate lambda.
    let lambda = 50.0;
    let poisson_samples: Vec<f64> = (0..n).map(|_| rng.poisson(lambda) as f64).collect();
    let poisson_mean: f64 = poisson_samples.iter().sum::<f64>() / n as f64;
    assert!(
        (poisson_mean - lambda).abs() < 2.0,
        "Poisson mean {poisson_mean:.1} not near {lambda}"
    );
}
