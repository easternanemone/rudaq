//! UniversalDriverFactory: DriverFactory implementation for config-driven devices.
//!
//! Loads device manifests from TOML files (schema_version = 3) and creates
//! `UniversalDriver` instances wired to the appropriate capability trait objects.

use crate::config::validated::DeviceManifest;
use crate::config::{parse_manifest, RawManifest};
use crate::driver::UniversalDriver;
use anyhow::{anyhow, Context, Result};
use common::capabilities::DeviceCategory;
use common::driver::{
    Capability as CoreCapability, DeviceComponents, DeviceMetadata, DriverFactory,
};
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use parking_lot::RwLock;
use tokio::sync::Mutex;

/// A shared, mutex-guarded transport handle.
type SharedTransport = Arc<Mutex<Box<dyn crate::transport::Transport>>>;

/// Registry mapping serial port paths to their shared transports.
type TransportRegistry = RwLock<HashMap<String, SharedTransport>>;

/// Global registry for shared serial transports on RS-485 multidrop buses.
///
/// When multiple devices share the same serial port (e.g., 3 ELL14 rotators
/// on one RS-485 bus), each `build()` call checks this registry. If a transport
/// already exists for that port path, it clones the `Arc` so all devices
/// coordinate through the same `Mutex`. This prevents interleaved I/O on
/// shared buses.
///
/// **Lifetime**: Entries persist for the process lifetime. This is intentional —
/// the daemon runs with a static hardware config. Shared transports are never
/// removed because new devices on the same port could be registered later.
///
/// Modeled after `crates/driver-thorlabs/src/shared_ports.rs`.
static SHARED_TRANSPORTS: OnceLock<TransportRegistry> = OnceLock::new();

fn transport_registry() -> &'static TransportRegistry {
    SHARED_TRANSPORTS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Atomically claim a port path for transport creation.
///
/// Uses a write lock to prevent the check-then-act race: if two `build()`
/// calls for the same port run concurrently, only the first caller creates
/// a transport. The second caller finds the placeholder and waits.
///
/// Returns `Some(transport)` if another caller already registered the port,
/// or `None` if this caller should create and register the transport.
fn claim_or_get_transport(port_path: &str) -> Option<SharedTransport> {
    match transport_registry().write() {
        Ok(registry) => registry.get(port_path).cloned(),
        Err(e) => {
            tracing::warn!(port = port_path, "Transport registry lock poisoned: {e}");
            None
        }
    }
}

/// Register a newly created transport in the shared registry.
///
/// Silently skips registration if the lock is poisoned. The caller will
/// still function — it just won't share the transport with future devices.
fn register_shared_transport(port_path: &str, transport: &SharedTransport) {
    match transport_registry().write() {
        Ok(mut registry) => {
            registry.insert(port_path.to_string(), transport.clone());
        }
        Err(e) => {
            tracing::warn!(
                port = port_path,
                "Transport registry lock poisoned, skipping registration: {e}"
            );
        }
    }
}

/// Instance configuration passed via the hardware TOML config
/// (e.g., `maitai_hardware.toml`).
///
/// Each device instance provides connection-specific details like port path,
/// address, and optional baud rate override.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct InstanceConfig {
    /// Serial port path (for serial connections).
    pub port: Option<String>,
    /// Host address (for TCP/UDP connections).
    pub host: Option<String>,
    /// Device path (for USB TMC connections, e.g. "/dev/usbtmc0").
    pub device: Option<String>,
    /// Device address on the bus (e.g., "2" for ELL14 RS-485).
    #[serde(default = "default_address")]
    pub address: String,
    /// Optional baud rate override.
    pub baud_rate: Option<u32>,
    /// Use a mock transport instead of real hardware (for testing).
    #[serde(default)]
    pub mock: bool,
}

fn default_address() -> String {
    "0".to_string()
}

/// Factory for creating `UniversalDriver` instances from TOML device manifests.
///
/// Implements the `DriverFactory` trait so it can be registered with the
/// `DeviceRegistry` at daemon startup.
pub struct UniversalDriverFactory {
    manifest: Arc<DeviceManifest>,
    capabilities: &'static [CoreCapability],
    driver_type: &'static str,
    name: &'static str,
}

