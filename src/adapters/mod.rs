//! Hardware adapter implementations
//!
//! This module contains implementations of the HardwareAdapter trait,
//! providing low-level I/O abstraction for different communication protocols.

pub mod mock_adapter;
pub mod serial_adapter;
pub mod visa_adapter;

pub use mock_adapter::MockAdapter;
pub use serial_adapter::SerialAdapter;
pub use visa_adapter::VisaAdapter;
