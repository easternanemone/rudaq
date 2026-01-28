//! Server-side frame downsampling for reduced bandwidth streaming.
//!
//! Provides 2x2 and 4x4 pixel averaging for preview and fast streaming modes.
//! These functions are designed for 16-bit camera data (little-endian u16 pixels).
//!
//! For odd dimensions, the last row/column is cropped (not padded) to preserve
//! scientific data integrity and ensure bandwidth savings are always achieved.

/// Downsample a frame by averaging 2x2 blocks of pixels.
///
/// Reduces frame size by 4x (2x in each dimension).
/// Input must be 16-bit little-endian pixel data.
///
/// For odd dimensions, the last row/column is cropped to ensure downsampling
/// always occurs. This preserves scientific data integrity (no synthetic padding).
///
/// # Arguments
/// * `data` - Raw pixel data (u16 little-endian)
/// * `width` - Original frame width in pixels
/// * `height` - Original frame height in pixels
///
/// # Returns
/// Tuple of (downsampled data, new width, new height)
///
/// Returns original data unchanged only if:
/// - Usable dimensions are less than 2x2 (too small to downsample)
/// - Data size doesn't match expected size for the original dimensions
pub fn downsample_2x2(data: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    // Calculate usable dimensions (floor to nearest multiple of 2)
    let usable_width = (width / 2) * 2;
    let usable_height = (height / 2) * 2;

    // If dimensions too small to downsample, return original
    if usable_width < 2 || usable_height < 2 {
        return (data.to_vec(), width, height);
    }

    // Validate data size for ORIGINAL dimensions (return original if mismatch)
    let expected_size = (width as usize) * (height as usize) * 2;
    if data.len() != expected_size {
        return (data.to_vec(), width, height);
    }

    let new_width = usable_width / 2;
    let new_height = usable_height / 2;
    let mut out = Vec::with_capacity((new_width * new_height * 2) as usize);

    // Average 2x2 blocks of u16 pixels (using only usable portion, cropping last row/col if odd)
    for y in (0..usable_height).step_by(2) {
        for x in (0..usable_width).step_by(2) {
            // Use ORIGINAL width for index calculation to correctly address pixels
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
/// For dimensions not divisible by 4, the remainder rows/columns are cropped
/// to ensure downsampling always occurs. This preserves scientific data integrity.
///
/// # Arguments
/// * `data` - Raw pixel data (u16 little-endian)
/// * `width` - Original frame width in pixels
/// * `height` - Original frame height in pixels
///
/// # Returns
/// Tuple of (downsampled data, new width, new height)
///
/// Returns original data unchanged only if:
/// - Usable dimensions are less than 4x4 (too small to downsample)
/// - Data size doesn't match expected size for the original dimensions
pub fn downsample_4x4(data: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    // Calculate usable dimensions (floor to nearest multiple of 4)
    let usable_width = (width / 4) * 4;
    let usable_height = (height / 4) * 4;

    // If dimensions too small to downsample, return original
    if usable_width < 4 || usable_height < 4 {
        return (data.to_vec(), width, height);
    }

    // Validate data size for ORIGINAL dimensions (return original if mismatch)
    let expected_size = (width as usize) * (height as usize) * 2;
    if data.len() != expected_size {
        return (data.to_vec(), width, height);
    }

    let new_width = usable_width / 4;
    let new_height = usable_height / 4;
    let mut out = Vec::with_capacity((new_width * new_height * 2) as usize);

    // Average 4x4 blocks of u16 pixels (using only usable portion, cropping remainder if not divisible by 4)
    for y in (0..usable_height).step_by(4) {
        for x in (0..usable_width).step_by(4) {
            // Use ORIGINAL width for index calculation to correctly address pixels
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
    fn test_odd_dimensions_cropped_2x2() {
        // Create a 5x5 test image - last row and column should be cropped
        // Usable area is 4x4, resulting in 2x2 output
        let mut data = Vec::new();
        // Fill 5x5 grid with value 1000
        for _ in 0..(5 * 5) {
            data.extend_from_slice(&1000u16.to_le_bytes());
        }

        let (result, w, h) = downsample_2x2(&data, 5, 5);
        assert_eq!(w, 2); // 4/2 = 2 (last column cropped)
        assert_eq!(h, 2); // 4/2 = 2 (last row cropped)
        assert_eq!(result.len(), 8); // 2x2 pixels * 2 bytes

        // All pixels should average to 1000
        for i in 0..4 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 1000);
        }
    }

    #[test]
    fn test_odd_dimensions_cropped_4x4() {
        // Create a 9x9 test image - last row and column should be cropped
        // Usable area is 8x8, resulting in 2x2 output
        let mut data = Vec::new();
        // Fill 9x9 grid with value 500
        for _ in 0..(9 * 9) {
            data.extend_from_slice(&500u16.to_le_bytes());
        }

        let (result, w, h) = downsample_4x4(&data, 9, 9);
        assert_eq!(w, 2); // 8/4 = 2 (last column cropped)
        assert_eq!(h, 2); // 8/4 = 2 (last row cropped)
        assert_eq!(result.len(), 8); // 2x2 pixels * 2 bytes

        // All pixels should average to 500
        for i in 0..4 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 500);
        }
    }

    #[test]
    fn test_too_small_returns_original_2x2() {
        // 1x1 image - too small to downsample
        let data = vec![0u8; 2]; // 1 pixel * 2 bytes
        let (result, w, h) = downsample_2x2(&data, 1, 1);
        assert_eq!(w, 1); // Should return original
        assert_eq!(h, 1);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_too_small_returns_original_4x4() {
        // 3x3 image - too small to downsample with 4x4 blocks
        let data = vec![0u8; 18]; // 9 pixels * 2 bytes
        let (result, w, h) = downsample_4x4(&data, 3, 3);
        assert_eq!(w, 3); // Should return original
        assert_eq!(h, 3);
        assert_eq!(result.len(), 18);
    }

    #[test]
    fn test_odd_width_only_2x2() {
        // 5x4 image - only width is odd, height is even
        // Usable area is 4x4, resulting in 2x2 output
        let mut data = Vec::new();
        for _ in 0..(5 * 4) {
            data.extend_from_slice(&200u16.to_le_bytes());
        }

        let (result, w, h) = downsample_2x2(&data, 5, 4);
        assert_eq!(w, 2); // 4/2 = 2 (last column cropped)
        assert_eq!(h, 2); // 4/2 = 2
        assert_eq!(result.len(), 8);

        // All pixels should average to 200
        for i in 0..4 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 200);
        }
    }

    #[test]
    fn test_odd_height_only_2x2() {
        // 4x5 image - only height is odd, width is even
        // Usable area is 4x4, resulting in 2x2 output
        let mut data = Vec::new();
        for _ in 0..(4 * 5) {
            data.extend_from_slice(&300u16.to_le_bytes());
        }

        let (result, w, h) = downsample_2x2(&data, 4, 5);
        assert_eq!(w, 2); // 4/2 = 2
        assert_eq!(h, 2); // 4/2 = 2 (last row cropped)
        assert_eq!(result.len(), 8);

        // All pixels should average to 300
        for i in 0..4 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 300);
        }
    }

    // ============================================================
    // Additional odd-dimension cropping tests (bd-8iu4)
    // ============================================================

    #[test]
    fn test_odd_width_101x100_2x2() {
        // 101x100 image - width is odd, height is even
        // Usable area is 100x100, resulting in 50x50 output
        let width = 101u32;
        let height = 100u32;
        let mut data = Vec::new();

        // Fill with value 400
        for _ in 0..(width * height) {
            data.extend_from_slice(&400u16.to_le_bytes());
        }

        let (result, w, h) = downsample_2x2(&data, width, height);

        // Verify output dimensions
        assert_eq!(w, 50); // 100/2 = 50 (last column cropped)
        assert_eq!(h, 50); // 100/2 = 50
        assert_eq!(result.len(), (50 * 50 * 2) as usize);

        // Verify all pixels average correctly
        for i in 0..(50 * 50) {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 400);
        }
    }

    #[test]
    fn test_odd_height_100x101_2x2() {
        // 100x101 image - width is even, height is odd
        // Usable area is 100x100, resulting in 50x50 output
        let width = 100u32;
        let height = 101u32;
        let mut data = Vec::new();

        // Fill with value 500
        for _ in 0..(width * height) {
            data.extend_from_slice(&500u16.to_le_bytes());
        }

        let (result, w, h) = downsample_2x2(&data, width, height);

        // Verify output dimensions
        assert_eq!(w, 50); // 100/2 = 50
        assert_eq!(h, 50); // 100/2 = 50 (last row cropped)
        assert_eq!(result.len(), (50 * 50 * 2) as usize);

        // Verify all pixels average correctly
        for i in 0..(50 * 50) {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 500);
        }
    }

    #[test]
    fn test_both_odd_101x101_2x2() {
        // 101x101 image - both dimensions odd
        // Usable area is 100x100, resulting in 50x50 output
        let width = 101u32;
        let height = 101u32;
        let mut data = Vec::new();

        // Fill with value 600
        for _ in 0..(width * height) {
            data.extend_from_slice(&600u16.to_le_bytes());
        }

        let (result, w, h) = downsample_2x2(&data, width, height);

        // Verify output dimensions
        assert_eq!(w, 50); // 100/2 = 50 (last column cropped)
        assert_eq!(h, 50); // 100/2 = 50 (last row cropped)
        assert_eq!(result.len(), (50 * 50 * 2) as usize);

        // Verify all pixels average correctly
        for i in 0..(50 * 50) {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 600);
        }
    }

    #[test]
    fn test_odd_width_103x100_4x4() {
        // 103x100 image - width not divisible by 4, height is divisible
        // Usable area is 100x100, resulting in 25x25 output
        let width = 103u32;
        let height = 100u32;
        let mut data = Vec::new();

        // Fill with value 700
        for _ in 0..(width * height) {
            data.extend_from_slice(&700u16.to_le_bytes());
        }

        let (result, w, h) = downsample_4x4(&data, width, height);

        // Verify output dimensions
        assert_eq!(w, 25); // 100/4 = 25 (3 columns cropped)
        assert_eq!(h, 25); // 100/4 = 25
        assert_eq!(result.len(), (25 * 25 * 2) as usize);

        // Verify all pixels average correctly
        for i in 0..(25 * 25) {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 700);
        }
    }

    #[test]
    fn test_odd_height_100x103_4x4() {
        // 100x103 image - width is divisible by 4, height is not
        // Usable area is 100x100, resulting in 25x25 output
        let width = 100u32;
        let height = 103u32;
        let mut data = Vec::new();

        // Fill with value 800
        for _ in 0..(width * height) {
            data.extend_from_slice(&800u16.to_le_bytes());
        }

        let (result, w, h) = downsample_4x4(&data, width, height);

        // Verify output dimensions
        assert_eq!(w, 25); // 100/4 = 25
        assert_eq!(h, 25); // 100/4 = 25 (3 rows cropped)
        assert_eq!(result.len(), (25 * 25 * 2) as usize);

        // Verify all pixels average correctly
        for i in 0..(25 * 25) {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 800);
        }
    }

    #[test]
    fn test_both_not_divisible_103x103_4x4() {
        // 103x103 image - both dimensions not divisible by 4
        // Usable area is 100x100, resulting in 25x25 output
        let width = 103u32;
        let height = 103u32;
        let mut data = Vec::new();

        // Fill with value 900
        for _ in 0..(width * height) {
            data.extend_from_slice(&900u16.to_le_bytes());
        }

        let (result, w, h) = downsample_4x4(&data, width, height);

        // Verify output dimensions
        assert_eq!(w, 25); // 100/4 = 25 (3 columns cropped)
        assert_eq!(h, 25); // 100/4 = 25 (3 rows cropped)
        assert_eq!(result.len(), (25 * 25 * 2) as usize);

        // Verify all pixels average correctly
        for i in 0..(25 * 25) {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 900);
        }
    }

    #[test]
    fn test_cropped_pixels_ignored_2x2() {
        // 5x5 image where the last row and column have different values
        // This verifies that cropped pixels are actually ignored, not included
        let width = 5u32;
        let height = 5u32;
        let mut data = Vec::new();

        // Fill 5x5 grid: first 4x4 has value 100, last row/column has value 9999
        for y in 0..height {
            for x in 0..width {
                let value = if x < 4 && y < 4 { 100u16 } else { 9999u16 };
                data.extend_from_slice(&value.to_le_bytes());
            }
        }

        let (result, w, h) = downsample_2x2(&data, width, height);

        // Verify dimensions
        assert_eq!(w, 2); // 4/2 = 2
        assert_eq!(h, 2); // 4/2 = 2

        // All output pixels should be 100 (the 9999 values should be ignored)
        for i in 0..4 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(
                pixel, 100,
                "Pixel {} should be 100 (cropped pixels should be ignored)",
                i
            );
        }
    }

    #[test]
    fn test_cropped_pixels_ignored_4x4() {
        // 9x9 image where the last row and column have different values
        // This verifies that cropped pixels are actually ignored, not included
        let width = 9u32;
        let height = 9u32;
        let mut data = Vec::new();

        // Fill 9x9 grid: first 8x8 has value 200, last row/column has value 8888
        for y in 0..height {
            for x in 0..width {
                let value = if x < 8 && y < 8 { 200u16 } else { 8888u16 };
                data.extend_from_slice(&value.to_le_bytes());
            }
        }

        let (result, w, h) = downsample_4x4(&data, width, height);

        // Verify dimensions
        assert_eq!(w, 2); // 8/4 = 2
        assert_eq!(h, 2); // 8/4 = 2

        // All output pixels should be 200 (the 8888 values should be ignored)
        for i in 0..4 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(
                pixel, 200,
                "Pixel {} should be 200 (cropped pixels should be ignored)",
                i
            );
        }
    }

    #[test]
    fn test_varying_remainder_2x2() {
        // Test different remainder values for 2x2 (remainder 1)
        // 7x7 image: usable 6x6, output 3x3
        let width = 7u32;
        let height = 7u32;
        let mut data = Vec::new();

        for _ in 0..(width * height) {
            data.extend_from_slice(&250u16.to_le_bytes());
        }

        let (result, w, h) = downsample_2x2(&data, width, height);

        assert_eq!(w, 3); // 6/2 = 3 (1 column cropped)
        assert_eq!(h, 3); // 6/2 = 3 (1 row cropped)
        assert_eq!(result.len(), (3 * 3 * 2) as usize);
    }

    #[test]
    fn test_varying_remainder_4x4() {
        // Test different remainder values for 4x4
        // 14x15 image: usable 12x12, output 3x3
        let width = 14u32;
        let height = 15u32;
        let mut data = Vec::new();

        for _ in 0..(width * height) {
            data.extend_from_slice(&350u16.to_le_bytes());
        }

        let (result, w, h) = downsample_4x4(&data, width, height);

        assert_eq!(w, 3); // 12/4 = 3 (2 columns cropped)
        assert_eq!(h, 3); // 12/4 = 3 (3 rows cropped)
        assert_eq!(result.len(), (3 * 3 * 2) as usize);

        // Verify values
        for i in 0..9 {
            let pixel = u16::from_le_bytes([result[i * 2], result[i * 2 + 1]]);
            assert_eq!(pixel, 350);
        }
    }

    #[test]
    fn test_remainder_1_2_3_for_4x4() {
        // Test remainder 1: 101x100 -> usable 100x100 -> output 25x25
        let (_, w1, h1) = downsample_4x4(&vec![0u8; 101 * 100 * 2], 101, 100);
        assert_eq!(w1, 25);
        assert_eq!(h1, 25);

        // Test remainder 2: 102x100 -> usable 100x100 -> output 25x25
        let (_, w2, h2) = downsample_4x4(&vec![0u8; 102 * 100 * 2], 102, 100);
        assert_eq!(w2, 25);
        assert_eq!(h2, 25);

        // Test remainder 3: 103x100 -> usable 100x100 -> output 25x25
        let (_, w3, h3) = downsample_4x4(&vec![0u8; 103 * 100 * 2], 103, 100);
        assert_eq!(w3, 25);
        assert_eq!(h3, 25);

        // Test height remainders too
        let (_, w4, h4) = downsample_4x4(&vec![0u8; 100 * 101 * 2], 100, 101);
        assert_eq!(w4, 25);
        assert_eq!(h4, 25);

        let (_, w5, h5) = downsample_4x4(&vec![0u8; 100 * 102 * 2], 100, 102);
        assert_eq!(w5, 25);
        assert_eq!(h5, 25);

        let (_, w6, h6) = downsample_4x4(&vec![0u8; 100 * 103 * 2], 100, 103);
        assert_eq!(w6, 25);
        assert_eq!(h6, 25);
    }

    // =========================================================================
    // Compression ratio verification tests (bd-8w9t)
    // =========================================================================

    /// Helper to create test frame data of specified dimensions
    fn create_test_frame(width: u32, height: u32) -> Vec<u8> {
        let num_pixels = (width as usize) * (height as usize);
        let mut data = Vec::with_capacity(num_pixels * 2);
        for i in 0..num_pixels {
            // Use varying pixel values to ensure averaging works correctly
            let value = ((i % 65536) as u16).to_le_bytes();
            data.extend_from_slice(&value);
        }
        data
    }

    /// Helper to calculate expected compression ratio for 2x2 downsampling
    /// Returns (expected_width, expected_height, expected_bytes)
    fn expected_2x2_output(width: u32, height: u32) -> (u32, u32, usize) {
        let usable_width = (width / 2) * 2;
        let usable_height = (height / 2) * 2;
        let new_width = usable_width / 2;
        let new_height = usable_height / 2;
        let expected_bytes = (new_width as usize) * (new_height as usize) * 2;
        (new_width, new_height, expected_bytes)
    }

    /// Helper to calculate expected compression ratio for 4x4 downsampling
    /// Returns (expected_width, expected_height, expected_bytes)
    fn expected_4x4_output(width: u32, height: u32) -> (u32, u32, usize) {
        let usable_width = (width / 4) * 4;
        let usable_height = (height / 4) * 4;
        let new_width = usable_width / 4;
        let new_height = usable_height / 4;
        let expected_bytes = (new_width as usize) * (new_height as usize) * 2;
        (new_width, new_height, expected_bytes)
    }

    #[test]
    fn test_compression_ratio_2x2_small() {
        // Small frame: 100x100 16-bit -> 50x50 16-bit
        // Input: 100*100*2 = 20,000 bytes
        // Output: 50*50*2 = 5,000 bytes (4x compression)
        let width = 100;
        let height = 100;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 20_000, "Input should be 20KB");

        let (result, w, h) = downsample_2x2(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_2x2_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 50, "Expected 50 pixel width");
        assert_eq!(h, 50, "Expected 50 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 5_000, "Output should be 5KB");

        // Verify compression ratio is approximately 4x
        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 4.0).abs() < 0.01,
            "Expected 4x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_ratio_2x2_medium() {
        // Medium frame: 512x512 16-bit -> 256x256 16-bit
        // Input: 512*512*2 = 524,288 bytes (512KB)
        // Output: 256*256*2 = 131,072 bytes (128KB) - 4x compression
        let width = 512;
        let height = 512;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 524_288, "Input should be 512KB");

        let (result, w, h) = downsample_2x2(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_2x2_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 256, "Expected 256 pixel width");
        assert_eq!(h, 256, "Expected 256 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 131_072, "Output should be 128KB");

        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 4.0).abs() < 0.01,
            "Expected 4x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_ratio_2x2_large() {
        // Large frame: 1000x1000 16-bit -> 500x500 16-bit
        // Input: 1000*1000*2 = 2,000,000 bytes (2MB)
        // Output: 500*500*2 = 500,000 bytes (500KB) - 4x compression
        let width = 1000;
        let height = 1000;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 2_000_000, "Input should be 2MB");

        let (result, w, h) = downsample_2x2(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_2x2_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 500, "Expected 500 pixel width");
        assert_eq!(h, 500, "Expected 500 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 500_000, "Output should be 500KB");

        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 4.0).abs() < 0.01,
            "Expected 4x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_ratio_4x4_small() {
        // Small frame: 100x100 16-bit -> 25x25 16-bit
        // Input: 100*100*2 = 20,000 bytes
        // Output: 25*25*2 = 1,250 bytes - 16x compression
        let width = 100;
        let height = 100;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 20_000, "Input should be 20KB");

        let (result, w, h) = downsample_4x4(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_4x4_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 25, "Expected 25 pixel width");
        assert_eq!(h, 25, "Expected 25 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 1_250, "Output should be 1.25KB");

        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 16.0).abs() < 0.01,
            "Expected 16x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_ratio_4x4_medium() {
        // Medium frame: 512x512 16-bit -> 128x128 16-bit
        // Input: 512*512*2 = 524,288 bytes (512KB)
        // Output: 128*128*2 = 32,768 bytes (32KB) - 16x compression
        let width = 512;
        let height = 512;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 524_288, "Input should be 512KB");

        let (result, w, h) = downsample_4x4(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_4x4_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 128, "Expected 128 pixel width");
        assert_eq!(h, 128, "Expected 128 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 32_768, "Output should be 32KB");

        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 16.0).abs() < 0.01,
            "Expected 16x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_ratio_4x4_large() {
        // Large frame: 1000x1000 16-bit -> 250x250 16-bit
        // Input: 1000*1000*2 = 2,000,000 bytes (2MB)
        // Output: 250*250*2 = 125,000 bytes (125KB) - 16x compression
        let width = 1000;
        let height = 1000;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 2_000_000, "Input should be 2MB");

        let (result, w, h) = downsample_4x4(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_4x4_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 250, "Expected 250 pixel width");
        assert_eq!(h, 250, "Expected 250 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 125_000, "Output should be 125KB");

        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 16.0).abs() < 0.01,
            "Expected 16x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_ratio_2x2_non_square() {
        // Non-square frame: 1920x1080 (common video resolution)
        // Input: 1920*1080*2 = 4,147,200 bytes
        // Output: 960*540*2 = 1,036,800 bytes - 4x compression
        let width = 1920;
        let height = 1080;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 4_147_200, "Input should be ~4MB");

        let (result, w, h) = downsample_2x2(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_2x2_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 960, "Expected 960 pixel width");
        assert_eq!(h, 540, "Expected 540 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 1_036_800, "Output should be ~1MB");

        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 4.0).abs() < 0.01,
            "Expected 4x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_ratio_4x4_non_square() {
        // Non-square frame: 1920x1080 (common video resolution)
        // Input: 1920*1080*2 = 4,147,200 bytes
        // Output: 480*270*2 = 259,200 bytes - 16x compression
        let width = 1920;
        let height = 1080;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        assert_eq!(input_bytes, 4_147_200, "Input should be ~4MB");

        let (result, w, h) = downsample_4x4(&data, width, height);
        let (exp_w, exp_h, exp_bytes) = expected_4x4_output(width, height);

        assert_eq!(w, exp_w, "Output width mismatch");
        assert_eq!(h, exp_h, "Output height mismatch");
        assert_eq!(w, 480, "Expected 480 pixel width");
        assert_eq!(h, 270, "Expected 270 pixel height");
        assert_eq!(result.len(), exp_bytes, "Output buffer length mismatch");
        assert_eq!(result.len(), 259_200, "Output should be ~259KB");

        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            (ratio - 16.0).abs() < 0.01,
            "Expected 16x compression, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_output_buffer_matches_dimensions_2x2() {
        // Test that output buffer length exactly matches new_width * new_height * 2
        // for various sizes
        let test_cases = [(10, 10), (100, 100), (256, 256), (1024, 768), (2048, 2048)];

        for (width, height) in test_cases {
            let data = create_test_frame(width, height);
            let (result, w, h) = downsample_2x2(&data, width, height);

            let expected_len = (w as usize) * (h as usize) * 2;
            assert_eq!(
                result.len(),
                expected_len,
                "Buffer length mismatch for {}x{}: got {} bytes, expected {} bytes",
                width,
                height,
                result.len(),
                expected_len
            );
        }
    }

    #[test]
    fn test_output_buffer_matches_dimensions_4x4() {
        // Test that output buffer length exactly matches new_width * new_height * 2
        // for various sizes
        let test_cases = [(16, 16), (100, 100), (256, 256), (1024, 768), (2048, 2048)];

        for (width, height) in test_cases {
            let data = create_test_frame(width, height);
            let (result, w, h) = downsample_4x4(&data, width, height);

            let expected_len = (w as usize) * (h as usize) * 2;
            assert_eq!(
                result.len(),
                expected_len,
                "Buffer length mismatch for {}x{}: got {} bytes, expected {} bytes",
                width,
                height,
                result.len(),
                expected_len
            );
        }
    }

    #[test]
    fn test_compression_with_odd_dimensions_2x2() {
        // Odd dimensions should still achieve approximately 4x compression
        // 1001x1001 -> usable 1000x1000 -> 500x500
        let width = 1001;
        let height = 1001;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        let (result, w, h) = downsample_2x2(&data, width, height);

        assert_eq!(w, 500, "Expected 500 pixel width (1000/2)");
        assert_eq!(h, 500, "Expected 500 pixel height (1000/2)");
        assert_eq!(result.len(), 500_000, "Output should be 500KB");

        // Compression ratio slightly less than 4x due to cropping
        // Input: 1001*1001*2 = 2,004,002 bytes
        // Output: 500*500*2 = 500,000 bytes
        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            ratio > 4.0,
            "Compression ratio should be slightly better than 4x due to cropping, got {:.2}x",
            ratio
        );
    }

    #[test]
    fn test_compression_with_odd_dimensions_4x4() {
        // Odd dimensions should still achieve approximately 16x compression
        // 1003x1003 -> usable 1000x1000 -> 250x250
        let width = 1003;
        let height = 1003;
        let data = create_test_frame(width, height);

        let input_bytes = data.len();
        let (result, w, h) = downsample_4x4(&data, width, height);

        assert_eq!(w, 250, "Expected 250 pixel width (1000/4)");
        assert_eq!(h, 250, "Expected 250 pixel height (1000/4)");
        assert_eq!(result.len(), 125_000, "Output should be 125KB");

        // Compression ratio slightly better than 16x due to cropping
        // Input: 1003*1003*2 = 2,012,018 bytes
        // Output: 250*250*2 = 125,000 bytes
        let ratio = input_bytes as f64 / result.len() as f64;
        assert!(
            ratio > 16.0,
            "Compression ratio should be slightly better than 16x due to cropping, got {:.2}x",
            ratio
        );
    }

    // =========================================================================
    // Data Integrity Tests (bd-p55f)
    // Verify averaged values are mathematically correct
    // =========================================================================

    /// Helper to create a frame from a 2D array of u16 values
    fn create_frame_from_pixels(pixels: &[&[u16]]) -> Vec<u8> {
        let mut data = Vec::new();
        for row in pixels {
            for &val in *row {
                data.extend_from_slice(&val.to_le_bytes());
            }
        }
        data
    }

    /// Helper to read a pixel from downsampled output
    fn read_pixel(data: &[u8], x: usize, y: usize, width: usize) -> u16 {
        let idx = (y * width + x) * 2;
        u16::from_le_bytes([data[idx], data[idx + 1]])
    }

    #[test]
    fn test_downsample_2x2_data_integrity_unique_values() {
        // Create a 4x4 image where each 2x2 block has unique values
        // Block (0,0): [100, 200, 300, 400] -> avg = 250
        // Block (1,0): [500, 600, 700, 800] -> avg = 650
        // Block (0,1): [1000, 2000, 3000, 4000] -> avg = 2500
        // Block (1,1): [10000, 20000, 30000, 40000] -> avg = 25000
        let pixels: &[&[u16]] = &[
            &[100, 200, 500, 600],
            &[300, 400, 700, 800],
            &[1000, 2000, 10000, 20000],
            &[3000, 4000, 30000, 40000],
        ];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 4, 4);
        assert_eq!((w, h), (2, 2));

        // Verify each averaged pixel
        assert_eq!(
            read_pixel(&result, 0, 0, 2),
            250,
            "Block (0,0): avg(100,200,300,400) should be 250"
        );
        assert_eq!(
            read_pixel(&result, 1, 0, 2),
            650,
            "Block (1,0): avg(500,600,700,800) should be 650"
        );
        assert_eq!(
            read_pixel(&result, 0, 1, 2),
            2500,
            "Block (0,1): avg(1000,2000,3000,4000) should be 2500"
        );
        assert_eq!(
            read_pixel(&result, 1, 1, 2),
            25000,
            "Block (1,1): avg(10000,20000,30000,40000) should be 25000"
        );
    }

    #[test]
    fn test_downsample_4x4_data_integrity_unique_values() {
        // Create an 8x8 image with distinct 4x4 blocks
        // Block (0,0): all values = 100 -> avg = 100
        // Block (1,0): values 0..15 -> avg = (0+1+...+15)/16 = 120/16 = 7 (truncated)
        // Block (0,1): all values = 1000 -> avg = 1000
        // Block (1,1): all values = 1000 -> avg = 1000

        let mut pixels: Vec<Vec<u16>> = vec![vec![0; 8]; 8];

        // Fill top-left 4x4 block with 100
        for y in 0..4 {
            for x in 0..4 {
                pixels[y][x] = 100;
            }
        }

        // Fill top-right 4x4 block with sequential values 0..15
        // Sum = 0+1+2+...+15 = 120, avg = 120/16 = 7 (truncated)
        let mut val = 0u16;
        for y in 0..4 {
            for x in 4..8 {
                pixels[y][x] = val;
                val += 1;
            }
        }

        // Fill bottom-left 4x4 block with 1000
        for y in 4..8 {
            for x in 0..4 {
                pixels[y][x] = 1000;
            }
        }

        // Fill bottom-right 4x4 block with 1000
        for y in 4..8 {
            for x in 4..8 {
                pixels[y][x] = 1000;
            }
        }

        let pixel_refs: Vec<&[u16]> = pixels.iter().map(|r| r.as_slice()).collect();
        let data = create_frame_from_pixels(&pixel_refs);

        let (result, w, h) = downsample_4x4(&data, 8, 8);
        assert_eq!((w, h), (2, 2));

        assert_eq!(
            read_pixel(&result, 0, 0, 2),
            100,
            "Block (0,0): uniform 100 should average to 100"
        );
        assert_eq!(
            read_pixel(&result, 1, 0, 2),
            7,
            "Block (1,0): sum(0..15)=120, 120/16=7 (truncated)"
        );
        assert_eq!(
            read_pixel(&result, 0, 1, 2),
            1000,
            "Block (0,1): uniform 1000 should average to 1000"
        );
        assert_eq!(
            read_pixel(&result, 1, 1, 2),
            1000,
            "Block (1,1): uniform 1000 should average to 1000"
        );
    }

    #[test]
    fn test_downsample_2x2_horizontal_gradient() {
        // Test that spatial positioning is correct with a horizontal gradient
        // Each column pair has the same value, increasing from left to right
        // 6x4 image -> 3x2 output
        let pixels: &[&[u16]] = &[
            &[0, 0, 100, 100, 200, 200],
            &[0, 0, 100, 100, 200, 200],
            &[0, 0, 100, 100, 200, 200],
            &[0, 0, 100, 100, 200, 200],
        ];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 6, 4);
        assert_eq!((w, h), (3, 2));

        // First row of output
        assert_eq!(read_pixel(&result, 0, 0, 3), 0);
        assert_eq!(read_pixel(&result, 1, 0, 3), 100);
        assert_eq!(read_pixel(&result, 2, 0, 3), 200);

        // Second row should be identical
        assert_eq!(read_pixel(&result, 0, 1, 3), 0);
        assert_eq!(read_pixel(&result, 1, 1, 3), 100);
        assert_eq!(read_pixel(&result, 2, 1, 3), 200);
    }

    #[test]
    fn test_downsample_2x2_vertical_gradient() {
        // Test vertical gradient: each row pair has the same value, increasing downward
        // 4x6 image -> 2x3 output
        let pixels: &[&[u16]] = &[
            &[0, 0, 0, 0],
            &[0, 0, 0, 0],
            &[100, 100, 100, 100],
            &[100, 100, 100, 100],
            &[200, 200, 200, 200],
            &[200, 200, 200, 200],
        ];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 4, 6);
        assert_eq!((w, h), (2, 3));

        // Verify vertical gradient preserved
        assert_eq!(read_pixel(&result, 0, 0, 2), 0);
        assert_eq!(read_pixel(&result, 1, 0, 2), 0);
        assert_eq!(read_pixel(&result, 0, 1, 2), 100);
        assert_eq!(read_pixel(&result, 1, 1, 2), 100);
        assert_eq!(read_pixel(&result, 0, 2, 2), 200);
        assert_eq!(read_pixel(&result, 1, 2, 2), 200);
    }

    #[test]
    fn test_downsample_2x2_8bit_range() {
        // Test with values in 8-bit range (0-255)
        // Block values: [0, 128, 64, 192] -> avg = 96
        let pixels: &[&[u16]] = &[&[0, 128], &[64, 192]];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 2, 2);
        assert_eq!((w, h), (1, 1));

        // (0 + 128 + 64 + 192) / 4 = 384 / 4 = 96
        assert_eq!(read_pixel(&result, 0, 0, 1), 96);
    }

    #[test]
    fn test_downsample_2x2_16bit_range_max() {
        // Test with max u16 values to verify no overflow
        // [65535, 65535, 65535, 65535] -> avg = 65535
        let pixels: &[&[u16]] = &[&[65535, 65535], &[65535, 65535]];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(
            read_pixel(&result, 0, 0, 1),
            65535,
            "Average of four 65535 values should be 65535"
        );
    }

    #[test]
    fn test_downsample_4x4_16bit_range_max() {
        // Test 4x4 with max u16 values to ensure no overflow
        // 16 pixels of 65535: sum = 16 * 65535 = 1048560, avg = 65535
        let pixels: Vec<Vec<u16>> = vec![vec![65535; 4]; 4];
        let pixel_refs: Vec<&[u16]> = pixels.iter().map(|r| r.as_slice()).collect();
        let data = create_frame_from_pixels(&pixel_refs);

        let (result, w, h) = downsample_4x4(&data, 4, 4);
        assert_eq!((w, h), (1, 1));
        assert_eq!(
            read_pixel(&result, 0, 0, 1),
            65535,
            "Average of sixteen 65535 values should be 65535"
        );
    }

    #[test]
    fn test_downsample_2x2_rounding_truncation() {
        // Test that integer division truncates (floors) as expected
        // Values: [1, 2, 3, 4] -> sum = 10, avg = 10/4 = 2 (not 2.5 rounded)
        let pixels: &[&[u16]] = &[&[1, 2], &[3, 4]];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(
            read_pixel(&result, 0, 0, 1),
            2,
            "10/4 should truncate to 2, not round to 3"
        );
    }

    #[test]
    fn test_downsample_4x4_rounding_truncation() {
        // Create 4x4 block with values that result in non-integer average
        // Values: fifteen 1s and one 2 -> sum = 17, avg = 17/16 = 1 (truncated)
        let mut pixels: Vec<Vec<u16>> = vec![vec![1; 4]; 4];
        pixels[3][3] = 2; // Make sum = 17
        let pixel_refs: Vec<&[u16]> = pixels.iter().map(|r| r.as_slice()).collect();
        let data = create_frame_from_pixels(&pixel_refs);

        let (result, w, h) = downsample_4x4(&data, 4, 4);
        assert_eq!((w, h), (1, 1));
        assert_eq!(
            read_pixel(&result, 0, 0, 1),
            1,
            "17/16 should truncate to 1"
        );
    }

    #[test]
    fn test_downsample_2x2_checkerboard_pattern() {
        // Checkerboard pattern tests that all 4 pixels contribute correctly
        // Pattern: 0, 1000 alternating -> each 2x2 block has two 0s and two 1000s -> avg = 500
        let pixels: &[&[u16]] = &[
            &[0, 1000, 0, 1000],
            &[1000, 0, 1000, 0],
            &[0, 1000, 0, 1000],
            &[1000, 0, 1000, 0],
        ];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 4, 4);
        assert_eq!((w, h), (2, 2));

        // Each 2x2 block has two 0s and two 1000s -> avg = 500
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    read_pixel(&result, x, y, 2),
                    500,
                    "Checkerboard block ({},{}) should average to 500",
                    x,
                    y
                );
            }
        }
    }

    #[test]
    fn test_downsample_4x4_diagonal_gradient() {
        // Test 4x4 with diagonal gradient pattern
        // Each pixel value = x + y (0 to 6 for 4x4)
        // Sum = all (x+y) for x,y in 0..4
        // = sum of x (0+1+2+3)*4 + sum of y (0+1+2+3)*4 = 6*4 + 6*4 = 48
        // avg = 48/16 = 3
        let mut pixels: Vec<Vec<u16>> = vec![vec![0; 4]; 4];
        for y in 0..4 {
            for x in 0..4 {
                pixels[y][x] = (x + y) as u16;
            }
        }
        let pixel_refs: Vec<&[u16]> = pixels.iter().map(|r| r.as_slice()).collect();
        let data = create_frame_from_pixels(&pixel_refs);

        let (result, w, h) = downsample_4x4(&data, 4, 4);
        assert_eq!((w, h), (1, 1));
        assert_eq!(
            read_pixel(&result, 0, 0, 1),
            3,
            "Diagonal gradient 4x4 should average to 3"
        );
    }

    #[test]
    fn test_downsample_2x2_preserves_relative_intensities() {
        // Test that relative intensities are preserved after downsampling
        // 8x4 image with distinct 2x2 blocks of increasing intensity
        let pixels: &[&[u16]] = &[
            &[1000, 1000, 2000, 2000, 3000, 3000, 4000, 4000],
            &[1000, 1000, 2000, 2000, 3000, 3000, 4000, 4000],
            &[5000, 5000, 6000, 6000, 7000, 7000, 8000, 8000],
            &[5000, 5000, 6000, 6000, 7000, 7000, 8000, 8000],
        ];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 8, 4);
        assert_eq!((w, h), (4, 2));

        // Verify the relative ordering is preserved
        let mut values = Vec::new();
        for y in 0..2 {
            for x in 0..4 {
                values.push(read_pixel(&result, x, y, 4));
            }
        }

        assert_eq!(
            values,
            vec![1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000],
            "Relative intensities should be preserved"
        );
    }

    #[test]
    fn test_downsample_4x4_scientific_accuracy() {
        // Test case relevant to scientific imaging:
        // Simulate a bright spot (high count region) surrounded by background
        // 8x8 image with a 4x4 bright region (10000) in top-left, rest is background (100)
        let mut pixels: Vec<Vec<u16>> = vec![vec![100; 8]; 8];

        // Bright spot in top-left 4x4
        for y in 0..4 {
            for x in 0..4 {
                pixels[y][x] = 10000;
            }
        }

        let pixel_refs: Vec<&[u16]> = pixels.iter().map(|r| r.as_slice()).collect();
        let data = create_frame_from_pixels(&pixel_refs);

        let (result, w, h) = downsample_4x4(&data, 8, 8);
        assert_eq!((w, h), (2, 2));

        // Top-left block should be bright (10000)
        assert_eq!(
            read_pixel(&result, 0, 0, 2),
            10000,
            "Bright spot should remain at 10000"
        );

        // Other blocks should be background (100)
        assert_eq!(
            read_pixel(&result, 1, 0, 2),
            100,
            "Background should be 100"
        );
        assert_eq!(
            read_pixel(&result, 0, 1, 2),
            100,
            "Background should be 100"
        );
        assert_eq!(
            read_pixel(&result, 1, 1, 2),
            100,
            "Background should be 100"
        );
    }

    #[test]
    fn test_downsample_2x2_mixed_values_exact_average() {
        // Test with values that produce an exact integer average
        // Block: [0, 4, 8, 12] -> sum = 24, avg = 6 (exact)
        let pixels: &[&[u16]] = &[&[0, 4], &[8, 12]];
        let data = create_frame_from_pixels(pixels);

        let (result, w, h) = downsample_2x2(&data, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(read_pixel(&result, 0, 0, 1), 6, "24/4 should be exactly 6");
    }

    #[test]
    fn test_downsample_4x4_sequential_values() {
        // Fill 4x4 with values 0..15 in row-major order
        // Sum = 0+1+2+...+15 = 120, avg = 120/16 = 7 (truncated from 7.5)
        let mut pixels: Vec<Vec<u16>> = vec![vec![0; 4]; 4];
        let mut val = 0u16;
        for y in 0..4 {
            for x in 0..4 {
                pixels[y][x] = val;
                val += 1;
            }
        }
        let pixel_refs: Vec<&[u16]> = pixels.iter().map(|r| r.as_slice()).collect();
        let data = create_frame_from_pixels(&pixel_refs);

        let (result, w, h) = downsample_4x4(&data, 4, 4);
        assert_eq!((w, h), (1, 1));
        assert_eq!(
            read_pixel(&result, 0, 0, 1),
            7,
            "sum(0..15)=120, 120/16=7 (truncated)"
        );
    }

    #[test]
    fn test_downsample_2x2_all_same_value() {
        // When all pixels are the same, average should equal that value
        for test_val in [0u16, 1, 255, 1000, 32768, 65535] {
            let pixels: &[&[u16]] = &[&[test_val, test_val], &[test_val, test_val]];
            let data = create_frame_from_pixels(pixels);

            let (result, w, h) = downsample_2x2(&data, 2, 2);
            assert_eq!((w, h), (1, 1));
            assert_eq!(
                read_pixel(&result, 0, 0, 1),
                test_val,
                "Uniform value {} should average to {}",
                test_val,
                test_val
            );
        }
    }

    #[test]
    fn test_downsample_4x4_all_same_value() {
        // When all pixels are the same, average should equal that value
        for test_val in [0u16, 1, 255, 1000, 32768, 65535] {
            let pixels: Vec<Vec<u16>> = vec![vec![test_val; 4]; 4];
            let pixel_refs: Vec<&[u16]> = pixels.iter().map(|r| r.as_slice()).collect();
            let data = create_frame_from_pixels(&pixel_refs);

            let (result, w, h) = downsample_4x4(&data, 4, 4);
            assert_eq!((w, h), (1, 1));
            assert_eq!(
                read_pixel(&result, 0, 0, 1),
                test_val,
                "Uniform value {} should average to {}",
                test_val,
                test_val
            );
        }
    }
}
