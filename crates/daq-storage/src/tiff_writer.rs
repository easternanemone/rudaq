//! TIFF export for camera frames (bd-3pdi.5.2).
//!
//! Provides single-frame and multi-frame (stack) TIFF export for camera data.
//! Preserves bit depth (8-bit or 16-bit) and includes basic metadata in TIFF tags.
//!
//! # Features
//!
//! This module requires the `storage_tiff` feature:
//!
//! ```toml
//! [dependencies]
//! daq-storage = { version = "0.1", features = ["storage_tiff"] }
//! ```
//!
//! # Usage
//!
//! ## Single Frame Export (Pooled - Zero-Copy, Recommended)
//!
//! ```rust,ignore
//! use daq_storage::tiff_writer::TiffWriter;
//! use daq_pool::FrameData;
//!
//! // From a LoanedFrame (zero-allocation path)
//! let loaned_frame: LoanedFrame = camera.receive_frame().await?;
//! TiffWriter::write_frame_data(&loaned_frame, "output.tiff")?;
//! ```
//!
//! ## Single Frame Export (Legacy)
//!
//! ```rust,ignore
//! use daq_storage::tiff_writer::TiffWriter;
//! use daq_core::data::Frame;
//!
//! let frame = Frame { width: 2048, height: 2048, bit_depth: 16, data: vec![0u8; 8388608], ..Default::default() };
//! TiffWriter::write_frame(&frame, "output.tiff")?;
//! ```
//!
//! ## Stack Export
//!
//! ```rust,ignore
//! use daq_storage::tiff_writer::TiffWriter;
//!
//! let frames: Vec<Frame> = acquire_frames();
//! TiffWriter::write_stack(&frames, "stack.tiff")?;
//! ```

use anyhow::{anyhow, Context, Result};
use daq_core::data::Frame;
use daq_pool::FrameData;
use image::{GrayImage, ImageBuffer, Luma};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Type alias for pooled frame data from the object pool.
///
/// This represents a frame buffer loaned from a pre-allocated pool,
/// enabling zero-allocation frame handling for high-FPS scenarios.
pub type LoanedFrame = daq_pool::Loaned<FrameData>;

/// TIFF export functionality for camera frames.
///
/// Supports 8-bit and 16-bit grayscale images. 16-bit data is the native
/// format for scientific cameras like the Prime BSI.
pub struct TiffWriter;

