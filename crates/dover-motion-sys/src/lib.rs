//! FFI bindings for Dover Motion's MotionSynergyAPI C++ library.
//!
//! This crate provides low-level Rust bindings to the Dover Motion MotionSynergyAPI,
//! which controls the SmartStage product range (SmartStage XY, SmartStage Linear, DOF-5)
//! and Dover Motion Control Module (DMCM) hardware.
//!
//! # Platform Support
//!
//! - **Windows (Primary)**: Full SDK support with MotionSynergyCore.dll
//! - **Linux (Secondary)**: Supported via libMotionSynergyCore.so
//!
//! # Feature Flags
//!
//! - `dover-sdk`: Enable real SDK bindings (requires Dover Motion SDK installed)
//! - Default (no features): Use dummy bindings for development/CI without hardware
//!
//! # Usage
//!
//! This is a low-level FFI crate. Most users should use the higher-level
//! `driver-dover-motion` crate which provides safe Rust wrappers.
//!
//! ## With Hardware
//!
//! ```toml
//! [dependencies]
//! dover-motion-sys = { version = "0.1", features = ["dover-sdk"] }
//! ```
//!
//! ## Without Hardware (Mock Mode)
//!
//! ```toml
//! [dependencies]
//! dover-motion-sys = "0.1"
//! ```
//!
//! # Critical Functions for LIBS Experiments
//!
//! Based on Dover Motion API User Manual Section 6.2.2 (IAxisDevice):
//!
//! - **Trigger on Position (TOP)**: `EnableTriggerOnPosition()`, `DisableTriggerOnPosition()`
//!   - Generates GPIO pulses at specified position increments
//!   - Essential for synchronized LIBS laser triggering
//!   - Supports bidirectional triggering
//!   - Configurable pulse width (50ns - 204,800ns, in 50ns increments)
//!
//! - **Motion Control**:
//!   - `MoveAbsolute()`: Move to absolute position
//!   - `MoveRelative()`: Move relative distance
//!   - `Stop()`: Stop motion
//!   - `Home()`: Home axis
//!
//! - **Position Queries**:
//!   - `GetActualPosition()`: Read encoder position
//!   - `GetCommandedPosition()`: Read commanded position
//!
//! - **Velocity/Acceleration**:
//!   - `SetVelocity()`, `GetVelocity()`
//!   - `SetAcceleration()`, `GetAcceleration()`
//!   - `SetDeceleration()`, `GetDeceleration()`
//!
//! # Safety
//!
//! All functions in this crate are `unsafe` as they directly call C++ FFI functions.
//! The caller must ensure:
//!
//! - Proper initialization via `Configure()`, `Connect()`, `Initialize()`
//! - Proper cleanup via `Shutdown()`
//! - Valid pointer lifetimes
//! - Thread safety (SDK may not be thread-safe)
//!
//! # Windows Wide String Handling
//!
//! Some Windows API functions may use `WCHAR` (UTF-16). Use the `widestring` crate
//! for safe conversion between Rust strings and Windows wide strings.
//!
//! # Documentation References
//!
//! - Dover Motion - Motion Synergy API User Manual (Document 102925)
//! - Section 6: C++ Software Integration (pp. 121-163)
//! - Section 6.2: Public Interface (pp. 127-163)

#![cfg_attr(not(feature = "dover-sdk"), allow(dead_code))]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(improper_ctypes)] // C++ types may not follow C ABI rules
#![allow(unused_imports)] // Generated bindings may have unused imports

// Include generated bindings
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_communication_settings_size() {
        // Ensure CommunicationSettings struct is defined
        let _settings = CommunicationSettings::default();
    }

    #[cfg(feature = "dover-sdk")]
    #[test]
    fn test_bindings_exist() {
        // This test only runs when dover-sdk feature is enabled
        // It verifies that the bindings were actually generated

        // We can't call functions without hardware, but we can check types exist
        let _api: Option<*mut MotionSynergyAPI> = None;
        let _device: Option<*mut IAxisDevice> = None;
    }

    #[cfg(not(feature = "dover-sdk"))]
    #[test]
    fn test_dummy_bindings() {
        // Verify dummy bindings compile
        let _settings = CommunicationSettings::default();
    }
}
