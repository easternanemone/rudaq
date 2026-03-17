//! Echelle spectroscopy calibration, extraction, and simulation.
//!
//! This crate provides the complete echelle spectrograph data reduction pipeline:
//!
//! - **Calibration types** - Profile schema, trace models, wavelength solutions
//! - **Trace detection** - PypeIt-inspired flat-frame order detection
//! - **Rectification** - Order sub-image extraction with aperture masks
//! - **Optimal extraction** - Horne 1986 inverse-variance weighted extraction
//! - **Scattered light** - 2D Chebyshev inter-order background subtraction
//! - **Wavelength fitting** - Arc line detection and Chebyshev wavelength solutions
//! - **Calibration pipeline** - End-to-end arc frame to wavelength-calibrated profile
//! - **Simulation** - Synthetic echelleogram generation for pipeline development

pub mod calibration_pipeline;
pub mod optimal_extraction;
pub mod rectification;
pub mod scattered_light;
pub mod simulation;
pub mod trace_fitting;
pub mod types;
pub mod wavelength_fitting;

// Re-export commonly used types at crate root
pub use types::*;
