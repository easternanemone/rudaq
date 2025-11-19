//! Hardware Communication Adapters
//!
//! Low-level serial, VISA, and other hardware communication protocols.
//! Includes shared resource management for V2/V4 coexistence.

pub mod serial_adapter_v4;
pub mod visa_adapter_v4;
pub mod visa_session_manager;
pub mod pvcam_adapter;
pub mod shared_serial_port;

pub use serial_adapter_v4::{SerialAdapterV4, SerialAdapterV4Builder};
pub use visa_adapter_v4::{VisaAdapterV4, VisaAdapterV4Builder};
pub use visa_session_manager::{VisaSessionHandle, VisaSessionManager};
pub use pvcam_adapter::{
    AcquisitionGuard, CameraHandle, MockPvcamAdapter, PvcamAdapter, PvcamFrame, PxRegion,
};
pub use shared_serial_port::{
    SerialGuard, SerialParity, SerialPortConfig, SharedSerialPort,
};
