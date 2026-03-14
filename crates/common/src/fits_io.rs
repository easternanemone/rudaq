//! FITS file I/O for calibration frame import.
//!
//! Provides functions to load FITS images as flat `Vec<f32>` arrays (row-major),
//! consistent with the echelle extraction pipeline's frame representation.
//! Metadata (exposure time, gain, instrument, etc.) is extracted from standard
//! FITS header keywords.
//!
//! Requires the `fits` feature flag and the `cfitsio` C library at link time.
//!
//! # Example
//!
//! ```rust,ignore
//! use common::fits_io::{load_fits_frame, FitsMetadata};
//! use std::path::Path;
//!
//! let (pixels, width, height, meta) = load_fits_frame(Path::new("flat.fits"))?;
//! println!("{}x{}, exposure={:?}s", width, height, meta.exposure_secs);
//! ```

// Pixel-index and dimension casts: FITS dimensions are small enough that
// usize <-> u32 and usize -> f32 conversions are lossless in practice.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use fitsio::hdu::HduInfo;
use fitsio::FitsFile;
use std::path::Path;

/// Metadata extracted from FITS header keywords.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FitsMetadata {
    /// Exposure time in seconds (EXPTIME keyword).
    pub exposure_secs: Option<f64>,
    /// Detector gain in electrons per ADU (GAIN keyword).
    pub gain_e_per_adu: Option<f64>,
    /// Instrument name (INSTRUME keyword).
    pub instrument: Option<String>,
    /// Observation timestamp (DATE-OBS keyword).
    pub obs_time: Option<String>,
    /// Image width in pixels (NAXIS1).
    pub naxis1: usize,
    /// Image height in pixels (NAXIS2).
    pub naxis2: usize,
}

/// Load the primary HDU of a FITS file as an f32 image.
///
/// Returns `(pixels, width, height, metadata)` where `pixels` is a flat
/// row-major `Vec<f32>` with length `width * height`.
///
/// Integer pixel data (i16, i32) is converted to f32. BSCALE/BZERO
/// transformations are applied automatically by cfitsio when present.
///
/// # Errors
///
/// Returns `Err` if the file cannot be opened, the primary HDU is not an
/// image, or the image is not 2-dimensional.
pub fn load_fits_frame(path: &Path) -> Result<(Vec<f32>, u32, u32, FitsMetadata), String> {
    load_fits_frame_hdu(path, 0)
}

/// Load a specific HDU (extension) of a FITS file as an f32 image.
///
/// `hdu_index` is 0-based: 0 is the primary HDU, 1 is the first extension, etc.
///
/// Returns `(pixels, width, height, metadata)` where `pixels` is a flat
/// row-major `Vec<f32>` with length `width * height`.
///
/// # Errors
///
/// Returns `Err` if the file cannot be opened, the specified HDU is not an
/// image, or the image is not 2-dimensional.
pub fn load_fits_frame_hdu(
    path: &Path,
    hdu_index: usize,
) -> Result<(Vec<f32>, u32, u32, FitsMetadata), String> {
    let mut fptr = FitsFile::open(path).map_err(|e| format!("failed to open FITS file: {e}"))?;

    let hdu = fptr
        .hdu(hdu_index)
        .map_err(|e| format!("failed to access HDU {hdu_index}: {e}"))?;

    // Extract image dimensions from HDU info.
    let (naxis1, naxis2) = match &hdu.info {
        HduInfo::ImageInfo { shape, .. } => {
            if shape.len() != 2 {
                return Err(format!(
                    "expected 2D image, got {}D (shape: {shape:?})",
                    shape.len()
                ));
            }
            // fitsio shape is [NAXIS2, NAXIS1] (rows, cols) following FITS convention
            (shape[1], shape[0])
        }
        HduInfo::TableInfo { .. } => {
            return Err("HDU is a table, not an image".to_string());
        }
        HduInfo::AnyInfo => {
            return Err("HDU type is unknown (AnyInfo)".to_string());
        }
    };

    // Read image data as f32. cfitsio handles BSCALE/BZERO and type conversion
    // (e.g., i16 -> f32) automatically when the requested output type differs
    // from the on-disk type.
    let pixels: Vec<f32> = hdu
        .read_image(&mut fptr)
        .map_err(|e| format!("failed to read image data: {e}"))?;

    let expected_len = naxis1 * naxis2;
    if pixels.len() != expected_len {
        return Err(format!(
            "pixel count mismatch: expected {expected_len} ({}x{}), got {}",
            naxis1,
            naxis2,
            pixels.len()
        ));
    }

    // Extract metadata from header keywords (all optional, failures are ignored).
    let metadata = extract_metadata(&hdu, &mut fptr, naxis1, naxis2);

    let width = naxis1 as u32;
    let height = naxis2 as u32;

    Ok((pixels, width, height, metadata))
}

