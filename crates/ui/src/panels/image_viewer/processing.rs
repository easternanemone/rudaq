//! Frame-to-RGBA image processing pipeline.

use super::colormap::{Colormap, ContrastMode, ScaleMode};
use std::sync::Arc;

/// Request for background RGBA conversion (bd-xifj, bd-j6xm)
pub(super) struct RgbaConversionRequest {
    /// Raw frame data (Arc for zero-copy sharing)
    pub(super) data: Arc<[u8]>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) bit_depth: u32,
    pub(super) frame_number: u64,
    /// Display parameters for conversion
    pub(super) colormap: Colormap,
    pub(super) scale_mode: ScaleMode,
    pub(super) display_min: f32,
    pub(super) display_max: f32,
    pub(super) auto_contrast: bool,
    /// Contrast enhancement mode (bd-j6xm)
    pub(super) contrast_mode: ContrastMode,
    /// Percentile thresholds for auto-percentile mode (bd-j6xm)
    pub(super) percentile_low: f32,
    pub(super) percentile_high: f32,
    /// Colorbar midpoint for gamma-like adjustment (bd-07j1)
    pub(super) colorbar_midpoint: f32,
}

/// Result of background RGBA conversion (bd-xifj)
pub(super) struct RgbaConversionResult {
    /// Converted RGBA data
    pub(super) rgba: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    /// Frame number for debugging and ordering (kept for future use)
    #[allow(dead_code)]
    pub(super) frame_number: u64,
    /// Computed display min/max (for auto-contrast feedback)
    pub(super) computed_min: f32,
    pub(super) computed_max: f32,
}

