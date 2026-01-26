//! Mock Hardware Drivers for rust-daq
//!
//! This crate provides simulated hardware devices for testing without physical hardware.
//! All mock devices use async-safe operations (tokio::time::sleep, not std::thread::sleep).
//!
//! # Available Mock Drivers
//!
//! - [`MockStage`] - Simulated motion stage with realistic timing
//! - [`MockCamera`] - Simulated camera with trigger and streaming support
//! - [`MockPowerMeter`] - Simulated power meter with configurable readings
//!
//! # Performance Characteristics
//!
//! - MockStage: 10mm/sec motion speed, 50ms settling time
//! - MockCamera: 33ms frame readout (~30fps simulation)
//! - MockPowerMeter: ~1% noise on readings
//!
//! # Driver Factory Pattern
//!
//! Each mock driver provides a `DriverFactory` implementation for use with
//! the daq-hardware registry:
//!
//! ```rust,ignore
//! use daq_driver_mock::{MockStageFactory, MockCameraFactory, MockPowerMeterFactory};
//! use daq_hardware::DeviceRegistry;
//!
//! let registry = DeviceRegistry::new();
//! registry.register_factory(Box::new(MockStageFactory));
//! registry.register_factory(Box::new(MockCameraFactory));
//! registry.register_factory(Box::new(MockPowerMeterFactory));
//! ```

pub mod common;
mod mock_camera;
mod mock_power_meter;
mod mock_stage;
mod pattern;

// Re-export common types
pub use common::{ErrorConfig, ErrorScenario, MockMode, MockRng, TimingConfig};

// Re-export driver types
pub use mock_camera::{MockCamera, MockCameraFactory};
pub use mock_power_meter::{MockPowerMeter, MockPowerMeterFactory};
pub use mock_stage::{MockStage, MockStageFactory};

// Re-export for convenience
pub use pattern::generate_test_pattern;

/// Force the linker to include this crate's driver factory registrations.
///
/// This function is called by `daq_drivers::link_drivers()` to ensure driver
/// factories are not stripped by the linker. Without this explicit reference,
/// the linker may optimize away driver crates that register factories.
///
/// # Usage
///
/// This function is automatically called by `daq_drivers::link_drivers()` when
/// the `mock` feature is enabled. You typically don't need to call it directly.
#[inline(never)]
pub fn link() {
    // Reference types from the crate to create dependencies that the linker
    // cannot optimize away.
    std::hint::black_box(std::any::TypeId::of::<MockStage>());
    std::hint::black_box(std::any::TypeId::of::<MockCamera>());
    std::hint::black_box(std::any::TypeId::of::<MockPowerMeter>());
}

/// Register all mock driver factories with a device registry.
///
/// Convenience function to register all mock factories at once.
///
/// # Example
///
/// ```rust,ignore
/// use daq_driver_mock::register_all;
/// use daq_hardware::DeviceRegistry;
///
/// let registry = DeviceRegistry::new();
/// register_all(&registry);
/// ```
pub fn register_all(registry: &impl FactoryRegistry) {
    registry.register_factory(Box::new(MockStageFactory));
    registry.register_factory(Box::new(MockCameraFactory));
    registry.register_factory(Box::new(MockPowerMeterFactory));
}

/// Trait for registries that can accept driver factories.
///
/// This allows the mock driver crate to work with any registry implementation
/// without depending on daq-hardware directly.
pub trait FactoryRegistry {
    /// Register a driver factory.
    fn register_factory(&self, factory: Box<dyn daq_core::driver::DriverFactory>);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_does_not_panic() {
        link();
    }
}