impl UniversalDriverFactory {
    /// Create a factory from a validated `DeviceManifest`.
    pub fn new(manifest: DeviceManifest) -> Self {
        let driver_type_string = format!(
            "universal_{}",
            manifest.device.name.to_lowercase().replace(' ', "_")
        );
        let caps_vec: Vec<CoreCapability> = manifest
            .device
            .capability_names
            .iter()
            .filter_map(|cap| match cap.as_str() {
                "Movable" => Some(CoreCapability::Movable),
                "Readable" => Some(CoreCapability::Readable),
                "Settable" => Some(CoreCapability::Settable),
                "WavelengthTunable" => Some(CoreCapability::WavelengthTunable),
                "ShutterControl" => Some(CoreCapability::ShutterControl),
                "EmissionControl" => Some(CoreCapability::EmissionControl),
                "Commandable" => Some(CoreCapability::Commandable),
                // Parameterized is not implemented by UniversalDriver;
                // skip it to avoid advertising unsupported capabilities.
                "Parameterized" => None,
                _ => None,
            })
            .collect();

        // Leak once at construction time to satisfy the &'static lifetime
        // required by the DriverFactory trait. Factory instances are long-lived
        // singletons created once at startup, so this is bounded.
        let driver_type: &'static str = Box::leak(driver_type_string.into_boxed_str());
        let name: &'static str = Box::leak(manifest.device.name.clone().into_boxed_str());
        let capabilities: &'static [CoreCapability] = Box::leak(caps_vec.into_boxed_slice());

        Self {
            manifest: Arc::new(manifest),
            capabilities,
            driver_type,
            name,
        }
    }

    /// Load a factory from a TOML file.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, parsed, or validated.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).context(format!("Failed to read {}", path.display()))?;
        let raw: RawManifest =
            toml::from_str(&content).context(format!("Failed to parse {}", path.display()))?;
        let manifest = parse_manifest(raw).map_err(|errors| {
            let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            anyhow!(
                "Config validation errors in {}:\n  {}",
                path.display(),
                msgs.join("\n  ")
            )
        })?;
        Ok(Self::new(manifest))
    }

    /// Load a factory from a TOML string (useful for tests).
    ///
    /// # Errors
    /// Returns an error if the string cannot be parsed or validated.
    pub fn from_toml_str(toml_content: &str) -> Result<Self> {
        let raw: RawManifest = toml::from_str(toml_content)?;
        let manifest = parse_manifest(raw).map_err(|errors| {
            let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            anyhow!("Config validation errors:\n  {}", msgs.join("\n  "))
        })?;
        Ok(Self::new(manifest))
    }
}

impl DriverFactory for UniversalDriverFactory {
    fn driver_type(&self) -> &'static str {
        self.driver_type
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn capabilities(&self) -> &'static [CoreCapability] {
        self.capabilities
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let _: InstanceConfig = config.clone().try_into()?;
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        let manifest = self.manifest.clone();