/// Helper function to get pixel value from frame data (bd-pgcb)
///
/// Used by crosshair feature. Free function to avoid borrow checker issues in closures.
pub(super) fn get_pixel_value_inline(
    frame_data: &[u8],
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    bit_depth: u32,
) -> Option<u32> {
    if x >= width || y >= height {
        return None;
    }

    let pixel_index = (y * width + x) as usize;

    match bit_depth {
        8 => {
            // 8-bit grayscale: 1 byte per pixel
            frame_data.get(pixel_index).map(|&b| u32::from(b))
        }
        12 | 16 => {
            // 12-bit or 16-bit: 2 bytes per pixel (little-endian)
            let byte_index = pixel_index * 2;
            if byte_index + 1 < frame_data.len() {
                let low = u32::from(frame_data[byte_index]);
                let high = u32::from(frame_data[byte_index + 1]);
                Some(low | (high << 8))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Convert raw frame data to RGBA, reusing the provided buffer (bd-wdx3, bd-xifj)
///
/// This is a free function that can be called from both the UI thread and background threads.
/// The buffer is resized as needed but not shrunk, avoiding allocations for same-size frames.
pub(super) fn convert_frame_to_rgba_into(
    req: &RgbaConversionRequest,
    buffer: &mut Vec<u8>,
) -> (f32, f32) {
    let width = req.width;
    let height = req.height;
    let bit_depth = req.bit_depth;
    let colormap = req.colormap;
    let scale_mode = req.scale_mode;
    let display_min = req.display_min;
    let display_max = req.display_max;
    let _auto_contrast = req.auto_contrast;
    let colorbar_midpoint = req.colorbar_midpoint;
    let data = &req.data;

    // Guard against zero or invalid dimensions
    if width == 0 || height == 0 {
        buffer.clear();
        return (0.0, 1.0);
    }

    // Enforce frame size limits (DoS protection).
    // Mirror common::limits constants (common is optional, only with pvcam feature).
    const MAX_FRAME_DIMENSION: u32 = 65_536;
    const MAX_FRAME_BYTES: usize = 100 * 1024 * 1024; // 100 MB
    if width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION {
        tracing::warn!(width, height, "Frame dimensions exceed limit");
        buffer.clear();
        return (0.0, 1.0);
    }

    let pixel_count = (width as usize).saturating_mul(height as usize);
    let bytes_per_pixel = if bit_depth > 8 { 2usize } else { 1 };
    let frame_bytes = pixel_count.saturating_mul(bytes_per_pixel);
    if frame_bytes > MAX_FRAME_BYTES {
        tracing::warn!(
            width,
            height,
            bit_depth,
            frame_bytes,
            "Frame exceeds byte size limit"
        );
        buffer.clear();
        return (0.0, 1.0);
    }
    let required_size = pixel_count * 4;

    // bd-wdx3: Resize buffer only when needed (grows but never shrinks during session)
    buffer.resize(required_size, 255); // Pre-fill alpha channel

    // Get the bit depth's max value for normalization
    let bit_max = match bit_depth {
        8 => 255.0f32,
        12 => 4095.0,
        16 => 65535.0,
        _ => 65535.0,
    };

    // Compute min/max and optional histogram equalization LUT (bd-j6xm)
    let (effective_min, effective_max, hist_lut) = match req.contrast_mode {
        ContrastMode::Manual => (display_min, display_max, None),
        ContrastMode::AutoSimple => {
            let (min, max) = compute_minmax_from_data(data, bit_depth, bit_max);
            (min, max, None)
        }
        ContrastMode::AutoPercentile => {
            let (min, max) = compute_percentile_minmax(
                data,
                bit_depth,
                bit_max,
                req.percentile_low,
                req.percentile_high,
            );
            (min, max, None)
        }
        ContrastMode::HistogramEq => {
            let histogram = build_histogram(data, bit_depth, 256);
            let lut = compute_histogram_equalization_lut(&histogram, pixel_count);
            (0.0, 1.0, Some(lut))
        }
        ContrastMode::Clahe => {
            let histogram = build_histogram(data, bit_depth, 256);
            let lut = compute_clahe_lut(&histogram, pixel_count, 2.0);
            (0.0, 1.0, Some(lut))
        }
    };

    // Compute contrast range (avoid division by zero)
    let range = (effective_max - effective_min).max(0.001);

    // Apply colorbar midpoint adjustment (bd-07j1)
    // Convert midpoint to gamma: gamma = -log(0.5) / log(midpoint)
    let gamma = if colorbar_midpoint > 0.0
        && colorbar_midpoint < 1.0
        && (colorbar_midpoint - 0.5).abs() > f32::EPSILON
    {
        -std::f32::consts::LN_2 / colorbar_midpoint.ln()
    } else {
        1.0 // Linear (no adjustment)
    };

    match bit_depth {
        8 => {
            // 8-bit grayscale
            for (i, &pixel) in data.iter().take(pixel_count).enumerate() {
                let normalized = f32::from(pixel) / bit_max;

                // Apply histogram equalization if LUT available, otherwise linear contrast
                let contrasted = if let Some(ref lut) = hist_lut {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let bin = ((normalized * 255.0) as usize).min(255);
                    lut[bin]
                } else {
                    ((normalized - effective_min) / range).clamp(0.0, 1.0)
                };

                let scaled = scale_mode.apply(contrasted);
                // Apply colorbar midpoint adjustment (bd-07j1)
                let adjusted = if (gamma - 1.0).abs() > f32::EPSILON {
                    scaled.powf(gamma).clamp(0.0, 1.0)
                } else {
                    scaled
                };
                let [r, g, b] = colormap.apply(adjusted);
                buffer[i * 4] = r;
                buffer[i * 4 + 1] = g;
                buffer[i * 4 + 2] = b;
                // Alpha already set to 255
            }
        }
        12 | 16 => {
            // 16-bit (or 12-bit stored as 16-bit) little-endian
            for i in 0..pixel_count {
                let byte_idx = i * 2;
                if byte_idx + 1 >= data.len() {
                    break;
                }
                let pixel = u16::from_le_bytes([data[byte_idx], data[byte_idx + 1]]);
                let normalized = f32::from(pixel) / bit_max;

                // Apply histogram equalization if LUT available, otherwise linear contrast
                let contrasted = if let Some(ref lut) = hist_lut {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let bin = ((normalized * 255.0) as usize).min(255);
                    lut[bin]
                } else {
                    ((normalized - effective_min) / range).clamp(0.0, 1.0)
                };

                let scaled = scale_mode.apply(contrasted);
                // Apply colorbar midpoint adjustment (bd-07j1)
                let adjusted = if (gamma - 1.0).abs() > f32::EPSILON {
                    scaled.powf(gamma).clamp(0.0, 1.0)
                } else {
                    scaled
                };
                let [r, g, b] = colormap.apply(adjusted);
                buffer[i * 4] = r;
                buffer[i * 4 + 1] = g;
                buffer[i * 4 + 2] = b;
            }
        }
        _ => {
            // Unknown bit depth - show error pattern (checkerboard)
            let width_usize = width as usize;
            for i in 0..pixel_count {
                let checkerboard = ((i % width_usize) / 16 + (i / width_usize) / 16) % 2;
                let color = if checkerboard == 0 { 255u8 } else { 128u8 };
                buffer[i * 4] = color;
                buffer[i * 4 + 1] = 0;
                buffer[i * 4 + 2] = color;
            }
        }
    }

    (effective_min, effective_max)
}

/// Compute min/max values from frame data for auto-contrast (free function version)
pub(super) fn compute_minmax_from_data(data: &[u8], bit_depth: u32, bit_max: f32) -> (f32, f32) {
    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;

    match bit_depth {
        8 => {
            for &pixel in data {
                let val = f32::from(pixel);
                min_val = min_val.min(val);
                max_val = max_val.max(val);
            }
        }
        12 | 16 => {
            for chunk in data.chunks_exact(2) {
                let pixel = u16::from_le_bytes([chunk[0], chunk[1]]);
                let val = f32::from(pixel);
                min_val = min_val.min(val);
                max_val = max_val.max(val);
            }
        }
        _ => {
            return (0.0, 1.0);
        }
    }

    // Normalize to 0.0-1.0 range
    if min_val < max_val {
        (min_val / bit_max, max_val / bit_max)
    } else {
        (0.0, 1.0)
    }
}

/// Compute percentile-based min/max for outlier-robust auto-contrast (bd-j6xm)
///
/// Percentiles should be in range 0.0-100.0 (e.g., 0.1 and 99.9)
pub(super) fn compute_percentile_minmax(
    data: &[u8],
    bit_depth: u32,
    bit_max: f32,
    low: f32,
    high: f32,
) -> (f32, f32) {
    // Collect pixel values into a sorted vec for percentile computation
    let mut values: Vec<f32> = Vec::new();

    match bit_depth {
        8 => {
            values.reserve(data.len());
            for &pixel in data {
                values.push(f32::from(pixel));
            }
        }
        12 | 16 => {
            values.reserve(data.len() / 2);
            for chunk in data.chunks_exact(2) {
                let pixel = u16::from_le_bytes([chunk[0], chunk[1]]);
                values.push(f32::from(pixel));
            }
        }
        _ => {
            return (0.0, 1.0);
        }
    }

    if values.is_empty() {
        return (0.0, 1.0);
    }

    // Sort for percentile calculation
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Calculate percentile indices
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    let low_idx =
        ((low / 100.0) * (values.len() as f32)).clamp(0.0, (values.len() - 1) as f32) as usize;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    let high_idx =
        ((high / 100.0) * (values.len() as f32)).clamp(0.0, (values.len() - 1) as f32) as usize;

    let min_val = values[low_idx];
    let max_val = values[high_idx];

    // Normalize to 0.0-1.0 range
    if min_val < max_val {
        (min_val / bit_max, max_val / bit_max)
    } else {
        (0.0, 1.0)
    }
}

/// Build histogram for image data (bd-j6xm)
pub(super) fn build_histogram(data: &[u8], bit_depth: u32, bins: usize) -> Vec<u32> {
    let mut hist = vec![0u32; bins];

    #[allow(clippy::cast_precision_loss)]
    let bin_scale = (bins - 1) as f32
        / match bit_depth {
            8 => 255.0,
            12 => 4095.0,
            16 => 65535.0,
            _ => 65535.0,
        };

    match bit_depth {
        8 => {
            for &pixel in data {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let bin = ((f32::from(pixel) * bin_scale) as usize).min(bins - 1);
                hist[bin] += 1;
            }
        }
        12 | 16 => {
            for chunk in data.chunks_exact(2) {
                let pixel = u16::from_le_bytes([chunk[0], chunk[1]]);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let bin = ((f32::from(pixel) * bin_scale) as usize).min(bins - 1);
                hist[bin] += 1;
            }
        }
        _ => {}
    }

    hist
}

/// Apply histogram equalization mapping (bd-j6xm)
///
/// Returns a lookup table mapping input intensity (0.0-1.0) to output intensity (0.0-1.0)
#[allow(clippy::cast_precision_loss)]
pub(super) fn compute_histogram_equalization_lut(
    histogram: &[u32],
    total_pixels: usize,
) -> Vec<f32> {
    let bins = histogram.len();
    let mut lut = vec![0.0f32; bins];

    // Compute cumulative distribution function
    let mut cdf = vec![0u32; bins];
    cdf[0] = histogram[0];
    for i in 1..bins {
        cdf[i] = cdf[i - 1] + histogram[i];
    }

    // Find first non-zero value for normalization
    let cdf_min = *cdf.iter().find(|&&x| x > 0).unwrap_or(&0);
    #[allow(clippy::cast_possible_truncation)]
    let cdf_range = (total_pixels as u32).saturating_sub(cdf_min).max(1);

    // Build equalization lookup table
    for i in 0..bins {
        lut[i] = (cdf[i].saturating_sub(cdf_min) as f32 / cdf_range as f32).clamp(0.0, 1.0);
    }

    lut
}

/// Apply Contrast Limited Adaptive Histogram Equalization (CLAHE) (bd-j6xm)
///
/// Simplified implementation with fixed clip limit. For better quality, consider using
/// a proper CLAHE library with tile-based processing.
pub(super) fn compute_clahe_lut(
    histogram: &[u32],
    total_pixels: usize,
    clip_limit: f32,
) -> Vec<f32> {
    let bins = histogram.len();

    // Clip histogram to limit contrast enhancement
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    let clip_value = (clip_limit * total_pixels as f32 / bins as f32) as u32;
    let mut clipped_hist = histogram.to_vec();
    let mut excess = 0u32;

    for count in &mut clipped_hist {
        if *count > clip_value {
            excess += *count - clip_value;
            *count = clip_value;
        }
    }

    // Redistribute excess evenly
    #[allow(clippy::cast_possible_truncation)]
    let redistribute = excess / bins as u32;
    for count in &mut clipped_hist {
        *count += redistribute;
    }

    // Apply histogram equalization on clipped histogram
    compute_histogram_equalization_lut(&clipped_hist, total_pixels)
}

/// Whitelist for quick access camera parameters
pub(super) const QUICK_ACCESS_PARAMS: &[&str] = &[
    "exposure",    // Matches exposure_ms, exposure_mode, etc.
    "gain",        // Matches gain_index, gain_mode
    "speed",       // Matches speed_index, speed_mode
    "temperature", // Matches temperature, temperature_setpoint
    "fan",         // Matches fan_speed
    "trigger",     // Matches trigger_mode
    "roi",         // Matches roi (hardware)
    "binning",     // Matches binning
];
