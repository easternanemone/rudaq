//! Mock Hardware Implementations
//!
//! Provides simulated hardware devices for testing without physical hardware.
//! All mock devices use async-safe operations (tokio::time::sleep, not std::thread::sleep).
//!
//! # Available Mocks
//!
//! - `MockStage` - Simulated motion stage with realistic timing
//! - `MockCamera` - Simulated camera with trigger and streaming support
//! - `MockPowerMeter` - Simulated power meter with configurable readings
//!
//! # Performance Characteristics
//!
//! - MockStage: 10mm/sec motion speed, 50ms settling time
//! - MockCamera: 33ms frame readout (30fps simulation)
//!
//! # Note
//!
//! This module re-exports from `driver-mock` crate (bd-ha9c Driver Decoupling).
//! The standalone crate provides DriverFactory-based implementations.

// Re-export all public items from driver-mock
pub use driver_mock::{
    MockCamera, MockCameraFactory, MockPowerMeter, MockPowerMeterFactory, MockStage,
    MockStageFactory, generate_test_pattern,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_exports_available() {
        // Verify re-exports work
        let _stage = MockStage::new();
        let _camera = MockCamera::new(640, 480);
        let _power_meter = MockPowerMeter::new(1.0); // base power of 1.0 W
    }
}
