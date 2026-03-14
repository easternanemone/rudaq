#![allow(dead_code)]
//! MVP echelle extraction helpers for ImageViewerPanel.
//!
//! The goal is a safe, calibration-profile-driven preview extraction path that
//! can operate directly on live frame bytes. This intentionally prioritizes
//! correctness and validation over maximal throughput.

use chrono::Utc;
use common::core::Measurement;
use common::echelle::{
    BadPixelMask, DetectorAxis, EchelleCalibrationProfile, EchelleFrameContext,
    EchelleOrderCalibration, EchelleSummationMode, EchelleTraceModel, EchelleWavelengthModel,
    PixelRegion, PolynomialBasis,
};
use serde_json::json;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct EchelleOrderPreview {
    pub(super) relative_index: u32,
    pub(super) physical_order_number: Option<i32>,
    pub(super) wavelength_unit: String,
    pub(super) wavelengths: Vec<f64>,
    pub(super) flux: Vec<f64>,
    pub(super) valid_fraction: Vec<f32>,
    pub(super) saturated: Vec<bool>,
    pub(super) total_samples: u32,
    pub(super) covered_samples: u32,
    pub(super) saturated_samples: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct EchelleMergedPreview {
    pub(super) wavelength_unit: String,
    pub(super) wavelengths: Vec<f64>,
    pub(super) flux: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct EchelleExtractionPreview {
    pub(super) frame_number: u64,
    pub(super) profile_display_name: String,
    pub(super) profile_id: Option<String>,
    pub(super) orders: Vec<EchelleOrderPreview>,
    pub(super) merged: Option<EchelleMergedPreview>,
}

impl EchelleExtractionPreview {
    pub(super) fn to_measurements(&self) -> Vec<Measurement> {
        let mut measurements =
            Vec::with_capacity(self.orders.len() + usize::from(self.merged.is_some()));
        let timestamp = Utc::now();

        for order in &self.orders {
            #[allow(clippy::cast_precision_loss)]
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
            measurements.push(Measurement::Spectrum {
                name: format!("echelle_order_{}", order.relative_index),
                frequencies: order.wavelengths.clone(),
                amplitudes: order.flux.clone(),
                frequency_unit: Some(order.wavelength_unit.clone()),
                amplitude_unit: Some("counts".to_string()),
                metadata: Some(json!({
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
                    "preview": true
                })),
                timestamp,
            });
        }

        if let Some(merged) = &self.merged {
            measurements.push(Measurement::Spectrum {
                name: "echelle_merged_preview".to_string(),
                frequencies: merged.wavelengths.clone(),
                amplitudes: merged.flux.clone(),
                frequency_unit: Some(merged.wavelength_unit.clone()),
                amplitude_unit: Some("counts".to_string()),
                metadata: Some(json!({
                    "kind": "echelle_merged",
                    "profile_id": self.profile_id,
                    "profile_display_name": self.profile_display_name,
                    "frame_number": self.frame_number,
                    "preview": true
                })),
                timestamp,
            });
        }

        measurements
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DecodedIntensityFrame<'a> {
    width: u32,
    height: u32,
    bit_depth: u32,
    pixels: DecodedPixels<'a>,
}

#[derive(Debug, Clone, PartialEq)]
enum DecodedPixels<'a> {
    U8(&'a [u8]),
    U16Borrowed(&'a [u16]),
    U16Owned(Vec<u16>),
}

impl<'a> DecodedIntensityFrame<'a> {
    pub(super) fn decode(
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
            bit_depth,
            pixels,
        })
    }

    #[inline]
    pub(super) fn width(&self) -> u32 {
        self.width
    }

    #[inline]
    pub(super) fn height(&self) -> u32 {
        self.height
    }

    #[inline]
    pub(super) fn get(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = (y * self.width + x) as usize;
        match &self.pixels {
            DecodedPixels::U8(p) => p.get(idx).copied().map(u32::from),
            DecodedPixels::U16Borrowed(p) => p.get(idx).copied().map(u32::from),
            DecodedPixels::U16Owned(p) => p.get(idx).copied().map(u32::from),
        }
    }
}

pub(super) fn extract_preview(
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

pub(super) fn extract_preview_masked(
    profile: &EchelleCalibrationProfile,
    frame_data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
    mask: Option<&BadPixelMask>,
) -> Result<EchelleExtractionPreview, String> {
    profile
        .validate_for_frame(EchelleFrameContext {
            width,
            height,
            bit_depth: Some(bit_depth),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;

    let decoded = DecodedIntensityFrame::decode(frame_data, width, height, bit_depth, None)?;
    extract_preview_with_scratch_inner(profile, &decoded, bit_depth, frame_number, mask)
}

pub(super) fn extract_preview_with_u16_scratch(
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
pub(super) fn extract_preview_with_u16_scratch_masked(
    profile: &EchelleCalibrationProfile,
    frame_data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    frame_number: u64,
    u16_scratch: &mut Vec<u16>,
    mask: Option<&BadPixelMask>,
) -> Result<EchelleExtractionPreview, String> {
    profile
        .validate_for_frame(EchelleFrameContext {
            width,
            height,
            bit_depth: Some(bit_depth),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;

    let decoded =
        DecodedIntensityFrame::decode(frame_data, width, height, bit_depth, Some(u16_scratch))?;
    extract_preview_with_scratch_inner(profile, &decoded, bit_depth, frame_number, mask)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub(super) fn order_sample_image_position(
    profile: &EchelleCalibrationProfile,
    order: &EchelleOrderCalibration,
    sample_index: usize,
) -> Option<(f32, f32)> {
    #[allow(clippy::cast_possible_truncation)]
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

    let mut orders = Vec::new();
    for order in profile.orders.iter().filter(|o| o.enabled) {
        let preview = extract_order(profile, order, decoded, saturation_threshold, mask)?;
        orders.push(preview);
    }

    let merged = build_merged_preview(&orders);
    Ok(EchelleExtractionPreview {
        frame_number,
        profile_display_name: profile.display_name.clone(),
        profile_id: profile.profile_id.clone(),
        orders,
        merged,
    })
}

#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
fn extract_order(
    profile: &EchelleCalibrationProfile,
    order: &EchelleOrderCalibration,
    frame: &DecodedIntensityFrame<'_>,
    saturation_threshold: u32,
    mask: Option<&BadPixelMask>,
) -> Result<EchelleOrderPreview, String> {
    let half_width = order
        .aperture_half_width_px
        .unwrap_or(profile.extraction.default_aperture_half_width_px);
    #[allow(clippy::cast_possible_truncation)]
    let radius = half_width.max(0.0).floor() as i32;

    #[allow(clippy::cast_sign_loss)]
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
        let center_sensor = eval_trace(&order.trace, sample_sensor).map_err(|e| {
            format!(
                "order {} trace evaluation failed at sample {}: {}",
                order.relative_index, sample_local, e
            )
        })?;
        let center_local = center_sensor - roi_cross_offset;

        let wavelength = eval_wavelength(
            &order.wavelength,
            sample_sensor,
            (sample_local - order.sample_start) as usize,
        )
        .map_err(|e| {
            format!(
                "order {} wavelength evaluation failed at sample {}: {}",
                order.relative_index, sample_local, e
            )
        })?;
        wavelengths.push(wavelength.0);

        #[allow(clippy::cast_possible_truncation)]
        let center_px = center_local.round() as i32;
        let mut sample_sum = 0.0f64;
        let mut valid = 0u32;
        #[allow(clippy::cast_precision_loss)]
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
                            sample_sum = f64::from(pixel);
                            valid = 1;
                            saturated_sample = pixel >= saturation_threshold;
                        }
                    }
                }
            }
            EchelleSummationMode::SimpleSum
            | EchelleSummationMode::SqrtWeightedSum
            | EchelleSummationMode::Optimal => {
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
                            sample_sum += f64::from(pixel);
                            valid += 1;
                            saturated_sample |= pixel >= saturation_threshold;
                        }
                    }
                }

                if valid > 0 {
                    if let Some(bg_cfg) = &profile.extraction.background {
                        if bg_cfg.enabled {
                            #[allow(clippy::cast_possible_wrap)]
                            let (bg_sum, bg_count) = sample_background_sidebands(
                                profile,
                                mask,
                                frame,
                                sample_local as i32,
                                center_px,
                                radius,
                                bg_cfg.inter_order_gap_min_px as i32,
                                bg_cfg.baseline_window_px as i32,
                            );
                            if bg_count > 0 {
                                let bg_mean = bg_sum / f64::from(bg_count);
                                sample_sum -= bg_mean * f64::from(valid);
                            }
                        }
                    }
                }
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
        let frac = valid as f32 / planned_span as f32;
        valid_fraction.push(frac);
        saturated.push(saturated_sample);
    }

    Ok(EchelleOrderPreview {
        relative_index: order.relative_index,
        physical_order_number: order.physical_order_number,
        wavelength_unit: wavelength_unit(&order.wavelength).to_string(),
        total_samples: order.sample_end - order.sample_start + 1,
        covered_samples,
        saturated_samples,
        wavelengths,
        flux,
        valid_fraction,
        saturated,
    })
}

#[allow(clippy::cast_sign_loss, clippy::too_many_arguments)]
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

    let mut bg_sum = 0.0f64;
    let mut bg_count = 0u32;
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
                    bg_sum += f64::from(pixel);
                    bg_count += 1;
                }
            }
        }
    }

    (bg_sum, bg_count)
}

