//! Test pattern generation for mock camera frames.

/// Simple pseudo-random number generator (LCG) for reproducible noise.
/// Uses the same algorithm as glibc for predictable cross-platform behavior.
#[inline]
fn prng(seed: u64) -> u64 {
    seed.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fffffff
}

/// Generates a diagnostic test pattern for camera validation.
///
/// The pattern includes:
/// - Checkerboard background for pixel alignment verification
/// - Corner markers (different shapes) for orientation detection
/// - Center crosshair for centering verification
/// - Gradient regions for colormap/intensity testing
/// - Frame number encoded in the pattern
/// - **Dynamic elements:**
///   - Background noise (varies each frame)
///   - Moving Gaussian hotspot that orbits the center
///   - Pulsing center ring intensity
///
/// # Arguments
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `frame_num` - Frame number (for animation/identification)
///
/// # Returns
/// A Vec<u16> containing the test pattern pixel data
pub fn generate_test_pattern(width: u32, height: u32, frame_num: u64) -> Vec<u16> {
    let mut buffer = vec![0u16; (width * height) as usize];
    let w = width as usize;
    let h = height as usize;

    // For very small images, just fill with a gradient and return
    if w < 64 || h < 64 {
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                // Simple diagonal gradient for small images
                let intensity = ((x + y) * 65535 / (w + h).max(1)) as u16;
                buffer[idx] = intensity;
            }
        }
        return buffer;
    }

    // Size parameters scaled to image dimensions (ensure non-zero for small images)
    let checker_size = (width.min(height) / 32).max(1) as usize; // ~20 pixels for 640x480
    let corner_size = (width.min(height) / 8).max(1) as usize; // ~60 pixels for 640x480
    let crosshair_thickness = 3usize;
    let crosshair_length = (width.min(height) / 6).max(1) as usize; // ~80 pixels for 640x480
    let gradient_height = (height / 10).max(1) as usize; // 10% of height for gradient bars

    // Center coordinates
    let cx = w / 2;
    let cy = h / 2;

    // === Dynamic elements ===
    // Moving hotspot: orbits around center with period of ~120 frames (~4 sec at 30fps)
    let orbit_radius = (width.min(height) / 5) as f64;
    let angle = (frame_num as f64 * 0.05) % (2.0 * std::f64::consts::PI);
    let hotspot_x = cx as f64 + orbit_radius * angle.cos();
    let hotspot_y = cy as f64 + orbit_radius * angle.sin();
    let hotspot_radius = 30.0f64; // Gaussian sigma

    // Pulsing intensity for center ring (oscillates between 70% and 100%)
    let pulse_phase = (frame_num as f64 * 0.15).sin(); // ~0.5 Hz at 30fps
    let ring_intensity = (0.85 + 0.15 * pulse_phase) * 65535.0;

    // Noise seed based on frame number
    let frame_seed = frame_num.wrapping_mul(2654435761);

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;

            // Per-pixel noise seed (combines frame and position for spatial variation)
            let noise_seed = prng(frame_seed ^ (idx as u64));
            let noise_value = ((noise_seed & 0xFFF) as i32 - 2048) as i16; // Range: -2048 to +2047

            // Layer 1: Checkerboard background (alternating ~25% and ~30% intensity)
            let checker_x = x / checker_size;
            let checker_y = y / checker_size;
            let base_value: i32 = if (checker_x + checker_y).is_multiple_of(2) {
                16384 // ~25% of 65535
            } else {
                19660 // ~30% of 65535
            };

            // Add noise to base value (small amplitude: ~3% of full scale)
            let mut pixel_value: u16 = (base_value + noise_value as i32).clamp(0, 65535) as u16;

            // Layer 2: Gradient regions at top and bottom
            // Top gradient: 0% to 100% intensity (left to right)
            if y < gradient_height {
                pixel_value = ((x as u32 * 65535) / width) as u16;
            }
            // Bottom gradient: 100% to 0% intensity (left to right)
            if y >= h - gradient_height {
                pixel_value = (((w - 1 - x) as u32 * 65535) / width) as u16;
            }

            // Layer 3: Corner markers for orientation detection
            // Top-left: Solid bright triangle (identifies origin)
            if x < corner_size && y < corner_size && x + y < corner_size {
                pixel_value = 65535; // Full white
            }
            // Top-right: Hollow rectangle outline
            if x >= w - corner_size && y < corner_size {
                let local_x = x - (w - corner_size);
                let local_y = y;
                let border = 5;
                if local_x < border
                    || local_x >= corner_size - border
                    || local_y < border
                    || local_y >= corner_size - border
                {
                    pixel_value = 52428; // ~80% intensity
                }
            }
            // Bottom-left: Filled circle
            if x < corner_size && y >= h - corner_size {
                let local_x = x as i32;
                let local_y = (y - (h - corner_size)) as i32;
                let center = (corner_size / 2) as i32;
                let radius = (corner_size / 3) as i32;
                let dx = local_x - center;
                let dy = local_y - center;
                if dx * dx + dy * dy <= radius * radius {
                    pixel_value = 45875; // ~70% intensity
                }
            }
            // Bottom-right: X mark
            if x >= w - corner_size && y >= h - corner_size {
                let local_x = x - (w - corner_size);
                let local_y = y - (h - corner_size);
                let thickness = 6;
                // Diagonal from top-left to bottom-right
                let diff1 = (local_x as i32 - local_y as i32).unsigned_abs() as usize;
                // Diagonal from top-right to bottom-left
                let diff2 = (local_x as i32 - (corner_size as i32 - 1 - local_y as i32))
                    .unsigned_abs() as usize;
                if diff1 < thickness || diff2 < thickness {
                    pixel_value = 39321; // ~60% intensity
                }
            }

            // Layer 4: Center crosshair
            let in_horizontal = y >= cy - crosshair_thickness / 2
                && y <= cy + crosshair_thickness / 2
                && x >= cx - crosshair_length
                && x <= cx + crosshair_length;
            let in_vertical = x >= cx - crosshair_thickness / 2
                && x <= cx + crosshair_thickness / 2
                && y >= cy - crosshair_length
                && y <= cy + crosshair_length;
            if in_horizontal || in_vertical {
                pixel_value = 65535; // Full white
            }

            // Layer 5: Center circle (distinguishable marker) - PULSING
            let dx_center = (x as i32 - cx as i32).abs();
            let dy_center = (y as i32 - cy as i32).abs();
            let dist_sq_center = dx_center * dx_center + dy_center * dy_center;
            let inner_radius = (crosshair_length / 3) as i32;
            let outer_radius = inner_radius + 4;
            if dist_sq_center >= inner_radius * inner_radius
                && dist_sq_center <= outer_radius * outer_radius
            {
                pixel_value = ring_intensity as u16; // Pulsing intensity
            }

            // Layer 6: Frame number indicator (small dots in top-left area below corner marker)
            // Encode low 4 bits of frame_num as 4 dots
            let dot_y_start = corner_size + 10;
            let dot_spacing = 15usize;
            let dot_radius = 5i32;
            if y >= dot_y_start && y < dot_y_start + 20 && x < corner_size + 10 {
                for bit in 0usize..4 {
                    let dot_x = (10 + bit * dot_spacing) as i32;
                    let dot_y = (dot_y_start + 10) as i32;
                    let dx = (x as i32 - dot_x).abs();
                    let dy = (y as i32 - dot_y).abs();
                    if dx * dx + dy * dy <= dot_radius * dot_radius {
                        if (frame_num >> bit) & 1 == 1 {
                            pixel_value = 65535; // On = white
                        } else {
                            pixel_value = 6553; // Off = ~10% (visible but dim)
                        }
                    }
                }
            }

            // Layer 7: Intensity test patches (stepped grayscale) on right edge
            let patch_height = h / 8;
            let patch_width = 40;
            if x >= w - patch_width && y >= gradient_height && y < h - gradient_height {
                let patch_idx = (y - gradient_height) / patch_height;
                // 8 levels from ~12.5% to 100%
                let intensity = ((patch_idx + 1) as u32 * 65535) / 8;
                pixel_value = intensity as u16;
            }

            // Layer 8: Moving Gaussian hotspot (orbits around center)
            let dx_hotspot = x as f64 - hotspot_x;
            let dy_hotspot = y as f64 - hotspot_y;
            let dist_sq_hotspot = dx_hotspot * dx_hotspot + dy_hotspot * dy_hotspot;
            let gaussian = (-dist_sq_hotspot / (2.0 * hotspot_radius * hotspot_radius)).exp();
            // Add hotspot intensity (additive blend, max 50% of full scale)
            let hotspot_contribution = (gaussian * 32768.0) as u32;
            pixel_value = (pixel_value as u32 + hotspot_contribution).min(65535) as u16;

            buffer[idx] = pixel_value;
        }
    }

    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_generates_correct_size() {
        let buffer = generate_test_pattern(640, 480, 0);
        assert_eq!(buffer.len(), 640 * 480);
    }

    #[test]
    fn test_pattern_small_image() {
        let buffer = generate_test_pattern(32, 32, 0);
        assert_eq!(buffer.len(), 32 * 32);
    }

    #[test]
    fn test_pattern_varies_with_frame_number() {
        let buffer1 = generate_test_pattern(100, 100, 0);
        let buffer2 = generate_test_pattern(100, 100, 1);
        // Patterns should differ due to noise and dynamic elements
        assert_ne!(buffer1, buffer2);
    }
}
