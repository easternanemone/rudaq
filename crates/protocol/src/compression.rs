//! LZ4 compression for frame data (bd-7rk0: gRPC improvements).
//!
//! This module provides compression and decompression helpers for frame streaming,
//! based on lessons learned from Rerun's well-tested gRPC implementation.
//!
//! LZ4 was chosen because:
//! - Fast compression/decompression (designed for speed over ratio)
//! - Camera data typically achieves 3-5x compression
//! - Reduces bandwidth from ~240MB/s to ~48-80MB/s for 4MP@30fps cameras

use crate::daq::{CompressionType, FrameData};

/// Compress frame data using LZ4.
///
/// Returns the original FrameData with:
/// - `data` field replaced with compressed bytes
/// - `compression` field set to `COMPRESSION_LZ4`
/// - `uncompressed_size` field set to original size
///
/// # Example
/// ```ignore
/// let mut frame = FrameData { data: raw_pixels, ..Default::default() };
/// compress_frame(&mut frame);
/// // frame.data is now LZ4 compressed
/// ```
pub fn compress_frame(frame: &mut FrameData) {
    let uncompressed_size = frame.data.len() as u32;
    let compressed = lz4_flex::compress_prepend_size(&frame.data);

    frame.data = compressed;
    frame.compression = CompressionType::CompressionLz4 as i32;
    frame.uncompressed_size = uncompressed_size;
}

/// Decompress frame data if compressed.
///
/// If the frame is uncompressed (`COMPRESSION_NONE`), returns the data as-is.
/// If the frame is LZ4 compressed, decompresses and replaces the data field.
///
/// # Returns
/// - `Ok(())` on success (frame.data is now decompressed)
/// - `Err(String)` if decompression fails
///
/// # Example
/// ```ignore
/// decompress_frame(&mut frame)?;
/// // frame.data is now uncompressed pixels
/// ```
pub fn decompress_frame(frame: &mut FrameData) -> Result<(), String> {
    match CompressionType::try_from(frame.compression) {
        Ok(CompressionType::CompressionNone) => Ok(()),
        Ok(CompressionType::CompressionLz4) => {
            let decompressed = lz4_flex::decompress_size_prepended(&frame.data)
                .map_err(|e| format!("LZ4 decompression failed: {e}"))?;

            // Validate decompressed size matches expected
            if decompressed.len() != frame.uncompressed_size as usize {
                return Err(format!(
                    "Decompressed size mismatch: got {} bytes, expected {}",
                    decompressed.len(),
                    frame.uncompressed_size
                ));
            }

            frame.data = decompressed;
            frame.compression = CompressionType::CompressionNone as i32;
            Ok(())
        }
        Err(_) => Err(format!("Unknown compression type: {}", frame.compression)),
    }
}

/// Calculate compression ratio for logging/metrics.
///
/// Returns the ratio of uncompressed to compressed size.
/// A value of 3.0 means the data was compressed to 1/3 of its original size.
pub fn compression_ratio(frame: &FrameData) -> f64 {
    if frame.data.is_empty() || frame.uncompressed_size == 0 {
        return 1.0;
    }
    frame.uncompressed_size as f64 / frame.data.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_roundtrip() {
        // Create test frame with compressible data (lots of zeros)
        let mut frame = FrameData {
            device_id: "test_camera".to_string(),
            width: 100,
            height: 100,
            bit_depth: 16,
            data: vec![0u8; 20000], // 100x100 16-bit = 20000 bytes
            frame_number: 1,
            timestamp_ns: 12345,
            ..Default::default()
        };

        let original_size = frame.data.len();

        // Compress
        compress_frame(&mut frame);

        // Verify compression occurred
        assert_eq!(frame.compression, CompressionType::CompressionLz4 as i32);
        assert_eq!(frame.uncompressed_size, original_size as u32);
        assert!(
            frame.data.len() < original_size,
            "Data should be smaller after compression"
        );

        let compressed_size = frame.data.len();
        let ratio = compression_ratio(&frame);
        assert!(
            ratio > 1.0,
            "Compression ratio should be > 1 for compressible data"
        );

        // Decompress
        decompress_frame(&mut frame).expect("Decompression should succeed");

        // Verify decompression restored original
        assert_eq!(frame.compression, CompressionType::CompressionNone as i32);
        assert_eq!(frame.data.len(), original_size);
        assert!(
            frame.data.iter().all(|&b| b == 0),
            "Data should be restored to zeros"
        );

        println!(
            "Compression test: {} -> {} bytes (ratio: {:.2}x)",
            original_size, compressed_size, ratio
        );
    }

    #[test]
    fn test_uncompressed_passthrough() {
        let mut frame = FrameData {
            data: vec![1, 2, 3, 4, 5],
            compression: CompressionType::CompressionNone as i32,
            ..Default::default()
        };

        decompress_frame(&mut frame).expect("Should succeed for uncompressed data");
        assert_eq!(frame.data, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_real_image_compression() {
        // Simulate a more realistic image with some structure
        let mut data = Vec::with_capacity(2048 * 2048 * 2);
        for y in 0..2048u16 {
            for x in 0..2048u16 {
                // Create a gradient pattern (compressible but not trivially)
                let value = ((x.wrapping_add(y)) % 256) as u16;
                data.extend_from_slice(&value.to_le_bytes());
            }
        }

        let mut frame = FrameData {
            width: 2048,
            height: 2048,
            bit_depth: 16,
            data,
            ..Default::default()
        };

        let original_size = frame.data.len();
        compress_frame(&mut frame);
        let ratio = compression_ratio(&frame);

        println!(
            "Realistic image: {} -> {} bytes (ratio: {:.2}x)",
            original_size,
            frame.data.len(),
            ratio
        );

        // Even structured data should compress somewhat
        assert!(ratio > 1.0);

        // Verify roundtrip
        decompress_frame(&mut frame).expect("Should decompress");
        assert_eq!(frame.data.len(), original_size);
    }
}
