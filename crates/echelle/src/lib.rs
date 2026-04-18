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
//! - **Calibration pipeline** - arc frame to wavelength-calibrated profile
//!   - Stage 1: Per-order echelle-equation seed + two-phase atlas matching
//!   - Stage 2: Physical Cauchy-series `y(m) = a + b/m² + c/m⁴` re-assignment
//!     of physical order number for traces that failed stage 1
//!   - Stage 3: Single global 2D Chebyshev fit `λ(x, m)` across all matched
//!     arc lines, iterative 3σ rejection (IRAF ECIDENTIFY / CERES / PypeIt
//!     standard); uncalibrated orders recovered by evaluating the global
//!     surface on their pixel domain at their Cauchy-predicted `m`
//! - **Simulation** - Synthetic echelleogram generation for pipeline development

pub mod blaze;
pub mod calibration_pipeline;
pub mod calibration_quality;
pub mod cauchy_dispersion;
pub mod chebyshev_2d;
pub mod optimal_extraction;
pub mod radiance_calibration;
pub mod rectification;
pub mod scattered_light;
pub mod simulation;
pub mod trace_fitting;
pub mod trace_validation;
pub mod types;
pub mod wavelength_fitting;

// Re-export commonly used types at crate root
pub use types::*;
