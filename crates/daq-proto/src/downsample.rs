//! Server-side frame downsampling for reduced bandwidth streaming.
//!
//! Provides 2x2 and 4x4 pixel averaging for preview and fast streaming modes.
//! These functions are designed for 16-bit camera data (little-endian u16 pixels).

/// Downsample a frame by averaging 2x2 blocks of pixels.
///
/// Reduces frame size by 4x (2x in each dimension).
/// Input must be 16-bit little-endian pixel data.
///
/// # Arguments
/// * `data` - Raw pixel data (u16 little-endian)
/// * `width` - Original frame width in pixels
/// * `height` - Original frame height in pixels
///
/// # Returns
/// Tuple of (downsampled data, new width, new height)
///
/// # Panics
/// Returns original data if dimensions are odd or data size doesn't match expected.
pub fn downsample_2x2(data: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    // Validate dimensions are even (return original if not)
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return (data.to_vec(), width, height);
    }

    // Validate data size (return original if mismatch)
    let expected_size = (width as usize) * (height as usize) * 2;
    if data.len() != expected_size {
        return (data.to_vec(), width, height);
    }

    let new_width = width / 2;
    let new_height = height / 2;
    let mut out = Vec::with_capacity((new_width * new_height * 2) as usize);

    // Average 2x2 blocks of u16 pixels
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            // Calculate indices for 2x2 block
            let idx = |px: u32, py: u32| ((py * width + px) * 2) as usize;

            let i00 = idx(x, y);
            let i01 = idx(x + 1, y);
            let i10 = idx(x, y + 1);
            let i11 = idx(x + 1, y + 1);

            // Read 4 pixels as u16 little-endian
            let p00 = u16::from_le_bytes([data[i00], data[i00 + 1]]);
            let p01 = u16::from_le_bytes([data[i01], data[i01 + 1]]);
            let p10 = u16::from_le_bytes([data[i10], data[i10 + 1]]);
            let p11 = u16::from_le_bytes([data[i11], data[i11 + 1]]);

            // Average the 4 pixels
            let avg = ((p00 as u32 + p01 as u32 + p10 as u32 + p11 as u32) / 4) as u16;
            out.extend_from_slice(&avg.to_le_bytes());
        }
    }

    (out, new_width, new_height)
}

/// Downsample a frame by averaging 4x4 blocks of pixels.
///
/// Reduces frame size by 16x (4x in each dimension).
/// Input must be 16-bit little-endian pixel data.
///
/// # Arguments
/// * `data` - Raw pixel data (u16 little-endian)
/// * `width` - Original frame width in pixels
/// * `height` - Original frame height in pixels
///
/// # Returns
/// Tuple of (downsampled data, new width, new height)
///
/// # Panics
/// Returns original data if dimensions aren't divisible by 4 or data size doesn't match.
pub fn downsample_4x4(data: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    // Validate dimensions are divisible by 4 (return original if not)
    if !width.is_multiple_of(4) || !height.is_multiple_of(4) {
        return (data.to_vec(), width, height);
    }

    // Validate data size (return original if mismatch)
    let expected_size = (width as usize) * (height as usize) * 2;
    if data.len() != expected_size {
        return (data.to_vec(), width, height);
    }

    let new_width = width / 4;
    let new_height = height / 4;
    let mut out = Vec::with_capacity((new_width * new_height * 2) as usize);

    // Average 4x4 blocks of u16 pixels
    for y in (0..height).step_by(4) {
        for x in (0..width).step_by(4) {
            let idx = |px: u32, py: u32| ((py * width + px) * 2) as usize;

            // Sum all 16 pixels in the 4x4 block
            let mut sum: u32 = 0;
            for dy in 0..4 {
                for dx in 0..4 {
                    let i = idx(x + dx, y + dy);
                    let pixel = u16::from_le_bytes([data[i], data[i + 1]]);
                    sum += pixel as u32;
                }
            }

            // Average (divide by 16)
            let avg = (sum / 16) as u16;
            out.extend_from_slice(&avg.to_le_bytes());
        }
    }

    (out, new_width, new_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downsample_2x2() {
        // Create a 4x4 test image with known values
        let mut data = Vec::new();
        // Row 0: [100, 200, 300, 400]
        // Row 1: [100, 200, 300, 400]
        // Row 2: [500, 600, 700, 800]
        // Row 3: [500, 600, 700, 800]
        for row in [[100u16, 200, 300, 400], [100, 200, 300, 400]] {
            for val in row {
                data.extend_from_slice(&val.to_le_bytes());
            }
        }
        for row in [[500u16, 600, 700, 800], [500, 600, 700, 800]] {
            for val in row {
                data.extend_from_slice(&val.to_le_bytes());
            }
        }

        let (result, w, h) = downsample_2x2(&data, 4, 4);
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(result.len(), 8); // 2x2 pixels * 2 bytes

        // Expected: top-left = avg(100,200,100,200) = 150
        let p00 = u16::from_le_bytes([result[0], result[1]]);
        assert_eq!(p00, 150);

        // top-right = avg(300,400,300,400) = 350
        let p01 = u16::from_le_bytes([result[2], result[3]]);
        assert_eq!(p01, 350);

        // bottom-left = avg(500,600,500,600) = 550
        let p10 = u16::from_le_bytes([result[4], result[5]]);
        assert_eq!(p10, 550);

        // bottom-right = avg(700,800,700,800) = 750
        let p11 = u16::from_le_bytes([result[6], result[7]]);
        assert_eq!(p11, 750);
    }

    #[test]
    fn test_downsample_4x4() {
        // Create an 8x8 test image with uniform value
        let value = 1000u16;
        let mut data = Vec::new();
        for _ in 0..(8 * 8) {
            data.extend_from_slice(&value.to_le_bytes());
        }

        let (result, w, h) = downsample_4x4(&data, 8, 8);
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(result.len(), 8); // 2x2 pixels * 2 bytes

        // All pixels should average to the same value
        for i in 0..4 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 1000);
        }
    }

    #[test]
    fn test_odd_dimensions_rejected() {
        let data = vec![0u8; 100];
        let (result, w, h) = downsample_2x2(&data, 5, 10);
        assert_eq!(w, 5); // Should return original
        assert_eq!(h, 10);
        assert_eq!(result.len(), 100);
    }
}
