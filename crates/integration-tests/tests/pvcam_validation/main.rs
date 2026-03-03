#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
#![cfg(all(feature = "pvcam", feature = "pvcam_sdk"))]
//! PVCAM Camera Hardware Validation Test Suite
//!
//! Comprehensive validation tests for Photometrics PVCAM camera driver.
//! Tests are written for Prime BSI (2048x2048) by default.
//!
//! These tests verify:
//! - Camera initialization and enumeration
//! - Frame acquisition (single and continuous)
//! - Region of Interest (ROI) configuration
//! - Binning control (1x1, 2x2, 4x4, 8x8)
//! - Exposure time control and accuracy
//! - Triggered acquisition modes
//! - Error handling and recovery
//! - Frame data integrity
//!
//! # Camera Models
//!
//! - **Prime BSI** (default): 2048 x 2048 pixel sensor
//! - **Prime 95B** (optional): 1200 x 1200 pixel sensor
//!   Enable with `--features prime_95b_tests`
//!
//! # Hardware Setup Requirements
//!
//! For actual hardware validation (not mock-based tests):
//!
//! 1. **Camera**:
//!    - Photometrics Prime BSI (default) or Prime 95B
//!    - Connected via USB 3.0 or PCIe interface
//!    - PVCAM SDK installed and configured
//!    - Camera powered on and recognized by system
//!
//! 2. **Environment Variables** (required):
//!    - `PVCAM_SDK_DIR=/opt/pvcam/sdk`
//!    - `PVCAM_LIB_DIR=/opt/pvcam/library/x86_64`
//!    - `PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode`
//!    - `LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH`
//!
//! 3. **Illumination** (for uniformity tests):
//!    - Uniform white light source (LED panel or integrating sphere)
//!    - Diffuse illumination across sensor area
//!    - Avoid bright spots or shadows
//!
//! 4. **Dark Environment** (for noise tests):
//!    - Camera lens cap or opaque cover
//!    - Room lights off
//!    - Allows measurement of dark current and read noise
//!
//! # Running Tests
//!
//! ```bash
//! # Run all mock tests (no hardware required)
//! cargo test --test pvcam_validation --features instrument_photometrics
//!
//! # Run with hardware tests (requires Prime BSI camera)
//! cargo test --test pvcam_validation \
//!   --features "instrument_photometrics,pvcam_hardware,hardware_tests" \
//!   -- --test-threads=1
//!
//! # Run with Prime 95B camera instead
//! cargo test --test pvcam_validation \
//!   --features "instrument_photometrics,pvcam_hardware,hardware_tests,prime_95b_tests" \
//!   -- --test-threads=1
//! ```

mod config_validation_tests;
mod hardware_camera_tests;
mod hardware_feature_tests;
mod helpers;
mod mock_camera_tests;
