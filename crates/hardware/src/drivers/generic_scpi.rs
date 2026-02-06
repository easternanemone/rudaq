use crate::config::{CapabilityMapping, ConnectionConfigV2, InstrumentManifest, UiConfig};
use crate::drivers::parsing::{parse_response, ParsedValue};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use minijinja::Environment;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

pub trait DeviceIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> DeviceIo for T {}

pub type DynDeviceIo = Box<dyn DeviceIo>;
pub type SharedDeviceIo = Arc<Mutex<DynDeviceIo>>;

#[derive(Clone)]
pub struct GenericScpiDriver {
    manifest: Arc<InstrumentManifest>,
    io: SharedDeviceIo,
    template_env: Environment<'static>,
}

impl GenericScpiDriver {
    pub fn new(manifest: InstrumentManifest, io: SharedDeviceIo) -> Result<Self> {
        let mut env = Environment::new();
        for (name, cmd) in &manifest.commands {
            env.add_template_owned(name.clone(), cmd.template.clone())
                .map_err(|e| anyhow!("Failed to register template '{}': {}", name, e))?;
        }

        Ok(Self {
            manifest: Arc::new(manifest),
            io,
            template_env: env,
        })
    }

    pub fn manifest(&self) -> &InstrumentManifest {
        &self.manifest
    }

    /// Get UI configuration if defined in manifest.
    pub fn ui_config(&self) -> Option<&UiConfig> {
        self.manifest.ui.as_ref()
    }

    fn terminator(&self) -> &str {
        match &self.manifest.connection {
            ConnectionConfigV2::Serial { terminator, .. } => terminator.as_str(),
            ConnectionConfigV2::Tcp { terminator, .. } => terminator.as_str(),
        }
    }

    pub async fn exec<T>(&self, command_name: &str, params: T) -> Result<String>
    where
        T: Serialize,
    {
        let template = self
            .template_env
            .get_template(command_name)
            .map_err(|e| anyhow!("Template error: {}", e))?;
        let cmd_str = template
            .render(params)
            .map_err(|e| anyhow!("Rendering error: {}", e))?;

        let terminator = self.terminator();
        let full_cmd = format!("{}{}", cmd_str, terminator);
        let mut io = self.io.lock().await;
        io.write_all(full_cmd.as_bytes())
            .await
            .map_err(|e| anyhow!("Failed to write command: {}", e))?;
        io.flush()
            .await
            .map_err(|e| anyhow!("Failed to flush command: {}", e))?;

        let cmd_config = self
            .manifest
            .commands
            .get(command_name)
            .ok_or_else(|| anyhow!("Unknown command: {}", command_name))?;
        if !cmd_config.query {
            return Ok(String::new());
        }

        let timeout_ms = match &self.manifest.connection {
            ConnectionConfigV2::Serial { timeout_ms, .. } => *timeout_ms,
            ConnectionConfigV2::Tcp { timeout_ms, .. } => *timeout_ms,
        };
        let timeout = Duration::from_millis(timeout_ms.max(1));

        let mut response = Vec::new();
        let mut buf = [0u8; 256];
        const MAX_RESPONSE_BYTES: usize = 64 * 1024;
        loop {
            let read = tokio::time::timeout(timeout, io.read(&mut buf))
                .await
                .map_err(|_| anyhow!("Timed out waiting for response"))?
                .map_err(|e| anyhow!("Failed to read response: {}", e))?;
            if read == 0 {
                break;
            }
            response.extend_from_slice(&buf[..read]);
            if response.len() > MAX_RESPONSE_BYTES {
                return Err(anyhow!(
                    "Response exceeded maximum size ({} bytes)",
                    MAX_RESPONSE_BYTES
                ));
            }
            if response.ends_with(terminator.as_bytes()) {
                break;
            }
        }

        let response = String::from_utf8(response)
            .map_err(|e| anyhow!("Response was not valid UTF-8: {}", e))?;
        Ok(response.trim().to_string())
    }

