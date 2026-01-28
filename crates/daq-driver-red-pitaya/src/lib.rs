//! Red Pitaya FPGA-based PID Controller Driver
//!
//! This crate provides a driver for Red Pitaya STEMlab boards running custom
//! PID FPGA bitstreams for laser power stabilization feedback loops.
//!
//! # Communication
//!
//! The driver communicates via SCPI over TCP (default port 5000).
//!
//! # Capabilities
//!
//! - `Readable` - Read current power level
//! - `Parameterized` - Expose PID parameters (Kp, Ki, Kd, setpoint, output limits)
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_driver_red_pitaya::RedPitayaPidFactory;
//! use common::driver::DriverFactory;
//!
//! // Register the factory
//! registry.register_factory(Box::new(RedPitayaPidFactory));
//!
//! // Create via config
//! let config = toml::toml! {
//!     host = "192.168.1.100"
//!     port = 5000
//! };
//! let components = factory.build(config.into()).await?;
//! ```
//!
//! # Mock Mode
//!
//! For testing without hardware, set `mock = true` in the configuration:
//!
//! ```rust,ignore
//! let config = toml::toml! {
//!     host = "192.168.1.100"
//!     port = 5000
//!     mock = true
//! };
//! ```

mod driver;
mod scpi;

pub use driver::{RedPitayaPidConfig, RedPitayaPidDriver, RedPitayaPidFactory};
pub use scpi::ScpiClient;

/// Force linker to include this crate's factories.
/// Call this from daq-hardware to ensure the factory is available.
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<RedPitayaPidFactory>());
}
