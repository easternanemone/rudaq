// TODO: Fix doc comment generic types (e.g., `Arc<Mutex>`) to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub use daq_core::capabilities;
pub mod drivers;
pub mod plugin;
pub mod port_resolver;
pub mod registry;
pub mod resource_pool;

pub use capabilities::*;
pub use registry::{DeviceConfig, DeviceInfo, DeviceRegistry, DriverType};
