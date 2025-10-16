//! V2 instrument implementations using new trait hierarchy
//!
//! This module contains instrument implementations that use the new
//! three-tier architecture from daq-core:
//! - HardwareAdapter layer for I/O
//! - Instrument trait with state management
//! - Meta-instrument traits (Camera, PowerMeter, etc.)
//!
//! Phase 2: MockInstrumentV2 validates the trait design.
//! Phase 3: Real instrument migrations (Newport1830C, MaiTai, ESP300, etc.)

pub mod esp300;
pub mod maitai;
pub mod mock_instrument;
pub mod newport_1830c;
pub mod scpi;

pub use esp300::ESP300V2;
pub use maitai::MaiTaiV2;
pub use mock_instrument::MockInstrumentV2;
pub use newport_1830c::Newport1830CV2;
pub use scpi::ScpiInstrumentV2;
