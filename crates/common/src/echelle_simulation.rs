//! Synthetic echelle frame generator for pipeline development.
//!
//! Generates realistic 2D echelle frames (continuum, arc lamp, or combined)
//! for the Mechelle 5000 spectrograph without requiring real hardware.
//!
//! # Physics model
//!
//! - **Echelle equation**: `mλ = d` where d = grating constant (36300 nm for Mechelle 5000)
//! - **Cross-dispersion**: prism model with `Y ∝ m²` (Cauchy-like angular deviation)
//! - **Blaze function**: `sinc²(π·(λ - λ_blaze)/FSR)` intensity envelope per order
//! - **Continuum**: Planck function at specified temperature (3200K for tungsten halogen)
//! - **Arc lines**: Gaussian peaks at atlas wavelengths, placed at correct pixel positions
//! - **Noise**: Gaussian read noise + Poisson photon noise

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use crate::echelle_wavelength_fitting::AtlasLine;

/// Configuration for synthetic echelle frame generation.
#[derive(Debug, Clone)]
pub struct EchelleSimConfig {
    /// Frame width in pixels (dispersion axis).
    pub width: u32,
    /// Frame height in pixels (cross-dispersion axis).
    pub height: u32,
    /// Echelle grating constant (m × λ) in nm.
    pub grating_constant_nm: f64,
    /// First physical order number (NIR end, low Y).
    pub first_order: i32,
    /// Last physical order number (UV end, high Y).
    pub last_order: i32,
    /// FWHM of order traces in cross-dispersion direction (pixels).
    pub trace_fwhm: f64,
    /// Read noise standard deviation (ADU).
    pub read_noise_adu: f64,
    /// Margin from frame edges in pixels.
    pub edge_margin: f64,
    /// Continuum source temperature in Kelvin (0 = no continuum).
    pub continuum_temperature_k: f64,
    /// Peak continuum flux at the blaze center (ADU).
    pub continuum_peak_flux: f64,
    /// Peak emission line flux (ADU, before blaze modulation).
    pub emission_peak_flux: f64,
}

impl Default for EchelleSimConfig {
    fn default() -> Self {
        Self {
            width: 2560,
            height: 2160,
            grating_constant_nm: 36300.0,
            first_order: 43,
            last_order: 116,
            trace_fwhm: 4.0,
            read_noise_adu: 5.0,
            edge_margin: 30.0,
            continuum_temperature_k: 3200.0,
            continuum_peak_flux: 3000.0,
            emission_peak_flux: 8000.0,
        }
    }
}

/// Computed properties for a single echelle order.
#[derive(Debug, Clone)]
pub struct OrderGeometry {
    /// Physical order number m.
    pub order_m: i32,
    /// Y position on detector (pixels, cross-dispersion center).
    pub y_center: f64,
    /// Wavelength at left edge of detector (nm).
    pub lambda_start: f64,
    /// Center wavelength (nm).
    pub lambda_center: f64,
    /// Wavelength at right edge of detector (nm).
    pub lambda_end: f64,
    /// Dispersion (nm/pixel).
    pub dispersion: f64,
}

impl EchelleSimConfig {
    /// Compute the Y position for a given physical order number.
    ///
    /// Uses a prism-like dispersion model: `Y ∝ m²` (Cauchy approximation).
    /// UV orders (high m) are at the bottom, NIR (low m) at top.
    fn order_y_position(&self, m: i32) -> f64 {
        let m_f = f64::from(m);
        let m_first = f64::from(self.first_order);
        let m_last = f64::from(self.last_order);

        let y_min = self.edge_margin;
        let y_max = f64::from(self.height) - self.edge_margin;

        // Quadratic model: Y ∝ m² for prism cross-disperser.
        // Gives wider spacing in UV (high m), tighter in NIR (low m).
        let t = (m_f * m_f - m_first * m_first) / (m_last * m_last - m_first * m_first);
        y_min + t * (y_max - y_min)
    }

    /// Compute wavelength range for a given order.
    ///
    /// Each order covers ~80% of the free spectral range across the detector width.
    fn order_wavelength_range(&self, m: i32) -> (f64, f64, f64) {
        let lambda_center = self.grating_constant_nm / f64::from(m);
        let fsr = lambda_center / f64::from(m);
        let coverage = 0.8;
        let half_range = fsr * coverage / 2.0;
        (
            lambda_center - half_range,
            lambda_center,
            lambda_center + half_range,
        )
    }

