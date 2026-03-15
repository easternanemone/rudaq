//! End-to-end integration tests for the echelle calibration pipeline.
//!
//! These tests exercise the full path from synthetic arc frame through
//! wavelength calibration to profile validation — the same flow a user
//! would trigger from the calibration workspace GUI.

use common::echelle::{
    AxisDirection, DetectorAxis, EchelleFrameCompatibility, EchelleOrientation,
    EchelleWavelengthModel, PolynomialBasis,
};
use common::echelle_calibration_pipeline::{
    run_calibration_pipeline, CalibrationPipelineConfig, SeedAnchor, WavelengthSeed,
};
use common::echelle_trace_fitting::TraceFitConfig;
use common::echelle_wavelength_fitting::{
    detect_arc_lines, fit_order_wavelength, load_hgar_atlas, match_lines_to_atlas, ArcDetectConfig,
    WlFitConfig,
};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Clamp a float to a non-negative usize pixel index.
fn px(v: f64) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        v.max(0.0) as usize
    }
}

/// Narrowing f64→f32 for synthetic pixel flux values.
fn flux32(v: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    {
        v as f32
    }
}

/// Build a synthetic arc frame with Gaussian emission lines placed at known
/// wavelengths within each order. This mirrors the pipeline's own test helper
/// but lives at integration scope to verify the public API.
#[allow(clippy::cast_precision_loss)] // pixel indices ≤ 300 — exact in f64
fn synthetic_arc_frame(
    width: usize,
    height: usize,
    orders: &[(f64, f64, f64)], // (center_y, lambda_start, lambda_end)
    wavelengths: &[f64],
    sigma_spectral: f64,
    peak_flux: f64,
    sigma_spatial: f64,
) -> Vec<f32> {
    let mut frame = vec![0.0f32; width * height];

    // First, lay down a continuum along each order trace so detect_orders
    // can find them via the spatial profile collapse.
    let continuum_flux = 50.0f64;
    for &(center_y, _lam_start, _lam_end) in orders {
        let y_lo = px(center_y - 4.0 * sigma_spatial);
        let y_hi = (px(center_y + 4.0 * sigma_spatial) + 1).min(height);
        for y in y_lo..y_hi {
            let dy = y as f64 - center_y;
            let spatial_weight = (-0.5 * (dy / sigma_spatial).powi(2)).exp();
            for x in 0..width {
                frame[y * width + x] += flux32(continuum_flux * spatial_weight);
            }
        }
    }

    // Then stamp emission lines as 2D Gaussians on top of the continuum.
    for &(center_y, lambda_start, lambda_end) in orders {
        for &wl in wavelengths {
            if wl < lambda_start || wl > lambda_end {
                continue;
            }
            // Map wavelength to pixel position (linear dispersion model).
            let frac = (wl - lambda_start) / (lambda_end - lambda_start);
            let center_x = frac * (width as f64 - 1.0);

            // Stamp a 2D Gaussian (spectral x spatial).
            let x_lo = px(center_x - 4.0 * sigma_spectral);
            let x_hi = (px(center_x + 4.0 * sigma_spectral) + 1).min(width);
            let y_lo = px(center_y - 4.0 * sigma_spatial);
            let y_hi = (px(center_y + 4.0 * sigma_spatial) + 1).min(height);

            for y in y_lo..y_hi {
                let dy = y as f64 - center_y;
                let spatial_weight = (-0.5 * (dy / sigma_spatial).powi(2)).exp();
                for x in x_lo..x_hi {
                    let dx = x as f64 - center_x;
                    let spectral_weight = (-0.5 * (dx / sigma_spectral).powi(2)).exp();
                    frame[y * width + x] += flux32(peak_flux * spatial_weight * spectral_weight);
                }
            }
        }
    }

    frame
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// Full pipeline: synthetic arc → calibration profile → validate → evaluate wavelengths.
#[test]
#[allow(clippy::cast_precision_loss)] // pixel indices ≤ 300 — exact in f64
fn test_calibration_pipeline_produces_valid_profile() {
    let width = 200;
    let height = 300;

    let orders = vec![
        (60.0, 400.0, 420.0),
        (150.0, 500.0, 525.0),
        (240.0, 700.0, 740.0),
    ];

    let atlas = load_hgar_atlas();
    let all_wl: Vec<f64> = atlas.iter().map(|a| a.wavelength_nm).collect();

    let frame = synthetic_arc_frame(width, height, &orders, &all_wl, 2.5, 2000.0, 2.5);

    let mut anchors = Vec::new();
    for (oi, &(_, lam_start, lam_end)) in orders.iter().enumerate() {
        anchors.push(SeedAnchor {
            order_index: u32::try_from(oi).unwrap(),
            pixel: 0.0,
            wavelength_nm: lam_start,
        });
        anchors.push(SeedAnchor {
            order_index: u32::try_from(oi).unwrap(),
            #[allow(clippy::cast_precision_loss)]
            pixel: (width - 1) as f64,
            wavelength_nm: lam_end,
        });
    }

    let config = CalibrationPipelineConfig {
        trace_config: TraceFitConfig {
            min_snr: 3.0,
            step_pixels: 5,
            poly_degree: 2,
            ..Default::default()
        },
        arc_config: ArcDetectConfig {
            sigdetect: 3.0,
            min_fwhm: 1.5,
            max_fwhm: 10.0,
            min_separation: 3.0,
            continuum_window: 51,
        },
        wl_config: WlFitConfig {
            poly_degree: 2,
            seed_tolerance_nm: 2.0,
            ..Default::default()
        },
        atlas: atlas.clone(),
        seed: WavelengthSeed::Anchors(anchors),
        frame_compat: EchelleFrameCompatibility {
            sensor_width: u32::try_from(width).unwrap(),
            sensor_height: u32::try_from(height).unwrap(),
            frame_width: u32::try_from(width).unwrap(),
            frame_height: u32::try_from(height).unwrap(),
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
        profile_name: "Integration Test HgAr".to_string(),
        min_lines_per_order: 2,
        ..Default::default()
    };

    let result = run_calibration_pipeline(
        &frame,
        u32::try_from(width).unwrap(),
        u32::try_from(height).unwrap(),
        &config,
    )
    .expect("pipeline should succeed on synthetic arc");

    // Basic sanity: detected 3 orders.
    assert_eq!(result.n_orders_detected, 3);

    // At least 1 order calibrated.
    assert!(
        result.n_orders_calibrated > 0,
        "no orders calibrated: {:?}",
        result.per_order_diagnostics
    );

    // Overall RMS should be low for clean synthetic data.
    assert!(
        result.overall_rms_nm < 0.5,
        "overall RMS {} nm is too high for synthetic data",
        result.overall_rms_nm
    );

    // Generated profile should be valid.
    let profile = &result.profile;
    assert!(!profile.orders.is_empty());

    // Round-trip: evaluate the fitted wavelength model at known pixel positions
    // and verify it returns sensible wavelengths within the expected range.
    for order in &profile.orders {
        match &order.wavelength {
            EchelleWavelengthModel::Polynomial {
                basis,
                coefficients,
                domain_start,
                domain_end,
                unit,
            } => {
                assert_eq!(
                    *basis,
                    PolynomialBasis::Chebyshev,
                    "pipeline should produce Chebyshev basis"
                );
                assert_eq!(unit, "nm");
                assert!(!coefficients.is_empty());
                // Evaluate at domain midpoint.
                let mid_pixel = (domain_start + domain_end) / 2.0;
                let wl = eval_chebyshev(coefficients, mid_pixel, *domain_start, *domain_end);
                assert!(
                    wl > 200.0 && wl < 900.0,
                    "wavelength {wl} nm at midpoint is out of HgAr range"
                );
            }
            EchelleWavelengthModel::Sampled { wavelengths, unit } => {
                assert!(!wavelengths.is_empty(), "sampled model has no wavelengths");
                assert_eq!(unit, "nm");
            }
        }
    }
}

/// The HgAr atlas should be non-empty, sorted, and contain known reference lines.
#[test]
fn test_hgar_atlas_contains_known_lines() {
    let atlas = load_hgar_atlas();
    assert!(atlas.len() >= 20, "atlas should have >= 20 lines");

    // Verify sorted by wavelength.
    for pair in atlas.windows(2) {
        assert!(
            pair[0].wavelength_nm <= pair[1].wavelength_nm,
            "atlas not sorted: {} > {}",
            pair[0].wavelength_nm,
            pair[1].wavelength_nm
        );
    }

    // Key mercury lines should be present.
    let hg_green_546 = atlas.iter().any(|l| (l.wavelength_nm - 546.07).abs() < 0.1);
    let hg_blue_435 = atlas.iter().any(|l| (l.wavelength_nm - 435.83).abs() < 0.1);
    assert!(hg_green_546, "Hg 546.07 nm (green) not in atlas");
    assert!(hg_blue_435, "Hg 435.83 nm (blue) not in atlas");

    // Key argon lines should be present.
    let ar_763 = atlas.iter().any(|l| (l.wavelength_nm - 763.51).abs() < 0.1);
    assert!(ar_763, "Ar 763.51 nm not in atlas");
}

/// Arc line detection should find peaks in a simple synthetic spectrum.
#[test]
#[allow(clippy::cast_precision_loss)] // pixel indices ≤ 500 — exact in f64
fn test_arc_line_detection_standalone() {
    // Create a 1D spectrum with 3 Gaussian peaks at known pixel positions.
    let n_pixels = 500;
    let peaks = vec![(100.0, 1000.0), (250.0, 800.0), (400.0, 1200.0)];
    let sigma = 3.0;

    let mut spectrum = vec![10.0f32; n_pixels]; // background
    for &(center, amplitude) in &peaks {
        for x in 0..n_pixels {
            let dx = x as f64 - center;
            spectrum[x] += flux32(amplitude * (-0.5 * (dx / sigma).powi(2)).exp());
        }
    }

    let config = ArcDetectConfig {
        sigdetect: 3.0,
        min_fwhm: 1.5,
        max_fwhm: 20.0,
        min_separation: 5.0,
        continuum_window: 51,
    };

    let lines = detect_arc_lines(&spectrum, 0, &config);
    assert!(lines.len() >= 3, "expected >= 3 lines, got {}", lines.len());

    // Verify detected positions are close to known peaks.
    for &(expected_px, _) in &peaks {
        let closest = lines
            .iter()
            .map(|l| (l.pixel_center - expected_px).abs())
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        assert!(
            closest < 1.0,
            "no detected line within 1 pixel of expected position {expected_px}"
        );
    }
}

/// Wavelength fitting should recover a known linear dispersion model.
#[test]
fn test_wavelength_fitting_recovers_linear_dispersion() {
    // Simulate a linear dispersion: wavelength = 500 + 0.05 * pixel.
    let atlas = vec![
        common::echelle_wavelength_fitting::AtlasLine {
            wavelength_nm: 500.0,
            species: "test".into(),
            strength: 100.0,
        },
        common::echelle_wavelength_fitting::AtlasLine {
            wavelength_nm: 505.0,
            species: "test".into(),
            strength: 100.0,
        },
        common::echelle_wavelength_fitting::AtlasLine {
            wavelength_nm: 510.0,
            species: "test".into(),
            strength: 100.0,
        },
        common::echelle_wavelength_fitting::AtlasLine {
            wavelength_nm: 515.0,
            species: "test".into(),
            strength: 100.0,
        },
        common::echelle_wavelength_fitting::AtlasLine {
            wavelength_nm: 520.0,
            species: "test".into(),
            strength: 100.0,
        },
    ];

    // Known pixel positions for each atlas line (from linear model).
    let lines: Vec<common::echelle_wavelength_fitting::ArcLine> = atlas
        .iter()
        .map(|a| {
            let pixel = (a.wavelength_nm - 500.0) / 0.05;
            common::echelle_wavelength_fitting::ArcLine {
                order: 0,
                pixel_center: pixel,
                pixel_sigma: 2.12, // FWHM ~5.0 / 2.355
                amplitude: 1000.0,
                wavelength_hint: Some(a.wavelength_nm),
                used: true,
            }
        })
        .collect();

    // Match and fit.
    let seed_fn = |px: f64| 500.0 + 0.05 * px;
    let matched = match_lines_to_atlas(&lines, &atlas, &seed_fn, 1.0);
    assert_eq!(matched.len(), 5, "all 5 lines should match");

    let config = WlFitConfig {
        poly_degree: 1, // linear should suffice
        ..Default::default()
    };

    let solution =
        fit_order_wavelength(&lines, &atlas, &matched, 0, &config).expect("fit should succeed");

    // Check RMS is near-zero for exact linear data.
    assert!(
        solution.rms_nm < 0.01,
        "RMS {} nm too high for linear data",
        solution.rms_nm
    );

    // Evaluate at pixel 200 → should give ~510 nm.
    let wl_at_200 = solution.eval(200.0);
    assert!(
        (wl_at_200 - 510.0).abs() < 0.1,
        "wavelength at pixel 200: expected ~510.0, got {wl_at_200}"
    );
}

// ─── Chebyshev evaluation helper ────────────────────────────────────────────

fn eval_chebyshev(coeffs: &[f64], pixel: f64, domain_start: f64, domain_end: f64) -> f64 {
    let range = domain_end - domain_start;
    let x_norm = if range.abs() < 1e-12 {
        0.0
    } else {
        2.0 * (pixel - domain_start) / range - 1.0
    };

    let mut t_prev = 1.0;
    let mut t_curr = x_norm;
    let mut result = coeffs.first().copied().unwrap_or(0.0);
    if coeffs.len() > 1 {
        result += coeffs[1] * x_norm;
    }
    for (_i, &c) in coeffs.iter().enumerate().skip(2) {
        let t_next = 2.0 * x_norm * t_curr - t_prev;
        result += c * t_next;
        t_prev = t_curr;
        t_curr = t_next;
    }
    result
}
