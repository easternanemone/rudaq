// TODO: Fix doc comment generic types (e.g., `Arc<Mutex>`) to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub use daq_core::capabilities;
pub mod config;
pub mod drivers;
pub mod factory;
pub mod plugin;
pub mod port_resolver;
pub mod registry;
pub mod resource_pool;

pub use capabilities::*;
pub use registry::{
    register_all_factories, register_mock_factories, DeviceConfig, DeviceInfo, DeviceRegistry,
    DriverType,
};

// Re-export declarative config types under a distinct name to avoid confusion
// with registry::DeviceConfig (which is for device registration)
pub use config::DeviceConfig as DeclarativeDeviceConfig;

// Re-export factory types for config-driven driver creation
pub use factory::{
    load_all_factories, ConfiguredBus, ConfiguredDriver, DriverFactory, GenericSerialDriverFactory,
    GenericSerialInstanceConfig,
};