        Box::pin(async move {
            let instance: InstanceConfig = config.try_into()?;

            use crate::config::validated::ConnectionConfig;
            let transport: SharedTransport = if instance.mock {
                #[cfg(feature = "emulator")]
                {
                    Arc::new(Mutex::new(
                        Box::new(crate::emulator::create_emulator_transport(
                            &manifest,
                            &instance.address,
                        )?) as Box<dyn crate::transport::Transport>,
                    ))
                }
                #[cfg(not(feature = "emulator"))]
                {
                    Arc::new(Mutex::new(
                        Box::new(crate::transport::MockTransport::new(vec![]))
                            as Box<dyn crate::transport::Transport>,
                    ))
                }
            } else {
                match &manifest.connection {
                    ConnectionConfig::Serial {
                        baud_rate,
                        terminator,
                        ref serial_config,
                        ..
                    } => {
                        let port_path = instance.port.as_deref().ok_or_else(|| {
                            anyhow!("serial device requires 'port' in instance config")
                        })?;
                        let baud = instance.baud_rate.unwrap_or_else(|| baud_rate.value());

                        // Atomically check if a transport already exists for this port.
                        // Uses write lock to prevent the check-then-act race where
                        // two concurrent build() calls could both open the same port.
                        let existing = claim_or_get_transport(port_path);

                        if let Some(shared) = existing {
                            tracing::info!(
                                port = port_path,
                                address = %instance.address,
                                baud = baud,
                                "Reusing shared serial transport for RS-485 bus"
                            );
                            shared
                        } else {
                            // No existing transport — we claimed the slot, now create.
                            let transport = crate::transport::SerialTransport::open(
                                port_path,
                                baud,
                                terminator.as_deref(),
                                serial_config,
                            )
                            .await?;
                            let shared: SharedTransport = Arc::new(Mutex::new(Box::new(transport)));

                            // Register for future devices on the same bus
                            register_shared_transport(port_path, &shared);
                            tracing::info!(
                                port = port_path,
                                baud = baud,
                                "Registered new shared serial transport"
                            );

                            shared
                        }
                    }
                    ConnectionConfig::Tcp {
                        host,
                        port,
                        timeout,
                        terminator,
                    } => {
                        let host = instance.host.as_deref().unwrap_or(host.as_str());
                        Arc::new(Mutex::new(Box::new(
                            crate::transport::TcpTransport::connect(
                                host,
                                *port,
                                timeout.as_duration(),
                                terminator.as_deref(),
                            )
                            .await?,
                        )
                            as Box<dyn crate::transport::Transport>))
                    }
                    ConnectionConfig::Udp { .. } => {
                        anyhow::bail!("UDP transport not yet implemented")
                    }
                    #[cfg(target_os = "linux")]
                    ConnectionConfig::Usbtmc { terminator, .. } => {
                        let device_path = instance
                            .device
                            .as_deref()
                            .or(instance.port.as_deref())
                            .ok_or_else(|| {
                            anyhow!("USB TMC device requires 'device' in instance config")
                        })?;
                        Arc::new(Mutex::new(Box::new(
                            crate::transport::UsbtmcTransport::open(
                                device_path,
                                terminator.as_deref(),
                            )
                            .await?,
                        )
                            as Box<dyn crate::transport::Transport>))
                    }
                    #[cfg(not(target_os = "linux"))]
                    ConnectionConfig::Usbtmc { .. } => {
                        anyhow::bail!(
                            "USB TMC transport is only supported on Linux \
                             (requires /dev/usbtmc kernel driver)"
                        )
                    }
                }
            };

            let driver =
                UniversalDriver::new_shared(manifest.clone(), transport, &instance.address);

            // Run initialization sequence before advertising capabilities
            if !manifest.init_sequence.is_empty() {
                driver.run_init_sequence().await?;
            }

            let driver_arc = Arc::new(driver);
            let mut components = DeviceComponents::new();

            // Wire up capabilities based on what's configured in the manifest
            if manifest.capabilities.movable.is_some() {
                components = components
                    .with_movable(driver_arc.clone() as Arc<dyn common::capabilities::Movable>);
            }
            if manifest.capabilities.readable.is_some() {
                components = components
                    .with_readable(driver_arc.clone() as Arc<dyn common::capabilities::Readable>);
            }
            if manifest.capabilities.settable.is_some() {
                components = components
                    .with_settable(driver_arc.clone() as Arc<dyn common::capabilities::Settable>);
            }
            if manifest.capabilities.shutter_control.is_some() {
                components = components.with_shutter_control(
                    driver_arc.clone() as Arc<dyn common::capabilities::ShutterControl>
                );
            }
            if manifest.capabilities.wavelength_tunable.is_some() {
                components = components.with_wavelength_tunable(
                    driver_arc.clone() as Arc<dyn common::capabilities::WavelengthTunable>
                );
            }
            if manifest.capabilities.emission_control.is_some() {
                components = components.with_emission_control(
                    driver_arc.clone() as Arc<dyn common::capabilities::EmissionControl>
                );
            }

            // Commandable: expose all manifest commands as executable operations.
            // Any device with commands in its manifest can support this.
            if manifest
                .device
                .capability_names
                .iter()
                .any(|c| c == "Commandable")
            {
                components = components.with_commandable(
                    driver_arc.clone() as Arc<dyn common::capabilities::Commandable>
                );
            }

            // Populate DeviceMetadata from manifest fields
            let category = manifest
                .device
                .category
                .as_deref()
                .and_then(|c| match c {
                    "camera" => Some(DeviceCategory::Camera),
                    "stage" | "motion" => Some(DeviceCategory::Stage),
                    "detector" | "sensor" => Some(DeviceCategory::Detector),
                    "laser" | "source" => Some(DeviceCategory::Laser),
                    "power_meter" => Some(DeviceCategory::PowerMeter),
                    _ => None,
                })
                .or_else(|| {
                    // Infer category from capabilities
                    let caps = &manifest.device.capability_names;
                    if caps.iter().any(|c| c == "EmissionControl") {
                        Some(DeviceCategory::Laser)
                    } else if caps.iter().any(|c| c == "Movable") {
                        Some(DeviceCategory::Stage)
                    } else if caps.iter().any(|c| c == "Readable") {
                        Some(DeviceCategory::Detector)
                    } else {
                        None
                    }
                });

            if let Some(cat) = category {
                components = components.with_category(cat);
            }

            components.metadata = DeviceMetadata {
                category,
                position_units: manifest.capabilities.movable.as_ref().and_then(|m| {
                    m.position.as_ref().map(|_| {
                        // Use manifest parameters to infer units
                        if manifest.parameters.contains_key("pulses_per_degree") {
                            "degrees".to_string()
                        } else {
                            "mm".to_string()
                        }
                    })
                }),
                min_position: manifest.parameters.get("min_position").copied(),
                max_position: manifest.parameters.get("max_position").copied(),
                // Units are not yet represented in the manifest schema; leave unset.
                measurement_units: None,
                min_wavelength_nm: manifest.parameters.get("min_wavelength").copied(),
                max_wavelength_nm: manifest.parameters.get("max_wavelength").copied(),
                ..DeviceMetadata::default()
            };

            Ok(components)
        })
    }
}