fn build_merged_preview(orders: &[EchelleOrderPreview]) -> Option<EchelleMergedPreview> {
    let first_unit = orders.first()?.wavelength_unit.clone();
    if orders.iter().any(|o| o.wavelength_unit != first_unit) {
        return None;
    }

    let mut samples: Vec<(f64, f64)> = Vec::new();
    for order in orders {
        samples.extend(
            order
                .wavelengths
                .iter()
                .copied()
                .zip(order.flux.iter().copied())
                .filter(|(w, _)| w.is_finite()),
        );
    }
    samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (wavelengths, flux): (Vec<_>, Vec<_>) = samples.into_iter().unzip();

    Some(EchelleMergedPreview {
        wavelength_unit: first_unit,
        wavelengths,
        flux,
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
        .any(|r| point_in_region(x, y, r))
        || mask.is_some_and(|m| m.is_masked(x, y))
}

#[inline]
fn point_in_region(x: u32, y: u32, r: &PixelRegion) -> bool {
    x >= r.x && x < r.x.saturating_add(r.width) && y >= r.y && y < r.y.saturating_add(r.height)
}

fn wavelength_unit(w: &EchelleWavelengthModel) -> &str {
    match w {
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
            .map(|w| (w, unit.as_str()))
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
            let mut acc = 0.0f64;
            for &c in coefficients.iter().rev() {
                acc = acc * x + c;
            }
            acc
        }
        PolynomialBasis::Chebyshev => {
            let t = ((2.0 * (x - domain_start)) / (domain_end - domain_start)) - 1.0;
            if coefficients.len() == 1 {
                return Ok(coefficients[0]);
            }
            let mut t0 = 1.0f64;
            let mut t1 = t;
            let mut acc = coefficients[0] * t0 + coefficients[1] * t1;
            for &c in coefficients.iter().skip(2) {
                let tn = 2.0 * t * t1 - t0;
                acc += c * tn;
                t0 = t1;
                t1 = tn;
            }
            acc
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::image_viewer::echelle_sidecar::EchelleSidecarRunner;
    use crate::panels::image_viewer::processing::get_pixel_value_inline;
    use chrono::Utc;
    use common::echelle::{
        AxisDirection, DetectorAxis, EchelleBackgroundConfig, EchelleCorrections,
        EchelleExtractionConfig, EchelleFrameCompatibility, EchelleOrientation, EchelleProvenance,
        EchelleSchemaVersion, EchelleTraceModel, EchelleWavelengthModel,
    };
    use protocol::compression::decompress_frame;
    use protocol::daq::{CompressionType, FrameData};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::time::Duration;

    #[test]
    fn decoded_frame_matches_existing_pixel_helper_for_8_and_16_bit() {
        let data8 = vec![1u8, 2, 3, 4, 5, 6];
        let decoded8 = DecodedIntensityFrame::decode(&data8, 3, 2, 8, None).unwrap();
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(
                    decoded8.get(x, y),
                    get_pixel_value_inline(&data8, x, y, 3, 2, 8)
                );
            }
        }

        let data16 = vec![
            1, 0, 2, 0, 3, 0, //
            4, 0, 5, 0, 6, 0, //
        ];
        let decoded16 = DecodedIntensityFrame::decode(&data16, 3, 2, 16, None).unwrap();
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(
                    decoded16.get(x, y),
                    get_pixel_value_inline(&data16, x, y, 3, 2, 16)
                );
            }
        }
    }

    #[test]
    fn extracts_simple_sum_order_and_builds_merged_preview() {
        let width = 8u32;
        let height = 6u32;
        let mut image = vec![0u8; (width * height) as usize];

        // Constant trace centered at y=3 with aperture radius=1 => sum rows 2,3,4.
        for x in 0..width {
            #[allow(clippy::cast_possible_truncation)]
            let v = (x + 1) as u8;
            for y in 2..=4 {
                image[(y * width + x) as usize] = v;
            }
        }

        let profile = synthetic_profile(width, height, 0, width - 1, 3.0, 1.0, None);
        let extracted = extract_preview(&profile, &image, width, height, 8, 42).unwrap();

        assert_eq!(extracted.frame_number, 42);
        assert_eq!(extracted.orders.len(), 1);
        let order = &extracted.orders[0];
        assert_eq!(order.total_samples, width);
        assert_eq!(order.covered_samples, width);
        assert_eq!(order.saturated_samples, 0);
        assert_eq!(order.wavelength_unit, "nm");
        assert_eq!(order.wavelengths.len(), width as usize);
        assert_eq!(order.flux.len(), width as usize);
        assert_eq!(order.flux[0], 3.0);
        assert_eq!(order.flux[7], 24.0);
        assert!(order.valid_fraction.iter().all(|&f| (f - 1.0).abs() < 1e-6));

        let merged = extracted.merged.as_ref().unwrap();
        assert_eq!(merged.wavelengths.len(), width as usize);
        assert_eq!(merged.flux[0], 3.0);

        let measurements = extracted.to_measurements();
        assert_eq!(measurements.len(), 2);
    }

    #[test]
    fn extraction_respects_excluded_regions_and_reports_partial_coverage() {
        let width = 4u32;
        let height = 5u32;
        let mut image = vec![0u8; (width * height) as usize];
        for x in 0..width {
            for y in 1..=3 {
                image[(y * width + x) as usize] = 10;
            }
        }

        let excluded = PixelRegion {
            x: 2,
            y: 2,
            width: 1,
            height: 1,
        };
        let profile = synthetic_profile(width, height, 0, width - 1, 2.0, 1.0, Some(excluded));
        let extracted = extract_preview(&profile, &image, width, height, 8, 1).unwrap();
        let order = &extracted.orders[0];

        assert_eq!(order.flux[2], 20.0); // middle sample loses one pixel from 3-pixel aperture
        assert!(order.valid_fraction[2] < 1.0);
        assert!(order.covered_samples >= 1);
    }

    #[test]
    fn extraction_applies_optional_background_sideband_subtraction() {
        let width = 6u32;
        let height = 9u32;
        let mut image = vec![0u8; (width * height) as usize];

        // Signal aperture rows 3..=5 at value 20, background sidebands rows 1,2,6,7 at value 5.
        for x in 0..width {
            for y in 3..=5 {
                image[(y * width + x) as usize] = 20;
            }
            for y in [1u32, 2, 6, 7] {
                image[(y * width + x) as usize] = 5;
            }
        }

        let mut profile = synthetic_profile(width, height, 0, width - 1, 4.0, 1.0, None);
        if let Some(bg) = &mut profile.extraction.background {
            bg.enabled = true;
            bg.inter_order_gap_min_px = 1;
            bg.baseline_window_px = 2;
        }

        let extracted = extract_preview(&profile, &image, width, height, 8, 1).unwrap();
        let order = &extracted.orders[0];

        // Raw signal sum is 3 * 20 = 60. Background mean from sidebands is 5, subtract 3 * 5 = 15.
        assert_eq!(order.flux[0], 45.0);
        assert_eq!(order.flux[5], 45.0);
    }

    #[test]
    fn extraction_marks_saturated_samples() {
        let width = 4u32;
        let height = 5u32;
        let mut image = vec![0u8; (width * height) as usize];
        for x in 0..width {
            for y in 1..=3 {
                image[(y * width + x) as usize] = 255;
            }
        }

        let profile = synthetic_profile(width, height, 0, width - 1, 2.0, 1.0, None);
        let extracted = extract_preview(&profile, &image, width, height, 8, 7).unwrap();
        let order = &extracted.orders[0];
        assert_eq!(order.saturated_samples, width);
        assert!(order.saturated.iter().all(|&s| s));
    }

    #[test]
    fn extraction_returns_empty_preview_when_all_orders_disabled() {
        let width = 4u32;
        let height = 5u32;
        let image = vec![0u8; (width * height) as usize];
        let mut profile = synthetic_profile(width, height, 0, width - 1, 2.0, 1.0, None);
        profile.orders[0].enabled = false;

        let extracted = extract_preview(&profile, &image, width, height, 8, 9).unwrap();
        assert!(extracted.orders.is_empty());
        assert!(extracted.merged.is_none());
        assert!(extracted.to_measurements().is_empty());
    }

    #[test]
    fn real_canned_hg2_frames_decompress_and_extract() {
        for label in ["hg2_001ms", "hg2_010ms", "hg2_100ms"] {
            let (payload, width, height, bit_depth, uncompressed_size) =
                load_real_canned_frame(label);

            let mut frame = FrameData {
                device_id: "istar_camera".to_string(),
                width,
                height,
                bit_depth,
                data: payload,
                frame_number: 0,
                timestamp_ns: 0,
                compression: CompressionType::CompressionLz4 as i32,
                uncompressed_size,
                ..Default::default()
            };
            decompress_frame(&mut frame).expect("LZ4 frame payload should decompress");

            assert_eq!(frame.width, 2048);
            assert_eq!(frame.height, 2048);
            assert_eq!(frame.bit_depth, 12);
            assert_eq!(frame.data.len(), uncompressed_size as usize);

            let mut profile =
                synthetic_profile(frame.width, frame.height, 1000, 1015, 1024.0, 2.0, None);
            profile.compatibility.bit_depth = Some(12);
            let extracted = extract_preview(
                &profile,
                &frame.data,
                frame.width,
                frame.height,
                frame.bit_depth,
                0,
            )
            .expect("real canned frame should pass extraction path");
            assert_eq!(extracted.orders.len(), 1);
            assert_eq!(extracted.orders[0].flux.len(), 16);
        }
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn real_canned_hg2_reference_regression_matches_declared_tolerances() {
        let dataset_dir = real_hg2_dataset_dir();
        let reference_dir = dataset_dir.join("reference");

        let tolerances = read_json_value(reference_dir.join("comparison_tolerances.json"));
        let capture_diagnostics = read_json_value(reference_dir.join("capture_diagnostics.json"));
        let dataset_index = read_json_value(reference_dir.join("dataset_reference_index.json"));

        assert_eq!(
            tolerances["dataset_id"].as_str(),
            Some("leabs-dev/2026-02-25-hg2")
        );
        assert_eq!(
            tolerances["dataset_classification"].as_str(),
            Some("diagnostic_ramp_like")
        );
        assert_eq!(
            tolerances["spectroscopic_truth_usable"].as_bool(),
            Some(false)
        );

        let raw_expectations = &tolerances["raw_frame_expectations"];
        #[allow(clippy::cast_possible_truncation)]
        let expected_width = raw_expectations["width"].as_u64().unwrap() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let expected_height = raw_expectations["height"].as_u64().unwrap() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let expected_bit_depth = raw_expectations["bit_depth_nominal"].as_u64().unwrap() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let expected_uncompressed = raw_expectations["uncompressed_size_bytes"]
            .as_u64()
            .unwrap() as u32;
        let expected_compression = raw_expectations["compression"].as_str().unwrap();

        let extraction_expectations = &tolerances["reference_extraction_expectations"];
        let expected_order_count = extraction_expectations["expected_order_count"]
            .as_u64()
            .unwrap();
        let expected_quality = extraction_expectations["expected_quality_classification"]
            .as_str()
            .unwrap();

        let labels = ["hg2_001ms", "hg2_010ms", "hg2_100ms"];
        let mut summaries_by_label = BTreeMap::new();
        for label in labels {
            let summary = read_json_value(dataset_dir.join(format!("{label}_summary.json")));
            assert_eq!(
                summary["frame"]["width"].as_u64(),
                Some(u64::from(expected_width))
            );
            assert_eq!(
                summary["frame"]["height"].as_u64(),
                Some(u64::from(expected_height))
            );
            assert_eq!(
                summary["frame"]["bit_depth"].as_u64(),
                Some(u64::from(expected_bit_depth))
            );
            assert_eq!(
                summary["frame"]["uncompressed_size"].as_u64(),
                Some(u64::from(expected_uncompressed))
            );
            assert_eq!(
                summary["frame"]["compression"].as_str(),
                Some(expected_compression)
            );

            let ref_summary =
                read_json_value(reference_dir.join(format!("{label}_reference_summary.json")));
            assert_eq!(ref_summary["label"].as_str(), Some(label));
            assert_eq!(
                ref_summary["dataset_quality_classification"]["kind"].as_str(),
                Some(expected_quality)
            );
            assert_eq!(
                ref_summary["dataset_quality_classification"]["usable_for_spectroscopic_truth"]
                    .as_bool(),
                Some(false)
            );
            let n_orders = ref_summary["order_detection"]["n_orders"].as_u64().unwrap();
            assert_eq!(n_orders, expected_order_count);
            assert_eq!(
                ref_summary["per_order_summary"]
                    .as_array()
                    .map(|x| x.len())
                    .unwrap_or_default(),
                n_orders as usize
            );
            assert_eq!(
                ref_summary["image_diagnostics"]["sampled_additive_ramp_match"].as_bool(),
                Some(true)
            );
            assert_eq!(
                ref_summary["image_diagnostics"]["likely_additive_ramp_pattern"].as_bool(),
                Some(true)
            );
            summaries_by_label.insert(label.to_string(), ref_summary);
        }

        let indexed = dataset_index["captures"].as_array().unwrap();
        assert_eq!(indexed.len(), labels.len());
        for item in indexed {
            let label = item["label"].as_str().unwrap();
            let n_orders = item["order_detection"]["n_orders"].as_u64().unwrap();
            assert_eq!(n_orders, expected_order_count);
            assert!(summaries_by_label.contains_key(label));
        }

        let capture_diag_map = capture_diagnostics["captures"].as_object().unwrap();
        for label in labels {
            let diag = &capture_diag_map[label];
            assert_eq!(diag["sampled_additive_ramp_match"].as_bool(), Some(true));
            assert_eq!(diag["likely_additive_ramp_pattern"].as_bool(), Some(true));
            assert_close(
                diag["residual_to_image_stddev_ratio"].as_f64().unwrap(),
                0.0,
                0.0,
            );
        }

        let pairwise_actual = capture_diagnostics["pairwise_differences"]
            .as_array()
            .unwrap();
        let mut pairwise_by_key: BTreeMap<(String, String), &Value> = BTreeMap::new();
        for item in pairwise_actual {
            let a = item["a"].as_str().unwrap().to_string();
            let b = item["b"].as_str().unwrap().to_string();
            pairwise_by_key.insert((a, b), item);
        }

        for spec in tolerances["pairwise_frame_tolerances"].as_array().unwrap() {
            let key = (
                spec["a"].as_str().unwrap().to_string(),
                spec["b"].as_str().unwrap().to_string(),
            );
            let actual = pairwise_by_key
                .get(&key)
                .unwrap_or_else(|| panic!("missing pairwise diagnostics entry for {:?}", key));

            #[allow(clippy::cast_possible_wrap)]
            let max_abs_target = spec["max_abs_diff"]["target"].as_u64().unwrap() as i64;
            #[allow(clippy::cast_possible_wrap)]
            let max_abs_tol = spec["max_abs_diff"]["tolerance"].as_u64().unwrap() as i64;
            let max_abs_actual = actual["max_abs_diff"].as_i64().unwrap();
            assert!(
                (max_abs_actual - max_abs_target).abs() <= max_abs_tol,
                "pair {:?} max_abs_diff actual={} target={} tol={}",
                key,
                max_abs_actual,
                max_abs_target,
                max_abs_tol
            );

            let mean_target = spec["mean_diff"]["target"].as_f64().unwrap();
            let mean_tol = spec["mean_diff"]["abs_tolerance"].as_f64().unwrap();
            let mean_actual = actual["mean_diff"].as_f64().unwrap();
            assert_close_with_context(&key, "mean_diff", mean_actual, mean_target, mean_tol);

            let p99_target = spec["p99_abs_diff"]["target"].as_f64().unwrap();
            let p99_tol = spec["p99_abs_diff"]["abs_tolerance"].as_f64().unwrap();
            let p99_actual = actual["p99_abs_diff"].as_f64().unwrap();
            assert_close_with_context(&key, "p99_abs_diff", p99_actual, p99_target, p99_tol);
        }
    }

    #[test]
    fn fixture_sidecar_offline_hg2_compares_to_rust_mvp_path() {
        // Prototype sidecar tests rely on Python being available locally.
        let python_available = std::process::Command::new("/usr/bin/env")
            .arg("python3")
            .arg("--version")
            .output()
            .is_ok();
        if !python_available {
            eprintln!("skipping fixture sidecar test: python3 unavailable");
            return;
        }

        let dataset_dir = real_hg2_dataset_dir();
        let script_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/echelle/fixture_sidecar_hg2.py");
        let runner = EchelleSidecarRunner::new(
            "/usr/bin/env",
            [
                "python3".to_string(),
                script_path.display().to_string(),
                "--dataset-dir".to_string(),
                dataset_dir.display().to_string(),
            ],
        )
        .with_timeout(Duration::from_secs(2));

        let health = runner
            .health_check()
            .expect("fixture sidecar health should work");
        assert_eq!(health.response["ok"], true);
        assert_eq!(health.response["result"]["service"], "fixture_sidecar_hg2");

        for label in ["hg2_001ms", "hg2_010ms", "hg2_100ms"] {
            let sidecar = runner
                .request_json(&serde_json::json!({
                    "request_id": format!("offline-{label}"),
                    "op": "extract_offline_fixture",
                    "label": label
                }))
                .expect("fixture sidecar extraction should succeed");
            assert_eq!(sidecar.response["ok"], true);

            let result = &sidecar.response["result"];
            assert_eq!(
                result["quality"]["dataset_classification"],
                "diagnostic_ramp_like"
            );
            assert_eq!(result["quality"]["spectroscopic_truth_usable"], false);
            assert_eq!(result["quality"]["reference_order_count"], 0);
            assert_eq!(
                result["reference_summary"]["order_detection"]["n_orders"],
                0
            );
            assert_eq!(
                result["provenance"]["dataset_id"],
                "leabs-dev/2026-02-25-hg2"
            );
            assert_eq!(result["provenance"]["label"], label);

            let (payload, width, height, bit_depth, uncompressed_size) =
                load_real_canned_frame(label);
            let mut frame = FrameData {
                device_id: "istar_camera".to_string(),
                width,
                height,
                bit_depth,
                data: payload,
                frame_number: 0,
                timestamp_ns: 0,
                compression: CompressionType::CompressionLz4 as i32,
                uncompressed_size,
                ..Default::default()
            };
            decompress_frame(&mut frame).expect("LZ4 frame payload should decompress");

            let pixels_u16 = frame
                .data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>();
            #[allow(clippy::cast_precision_loss)]
            let mean =
                pixels_u16.iter().map(|&v| f64::from(v)).sum::<f64>() / pixels_u16.len() as f64;
            let max = pixels_u16.iter().copied().max().unwrap_or_default();

            let side_diag = &result["quality"]["capture_image_diagnostics"];
            assert_close(mean, side_diag["mean"].as_f64().unwrap(), 0.0);
            assert_eq!(u64::from(max), side_diag["max"].as_u64().unwrap());
            assert_eq!(side_diag["likely_additive_ramp_pattern"], true);

            // Compare sidecar offline output to Rust MVP extraction on the same frame bytes.
            // The current fixture is diagnostic-ramp-like, so the sidecar (reference-tool-backed)
            // returns zero detected orders while the Rust path can still extract a forced preview
            // when given an explicit synthetic profile.
            let mut profile =
                synthetic_profile(frame.width, frame.height, 1000, 1015, 1024.0, 2.0, None);
            profile.compatibility.bit_depth = Some(12);
            let rust_preview = extract_preview(
                &profile,
                &frame.data,
                frame.width,
                frame.height,
                frame.bit_depth,
                0,
            )
            .expect("Rust MVP extraction should succeed on canned frame");
            assert_eq!(rust_preview.orders.len(), 1);
            assert_eq!(rust_preview.orders[0].flux.len(), 16);
            assert!(rust_preview.orders[0].flux.iter().any(|&v| v > 0.0));
        }
    }

    #[test]
    #[ignore = "benchmark harness; run manually to collect latency/throughput metrics"]
    #[allow(clippy::cast_precision_loss)]
    fn benchmark_real_canned_hg2_extraction_latency_and_live_budget() {
        let labels = ["hg2_001ms", "hg2_010ms", "hg2_100ms"];

        struct BenchFrame {
            label: &'static str,
            compressed_payload: Vec<u8>,
            width: u32,
            height: u32,
            bit_depth: u32,
            uncompressed_size: u32,
            profile: EchelleCalibrationProfile,
        }

        let frames = labels
            .iter()
            .map(|&label| {
                let (payload, width, height, bit_depth, uncompressed_size) =
                    load_real_canned_frame(label);
                let mut profile = synthetic_profile(width, height, 1000, 1015, 1024.0, 2.0, None);
                profile.compatibility.bit_depth = Some(12);
                BenchFrame {
                    label,
                    compressed_payload: payload,
                    width,
                    height,
                    bit_depth,
                    uncompressed_size,
                    profile,
                }
            })
            .collect::<Vec<_>>();

        let iterations = 40usize;
        let mut decode_extract_us = Vec::<f64>::with_capacity(iterations * frames.len());
        let mut extract_only_us = Vec::<f64>::with_capacity(iterations * frames.len());
        let mut per_label_extract_us: BTreeMap<String, Vec<f64>> = BTreeMap::new();

        for _ in 0..iterations {
            for bench in &frames {
                let mut frame = FrameData {
                    device_id: "istar_camera".to_string(),
                    width: bench.width,
                    height: bench.height,
                    bit_depth: bench.bit_depth,
                    data: bench.compressed_payload.clone(),
                    frame_number: 0,
                    timestamp_ns: 0,
                    compression: CompressionType::CompressionLz4 as i32,
                    uncompressed_size: bench.uncompressed_size,
                    ..Default::default()
                };

                let t0 = std::time::Instant::now();
                decompress_frame(&mut frame).expect("benchmark frame should decompress");
                let t1 = std::time::Instant::now();
                let preview = extract_preview(
                    &bench.profile,
                    &frame.data,
                    frame.width,
                    frame.height,
                    frame.bit_depth,
                    0,
                )
                .expect("benchmark extraction should succeed");
                let t2 = std::time::Instant::now();

                assert_eq!(preview.orders.len(), 1);
                let extract_us = (t2 - t1).as_secs_f64() * 1_000_000.0;
                let total_us = (t2 - t0).as_secs_f64() * 1_000_000.0;
                extract_only_us.push(extract_us);
                decode_extract_us.push(total_us);
                per_label_extract_us
                    .entry(bench.label.to_string())
                    .or_default()
                    .push(extract_us);
            }
        }

        let extract_stats = latency_stats(&extract_only_us);
        let total_stats = latency_stats(&decode_extract_us);
        #[allow(clippy::cast_precision_loss)]
        let frames_processed = (iterations * frames.len()) as f64;
        let total_elapsed_s = decode_extract_us.iter().sum::<f64>() / 1_000_000.0;
        let throughput_fps = if total_elapsed_s > 0.0 {
            frames_processed / total_elapsed_s
        } else {
            0.0
        };

        let live_budget_sim = [5.0, 10.0, 30.0]
            .iter()
            .map(|&fps| {
                let budget_us = 1_000_000.0 / fps;
                let over_budget = decode_extract_us
                    .iter()
                    .filter(|&&us| us > budget_us)
                    .count();
                serde_json::json!({
                    "target_fps": fps,
                    "frame_budget_us": budget_us,
                    "over_budget_frames": over_budget,
                    "over_budget_fraction": over_budget as f64 / decode_extract_us.len() as f64,
                    "p95_decode_plus_extract_fits_budget": total_stats.p95_us <= budget_us,
                })
            })
            .collect::<Vec<_>>();

        let per_label = per_label_extract_us
            .into_iter()
            .map(|(label, samples)| {
                let s = latency_stats(&samples);
                (
                    label,
                    serde_json::json!({
                        "count": samples.len(),
                        "mean_us": s.mean_us,
                        "p50_us": s.p50_us,
                        "p95_us": s.p95_us,
                        "p99_us": s.p99_us,
                        "max_us": s.max_us,
                    }),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let report = serde_json::json!({
            "dataset_id": "leabs-dev/2026-02-25-hg2",
            "harness": "ui::echelle_extraction benchmark_real_canned_hg2_extraction_latency_and_live_budget",
            "iterations_per_capture": iterations,
            "captures": labels,
            "samples_total": decode_extract_us.len(),
            "throughput_frames_per_sec_decode_plus_extract": throughput_fps,
            "decode_plus_extract_us": {
                "mean": total_stats.mean_us,
                "p50": total_stats.p50_us,
                "p95": total_stats.p95_us,
                "p99": total_stats.p99_us,
                "max": total_stats.max_us,
            },
            "extract_only_us": {
                "mean": extract_stats.mean_us,
                "p50": extract_stats.p50_us,
                "p95": extract_stats.p95_us,
                "p99": extract_stats.p99_us,
                "max": extract_stats.max_us,
            },
            "per_capture_extract_only_us": per_label,
            "live_budget_simulation": live_budget_sim,
            "notes": [
                "Bench uses real canned Hg2 frames from leabs-dev capture set.",
                "Current fixture is diagnostic-ramp-like; benchmark validates runtime characteristics, not spectroscopic correctness.",
                "UI responsiveness is approximated by frame-budget simulation using decode+extract timings."
            ]
        });

        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }

    fn synthetic_profile(
        width: u32,
        height: u32,
        sample_start: u32,
        sample_end: u32,
        trace_center_y: f64,
        aperture_half_width_px: f64,
        excluded_region: Option<PixelRegion>,
    ) -> EchelleCalibrationProfile {
        let sample_count = (sample_end - sample_start + 1) as usize;
        #[allow(clippy::cast_precision_loss)]
        let wavelengths = (0..sample_count)
            .map(|i| 500.0 + i as f64 * 0.1)
            .collect::<Vec<_>>();

        EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: Some("synthetic-test".to_string()),
            display_name: "Synthetic".to_string(),
            compatibility: EchelleFrameCompatibility {
                sensor_width: width,
                sensor_height: height,
                frame_width: width,
                frame_height: height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(8),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Positive,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: aperture_half_width_px,
                background: Some(EchelleBackgroundConfig {
                    enabled: false,
                    inter_order_gap_min_px: 3,
                    baseline_window_px: 7,
                }),
            },
            orders: vec![EchelleOrderCalibration {
                relative_index: 0,
                physical_order_number: Some(50),
                sample_start,
                sample_end,
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![trace_center_y],
                    domain_start: 0.0,
                    domain_end: f64::from(width - 1),
                },
                wavelength: EchelleWavelengthModel::Sampled {
                    wavelengths,
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: Some(aperture_half_width_px),
                enabled: true,
                notes: None,
            }],
            corrections: EchelleCorrections {
                excluded_regions: excluded_region.into_iter().collect(),
                ..Default::default()
            },
            provenance: EchelleProvenance {
                creator_tool: "ui-test".to_string(),
                creator_version: Some("0".to_string()),
                created_at_utc: Utc::now(),
                source_frame_ids: vec![],
                notes: None,
            },
        }
    }

    fn load_real_canned_frame(label: &str) -> (Vec<u8>, u32, u32, u32, u32) {
        let dir = real_hg2_dataset_dir();
        let summary_path = dir.join(format!("{label}_summary.json"));
        let payload_path = dir.join(format!("{label}_payload_lz4.bin"));

        let summary: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&summary_path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", summary_path.display())),
        )
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", summary_path.display()));

        let frame = &summary["frame"];
        #[allow(clippy::cast_possible_truncation)]
        let width = frame["width"].as_u64().unwrap() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let height = frame["height"].as_u64().unwrap() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let bit_depth = frame["bit_depth"].as_u64().unwrap() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let uncompressed_size = frame["uncompressed_size"].as_u64().unwrap() as u32;
        let payload = std::fs::read(&payload_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", payload_path.display()));

        (payload, width, height, bit_depth, uncompressed_size)
    }

    fn real_hg2_dataset_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/echelle/leabs-dev/2026-02-25-hg2")
    }

    fn read_json_value(path: std::path::PathBuf) -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display())),
        )
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
    }

    fn assert_close(actual: f64, expected: f64, tol: f64) {
        assert!(
            (actual - expected).abs() <= tol,
            "actual={} expected={} tol={}",
            actual,
            expected,
            tol
        );
    }

    fn assert_close_with_context(
        key: &(String, String),
        field: &str,
        actual: f64,
        expected: f64,
        tol: f64,
    ) {
        assert!(
            (actual - expected).abs() <= tol,
            "pair {:?} field {} actual={} expected={} tol={}",
            key,
            field,
            actual,
            expected,
            tol
        );
    }

    #[derive(Debug, Clone, Copy)]
    struct LatencyStats {
        mean_us: f64,
        p50_us: f64,
        p95_us: f64,
        p99_us: f64,
        max_us: f64,
    }

    fn latency_stats(samples: &[f64]) -> LatencyStats {
        assert!(
            !samples.is_empty(),
            "latency_stats requires non-empty samples"
        );
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        #[allow(clippy::cast_precision_loss)]
        let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
        let pct = |p: f64| -> f64 {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                clippy::cast_sign_loss
            )]
            let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
            sorted[idx]
        };
        LatencyStats {
            mean_us: mean,
            p50_us: pct(0.50),
            p95_us: pct(0.95),
            p99_us: pct(0.99),
            max_us: *sorted.last().unwrap(),
        }
    }
}
