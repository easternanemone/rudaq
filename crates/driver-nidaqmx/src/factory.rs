//! Driver factory for NI-DAQmx trigger registration

use crate::trigger::{NiDaqTrigger, TriggerMode};
use anyhow::{anyhow, Context, Result};
use common::driver::{Capability, DeviceComponents, DriverFactory};
use futures::future::BoxFuture;
use std::sync::Arc;
use tracing::info;

/// Factory for creating NI-DAQmx trigger devices
pub struct NiDaqTriggerFactory;

impl NiDaqTriggerFactory {
    /// Create a new factory instance
    pub fn new() -> Self {
        Self
    }
}

impl Default for NiDaqTriggerFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl DriverFactory for NiDaqTriggerFactory {
    fn driver_type(&self) -> &'static str {
        "nidaqmx_trigger"
    }

    fn name(&self) -> &'static str {
        "NI-DAQmx Trigger (PyO3 Bridge)"
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Triggerable]
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let table = config
            .as_table()
            .ok_or_else(|| anyhow!("Config must be a table"))?;

        // Validate required fields
        if !table.contains_key("high_time") {
            anyhow::bail!("Missing required field 'high_time'");
        }
        if !table.contains_key("low_time") {
            anyhow::bail!("Missing required field 'low_time'");
        }
        if !table.contains_key("samps_per_chan") {
            anyhow::bail!("Missing required field 'samps_per_chan'");
        }

        // Validate types
        let high_time = table
            .get("high_time")
            .and_then(|v| v.as_float())
            .ok_or_else(|| anyhow!("'high_time' must be a float"))?;

        let low_time = table
            .get("low_time")
            .and_then(|v| v.as_float())
            .ok_or_else(|| anyhow!("'low_time' must be a float"))?;

        let samps_per_chan = table
            .get("samps_per_chan")
            .and_then(|v| v.as_integer())
            .ok_or_else(|| anyhow!("'samps_per_chan' must be an integer"))?;

        // Validate ranges
        if high_time <= 0.0 {
            anyhow::bail!("'high_time' must be positive");
        }
        if low_time <= 0.0 {
            anyhow::bail!("'low_time' must be positive");
        }
        if samps_per_chan <= 0 {
            anyhow::bail!("'samps_per_chan' must be positive");
        }

        // Validate trigger_mode if present
        if let Some(mode) = table.get("trigger_mode") {
            let mode_str = mode
                .as_str()
                .ok_or_else(|| anyhow!("'trigger_mode' must be a string"))?;

            match mode_str {
                "digital" => {}
                "trigger_on_position" => {
                    // Validate trigger source
                    if !table.contains_key("trigger_source") {
                        anyhow::bail!(
                            "Missing 'trigger_source' for trigger_on_position mode"
                        );
                    }
                }
                _ => anyhow::bail!(
                    "Invalid trigger_mode '{}', must be 'digital' or 'trigger_on_position'",
                    mode_str
                ),
            }
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            info!("Building NI-DAQmx trigger device from config");

            let table = config
                .as_table()
                .ok_or_else(|| anyhow!("Config must be a table"))?;

            // Extract required parameters
            let high_time = table
                .get("high_time")
                .and_then(|v| v.as_float())
                .context("Missing or invalid 'high_time'")?;

            let low_time = table
                .get("low_time")
                .and_then(|v| v.as_float())
                .context("Missing or invalid 'low_time'")?;

            let samps_per_chan = table
                .get("samps_per_chan")
                .and_then(|v| v.as_integer())
                .context("Missing or invalid 'samps_per_chan'")? as u32;

            // Extract optional parameters
            let device_name = table
                .get("device_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Dev1");

            let counter = table
                .get("counter")
                .and_then(|v| v.as_str())
                .unwrap_or("ctr0");

            // Parse trigger mode
            let trigger_mode = match table.get("trigger_mode").and_then(|v| v.as_str()) {
                Some("trigger_on_position") => {
                    let trigger_source = table
                        .get("trigger_source")
                        .and_then(|v| v.as_str())
                        .context("Missing 'trigger_source' for trigger_on_position mode")?
                        .to_string();

                    let rising_edge = table
                        .get("rising_edge")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);

                    let retriggerable = table
                        .get("retriggerable")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    TriggerMode::TriggerOnPosition {
                        trigger_source,
                        rising_edge,
                        retriggerable,
                    }
                }
                Some("digital") | None => TriggerMode::Digital,
                Some(mode) => {
                    anyhow::bail!("Invalid trigger_mode: {}", mode);
                }
            };

            // Create driver
            let driver = Arc::new(
                NiDaqTrigger::with_device(
                    trigger_mode,
                    high_time,
                    low_time,
                    samps_per_chan,
                    device_name,
                    counter,
                )
                .await
                .context("Failed to create NI-DAQmx trigger")?,
            );

            info!("NI-DAQmx trigger device created successfully");

            // Return components
            Ok(DeviceComponents::new().with_triggerable(driver))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_factory_validation() {
        let factory = NiDaqTriggerFactory::new();

        // Valid config
        let config = toml::toml! {
            high_time = 0.1
            low_time = 0.001
            samps_per_chan = 1
        };
        assert!(factory.validate(&config).is_ok());

        // Missing field
        let config = toml::toml! {
            high_time = 0.1
            low_time = 0.001
        };
        assert!(factory.validate(&config).is_err());

        // Invalid type
        let config = toml::toml! {
            high_time = "not a number"
            low_time = 0.001
            samps_per_chan = 1
        };
        assert!(factory.validate(&config).is_err());
    }

    #[tokio::test]
    async fn test_factory_build_digital() {
        let factory = NiDaqTriggerFactory::new();

        let config = toml::toml! {
            high_time = 0.1
            low_time = 0.001
            samps_per_chan = 1
            trigger_mode = "digital"
        };

        let result = factory.build(config).await;
        assert!(result.is_ok());

        let components = result.unwrap();
        assert!(components.triggerable.is_some());
    }

    #[tokio::test]
    async fn test_factory_build_trigger_on_position() {
        let factory = NiDaqTriggerFactory::new();

        let config = toml::toml! {
            high_time = 0.001
            low_time = 0.001
            samps_per_chan = 1
            trigger_mode = "trigger_on_position"
            trigger_source = "/Dev1/PFI0"
            rising_edge = true
            retriggerable = false
        };

        let result = factory.build(config).await;
        assert!(result.is_ok());

        let components = result.unwrap();
        assert!(components.triggerable.is_some());
    }
}
