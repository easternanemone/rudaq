// Allow expected warnings in this FFI/mock driver crate:
// - unsafe_code: FFI requires unsafe blocks and traits
// - dead_code: Hardware-only code is unused in mock mode
// - unused_variables: Mock implementations may not use all parameters
// - unused_imports: Some imports are only used in hardware mode
// - unused_unsafe: Some unsafe blocks are no-ops in mock mode
// - unused_must_use: Mock implementations may discard Results
#![allow(
    unsafe_code,
    dead_code,
    unused_variables,
    unused_imports,
    unused_unsafe,
    unused_must_use
)]
// Clippy allows for FFI/mock patterns:
#![allow(
    clippy::missing_safety_doc,
    clippy::new_without_default,
    clippy::collapsible_if,
    clippy::manual_dangling_ptr,
    clippy::manual_range_contains,
    clippy::manual_is_multiple_of,
    clippy::type_complexity
)]

//! Dover Motion SmartStage Driver
//!
//! This crate provides a safe Rust driver for Dover Motion's SmartStage product range
//! (SmartStage XY, SmartStage Linear, DOF-5) via the MotionSynergyAPI C++ library.
//!
//! # Features
//!
//! - `dover-hardware`: Enable real hardware support (requires Dover Motion SDK)
//! - Default (no features): Use mock driver for testing/development
//!
//! # Capabilities
//!
//! - **Movable**: Absolute/relative motion, position queries, homing
//! - **Parameterized**: Observable parameters (position, velocity, acceleration)
//! - **TriggerOnPosition (TOP)**: Generate GPIO pulses at position intervals
//!   - Critical for LIBS experiments (synchronized laser triggering)
//!   - Bidirectional triggering support
//!   - Configurable pulse width (50ns - 204,800ns, in 50ns increments)
//!
//! # Usage
//!
//! ```rust,ignore
//! use driver_dover_motion::DoverAxisFactory;
//! use common::driver::DriverFactory;
//!
//! // Register the factory
//! registry.register_factory(Box::new(DoverAxisFactory));
//!
//! // Create via config
//! let config = toml::toml! {
//!     device_path = "C:\\ProgramData\\Dover Motion\\SmartStage.xml"
//!     axis_name = "X"
//!     communication_type = "USB"
//! };
//! let components = factory.build(config.into()).await?;
//! ```
//!
//! # Architecture
//!
//! This driver uses a multi-layer architecture:
//! - `dover-motion-sys`: Low-level FFI bindings (unsafe)
//! - `DoverAxisDriver`: Safe wrapper with async interface
//! - `DoverMockDriver`: Mock implementation for testing
//!
//! All blocking FFI calls are wrapped in `tokio::task::spawn_blocking` to avoid
//! blocking the async runtime.

pub mod driver;
pub mod factory;
pub mod mock;
pub mod trigger_on_position;

// Re-export main types
pub use driver::DoverAxisDriver;
pub use factory::{DoverAxisConfig, DoverAxisFactory};
pub use mock::DoverMockDriver;
pub use trigger_on_position::TriggerOnPositionConfig;