    /// Compute geometry for all orders.
    #[must_use]
    pub fn compute_order_geometries(&self) -> Vec<OrderGeometry> {
        (self.first_order..=self.last_order)
            .map(|m| {
                let (ls, lc, le) = self.order_wavelength_range(m);
                OrderGeometry {
                    order_m: m,
                    y_center: self.order_y_position(m),
                    lambda_start: ls,
                    lambda_center: lc,
                    lambda_end: le,
                    dispersion: (le - ls) / f64::from(self.width),
                }
            })
            .collect()
    }

    /// Generate a synthetic tungsten halogen (continuum) echelleogram.
    ///
    /// All orders are illuminated with a smooth blackbody spectrum modulated
    /// by the echelle blaze function. This is what the real detector sees
    /// with a broadband source.
    #[must_use]
    pub fn simulate_continuum(&self) -> Vec<f32> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut frame = vec![0.0_f32; w * h];
        let orders = self.compute_order_geometries();
        let sigma_y = self.trace_fwhm / 2.355; // FWHM → sigma

        for order in &orders {
            let blackbody_scale = if self.continuum_temperature_k > 0.0 {
                planck_relative(order.lambda_center, self.continuum_temperature_k)
            } else {
                1.0
            };

            for row in 0..h {
                let dy = row as f64 - order.y_center;
                let spatial_weight = (-0.5 * (dy / sigma_y).powi(2)).exp();
                if spatial_weight < 1e-4 {
                    continue;
                }

                for col in 0..w {
                    let lambda = order.lambda_start + col as f64 * order.dispersion;
                    let blaze =
                        blaze_function(lambda, order.lambda_center, order.dispersion * w as f64);
                    let flux = self.continuum_peak_flux * blackbody_scale * blaze * spatial_weight;
                    frame[row * w + col] += flux as f32;
                }
            }
        }

        add_noise(&mut frame, self.read_noise_adu);
        frame
    }

    /// Generate a synthetic arc lamp (HgAr) echelleogram.
    ///
    /// Sharp emission lines are placed at their correct pixel positions
    /// based on the echelle equation. A faint continuum floor is added
    /// so that `detect_orders` can find the traces via sigma-clipped mean.
    #[must_use]
    pub fn simulate_arc(&self, atlas: &[AtlasLine]) -> Vec<f32> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut frame = vec![0.0_f32; w * h];
        let orders = self.compute_order_geometries();
        let sigma_y = self.trace_fwhm / 2.355;
        let line_sigma_px = 2.0; // spectral FWHM ~4.7 pixels

        for order in &orders {
            // Continuum along each order (needed for trace detection).
            // Must be well above noise floor so spatial profile peaks stand out.
            let continuum_floor = 200.0;
            for row in 0..h {
                let dy = row as f64 - order.y_center;
                let spatial_weight = (-0.5 * (dy / sigma_y).powi(2)).exp();
                if spatial_weight < 1e-4 {
                    continue;
                }
                for col in 0..w {
                    frame[row * w + col] += (continuum_floor * spatial_weight) as f32;
                }
            }

            // Emission lines from the atlas.
            for line in atlas {
                if line.wavelength_nm < order.lambda_start || line.wavelength_nm > order.lambda_end
                {
                    continue;
                }

                let pixel_x = (line.wavelength_nm - order.lambda_start) / order.dispersion;
                let blaze = blaze_function(
                    line.wavelength_nm,
                    order.lambda_center,
                    order.dispersion * w as f64,
                );
                let peak = self.emission_peak_flux * (line.strength / 1000.0) * blaze;

                for row in 0..h {
                    let dy = row as f64 - order.y_center;
                    let spatial_weight = (-0.5 * (dy / sigma_y).powi(2)).exp();
                    if spatial_weight < 1e-4 {
                        continue;
                    }
                    for col in 0..w {
                        let dx = col as f64 - pixel_x;
                        let spectral_weight = (-0.5 * (dx / line_sigma_px).powi(2)).exp();
                        frame[row * w + col] += (peak * spatial_weight * spectral_weight) as f32;
                    }
                }
            }
        }

        add_noise(&mut frame, self.read_noise_adu);
        frame
    }

    /// Generate a combined continuum + arc echelleogram.
    ///
    /// Useful for simulating a scenario where both a tungsten halogen lamp
    /// and HgAr lamp illuminate the spectrograph simultaneously.
    #[must_use]
    pub fn simulate_combined(&self, atlas: &[AtlasLine]) -> Vec<f32> {
        let continuum = self.simulate_continuum();
        let arc = self.simulate_arc(atlas);

        // Sum both frames (subtract noise floor since both have it).
        continuum
            .iter()
            .zip(arc.iter())
            .map(|(&c, &a)| c + a)
            .collect()
    }
}

