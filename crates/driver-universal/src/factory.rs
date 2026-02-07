//! UniversalDriverFactory: DriverFactory implementation for config-driven devices.
//!
//! Loads device manifests from TOML files (schema_version = 3) and creates
//! `UniversalDriver` instances wired to the appropriate capability trait objects.

use crate::config::validated::DeviceManifest;
use crate::config::{parse_manifest, RawManifest};
use crate::driver::UniversalDriver;
use anyhow::{anyhow, Context, Result};
use common::driver::{Capability as CoreCapability, DeviceComponents, DriverFactory};
use futures::future::BoxFuture;
use std::path::Path;
use std::sync::Arc;

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
                // Commandable and Parameterized are not implemented by UniversalDriver;
                // skip them to avoid advertising unsupported capabilities.
                "Commandable" | "Parameterized" => None,
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
            let transport: Box<dyn crate::transport::Transport> = if instance.mock {
                Box::new(crate::transport::MockTransport::new(vec![]))
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
                        Box::new(
                            crate::transport::SerialTransport::open(
                                port_path,
                                baud,
                                terminator.as_deref(),
                                serial_config,
                            )
                            .await?,
                        )
                    }
                    ConnectionConfig::Tcp {
                        host,
                        port,
                        timeout,
                    } => {
                        let host = instance.host.as_deref().unwrap_or(host.as_str());
                        Box::new(
                            crate::transport::TcpTransport::connect(
                                host,
                                *port,
                                timeout.as_duration(),
                            )
                            .await?,
                        )
                    }
                    ConnectionConfig::Udp { .. } => {
                        anyhow::bail!("UDP transport not yet implemented")
                    }
                }
            };

            let driver = UniversalDriver::new(manifest.clone(), transport, &instance.address);

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
        let factory = UniversalDriverFactory::from_toml_str(
            r#"
schema_version = 3

[device]
name = "Thorlabs ELL14"
capabilities = ["Movable", "Parameterized"]

[connection]
type = "serial"
baud_rate = 9600
timeout_ms = 1000

[commands.move_absolute]
template = "{{ address }}ma{{ position_pulses | hex(8) }}"
parameters = { position_pulses = "int32" }
response = "position"

[commands.get_position]
template = "{{ address }}gp"
response = "position"

[commands.get_status]
template = "{{ address }}gs"
response = "status"

[commands.stop]
template = "{{ address }}st"
expects_response = false

[responses.position]
format = "{addr:1}PO{pulses:hex8}"

[responses.status]
format = "{addr:1}GS{code:hex2}"

[conversions.degrees_to_pulses]
formula = "round(degrees * 398.2222)"

[conversions.pulses_to_degrees]
formula = "pulses / 398.2222"

[capabilities.movable]
move_abs = { command = "move_absolute", input_conversion = "degrees_to_pulses", input_param = "position_pulses", from_param = "degrees" }
position = { command = "get_position", output_conversion = "pulses_to_degrees", output_field = "pulses" }
stop = { command = "stop" }

[capabilities.movable.wait_settled]
poll_command = "get_status"
success_condition = "code == 0"
poll_interval_ms = 50
timeout_ms = 10000
"#,
        )
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