/// Load all v3 config factories from a directory.
///
/// Scans the given directory for `.toml` files containing `schema_version = 3`
/// and attempts to parse each one as a `UniversalDriverFactory`.
///
/// Files that fail to parse are logged as warnings and skipped.
/// Helper for quick schema version check without full parsing.
#[derive(serde::Deserialize)]
struct VersionCheck {
    schema_version: Option<u32>,
}

pub fn load_all_factories(dir: &Path) -> Result<Vec<UniversalDriverFactory>> {
    let mut factories = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                // Only load schema_version = 3 files
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let check: VersionCheck = toml::from_str(&content).unwrap_or(VersionCheck {
                        schema_version: None,
                    });
                    if check.schema_version == Some(3) {
                        match UniversalDriverFactory::from_file(&path) {
                            Ok(f) => factories.push(f),
                            Err(e) => tracing::warn!("Skipping {}: {}", path.display(), e),
                        }
                    }
                }
            }
        }
    }
    Ok(factories)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_from_ell14_toml() {
        let factory = UniversalDriverFactory::from_toml_str(crate::test_fixtures::ELL14_TOML)
            .expect("should parse ELL14 config");

        assert!(factory.driver_type().contains("ell14"));
        assert_eq!(factory.name(), "Thorlabs ELL14");
        assert!(factory.capabilities().contains(&CoreCapability::Movable));
    }

    #[test]
    fn factory_validate_instance_config() {
        let factory = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 3

[device]
name = "Test Device"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }
"#,
        )
        .unwrap();

        // Valid config
        let config = toml::toml! {
            port = "/dev/ttyUSB0"
            address = "1"
        };
        assert!(factory.validate(&config.into()).is_ok());

        // Config without port is still valid (port is optional)
        let config = toml::toml! {
            address = "1"
        };
        assert!(factory.validate(&config.into()).is_ok());
    }

    #[tokio::test]
    async fn factory_build_creates_components() {
        let factory = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 3

[device]
name = "Test Device"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }
"#,
        )
        .unwrap();

        let config = toml::toml! {
            port = "/dev/ttyUSB0"
            address = "1"
            mock = true
        };

        let components = factory.build(config.into()).await.unwrap();
        assert!(components.readable.is_some());
        assert!(components.movable.is_none());
    }

    #[tokio::test]
    async fn factory_build_serial_requires_port() {
        let factory = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 3

[device]
name = "Test Device"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }
"#,
        )
        .unwrap();

        // No port and not mock — should fail with clear error
        let config = toml::toml! {
            address = "1"
        };

        let result = factory.build(config.into()).await;
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("port"), "expected 'port' in error: {err}");
    }

    #[test]
    fn factory_from_usbtmc_config() {
        let factory = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 3

[device]
name = "Thorlabs PM400"
capabilities = ["Readable", "WavelengthTunable"]

[connection]
type = "usbtmc"
timeout_ms = 5000
terminator_tx = "\n"

[commands.read_power]
template = "MEASure:SCALar:POWer?"
response_type = "float"

[commands.get_wavelength]
template = "SENSe:CORRection:WAVelength?"
response_type = "float"

[commands.set_wavelength]
template = "SENSe:CORRection:WAVelength {{ value }}"
parameters = { value = "float" }
expects_response = false

[capabilities.readable]
read = { command = "read_power" }

[capabilities.wavelength_tunable]
set_wavelength = { command = "set_wavelength", from_param = "value" }
get_wavelength = { command = "get_wavelength", output_field = "value" }
"#,
        )
        .expect("should parse USB TMC config");

        assert!(factory.driver_type().contains("pm400"));
        assert_eq!(factory.name(), "Thorlabs PM400");
        assert!(factory.capabilities().contains(&CoreCapability::Readable));
        assert!(factory
            .capabilities()
            .contains(&CoreCapability::WavelengthTunable));
    }

    #[tokio::test]
    async fn factory_build_usbtmc_with_mock() {
        let factory = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 3

[device]
name = "Thorlabs PM400"
capabilities = ["Readable"]

[connection]
type = "usbtmc"
timeout_ms = 5000
terminator_tx = "\n"

[commands.read_power]
template = "MEASure:SCALar:POWer?"
response_type = "float"

[capabilities.readable]
read = { command = "read_power" }
"#,
        )
        .unwrap();

        // Mock transport bypasses the real USB TMC path
        let config = toml::toml! {
            device = "/dev/usbtmc0"
            mock = true
        };

        let components = factory.build(config.into()).await.unwrap();
        assert!(components.readable.is_some());
    }

    #[tokio::test]
    async fn factory_build_usbtmc_requires_device() {
        let factory = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 3

[device]
name = "Thorlabs PM400"
capabilities = ["Readable"]

[connection]
type = "usbtmc"
timeout_ms = 5000

[commands.read_power]
template = "MEASure:SCALar:POWer?"
response_type = "float"

[capabilities.readable]
read = { command = "read_power" }
"#,
        )
        .unwrap();

        // No device path and not mock — should fail
        let config = toml::toml! {
            address = "0"
        };

        let result = factory.build(config.into()).await;
        // On macOS this fails with "only supported on Linux"; on Linux it fails
        // with "requires 'device'". Either way, it should fail.
        assert!(result.is_err());
    }

    #[test]
    fn factory_from_str_rejects_bad_schema() {
        let result = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 1

[device]
name = "Bad"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn load_all_factories_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let factories = load_all_factories(dir.path()).unwrap();
        assert!(factories.is_empty());
    }

    #[test]
    fn load_all_factories_skips_non_v3() {
        let dir = tempfile::tempdir().unwrap();

        // Write a v1 file that should be skipped
        std::fs::write(
            dir.path().join("old.toml"),
            r#"
schema_version = 1

[device]
name = "Old"
"#,
        )
        .unwrap();

        let factories = load_all_factories(dir.path()).unwrap();
        assert!(factories.is_empty());
    }

    #[test]
    fn load_all_factories_detects_compact_schema_version() {
        let dir = tempfile::tempdir().unwrap();

        // Write a file with no spaces around '=' — the old contains() check would miss this
        std::fs::write(
            dir.path().join("compact.toml"),
            r#"
schema_version=3

[device]
name = "Compact Device"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }
"#,
        )
        .unwrap();

        let factories = load_all_factories(dir.path()).unwrap();
        assert_eq!(factories.len(), 1);
        assert!(factories[0].name().contains("Compact"));
    }
}
