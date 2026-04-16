//! Free functions for populating a `DeviceRegistry`.
//!
//! These were extracted from `registry/mod.rs` to keep that file focused on
//! the runtime registry implementation. None of them touch private registry
//! fields — they all drive the registry through its public API.

use super::DeviceRegistry;
use super::types::{HardwareConfig, RegistrationFailure};
use common::error::DaqError;

// =============================================================================
// Hardware Configuration File Support
// =============================================================================

/// Populate a registry with plugins and devices from a hardware config.
///
/// This loads manifest-driver plugins from configured search paths and
/// registers all configured devices. **Factories must be registered first**
/// (via `register_mock_factories` or `driver_registry::register_all_factories`).
///
/// This is the abstract half of registry creation — it does not reference
/// any concrete driver crates, only the factory system already registered
/// on the `DeviceRegistry`.
///
/// # Example TOML format:
/// ```toml
/// # Optional: Plugin search paths (first = highest priority)
/// plugin_paths = [
///     "~/.config/rust-daq/plugins",
///     "/usr/share/rust-daq/plugins"
/// ]
///
/// [[devices]]
/// id = "rotator_2"
/// name = "ELL14 Rotation Mount (Addr 2)"
/// [devices.driver]
/// type = "ell14"
/// port = "/dev/ttyUSB0"
/// address = "2"
///
/// [[devices]]
/// id = "my_sensor"
/// name = "Custom Sensor (Plugin-Based)"
/// [devices.driver]
/// type = "plugin"
/// plugin_id = "my-sensor-v1"
/// address = "/dev/ttyUSB2"
/// ```
pub async fn populate_registry_from_config(
    registry: &DeviceRegistry,
    config: &HardwareConfig,
) -> Result<(), DaqError> {
    // Load plugins from configured search paths
    #[cfg(feature = "serial")]
    {
        let plugin_factory = registry.plugin_factory();
        let mut factory = plugin_factory.write().await;
        for path in &config.plugin_paths {
            // Expand ~ to home directory
            let expanded = if path.starts_with("~") {
                if let Some(home) = dirs::home_dir() {
                    home.join(path.strip_prefix("~").unwrap_or(path))
                } else {
                    path.clone()
                }
            } else {
                path.clone()
            };
            factory.add_search_path(expanded);
        }

        // Scan all paths and report errors
        let errors = factory.scan().await;
        for err in &errors {
            tracing::warn!("Plugin load warning: {}", err);
        }

        // Log loaded plugins
        let plugins = factory.available_plugins();
        if !plugins.is_empty() {
            tracing::info!("Loaded {} plugin(s): {:?}", plugins.len(), plugins);
        }
    }

    // Register all configured devices via factory system
    let mut success_count = 0;
    let mut failure_count = 0;

    for device_config in &config.devices {
        if !device_config.enabled {
            tracing::info!(
                device_id = %device_config.id,
                "Skipping disabled device"
            );
            continue;
        }

        let driver_type = &device_config.driver.driver_type;

        tracing::info!(
            device_id = %device_config.id,
            device_name = %device_config.name,
            driver_type = %driver_type,
            "Registering device"
        );

        let result = registry
            .register_from_toml(
                &device_config.id,
                &device_config.name,
                driver_type,
                device_config.driver.config.clone(),
            )
            .await;

        if let Err(e) = result {
            failure_count += 1;
            registry.record_registration_failure(RegistrationFailure {
                device_id: device_config.id.to_string(),
                device_name: device_config.name.clone(),
                driver_type: driver_type.clone(),
                error: e.to_string(),
            });
        } else {
            success_count += 1;
            tracing::info!(
                device_id = %device_config.id,
                "Device registered successfully"
            );
        }
    }

    // Summary logging
    if failure_count > 0 {
        tracing::warn!(
            success_count,
            failure_count,
            "Device registration completed with failures"
        );
    } else {
        tracing::info!(success_count, "All devices registered successfully");
    }

    Ok(())
}

