//! V2 instrument implementations using new trait hierarchy
//!
//! This module contains instrument implementations that use the new
//! three-tier architecture from daq-core:
//! - HardwareAdapter layer for I/O
//! - Instrument trait with state management
//! - Meta-instrument traits (Camera, PowerMeter, etc.)

pub mod elliptec;
pub mod esp300;
pub mod maitai;
pub mod mock_instrument;
pub mod newport_1830c;
pub mod newport_1830c_v3;  // Phase 2: V3 architecture implementation
pub mod pvcam;
pub mod pvcam_sdk;  // Added: SDK abstraction layer
pub mod pvcam_v3;   // Phase 2: V3 architecture implementation
pub mod scpi;

pub use elliptec::ElliptecV2;
pub use esp300::ESP300V2;
pub use maitai::MaiTaiV2;
pub use mock_instrument::MockInstrumentV2;
pub use newport_1830c::Newport1830CV2;
pub use newport_1830c_v3::Newport1830CV3;
pub use pvcam::PVCAMInstrumentV2;
pub use pvcam_v3::PVCAMCameraV3;
pub use scpi::ScpiInstrumentV2;