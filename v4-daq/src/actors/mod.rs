//! Kameo Actors for V4 Architecture
//!
//! Fault-tolerant actor implementations with supervision and message handling.

#[cfg(feature = "instrument_serial")]
pub mod newport_1830c;
#[cfg(feature = "instrument_serial")]
pub mod maitai;
#[cfg(feature = "instrument_serial")]
pub mod esp300;
pub mod pvcam;

pub mod scpi;
pub mod data_publisher;
pub mod hdf5_storage;
pub mod instrument_manager;

#[cfg(feature = "instrument_serial")]
pub use self::newport_1830c::Newport1830C;
#[cfg(feature = "instrument_serial")]
pub use self::maitai::MaiTai;
#[cfg(feature = "instrument_serial")]
pub use self::esp300::ESP300;
pub use self::pvcam::PVCAMActor;

pub use self::scpi::ScpiActor;
pub use self::data_publisher::{DataPublisher, DataConsumer, PublisherMetrics};
pub use self::hdf5_storage::HDF5Storage;
pub use self::instrument_manager::InstrumentManager;
