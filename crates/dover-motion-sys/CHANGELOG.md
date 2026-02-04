# Changelog

All notable changes to the `dover-motion-sys` crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial FFI bindings for Dover Motion MotionSynergyAPI
- Support for Windows (primary platform)
- Support for Linux (secondary platform)
- `dover-sdk` feature flag for conditional SDK binding generation
- Dummy bindings for development without SDK installed
- Build script with automatic OS detection and SDK path resolution
- Comprehensive documentation and README
- CI/CD configuration examples for GitHub Actions

### FFI Coverage

Based on Dover Motion API User Manual Section 6.2:

#### Core Classes (Section 6.2)
- `imp::IAxisDevice` (Section 6.2.2) - Main axis control interface
- `imp::MotionSynergyAPI` (Section 6.2.10) - Top-level API wrapper
- `imp::CommunicationSettings` (Section 6.2.1) - Serial/CAN configuration
- `imp::IMotionControllerConfiguration` (Section 6.2.7) - Controller configuration
- `imp::MotionErrorSettings` (Section 6.2.8) - Error handling configuration
- `imp::MotionTrackingSettings` (Section 6.2.9) - Motion tracking configuration

#### Critical Functions for LIBS Experiments
- Trigger on Position (TOP):
  - `EnableTriggerOnPosition()` - Generate GPIO pulses at position increments
  - `DisableTriggerOnPosition()` - Disable position-based triggering
- Motion Control:
  - `MoveAbsolute()`, `MoveRelative()` - Absolute and relative moves
  - `Stop()` - Stop motion
  - `Home()` - Home axis
- Position Queries:
  - `GetActualPosition()` - Read encoder position
  - `GetCommandedPosition()` - Read commanded position
- Velocity/Acceleration:
  - `SetVelocity()`, `GetVelocity()`
  - `SetAcceleration()`, `GetAcceleration()`
  - `SetDeceleration()`, `GetDeceleration()`

### Platform-Specific Features

#### Windows
- Automatic detection of SDK path: `C:\Program Files\Dover Motion\MotionSynergyAPI`
- Dynamic library linking: `MotionSynergyCore.dll`
- Support for environment variables:
  - `DOVER_SDK_DIR`
  - `DOVER_INCLUDE_DIR`
  - `DOVER_LIB_DIR`

#### Linux
- Automatic detection of SDK path: `/usr/local`
- Dynamic library linking: `libMotionSynergyCore.so`
- Support for environment variables:
  - `DOVER_SDK_DIR`
  - `DOVER_INCLUDE_DIR`
  - `DOVER_LIB_DIR`

### Dependencies
- `bindgen = "0.69"` - FFI binding generation from C++ headers

### Documentation
- Comprehensive README with platform-specific setup instructions
- CI/CD configuration guide
- Safety documentation for FFI usage
- Windows wide string handling guidance

## [0.1.0] - Initial Release (Planned)

Initial release of Dover Motion FFI bindings targeting Windows and Linux platforms.
