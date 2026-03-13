//! Signal-processing utilities for spectroscopy and LIBS data.
//!
//! This module provides pure-Rust processing algorithms that operate on
//! spectral data.  No hardware dependencies — suitable for use in tests,
//! benches, and cross-platform builds.

pub mod radiance_calibration;

pub use radiance_calibration::{
    linear_interp, load_calibration_file, load_calibration_file_with_metadata,
    CalibrationFileMetadata, RadianceCalibrator, Spectrum,
};