// =============================================================================
// Convenience Functions for Lab Configuration
// =============================================================================

/// Create a DeviceRegistry with mock devices for testing.
///
/// **Deprecated**: Prefer `driver_registry::create_canonical_mock_registry()` which
/// uses universal-driver emulators for universal-eligible instruments. This function
/// uses handwritten `driver-mock` implementations that don't exercise the manifest
/// emulator code path.
///
/// Requires the `test-util` feature (M-TEST-UTIL, bd-qd4zn).
#[cfg(any(test, feature = "test-util"))]
#[deprecated(
    note = "use driver_registry::create_canonical_mock_registry() for universal mock parity"
)]
pub async fn create_mock_registry() -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);

    registry
        .register_from_toml(
            "mock_stage",
            "Mock Stage",
            "mock_stage",
            toml::toml! { initial_position = 0.0 }.into(),
        )
        .await?;

    registry
        .register_from_toml(
            "mock_power_meter",
            "Mock Power Meter",
            "mock_power_meter",
            toml::toml! { base_power = 1e-6 }.into(),
        )
        .await?;

    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    Ok(registry)
}

/// Register all mock driver factories with a registry.
///
/// This enables using `register_from_toml()` for mock devices:
///
/// ```rust,ignore
/// use daq_hardware::registry::{DeviceRegistry, register_mock_factories};
///
/// let registry = DeviceRegistry::new();
/// register_mock_factories(&registry);
///
/// // Now register mock devices via TOML config
/// registry.register_from_toml(
///     "my_stage",
///     "My Test Stage",
///     "mock_stage",
///     toml::Value::Table(Default::default()),
/// ).await?;
/// ```
///
/// Requires the `test-util` feature (M-TEST-UTIL, bd-qd4zn).
#[cfg(any(test, feature = "test-util"))]
pub fn register_mock_factories(registry: &DeviceRegistry) {
    use driver_mock::{MockCameraFactory, MockPowerMeterFactory, MockStageFactory};

    registry.register_factory(Box::new(MockStageFactory));
    registry.register_factory(Box::new(MockCameraFactory));
    registry.register_factory(Box::new(MockPowerMeterFactory));
}

// NOTE: `register_all_factories` has been moved to the `driver-registry` crate.
// Use `driver_registry::register_all_factories()` for concrete driver registration.
// `register_mock_factories` remains here since it only needs driver-mock (always compiled).

// =============================================================================
// Universal manifest resolution
// =============================================================================

/// Derive the factory name for a `type = "universal"` device by reading its
/// manifest file.
///
/// The manifest TOML must contain `[device] name = "..."`. The factory name
/// is `universal_{name_lowercased_underscored}`, matching the convention in
/// `UniversalDriverFactory::new`.
pub(super) fn resolve_universal_factory_name(config: &toml::Value) -> Result<String, DaqError> {
    let manifest_path = config
        .get("manifest")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            DaqError::Configuration(
                "driver type 'universal' requires a 'manifest' field pointing \
                 to the device TOML manifest (e.g., manifest = \"config/devices/my_device.toml\")"
                    .to_string(),
            )
        })?;

    let content = std::fs::read_to_string(manifest_path).map_err(|e| {
        DaqError::Configuration(format!(
            "Failed to read universal manifest '{manifest_path}': {e}"
        ))
    })?;

    let table: toml::Value = toml::from_str(&content).map_err(|e| {
        DaqError::Configuration(format!(
            "Failed to parse universal manifest '{manifest_path}': {e}"
        ))
    })?;

    let device_name = table
        .get("device")
        .and_then(|d| d.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            DaqError::Configuration(format!(
                "Universal manifest '{manifest_path}' is missing [device].name"
            ))
        })?;

    let derived = format!("universal_{}", device_name.to_lowercase().replace(' ', "_"));

    tracing::info!(
        manifest = %manifest_path,
        derived_factory = %derived,
        "Resolved type='universal' to derived factory name"
    );

    Ok(derived)
}
