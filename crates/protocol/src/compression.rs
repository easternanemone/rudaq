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
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    let uncompressed_size = frame.data.len() as u32;
    let compressed = lz4_flex::compress_prepend_size(&frame.data);

    frame.data = compressed;
    frame.compression = CompressionType::CompressionLz4 as i32;
    frame.uncompressed_size = uncompressed_size;
}

/// Compress frame data using LZ4, reusing a pre-allocated buffer.
///
/// The buffer is resized as needed but never shrunk, eliminating per-frame
/// heap allocations after the first frame. Uses `lz4_flex::compress_into`
/// with a 4-byte LE size prefix for wire compatibility with `decompress_frame`.
///
/// The compressed data is written into `buffer`, then swapped into `frame.data`.
pub fn compress_frame_into(frame: &mut FrameData, buffer: &mut Vec<u8>) {
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    let uncompressed_size = frame.data.len() as u32;

    // 4-byte LE size prefix + worst-case compressed output
    let max_compressed = lz4_flex::block::get_maximum_output_size(frame.data.len());
    let required = 4 + max_compressed;
    buffer.resize(required, 0);

    // Write uncompressed size as 4-byte LE prefix (same format as compress_prepend_size)
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: frame sizes are bounded by MAX_FRAME_BYTES (100MB) which fits in u32
    let size_prefix = (frame.data.len() as u32).to_le_bytes();
    buffer[..4].copy_from_slice(&size_prefix);

    // Compress directly into the buffer after the size prefix
    let compressed_len = lz4_flex::compress_into(&frame.data, &mut buffer[4..])
        .expect("buffer is sized to get_maximum_output_size");

    buffer.truncate(4 + compressed_len);

    // Swap buffers: frame gets compressed data, buffer gets the (now-stale) raw data for reuse
    std::mem::swap(&mut frame.data, buffer);

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

/// Decompress frame data into a pre-allocated buffer, avoiding per-frame allocation.
///
/// The buffer is resized to `uncompressed_size` via [`Vec::resize`] (zero-filled) and
/// reused across frames.  After a two-call warmup the buffer stabilises at
/// `uncompressed_size` capacity and no further heap allocation occurs, provided
/// the frame size stays constant.  On the first call the buffer is grown to
/// `uncompressed_size`; the swap at the end returns it holding the compressed
/// bytes (smaller), so the second call resizes it again — after which it holds
/// the previous decompressed bytes at full capacity on every subsequent call.
///
/// The compressed data must carry the 4-byte LE size prefix written by
/// [`compress_frame`] / [`compress_frame_into`].
///
/// After a successful call, `frame.data` holds the decompressed pixel data and
/// `buffer` holds the previous (now-stale) compressed bytes, ready for reuse.
///
/// # Errors
/// Returns `Err(String)` if:
/// - `frame.data` is fewer than 4 bytes (missing size prefix)
/// - The LZ4 payload is malformed
/// - The decompressed length does not match `frame.uncompressed_size`
/// - The compression type is not recognised
pub fn decompress_frame_into(frame: &mut FrameData, buffer: &mut Vec<u8>) -> Result<(), String> {
    match CompressionType::try_from(frame.compression) {
        Ok(CompressionType::CompressionNone) => Ok(()),
        Ok(CompressionType::CompressionLz4) => {
            let expected_size = frame.uncompressed_size as usize;
            // Resize to the expected decompressed size. The zero-fill is
            // overwritten immediately by decompress_into, and ensures
            // the buffer is always fully initialized on both success and
            // error paths.
            buffer.resize(expected_size, 0);

            // Skip the 4-byte LE size prefix written by compress_prepend_size / compress_frame_into
            if frame.data.len() < 4 {
                return Err("LZ4 compressed data too short (missing size prefix)".to_string());
            }
            let compressed_payload = &frame.data[4..];

            let decompressed_len = lz4_flex::decompress_into(compressed_payload, buffer)
                .map_err(|e| format!("LZ4 decompression failed: {e}"))?;

            if decompressed_len != expected_size {
                return Err(format!(
                    "Decompressed size mismatch: got {} bytes, expected {}",
                    decompressed_len, expected_size
                ));
            }

            // Swap: frame gets decompressed data, buffer gets compressed data for reuse
            std::mem::swap(&mut frame.data, buffer);
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
#[allow(clippy::cast_precision_loss)]
// SAFETY: precision loss acceptable for metrics/display
pub fn compression_ratio(frame: &FrameData) -> f64 {
    if frame.data.is_empty() || frame.uncompressed_size == 0 {
        return 1.0;
    }
    f64::from(frame.uncompressed_size) / frame.data.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
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
                let value = (x.wrapping_add(y)) % 256;
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

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn test_compress_into_decompress_into_roundtrip() {
        let original_data = vec![0u8; 20000];
        let mut frame = FrameData {
            device_id: "test_camera".to_string(),
            width: 100,
            height: 100,
            bit_depth: 16,
            data: original_data.clone(),
            frame_number: 1,
            ..Default::default()
        };

        let mut compress_buf = Vec::new();
        compress_frame_into(&mut frame, &mut compress_buf);

        assert_eq!(frame.compression, CompressionType::CompressionLz4 as i32);
        assert_eq!(frame.uncompressed_size, 20000);
        assert!(frame.data.len() < 20000);

        // Decompress with buffer reuse
        let mut decompress_buf = Vec::new();
        decompress_frame_into(&mut frame, &mut decompress_buf).expect("should decompress");

        assert_eq!(frame.data.len(), 20000);
        assert_eq!(frame.data, original_data);
        assert_eq!(frame.compression, CompressionType::CompressionNone as i32);
    }

    #[test]
    fn test_buffer_reuse_across_frames() {
        let mut compress_buf = Vec::new();
        let mut decompress_buf = Vec::new();

        for i in 0..5u8 {
            let data = vec![i; 10000];
            let mut frame = FrameData {
                width: 100,
                height: 50,
                bit_depth: 16,
                data: data.clone(),
                frame_number: u64::from(i),
                ..Default::default()
            };

            compress_frame_into(&mut frame, &mut compress_buf);
            decompress_frame_into(&mut frame, &mut decompress_buf).expect("roundtrip");
            assert_eq!(frame.data, data);
        }

        // Buffers should have been reused (capacity >= frame size)
        assert!(compress_buf.capacity() >= 10000);
        assert!(decompress_buf.capacity() >= 10000);
    }

    #[test]
    fn test_compress_into_wire_compatible_with_decompress_frame() {
        let mut frame = FrameData {
            data: vec![42u8; 5000],
            width: 50,
            height: 50,
            bit_depth: 16,
            ..Default::default()
        };

        // Compress with buffer-reuse API
        let mut buf = Vec::new();
        compress_frame_into(&mut frame, &mut buf);

        // Decompress with original (allocating) API — tests wire compatibility
        decompress_frame(&mut frame).expect("wire-compatible decompress");
        assert_eq!(frame.data, vec![42u8; 5000]);
    }

    #[test]
    fn test_compress_frame_decompress_into_compatibility() {
        let mut frame = FrameData {
            data: vec![99u8; 8000],
            width: 100,
            height: 40,
            bit_depth: 16,
            ..Default::default()
        };

        // Compress with original API
        compress_frame(&mut frame);

        // Decompress with buffer-reuse API
        let mut buf = Vec::new();
        decompress_frame_into(&mut frame, &mut buf).expect("cross-compatible decompress");
        assert_eq!(frame.data, vec![99u8; 8000]);
    }

    #[test]
    fn test_decompress_into_uncompressed_passthrough() {
        let mut frame = FrameData {
            data: vec![1, 2, 3, 4, 5],
            compression: CompressionType::CompressionNone as i32,
            ..Default::default()
        };

        let mut buf = Vec::new();
        decompress_frame_into(&mut frame, &mut buf).expect("passthrough");
        assert_eq!(frame.data, vec![1, 2, 3, 4, 5]);
    }
}