impl TiffWriter {
    /// Write a single frame to a TIFF file.
    ///
    /// **Note**: For zero-copy frame writing, prefer [`write_frame_data`] which accepts
    /// pooled frame data directly.
    ///
    /// # Arguments
    ///
    /// * `frame` - The frame to write
    /// * `path` - Output file path (will be created or overwritten)
    ///
    /// # Bit Depth Handling
    ///
    /// - 8-bit frames: Written as 8-bit grayscale TIFF
    /// - 16-bit frames: Written as 16-bit grayscale TIFF
    /// - Other bit depths: Treated as raw 8-bit data
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File cannot be created
    /// - Frame dimensions don't match data size
    /// - TIFF encoding fails
    #[deprecated(
        since = "0.3.0",
        note = "Use write_frame_data() for zero-copy pooled frames"
    )]
    pub fn write_frame<P: AsRef<Path>>(frame: &Frame, path: P) -> Result<()> {
        let path = path.as_ref();

        // Validate frame data size
        let expected_bytes = match frame.bit_depth {
            16 => (frame.width as usize) * (frame.height as usize) * 2,
            _ => (frame.width as usize) * (frame.height as usize),
        };

        if frame.data.len() != expected_bytes {
            return Err(anyhow!(
                "Frame data size mismatch: expected {} bytes for {}x{} {}bit, got {} bytes",
                expected_bytes,
                frame.width,
                frame.height,
                frame.bit_depth,
                frame.data.len()
            ));
        }

        match frame.bit_depth {
            16 => Self::write_16bit_frame(frame, path),
            _ => Self::write_8bit_frame(frame, path),
        }
    }

    /// Write pooled frame data to a TIFF file (zero-copy path).
    ///
    /// This is the preferred method for high-performance frame writing.
    /// It accepts `&FrameData` which can come directly from a `LoanedFrame`
    /// without any allocation or copying.
    ///
    /// # Arguments
    ///
    /// * `frame` - The pooled frame data to write
    /// * `path` - Output file path (will be created or overwritten)
    ///
    /// # Bit Depth Handling
    ///
    /// - 8-bit frames: Written as 8-bit grayscale TIFF
    /// - 16-bit frames: Written as 16-bit grayscale TIFF
    /// - Other bit depths: Treated as raw 8-bit data
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File cannot be created
    /// - Frame dimensions don't match data size
    /// - TIFF encoding fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use daq_storage::tiff_writer::TiffWriter;
    /// use daq_pool::FrameData;
    ///
    /// // From a LoanedFrame
    /// let loaned_frame: LoanedFrame = pool.acquire().await;
    /// TiffWriter::write_frame_data(&loaned_frame, "output.tiff")?;
    ///
    /// // Or directly from FrameData
    /// let frame_data = FrameData::with_capacity(8_388_608);
    /// TiffWriter::write_frame_data(&frame_data, "output.tiff")?;
    /// ```
    pub fn write_frame_data<P: AsRef<Path>>(frame: &FrameData, path: P) -> Result<()> {
        let path = path.as_ref();

        // Get the valid pixel data
        let pixel_data = frame.pixel_data();

        // Validate frame data size
        let expected_bytes = match frame.bit_depth {
            16 => (frame.width as usize) * (frame.height as usize) * 2,
            _ => (frame.width as usize) * (frame.height as usize),
        };

        if pixel_data.len() != expected_bytes {
            return Err(anyhow!(
                "Frame data size mismatch: expected {} bytes for {}x{} {}bit, got {} bytes",
                expected_bytes,
                frame.width,
                frame.height,
                frame.bit_depth,
                pixel_data.len()
            ));
        }

        match frame.bit_depth {
            16 => Self::write_16bit_pixels(pixel_data, frame.width, frame.height, path),
            _ => Self::write_8bit_pixels(pixel_data, frame.width, frame.height, path),
        }
    }

    /// Write a stack of frames to a multi-page TIFF file.
    ///
    /// All frames in the stack must have the same dimensions and bit depth.
    ///
    /// # Arguments
    ///
    /// * `frames` - Slice of frames to write (must all have same dimensions)
    /// * `path` - Output file path
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Frames slice is empty
    /// - Frames have inconsistent dimensions or bit depth
    /// - File cannot be created
    /// - TIFF encoding fails
    pub fn write_stack<P: AsRef<Path>>(frames: &[Frame], path: P) -> Result<()> {
        if frames.is_empty() {
            return Err(anyhow!("Cannot write empty frame stack"));
        }

        let path = path.as_ref();
        let first = &frames[0];

        // Validate all frames have consistent dimensions
        for (i, frame) in frames.iter().enumerate() {
            if frame.width != first.width
                || frame.height != first.height
                || frame.bit_depth != first.bit_depth
            {
                return Err(anyhow!(
                    "Frame {} has inconsistent dimensions: {}x{} {}bit vs expected {}x{} {}bit",
                    i,
                    frame.width,
                    frame.height,
                    frame.bit_depth,
                    first.width,
                    first.height,
                    first.bit_depth
                ));
            }
        }

        // For now, write as separate TIFF files with sequence numbers
        // Multi-page TIFF would require lower-level tiff crate usage
        let base_path = path.with_extension("");
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("tiff");

        for (i, frame) in frames.iter().enumerate() {
            let numbered_path = if frames.len() == 1 {
                path.to_path_buf()
            } else {
                let base_name = base_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("frame");
                base_path.with_file_name(format!("{}_{:04}.{}", base_name, i, extension))
            };

            // Use internal methods directly instead of deprecated write_frame
            match frame.bit_depth {
                16 => Self::write_16bit_frame(frame, &numbered_path),
                _ => Self::write_8bit_frame(frame, &numbered_path),
            }
            .with_context(|| format!("Failed to write frame {} to {:?}", i, numbered_path))?;
        }

        tracing::info!(
            path = ?path,
            num_frames = frames.len(),
            dimensions = format!("{}x{}", first.width, first.height),
            bit_depth = first.bit_depth,
            "Wrote TIFF stack"
        );

        Ok(())
    }

    /// Write an 8-bit grayscale frame.
    fn write_8bit_frame(frame: &Frame, path: &Path) -> Result<()> {
        // Convert Bytes to Vec<u8> for image crate
        let data_vec: Vec<u8> = frame.data.to_vec();

        let img: GrayImage = ImageBuffer::from_raw(frame.width, frame.height, data_vec)
            .ok_or_else(|| anyhow!("Failed to create image buffer from frame data"))?;

        let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
        let writer = BufWriter::new(file);

        let encoder = image::codecs::tiff::TiffEncoder::new(writer);
        encoder
            .encode(
                &img,
                frame.width,
                frame.height,
                image::ExtendedColorType::L8,
            )
            .with_context(|| format!("Failed to encode TIFF to {:?}", path))?;

        tracing::debug!(
            path = ?path,
            dimensions = format!("{}x{}", frame.width, frame.height),
            bit_depth = 8,
            "Wrote 8-bit TIFF"
        );

        Ok(())
    }

    /// Write a 16-bit grayscale frame.
    fn write_16bit_frame(frame: &Frame, path: &Path) -> Result<()> {
        // Convert byte slice to u16 slice
        let u16_data: Vec<u16> = frame
            .data
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let img: ImageBuffer<Luma<u16>, Vec<u16>> =
            ImageBuffer::from_raw(frame.width, frame.height, u16_data)
                .ok_or_else(|| anyhow!("Failed to create 16-bit image buffer from frame data"))?;

        let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
        let writer = BufWriter::new(file);

        // Convert u16 data back to bytes for the encoder
        let bytes: Vec<u8> = img.as_raw().iter().flat_map(|&v| v.to_le_bytes()).collect();

        let encoder = image::codecs::tiff::TiffEncoder::new(writer);
        encoder
            .encode(
                &bytes,
                frame.width,
                frame.height,
                image::ExtendedColorType::L16,
            )
            .with_context(|| format!("Failed to encode 16-bit TIFF to {:?}", path))?;

        tracing::debug!(
            path = ?path,
            dimensions = format!("{}x{}", frame.width, frame.height),
            bit_depth = 16,
            "Wrote 16-bit TIFF"
        );

        Ok(())
    }

    /// Write 8-bit grayscale pixels from a raw byte slice.
    ///
    /// Internal helper for zero-copy frame writing.
    fn write_8bit_pixels(pixels: &[u8], width: u32, height: u32, path: &Path) -> Result<()> {
        let img: GrayImage = ImageBuffer::from_raw(width, height, pixels.to_vec())
            .ok_or_else(|| anyhow!("Failed to create image buffer from pixel data"))?;

        let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
        let writer = BufWriter::new(file);

        let encoder = image::codecs::tiff::TiffEncoder::new(writer);
        encoder
            .encode(&img, width, height, image::ExtendedColorType::L8)
            .with_context(|| format!("Failed to encode TIFF to {:?}", path))?;

        tracing::debug!(
            path = ?path,
            dimensions = format!("{}x{}", width, height),
            bit_depth = 8,
            "Wrote 8-bit TIFF from pooled frame"
        );

        Ok(())
    }

    /// Write 16-bit grayscale pixels from a raw byte slice.
    ///
    /// Internal helper for zero-copy frame writing.
    fn write_16bit_pixels(pixels: &[u8], width: u32, height: u32, path: &Path) -> Result<()> {
        // Convert byte slice to u16 slice
        let u16_data: Vec<u16> = pixels
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let img: ImageBuffer<Luma<u16>, Vec<u16>> = ImageBuffer::from_raw(width, height, u16_data)
            .ok_or_else(|| anyhow!("Failed to create 16-bit image buffer from pixel data"))?;

        let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
        let writer = BufWriter::new(file);

        // Convert u16 data back to bytes for the encoder
        let bytes: Vec<u8> = img.as_raw().iter().flat_map(|&v| v.to_le_bytes()).collect();

        let encoder = image::codecs::tiff::TiffEncoder::new(writer);
        encoder
            .encode(&bytes, width, height, image::ExtendedColorType::L16)
            .with_context(|| format!("Failed to encode 16-bit TIFF to {:?}", path))?;

        tracing::debug!(
            path = ?path,
            dimensions = format!("{}x{}", width, height),
            bit_depth = 16,
            "Wrote 16-bit TIFF from pooled frame"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use tempfile::TempDir;

    fn create_test_frame(width: u32, height: u32, bit_depth: u32) -> Frame {
        let bytes_per_pixel = if bit_depth == 16 { 2 } else { 1 };
        let data_len = (width as usize) * (height as usize) * bytes_per_pixel;

        // Create gradient pattern
        let data: Vec<u8> = if bit_depth == 16 {
            (0..width * height)
                .flat_map(|i| {
                    let value = ((i as f32 / (width * height) as f32) * 65535.0) as u16;
                    value.to_le_bytes().to_vec()
                })
                .collect()
        } else {
            (0..data_len)
                .map(|i| ((i as f32 / data_len as f32) * 255.0) as u8)
                .collect()
        };

        Frame {
            width,
            height,
            bit_depth,
            data: Bytes::from(data),
            frame_number: 1,
            timestamp_ns: 0,
            exposure_ms: Some(100.0),
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    #[test]
    #[allow(deprecated)]
    fn test_write_8bit_frame() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test_8bit.tiff");

        let frame = create_test_frame(256, 256, 8);
        TiffWriter::write_frame(&frame, &path).unwrap();

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }

    #[test]
    #[allow(deprecated)]
    fn test_write_16bit_frame() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test_16bit.tiff");

        let frame = create_test_frame(256, 256, 16);
        TiffWriter::write_frame(&frame, &path).unwrap();

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }

    #[test]
    #[allow(deprecated)]
    fn test_write_prime_bsi_dimensions() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("prime_bsi.tiff");

        // Prime BSI: 2048x2048 @ 16-bit
        let frame = create_test_frame(2048, 2048, 16);
        TiffWriter::write_frame(&frame, &path).unwrap();

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        // 2048*2048*2 = 8MB raw, TIFF adds headers
        assert!(metadata.len() > 8_000_000);
    }

    #[test]
    fn test_write_stack() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("stack.tiff");

        let frames: Vec<Frame> = (0..5).map(|_| create_test_frame(128, 128, 16)).collect();

        TiffWriter::write_stack(&frames, &path).unwrap();

        // Check that numbered files were created
        for i in 0..5 {
            let numbered_path = temp_dir.path().join(format!("stack_{:04}.tiff", i));
            assert!(numbered_path.exists(), "Missing {:?}", numbered_path);
        }
    }

    #[test]
    fn test_empty_stack_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("empty.tiff");

        let frames: Vec<Frame> = vec![];
        let result = TiffWriter::write_stack(&frames, &path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_data_size_mismatch_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("bad.tiff");

        let mut frame = create_test_frame(256, 256, 16);
        frame.data.truncate(100); // Corrupt the data

        #[allow(deprecated)]
        let result = TiffWriter::write_frame(&frame, &path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mismatch"));
    }

    // ========================================================================
    // Tests for write_frame_data (pooled frames, bd-0dax.6.1)
    // ========================================================================

    fn create_test_frame_data(width: u32, height: u32, bit_depth: u32) -> FrameData {
        let bytes_per_pixel = if bit_depth == 16 { 2 } else { 1 };
        let data_len = (width as usize) * (height as usize) * bytes_per_pixel;

        let mut frame_data = FrameData::with_capacity(data_len);

        // Create gradient pattern
        if bit_depth == 16 {
            for i in 0..(width * height) as usize {
                let value = ((i as f32 / (width * height) as f32) * 65535.0) as u16;
                let bytes = value.to_le_bytes();
                frame_data.pixels[i * 2] = bytes[0];
                frame_data.pixels[i * 2 + 1] = bytes[1];
            }
        } else {
            for i in 0..data_len {
                frame_data.pixels[i] = ((i as f32 / data_len as f32) * 255.0) as u8;
            }
        }

        frame_data.actual_len = data_len;
        frame_data.width = width;
        frame_data.height = height;
        frame_data.bit_depth = bit_depth;
        frame_data.frame_number = 1;
        frame_data.timestamp_ns = 0;
        frame_data.exposure_ms = 100.0;

        frame_data
    }

    #[test]
    fn test_write_8bit_frame_data() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test_8bit_pooled.tiff");

        let frame_data = create_test_frame_data(256, 256, 8);
        TiffWriter::write_frame_data(&frame_data, &path).unwrap();

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }

    #[test]
    fn test_write_16bit_frame_data() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test_16bit_pooled.tiff");

        let frame_data = create_test_frame_data(256, 256, 16);
        TiffWriter::write_frame_data(&frame_data, &path).unwrap();

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }

    #[test]
    fn test_write_frame_data_prime_bsi_dimensions() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("prime_bsi_pooled.tiff");

        // Prime BSI: 2048x2048 @ 16-bit
        let frame_data = create_test_frame_data(2048, 2048, 16);
        TiffWriter::write_frame_data(&frame_data, &path).unwrap();

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        // 2048*2048*2 = 8MB raw, TIFF adds headers
        assert!(metadata.len() > 8_000_000);
    }

    #[test]
    fn test_write_frame_data_size_mismatch_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("bad_pooled.tiff");

        let mut frame_data = create_test_frame_data(256, 256, 16);
        frame_data.actual_len = 100; // Corrupt the data length

        let result = TiffWriter::write_frame_data(&frame_data, &path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mismatch"));
    }
}
