use crate::driver::{GenericSerialDriver, SharedPort};
use anyhow::{anyhow, Result};
use common::driver::{
    Capability as CoreCapability, DeviceComponents, DriverFactory as DriverFactoryTrait,
};
use futures::future::BoxFuture;
use plugin_api::config::InstrumentConfig;
use std::path::Path;
use std::sync::Arc;

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
    capabilities: Vec<CoreCapability>,
    driver_type: String,
    name: String,
    port_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, SharedPort>>>,
}

impl GenericSerialDriverFactory {
    pub fn new(config: InstrumentConfig) -> Self {
        let driver_type = config.device.protocol.to_lowercase();
        let name = config.device.name.clone();
        let capabilities = config
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

        Self {
            config,
            capabilities,
            driver_type,
            name,
            port_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: InstrumentConfig = toml::from_str(&content)?;
        config.validate().map_err(|e: String| anyhow!(e))?;
        Ok(Self::new(config))
    }
}

impl DriverFactoryTrait for GenericSerialDriverFactory {
    fn driver_type(&self) -> &'static str {
        Box::leak(self.driver_type.clone().into_boxed_str())
    }
    fn name(&self) -> &'static str {
        Box::leak(self.name.clone().into_boxed_str())
    }
    fn capabilities(&self) -> &'static [CoreCapability] {
        Box::leak(self.capabilities.clone().into_boxed_slice())
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

            // Check cache first (lock held briefly, no await)
            let cached = {
                let cache = port_cache.lock().unwrap_or_else(|p| p.into_inner());
                cache.get(&resolved_path).cloned()
            };

            let shared_port = match cached {
                Some(p) => p,
                None => {
                    use common::serial::{open_serial_async, wrap_shared_unbuffered};
                    let dyn_port =
                        open_serial_async(&resolved_path, baud_rate, "GenericSerial").await?;
                    let shared: SharedPort = wrap_shared_unbuffered(dyn_port);
                    let mut cache = port_cache.lock().unwrap_or_else(|p| p.into_inner());
                    // Double-check: use existing if another task inserted while we opened
                    cache.entry(resolved_path).or_insert(shared).clone()
                }
            };

            let driver = GenericSerialDriver::new(inst_config, shared_port, &instance.address)?;
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
