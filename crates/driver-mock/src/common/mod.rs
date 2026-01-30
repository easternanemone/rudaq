//! Common infrastructure for mock devices.
//!
//! Provides reusable components for building mock hardware drivers:
//!
//! - **mode**: Operational modes (Instant, Realistic, Chaos)
//! - **timing**: Hardware-like timing configurations
//! - **errors**: Error injection framework
//! - **rng**: Seeded random number generator

pub mod errors;
pub mod mode;
pub mod rng;
pub mod timing;

// Re-export commonly used types
pub use errors::{ErrorConfig, ErrorScenario};
pub use mode::MockMode;
pub use rng::MockRng;
pub use timing::TimingConfig;