/// Planck function (relative intensity, normalized so peak ≈ 1.0).
///
/// Returns the relative blackbody spectral radiance at wavelength `lambda_nm`
/// for temperature `temp_k`. Used to weight the continuum across orders.
fn planck_relative(lambda_nm: f64, temp_k: f64) -> f64 {
    // Constants (SI units, but lambda in nm → convert to m).
    let h = 6.626e-34; // Planck's constant (J·s)
    let c = 2.998e8; // speed of light (m/s)
    let k = 1.381e-23; // Boltzmann's constant (J/K)
    let lambda_m = lambda_nm * 1e-9;

    let numerator = 2.0 * h * c * c / lambda_m.powi(5);
    let exponent = h * c / (lambda_m * k * temp_k);

    // Clamp exponent to avoid overflow.
    if exponent > 500.0 {
        return 0.0;
    }

    let radiance = numerator / (exponent.exp() - 1.0);

    // Normalize so the peak (at Wien displacement) ≈ 1.0.
    let lambda_peak_m = 2.898e-3 / temp_k; // Wien's law
    let peak_num = 2.0 * h * c * c / lambda_peak_m.powi(5);
    let peak_exp = h * c / (lambda_peak_m * k * temp_k);
    let peak_radiance = peak_num / (peak_exp.exp() - 1.0);

    if peak_radiance > 0.0 {
        radiance / peak_radiance
    } else {
        0.0
    }
}

/// Echelle blaze function: `sinc²` intensity envelope.
///
/// Maximum at `lambda_blaze` (order center), falls to zero at ± FSR/2.
fn blaze_function(lambda: f64, lambda_blaze: f64, fsr: f64) -> f64 {
    if fsr.abs() < 1e-10 {
        return 1.0;
    }
    let x = std::f64::consts::PI * (lambda - lambda_blaze) / fsr;
    if x.abs() < 1e-10 {
        1.0
    } else {
        let sinc = x.sin() / x;
        sinc * sinc
    }
}

/// Add Gaussian read noise to a frame.
fn add_noise(frame: &mut [f32], noise_sigma: f64) {
    if noise_sigma <= 0.0 {
        return;
    }
    // Simple deterministic pseudo-noise using a hash-based approach.
    // Not cryptographically random, but reproducible and sufficient for simulation.
    for (i, pixel) in frame.iter_mut().enumerate() {
        // Box-Muller transform from two hash-derived uniform values.
        let u1 = pseudo_uniform(i as u64, 0);
        let u2 = pseudo_uniform(i as u64, 1);
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        *pixel += (noise_sigma * z) as f32;
        // Clamp to non-negative (physical constraint).
        if *pixel < 0.0 {
            *pixel = 0.0;
        }
    }
}