/// Read optional metadata from FITS header keywords.
///
/// Missing or unparseable keywords are silently ignored -- metadata fields
/// remain `None` rather than causing a load failure.
fn extract_metadata(
    hdu: &fitsio::hdu::FitsHdu,
    fptr: &mut FitsFile,
    naxis1: usize,
    naxis2: usize,
) -> FitsMetadata {
    let exposure_secs: Option<f64> = hdu.read_key(fptr, "EXPTIME").ok();
    let gain_e_per_adu: Option<f64> = hdu.read_key(fptr, "GAIN").ok();
    let instrument: Option<String> = hdu.read_key(fptr, "INSTRUME").ok();
    let obs_time: Option<String> = hdu.read_key(fptr, "DATE-OBS").ok();

    FitsMetadata {
        exposure_secs,
        gain_e_per_adu,
        instrument,
        obs_time,
        naxis1,
        naxis2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitsio::images::{ImageDescription, ImageType};
    use std::path::PathBuf;

    /// Helper: create a temporary FITS file with a float image and optional header keywords.
    fn create_test_fits(
        dir: &tempfile::TempDir,
        filename: &str,
        width: usize,
        height: usize,
        pixels: &[f32],
        keywords: &[(&str, HeaderValue)],
    ) -> PathBuf {
        let path = dir.path().join(filename);
        let description = ImageDescription {
            data_type: ImageType::Float,
            dimensions: &[height, width], // FITS convention: [NAXIS2, NAXIS1]
        };
        let mut fptr = FitsFile::create(&path)
            .with_custom_primary(&description)
            .overwrite()
            .open()
            .expect("failed to create test FITS file");

        let hdu = fptr.primary_hdu().expect("failed to get primary HDU");
        hdu.write_image(&mut fptr, pixels)
            .expect("failed to write image data");

        for (key, value) in keywords {
            match value {
                HeaderValue::F64(v) => hdu
                    .write_key(&mut fptr, key, *v)
                    .expect("failed to write f64 key"),
                HeaderValue::Str(v) => hdu
                    .write_key(&mut fptr, key, v.as_str())
                    .expect("failed to write string key"),
            }
        }

        drop(fptr);
        path
    }

    /// Typed header values for the test helper.
    enum HeaderValue {
        F64(f64),
        Str(String),
    }

    #[test]
    fn test_round_trip_float_image() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let width = 8;
        let height = 6;
        let pixels: Vec<f32> = (0..width * height).map(|i| i as f32).collect();

        let path = create_test_fits(&dir, "test.fits", width, height, &pixels, &[]);

        let (loaded, w, h, meta) = load_fits_frame(&path).expect("load_fits_frame failed");
        assert_eq!(w, width as u32);
        assert_eq!(h, height as u32);
        assert_eq!(meta.naxis1, width);
        assert_eq!(meta.naxis2, height);
        assert_eq!(loaded.len(), pixels.len());
        assert_eq!(loaded, pixels);
    }

    #[test]
    fn test_metadata_extraction() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let width = 4;
        let height = 4;
        let pixels: Vec<f32> = vec![1.0; width * height];

        let keywords = vec![
            ("EXPTIME", HeaderValue::F64(2.5)),
            ("GAIN", HeaderValue::F64(1.4)),
            ("INSTRUME", HeaderValue::Str("TestCam".to_string())),
            (
                "DATE-OBS",
                HeaderValue::Str("2025-01-15T12:00:00".to_string()),
            ),
        ];

        let path = create_test_fits(&dir, "meta.fits", width, height, &pixels, &keywords);

        let (_, _, _, meta) = load_fits_frame(&path).expect("load_fits_frame failed");
        assert_eq!(meta.exposure_secs, Some(2.5));
        assert_eq!(meta.gain_e_per_adu, Some(1.4));
        assert_eq!(meta.instrument.as_deref(), Some("TestCam"));
        assert_eq!(meta.obs_time.as_deref(), Some("2025-01-15T12:00:00"));
    }

    #[test]
    fn test_missing_metadata_is_none() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let width = 2;
        let height = 2;
        let pixels = vec![0.0_f32; 4];

        let path = create_test_fits(&dir, "no_meta.fits", width, height, &pixels, &[]);

        let (_, _, _, meta) = load_fits_frame(&path).expect("load_fits_frame failed");
        assert_eq!(meta.exposure_secs, None);
        assert_eq!(meta.gain_e_per_adu, None);
        assert_eq!(meta.instrument, None);
        assert_eq!(meta.obs_time, None);
    }

    #[test]
    fn test_load_hdu_index() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("multi_hdu.fits");

        // Create a FITS file with a primary image, then add an extension.
        let primary_desc = ImageDescription {
            data_type: ImageType::Float,
            dimensions: &[2, 3], // 2 rows x 3 cols
        };
        let mut fptr = FitsFile::create(&path)
            .with_custom_primary(&primary_desc)
            .overwrite()
            .open()
            .expect("failed to create FITS file");

        let primary = fptr.primary_hdu().expect("primary HDU");
        let primary_pixels: Vec<f32> = vec![1.0; 6];
        primary
            .write_image(&mut fptr, &primary_pixels)
            .expect("write primary");

        // Create an image extension (HDU 1).
        let ext_desc = ImageDescription {
            data_type: ImageType::Float,
            dimensions: &[4, 5], // 4 rows x 5 cols
        };
        let ext_hdu = fptr
            .create_image("SCI", &ext_desc)
            .expect("create extension");
        let ext_pixels: Vec<f32> = (0..20).map(|i| (i as f32) * 10.0).collect();
        ext_hdu
            .write_image(&mut fptr, &ext_pixels)
            .expect("write extension");

        drop(fptr);

        // Load the extension by index.
        let (loaded, w, h, meta) =
            load_fits_frame_hdu(&path, 1).expect("load_fits_frame_hdu failed");
        assert_eq!(w, 5);
        assert_eq!(h, 4);
        assert_eq!(meta.naxis1, 5);
        assert_eq!(meta.naxis2, 4);
        assert_eq!(loaded, ext_pixels);
    }

    #[test]
    fn test_nonexistent_file_error() {
        let result = load_fits_frame(Path::new("/nonexistent/path/fake.fits"));
        assert!(result.is_err());
        let err = result.expect_err("should error");
        assert!(
            err.contains("failed to open"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn test_integer_image_converted_to_f32() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("int16.fits");
        let width = 3;
        let height = 2;

        // Create a Short (i16) image.
        let desc = ImageDescription {
            data_type: ImageType::Short,
            dimensions: &[height, width],
        };
        let mut fptr = FitsFile::create(&path)
            .with_custom_primary(&desc)
            .overwrite()
            .open()
            .expect("create");

        let hdu = fptr.primary_hdu().expect("primary");
        let i16_pixels: Vec<i16> = vec![100, 200, 300, 400, 500, 600];
        hdu.write_image(&mut fptr, &i16_pixels).expect("write");
        drop(fptr);

        // Read back as f32 -- cfitsio converts automatically.
        let (loaded, w, h, _) = load_fits_frame(&path).expect("load");
        assert_eq!(w, width as u32);
        assert_eq!(h, height as u32);
        let expected: Vec<f32> = i16_pixels.iter().map(|&v| f32::from(v)).collect();
        assert_eq!(loaded, expected);
    }

    #[test]
    fn test_fits_metadata_default() {
        let meta = FitsMetadata::default();
        assert_eq!(meta.exposure_secs, None);
        assert_eq!(meta.gain_e_per_adu, None);
        assert_eq!(meta.instrument, None);
        assert_eq!(meta.obs_time, None);
        assert_eq!(meta.naxis1, 0);
        assert_eq!(meta.naxis2, 0);
    }
}
