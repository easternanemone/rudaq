//! V4 DAQ Architecture - Clean Implementation
//!
//! This is the clean V4-only workspace member containing:
//! - Kameo fault-tolerant actors for instrument control
//! - Apache Arrow data structures for efficient data handling
//! - HDF5 storage adapter for scientific data persistence
//! - VISA/Serial hardware communication protocols
//!
//! # Features
//!
//! - `instrument_serial` - Enable serial port communication (serialport)
//! - `instrument_visa` - Enable VISA instrument communication
//! - `storage_hdf5` - Enable HDF5 storage backend
//! - `gui` - Enable GUI components with egui
//! - `full` - Enable all features
//!
//! # Example
//!
//! ```no_run
//! use v4_daq::actors::Newport1830C;
//!
//! #[tokio::main]
//! async fn main() {
//!     let actor = Newport1830C::new();
//!     // Use with Kameo for fault-tolerant actor supervision
//! }
//! ```

pub mod actors;
pub mod adapters;
pub mod traits;
pub mod hardware;
pub mod config;
pub mod runtime;
pub mod testing;

#[cfg(feature = "gui")]
pub mod gui;

// Re-exports for convenience
pub use actors::{DataPublisher, HDF5Storage, InstrumentManager};
#[cfg(feature = "instrument_serial")]
pub use actors::Newport1830C;
pub use traits::power_meter::{PowerMeter, PowerMeasurement, PowerUnit, Wavelength};
pub use hardware::SerialAdapterV4;
pub use config::{V4Config, InstrumentDefinition, StorageConfig, ConfigError};