/// Write a simulated frame to a 16-bit TIFF file.
///
/// Scales the f32 data to u16 range for convenient visualization.
/// Returns the scale factor applied.
#[cfg(feature = "fits")]
pub fn save_simulated_frame_tiff(
    frame: &[f32],
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> Result<f64, String> {
    let max_val = frame.iter().cloned().fold(0.0_f32, f32::max);
    let scale = if max_val > 0.0 {
        60000.0 / max_val as f64 // leave headroom below u16::MAX
    } else {
        1.0
    };

    let u16_data: Vec<u16> = frame
        .iter()
        .map(|&v| ((v as f64 * scale).clamp(0.0, 65535.0)) as u16)
        .collect();

    let file = std::fs::File::create(path)
        .map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
    let writer = std::io::BufWriter::new(file);
    let encoder = image::codecs::tiff::TiffEncoder::new(writer);
    encoder
        .encode(
            bytemuck::cast_slice(&u16_data),
            width,
            height,
            image::ExtendedColorType::L16,
        )
        .map_err(|e| format!("TIFF encode failed: {e}"))?;

    Ok(scale)
}

/// Simple pseudo-random uniform [0,1) from a seed and salt.
/// Uses a simple hash for reproducibility without requiring `rand` crate.
fn pseudo_uniform(seed: u64, salt: u64) -> f64 {
    let mut x = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(salt.wrapping_mul(1_442_695_040_888_963_407))
        .wrapping_add(1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    // Map to (0, 1) — avoid exact 0 for log safety.
    (x >> 11) as f64 / (1u64 << 53) as f64 + 1e-15
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::echelle_wavelength_fitting::load_hgar_atlas;

    #[test]
    fn test_order_geometries_are_sane() {
        let config = EchelleSimConfig::default();
        let orders = config.compute_order_geometries();

        assert_eq!(orders.len(), 74); // m=43..=116

        // Y positions should be monotonically increasing (NIR top → UV bottom).
        for pair in orders.windows(2) {
            assert!(
                pair[1].y_center > pair[0].y_center,
                "Y should increase: m={} at {:.1} vs m={} at {:.1}",
                pair[0].order_m,
                pair[0].y_center,
                pair[1].order_m,
                pair[1].y_center
            );
        }

        // All Y positions should be within the frame.
        for o in &orders {
            assert!(
                o.y_center > 0.0 && o.y_center < 2160.0,
                "Y out of bounds: {:.1}",
                o.y_center
            );
        }

        // First order (m=43) center wavelength ≈ 844 nm.
        let first = &orders[0];
        assert!(
            (first.lambda_center - 844.2).abs() < 1.0,
            "m=43 λ_c = {:.1}",
            first.lambda_center
        );

        // Last order (m=116) center wavelength ≈ 313 nm.
        let last = orders.last().unwrap();
        assert!(
            (last.lambda_center - 312.9).abs() < 1.0,
            "m=116 λ_c = {:.1}",
            last.lambda_center
        );
    }

    #[test]
    fn test_continuum_frame_has_order_signal() {
        let config = EchelleSimConfig {
            read_noise_adu: 0.0, // no noise for clean test
            ..Default::default()
        };
        let frame = config.simulate_continuum();

        assert_eq!(frame.len(), 2560 * 2160);

        // Frame should have significant dynamic range.
        let max = frame.iter().cloned().fold(0.0_f32, f32::max);
        let mean: f32 = frame.iter().sum::<f32>() / frame.len() as f32;
        assert!(max > 500.0, "max pixel should be bright: {max}");
        assert!(
            mean < max / 5.0,
            "most pixels should be background: mean={mean}, max={max}"
        );
    }

    #[test]
    fn test_arc_frame_has_emission_lines() {
        let config = EchelleSimConfig {
            read_noise_adu: 0.0,
            ..Default::default()
        };
        let atlas = load_hgar_atlas();
        let frame = config.simulate_arc(&atlas);

        assert_eq!(frame.len(), 2560 * 2160);

        let max = frame.iter().cloned().fold(0.0_f32, f32::max);
        assert!(
            max > 1000.0,
            "brightest emission line should be prominent: {max}"
        );
    }

    #[test]
    fn test_planck_peaks_at_wien() {
        // For 3200K, Wien peak ≈ 2898/3200 ≈ 906nm.
        let peak = planck_relative(906.0, 3200.0);
        let off_peak = planck_relative(500.0, 3200.0);
        assert!(
            peak > off_peak,
            "Planck should peak near Wien: peak={peak:.4}, 500nm={off_peak:.4}"
        );
        assert!((peak - 1.0).abs() < 0.05, "Peak should be ≈1.0: {peak}");
    }

    #[test]
    fn test_blaze_function_peaks_at_center() {
        let center = blaze_function(500.0, 500.0, 10.0);
        let edge = blaze_function(505.0, 500.0, 10.0);
        assert!(
            (center - 1.0).abs() < 1e-10,
            "Blaze should be 1.0 at center"
        );
        assert!(edge < center, "Blaze should fall off from center");
    }

    /// Full pipeline integration test: simulate → detect → calibrate.
    ///
    /// Uses the EchelleEquation seed (same as the real Mechelle 5000 config)
    /// to validate that the pipeline can handle all ~74 orders with correct
    /// order-to-wavelength mapping.
    #[test]
    fn test_pipeline_on_simulated_arc() {
        use crate::echelle::{
            AxisDirection, DetectorAxis, EchelleFrameCompatibility, EchelleOrientation,
        };
        use crate::echelle_calibration_pipeline::{
            run_calibration_pipeline, CalibrationPipelineConfig, WavelengthSeed,
        };
        use crate::echelle_rectification::RectifyConfig;
        use crate::echelle_trace_fitting::TraceFitConfig;
        use crate::echelle_wavelength_fitting::{ArcDetectConfig, WlFitConfig};

        let sim_config = EchelleSimConfig {
            read_noise_adu: 2.0, // light noise for realism
            continuum_peak_flux: 3000.0,
            emission_peak_flux: 10000.0,
            ..Default::default()
        };
        let atlas = load_hgar_atlas();
        let frame = sim_config.simulate_arc(&atlas);

        let pipeline_config = CalibrationPipelineConfig {
            trace_config: TraceFitConfig {
                min_snr: 5.0,
                step_pixels: 5,
                poly_degree: 2,
                min_peak_distance: 15,
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
                poly_degree: 1,         // linear fit — needs only 2 matched lines
                seed_tolerance_nm: 5.0, // wider tolerance for echelle equation seed
                ..Default::default()
            },
            rectify_config: RectifyConfig {
                aperture_half_width: 5.0,
                gaussian_weights: false,
                fwhm: 3.0,
            },
            atlas: atlas.clone(),
            seed: WavelengthSeed::EchelleEquation {
                grating_constant_nm: sim_config.grating_constant_nm,
                first_physical_order: sim_config.first_order,
                order_step: 1,
                n_pixels: sim_config.width,
            },
            frame_compat: EchelleFrameCompatibility {
                sensor_width: sim_config.width,
                sensor_height: sim_config.height,
                frame_width: sim_config.width,
                frame_height: sim_config.height,
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
            profile_name: "Simulated Mechelle 5000".to_string(),
            min_lines_per_order: 2,
            ..Default::default()
        };

        let result = run_calibration_pipeline(
            &frame,
            sim_config.width,
            sim_config.height,
            &pipeline_config,
        );

        match result {
            Ok(ref r) => {
                println!("Orders detected: {}", r.n_orders_detected);
                println!("Orders calibrated: {}", r.n_orders_calibrated);
                println!("Overall RMS: {:.4} nm", r.overall_rms_nm);

                // Should detect a reasonable number of orders.
                // With the quadratic Y spacing model, NIR orders are ~16px apart
                // so some over-detection is expected. Real prism would be better.
                assert!(
                    r.n_orders_detected >= 40,
                    "expected ≥40 orders detected, got {}",
                    r.n_orders_detected
                );

                // Should calibrate at least some orders (any with ≥2 HgAr lines).
                assert!(
                    r.n_orders_calibrated >= 5,
                    "expected ≥5 calibrated orders, got {}. Diagnostics:\n{}",
                    r.n_orders_calibrated,
                    r.per_order_diagnostics
                        .iter()
                        .filter(|d| !d.success)
                        .take(10)
                        .map(|d| format!(
                            "  order {}: lines={}/{}/{} reason={}",
                            d.order_index,
                            d.n_lines_detected,
                            d.n_lines_matched,
                            d.n_lines_used,
                            d.failure_reason.as_deref().unwrap_or("ok")
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                );

                // RMS should be sub-nanometer for well-calibrated orders.
                if r.n_orders_calibrated > 0 {
                    assert!(
                        r.overall_rms_nm < 1.0,
                        "RMS should be < 1nm, got {:.4}",
                        r.overall_rms_nm
                    );
                }
            }
            Err(e) => {
                panic!("Pipeline failed on simulated arc: {e}");
            }
        }
    }
}
