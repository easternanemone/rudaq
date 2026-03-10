//! # driver-registry
//!
//! Concrete driver factory registration for rust-daq.
//!
//! This crate owns the mapping from feature flags to concrete driver factories.
//! It breaks the "god-crate" anti-pattern where `hardware` depended on every
//! concrete driver. After this refactoring:
//!
//! - **`hardware`** = traits + `DeviceRegistry` + abstract APIs (no concrete drivers)
//! - **`driver-registry`** = concrete factory wiring + feature-flag gating
//!
//! ## Quick Example
//!
//! ```rust,ignore
//! use driver_registry::{register_all_factories, create_registry_from_config};
//! use hardware::registry::{DeviceRegistry, HardwareConfig};
//!
//! // Option A: Full registry from config file
//! let registry = create_registry_from_file(Path::new("config/maitai.toml")).await?;
//!
//! // Option B: Manual wiring
//! let registry = DeviceRegistry::new();
//! register_all_factories(&registry, Some(Path::new("config/devices"))).await?;
//! ```

use common::driver::DriverFactory;
use common::error::DaqError;
use hardware::registry::{register_mock_factories, DeviceRegistry, HardwareConfig};

/// Register all available hardware driver factories.
///
/// This registers factories for all enabled hardware drivers:
/// - Mock drivers (always available)
/// - Andor iStar / Shamrock (when `andor` feature enabled)
/// - PVCAM cameras (when `pvcam` feature enabled)
/// - Comedi DAQ (when `comedi` feature enabled)
/// - Config-driven devices from TOML manifests (always available)
///
/// Legacy serial drivers (Thorlabs, Newport, Spectra-Physics, Red Pitaya) have been
/// removed. Serial/TCP/SCPI devices now use driver-universal TOML manifests.
///
/// # Example
///
/// ```rust,ignore
/// use driver_registry::register_all_factories;
/// use hardware::registry::DeviceRegistry;
/// use std::path::Path;
///
/// let registry = DeviceRegistry::new();
/// register_all_factories(&registry, Some(Path::new("config/devices"))).await?;
/// ```
#[allow(clippy::unused_async)] // Async for API compat; concrete driver features add .await points
pub async fn register_all_factories(
    registry: &DeviceRegistry,
    config_dir: Option<&std::path::Path>,
) -> Result<(), DaqError> {
    // Register mock factories (always available)
    register_mock_factories(registry);

    // Register Andor SDK3 factories (iStar camera + Shamrock spectrograph)
    #[cfg(feature = "andor")]
    {
        use driver_andor_sdk3::{AndorCameraFactory, AndorSpectrographFactory};
        registry.register_factory(Box::new(AndorCameraFactory));
        registry.register_factory(Box::new(AndorSpectrographFactory));
    }

    // Register PVCAM factory
    #[cfg(feature = "pvcam")]
    {
        use driver_pvcam::PvcamFactory;
        registry.register_factory(Box::new(PvcamFactory));
    }

    // Register Comedi factories (NI DAQ, etc.)
    #[cfg(feature = "comedi")]
    {
        use driver_comedi::{
            ComediAnalogInputFactory, ComediAnalogOutputFactory, ComediCounterFactory,
            ComediDigitalIOFactory,
        };
        registry.register_factory(Box::new(ComediAnalogInputFactory));
        registry.register_factory(Box::new(ComediAnalogOutputFactory));
        registry.register_factory(Box::new(ComediDigitalIOFactory));
        registry.register_factory(Box::new(ComediCounterFactory));
    }

    // Load and register config-driven factories from TOML files (schema_version=3)
    if let Some(dir) = config_dir {
        if dir.exists() {
            match driver_universal::factory::load_all_factories(dir) {
                Ok(factories) => {
                    for factory in factories {
                        let driver_type = factory.driver_type().to_string();
                        registry.register_factory(Box::new(factory));
                        tracing::debug!(driver_type = %driver_type, "Registered universal config factory");
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load config factories from {}: {}",
                        dir.display(),
                        e
                    );
                }
            }
        }
    }

    Ok(())
}

/// Load hardware config from a file and create a fully-populated `DeviceRegistry`.
///
/// This is the primary entry point for production registry creation. It:
/// 1. Parses the TOML hardware config
/// 2. Registers all available driver factories (mock + concrete + universal)
/// 3. Loads manifest-driver plugins from configured search paths
/// 4. Registers all configured devices
///
/// For test/mock usage, prefer [`hardware::registry::create_mock_registry`].
pub async fn create_registry_from_config(
    config: &HardwareConfig,
    config_dir: Option<&std::path::Path>,
) -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();

    // Register all available driver factories BEFORE loading devices
    register_all_factories(&registry, config_dir).await?;

    // Load plugins and register devices from config
    hardware::registry::populate_registry_from_config(&registry, config).await?;

    Ok(registry)
}

/// Load hardware configuration from a file and create a DeviceRegistry.
///
/// Convenience wrapper around [`create_registry_from_config`] that handles
/// file parsing and device manifest directory resolution.
pub async fn create_registry_from_file(path: &std::path::Path) -> Result<DeviceRegistry, DaqError> {
    let config = HardwareConfig::from_file(path)?;
    // Default factory directory is alongside the hardware config (config/devices).
    let config_dir = path
        .parent()
        .map(|p| p.join("devices"))
        .filter(|p| p.exists());
    create_registry_from_config(&config, config_dir.as_deref()).await
}
