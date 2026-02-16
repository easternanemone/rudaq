// Allow expected warnings in this FFI/mock driver crate:
// - unsafe_code: FFI requires unsafe blocks
// - dead_code: Hardware-only structs have fields unused in mock mode
// - unused_variables: Mock implementations may not use all parameters
// - unused_imports: Some imports are only used in hardware mode
// - unused_must_use: Mock implementations may discard Results
#![allow(
    unsafe_code,
    dead_code,
    unused_variables,
    unused_imports,
    unused_must_use
)]
// Clippy allows for FFI/mock patterns:
#![allow(
    clippy::missing_safety_doc,
    clippy::new_without_default,
    clippy::unused_async,
    clippy::collapsible_if
)]

//! Andor iStar Camera and Shamrock Spectrograph Driver
//!
//! Safe Rust wrapper for Andor SDK3, providing drivers for:
//! - **Andor iStar**: Intensified CCD camera with MCP gain and DDG timing
//! - **Shamrock Spectrograph**: Grating-based spectrograph with wavelength tuning
//!
//! # Architecture
//!
//! This crate follows the componentized driver pattern:
//! - `camera/`: iStar camera driver (FrameProducer + Triggerable + Parameterized)
//! - `spectrograph/`: Shamrock spectrograph driver (WavelengthTunable + ShutterControl + Parameterized)
//! - `mock/`: Full mock implementations for cross-platform development
//! - `factory/`: DriverFactory implementations for plugin registration
//!
//! # Features
//!
//! - `camera`: Enable iStar camera driver
//! - `spectrograph`: Enable Shamrock spectrograph driver
//! - `hardware`: Enable real SDK3 hardware (Windows only)
//! - Default: Mock implementations only
//!
//! # Safety
//!
//! The SDK3 API uses wide strings (UTF-16) and manual memory management.
//! This crate provides safe abstractions over the unsafe FFI layer.
//!
//! # Example
//!
//! ```rust,no_run
//! use driver_andor_sdk3::camera::AndorCamera;
//! use common::capabilities::{FrameProducer, Triggerable, ExposureControl};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create camera (validates device identity)
//! let camera = AndorCamera::new_async(0).await?;
//!
//! // Configure acquisition
//! camera.set_trigger_mode("External").await?;
//! camera.set_exposure(0.001).await?;  // 1ms exposure
//! camera.set_mcp_gain(3600).await?;   // MCP gain
//!
//! // Start streaming
//! camera.start_stream().await?;
//! # Ok(())
//! # }
//! ```

pub mod buffer;
pub mod camera;
pub mod error;
pub mod factory;
pub mod mock;
pub mod spectrograph;
pub mod types;

// Re-export main types
pub use camera::AndorCamera;
pub use error::{AndorError, AndorResult};
pub use factory::{AndorCameraFactory, AndorSpectrographFactory};
pub use spectrograph::AndorSpectrograph;
pub use types::*;

/// Linker reference function to ensure this crate is not stripped.
///
/// Called by `drivers::link_drivers()` when the `andor_sdk3` feature is enabled.
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<AndorCamera>());
    std::hint::black_box(std::any::TypeId::of::<AndorSpectrograph>());
}
