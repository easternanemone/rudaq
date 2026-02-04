# Changelog

All notable changes to the driver-dover-motion crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial implementation of Dover Motion SmartStage driver
- Safe Rust wrapper around dover-motion-sys FFI bindings
- `DoverAxisDriver` with async interface for hardware control
- `DoverMockDriver` for testing without hardware
- `Movable` trait implementation (absolute/relative motion, position queries, homing)
- `Parameterized` trait implementation (position, velocity, acceleration parameters)
- `TriggerOnPosition` trait implementation (position-based GPIO triggering for LIBS)
- `DoverAxisFactory` for plugin architecture integration
- `TriggerOnPositionConfig` configuration type with validation
- Feature flags: `dover-hardware` for real hardware, default for mock mode
- Comprehensive unit tests for mock driver and factory
- Documentation and examples for LIBS experiments

### Dependencies

- dover-motion-sys: FFI bindings to MotionSynergyAPI
- common: rust-daq core capabilities and traits
- tokio: Async runtime with spawn_blocking for FFI calls
- anyhow: Error handling
- serde: Configuration deserialization

## [0.1.0] - TBD

- Initial release