    pub async fn exec_parsed<T>(&self, command_name: &str, params: T) -> Result<ParsedValue>
    where
        T: Serialize,
    {
        let raw = self.exec(command_name, params).await?;
        let cmd_config = self
            .manifest
            .commands
            .get(command_name)
            .ok_or_else(|| anyhow!("Unknown command: {}", command_name))?;
        parse_response(&raw, cmd_config.response_type).map_err(|e| anyhow!("Parsing error: {}", e))
    }

    fn capabilities(&self) -> &CapabilityMapping {
        &self.manifest.capabilities
    }
}

#[async_trait]
impl crate::capabilities::Movable for GenericScpiDriver {
    async fn move_abs(&self, pos: f64) -> Result<()> {
        let Some(mapping) = &self.capabilities().movable else {
            return Err(anyhow!("Movable capability not mapped"));
        };
        let ctx = serde_json::json!({ "value": pos });
        self.exec(&mapping.move_abs, ctx).await?;
        Ok(())
    }

    async fn move_rel(&self, dist: f64) -> Result<()> {
        let Some(mapping) = &self.capabilities().movable else {
            return Err(anyhow!("Movable capability not mapped"));
        };
        let ctx = serde_json::json!({ "value": dist });
        self.exec(&mapping.move_rel, ctx).await?;
        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        let Some(mapping) = &self.capabilities().movable else {
            return Err(anyhow!("Movable capability not mapped"));
        };
        let parsed = self.exec_parsed(&mapping.position, ()).await?;
        parsed
            .as_f64()
            .ok_or_else(|| anyhow!("Position response was not numeric"))
    }

    async fn wait_settled(&self) -> Result<()> {
        let Some(mapping) = &self.capabilities().movable else {
            return Err(anyhow!("Movable capability not mapped"));
        };
        if let Some(command) = &mapping.wait_settled {
            self.exec(command, ()).await?;
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        let Some(mapping) = &self.capabilities().movable else {
            return Err(anyhow!("Movable capability not mapped"));
        };
        self.exec(&mapping.stop, ()).await?;
        Ok(())
    }
}

#[async_trait]
impl crate::capabilities::Readable for GenericScpiDriver {
    async fn read(&self) -> Result<f64> {
        let Some(mapping) = &self.capabilities().readable else {
            return Err(anyhow!("Readable capability not mapped"));
        };
        let parsed = self.exec_parsed(&mapping.read, ()).await?;
        parsed
            .as_f64()
            .ok_or_else(|| anyhow!("Readable response was not numeric"))
    }
}

#[async_trait]
impl crate::capabilities::WavelengthTunable for GenericScpiDriver {
    async fn set_wavelength(&self, wl: f64) -> Result<()> {
        let Some(mapping) = &self.capabilities().wavelength_tunable else {
            return Err(anyhow!("WavelengthTunable capability not mapped"));
        };
        let ctx = serde_json::json!({ "value": wl });
        self.exec(&mapping.set_wavelength, ctx).await?;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        let Some(mapping) = &self.capabilities().wavelength_tunable else {
            return Err(anyhow!("WavelengthTunable capability not mapped"));
        };
        let parsed = self.exec_parsed(&mapping.get_wavelength, ()).await?;
        parsed
            .as_f64()
            .ok_or_else(|| anyhow!("Wavelength response was not numeric"))
    }

    fn wavelength_range(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
}

#[async_trait]
impl crate::capabilities::ShutterControl for GenericScpiDriver {
    async fn open_shutter(&self) -> Result<()> {
        let Some(mapping) = &self.capabilities().shutter_control else {
            return Err(anyhow!("ShutterControl capability not mapped"));
        };
        self.exec(&mapping.open_shutter, ()).await?;
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        let Some(mapping) = &self.capabilities().shutter_control else {
            return Err(anyhow!("ShutterControl capability not mapped"));
        };
        self.exec(&mapping.close_shutter, ()).await?;
        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        let Some(mapping) = &self.capabilities().shutter_control else {
            return Err(anyhow!("ShutterControl capability not mapped"));
        };
        let parsed = self.exec_parsed(&mapping.is_shutter_open, ()).await?;
        parsed
            .as_bool()
            .ok_or_else(|| anyhow!("Shutter response was not boolean"))
    }
}
