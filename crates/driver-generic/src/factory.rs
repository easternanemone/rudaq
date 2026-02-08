use crate::driver::{GenericSerialDriver, SharedPort};
use anyhow::{anyhow, Context, Result};
use common::driver::{
    Capability as CoreCapability, DeviceComponents, DriverFactory as DriverFactoryTrait,
};
use futures::future::BoxFuture;
use plugin_api::config::{DeviceConfig, InstrumentConfig};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::OnceCell;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GenericSerialInstanceConfig {
    pub port: String,
    #[serde(default = "default_address")]
    pub address: String,
    pub baud_rate: Option<u32>,
}

fn default_address() -> String {
    "0".to_string()
}

pub struct GenericSerialDriverFactory {
    config: InstrumentConfig,
    /// Pre-leaked at construction time (not per-call).
    capabilities: &'static [CoreCapability],
    driver_type: &'static str,
    name: &'static str,
    port_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<OnceCell<SharedPort>>>>>,
}

impl GenericSerialDriverFactory {
    pub fn new(config: InstrumentConfig) -> Self {
        // Leak once during construction to satisfy the &'static lifetime
        // required by the DriverFactory trait. Factory instances are long-lived
        // singletons created once at startup, so this is bounded.
        let driver_type: &'static str =
            Box::leak(config.device.protocol.to_lowercase().into_boxed_str());
        let name: &'static str = Box::leak(config.device.name.clone().into_boxed_str());
        let caps_vec: Vec<CoreCapability> = config
            .device
            .capabilities
            .iter()
            .filter_map(|cap: &String| match cap.as_str() {
                "Movable" => Some(CoreCapability::Movable),
                "Readable" => Some(CoreCapability::Readable),
                "WavelengthTunable" => Some(CoreCapability::WavelengthTunable),
                "ShutterControl" => Some(CoreCapability::ShutterControl),
                _ => None,
            })
            .collect();
        let capabilities: &'static [CoreCapability] = Box::leak(caps_vec.into_boxed_slice());

        Self {
            config,
            capabilities,
            driver_type,
            name,
            port_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        // Load config with inheritance support
        let mut config =
            Self::load_config_with_inheritance(path, &mut std::collections::HashSet::new())?;

        // Expand shorthand parameter definitions to full form
        config.expand_shorthand();

        config.validate().map_err(|e: String| anyhow!(e))?;
        Ok(Self::new(config))
    }

    fn load_config_with_inheritance(
        path: &Path,
        visited: &mut std::collections::HashSet<std::path::PathBuf>,
    ) -> Result<InstrumentConfig> {
        let canonical_path = path.canonicalize()?;

        // Detect circular dependencies
        if visited.contains(&canonical_path) {
            return Err(anyhow!(
                "Circular dependency detected: {}",
                canonical_path.display()
            ));
        }
        visited.insert(canonical_path.clone());

        let content = std::fs::read_to_string(path)?;
        let mut config: InstrumentConfig = toml::from_str(&content)?;

        // Handle inheritance
        if let Some(base_path_str) = &config.extends {
            let base_dir = path.parent().unwrap_or(Path::new("."));
            let base_file = base_dir.join(base_path_str);

            let base = Self::load_config_with_inheritance(&base_file, visited)?;
            config = Self::merge_configs(base, config)?;
        }

        Ok(config)
    }

    fn merge_configs(base: InstrumentConfig, child: InstrumentConfig) -> Result<InstrumentConfig> {
        Ok(InstrumentConfig {
            // Don't propagate extends from base
            extends: None,

            // Device config: merge fields, child wins
            device: DeviceConfig {
                name: child.device.name,
                protocol: if child.device.protocol.is_empty() {
                    base.device.protocol
                } else {
                    child.device.protocol
                },
                capabilities: if child.device.capabilities.is_empty() {
                    base.device.capabilities
                } else {
                    child.device.capabilities
                },
                description: child.device.description.or(base.device.description),
                manufacturer: child.device.manufacturer.or(base.device.manufacturer),
                model: child.device.model.or(base.device.model),
            },

            // Connection config: merge fields, child wins
            connection: Self::merge_connection(base.connection, child.connection),

            // HashMaps: merge with child keys winning
            parameters: Self::merge_hashmaps(base.parameters, child.parameters),
            commands: Self::merge_hashmaps(base.commands, child.commands),
            responses: Self::merge_hashmaps(base.responses, child.responses),
            conversions: Self::merge_hashmaps(base.conversions, child.conversions),
            error_codes: Self::merge_hashmaps(base.error_codes, child.error_codes),

            // Trait mapping: special merge
            trait_mapping: Self::merge_trait_mapping(base.trait_mapping, child.trait_mapping),

            // Retry: child wins if present
            default_retry: child.default_retry.or(base.default_retry),

            // Expanded parameters: will be populated by expand_shorthand() later
            expanded_parameters: None,

            // Init sequence: merge base and child sequences
            init_sequence: {
                let mut seq = base.init_sequence;
                seq.extend(child.init_sequence);
                seq
            },
        })
    }

    fn merge_connection(
        base: plugin_api::config::ConnectionConfig,
        child: plugin_api::config::ConnectionConfig,
    ) -> plugin_api::config::ConnectionConfig {
        use plugin_api::config::{
            default_baud_rate, default_data_bits, default_stop_bits, default_timeout_ms,
        };

        // For numeric/enum fields, inherit from base if child has the default value.
        // This allows child configs to omit fields they want to inherit.
        plugin_api::config::ConnectionConfig {
            r#type: child.r#type, // Type is typically explicit, not inherited
            baud_rate: if child.baud_rate == default_baud_rate() {
                base.baud_rate
            } else {
                child.baud_rate
            },
            data_bits: if child.data_bits == default_data_bits() {
                base.data_bits
            } else {
                child.data_bits
            },
            stop_bits: if child.stop_bits == default_stop_bits() {
                base.stop_bits
            } else {
                child.stop_bits
            },
            parity: if child.parity == Default::default() {
                base.parity
            } else {
                child.parity
            },
            flow_control: if child.flow_control == Default::default() {
                base.flow_control
            } else {
                child.flow_control
            },
            // For strings, empty can mean "inherit from base"
            terminator_tx: if child.terminator_tx.is_empty() {
                base.terminator_tx
            } else {
                child.terminator_tx
            },
            terminator_rx: if child.terminator_rx.is_empty() {
                base.terminator_rx
            } else {
                child.terminator_rx
            },
            timeout_ms: if child.timeout_ms == default_timeout_ms() {
                base.timeout_ms
            } else {
                child.timeout_ms
            },
            bus: child.bus.or(base.bus),
        }
    }

    fn merge_hashmaps<T: Clone>(
        mut base: HashMap<String, T>,
        child: HashMap<String, T>,
    ) -> HashMap<String, T> {
        base.extend(child);
        base
    }

    fn merge_trait_mapping(
        mut base: plugin_api::config::TraitMappingConfig,
        child: plugin_api::config::TraitMappingConfig,
    ) -> plugin_api::config::TraitMappingConfig {
        // Merge movable
        if child.movable.is_some() {
            base.movable = child.movable;
        }

        // Merge other traits
        base.traits.extend(child.traits);

        base
    }
}

impl DriverFactoryTrait for GenericSerialDriverFactory {
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
        let _: GenericSerialInstanceConfig = config.clone().try_into()?;
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        let inst_config = self.config.clone();
        let port_cache = self.port_cache.clone();
        Box::pin(async move {
            let instance: GenericSerialInstanceConfig = config.try_into()?;
            let baud_rate = instance
                .baud_rate
                .unwrap_or(inst_config.connection.baud_rate);

            // Wait, where is port_resolver? Let's assume it's accessible or just use the port string directly for now if unsure.
            // Actually, I saw it in daq-hardware. But I don't want to depend on daq-hardware if possible.
            // Let's just use the port directly for now.
            let resolved_path = instance.port.clone();

            let cell = {
                let mut cache = port_cache.lock().unwrap_or_else(|p| p.into_inner());
                cache
                    .entry(resolved_path.clone())
                    .or_insert_with(|| Arc::new(OnceCell::new()))
                    .clone()
            };

            let resolved_path_clone = resolved_path.clone();
            let shared_port = cell
                .get_or_try_init(|| async move {
                    use common::serial::{open_serial_async, wrap_shared_unbuffered};
                    let dyn_port =
                        open_serial_async(&resolved_path_clone, baud_rate, "GenericSerial").await?;
                    Ok::<SharedPort, anyhow::Error>(wrap_shared_unbuffered(dyn_port))
                })
                .await?
                .clone();

            let driver = GenericSerialDriver::new(inst_config, shared_port, &instance.address)?;

            // Validate device connection by running init sequence
            driver.run_init_sequence().await.context(
                "Failed to run init sequence - device may not be connected or responding",
            )?;

            let driver_arc = Arc::new(driver);
            Ok(DeviceComponents {
                movable: Some(driver_arc.clone() as Arc<dyn common::capabilities::Movable>),
                readable: Some(driver_arc.clone() as Arc<dyn common::capabilities::Readable>),
                wavelength_tunable: Some(
                    driver_arc.clone() as Arc<dyn common::capabilities::WavelengthTunable>
                ),
                shutter_control: Some(driver_arc as Arc<dyn common::capabilities::ShutterControl>),
                ..Default::default()
            })
        })
    }
}

pub fn load_all_factories(dir: &Path) -> Result<Vec<GenericSerialDriverFactory>> {
    let mut factories = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                if let Ok(f) = GenericSerialDriverFactory::from_file(&path) {
                    factories.push(f);
                }
            }
        }
    }
    Ok(factories)
}
