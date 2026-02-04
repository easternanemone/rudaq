//! DriverFactory implementation for Dover Motion axis drivers.

use anyhow::{anyhow, Context, Result};
use common::driver::{Capability, DeviceComponents, DriverFactory};
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;

#[cfg(feature = "dover-hardware")]
use crate::driver::DoverAxisDriver;
#[cfg(not(feature = "dover-hardware"))]
use crate::mock::DoverMockDriver;

/// Configuration for Dover Motion axis driver.
///
/// # Example TOML
///
/// ```toml
/// [[devices]]
/// id = "smartstage_x"
/// type = "dover_axis"
/// enabled = true
///
/// [devices.config]
/// device_path = "C:\\ProgramData\\Dover Motion\\SmartStage.xml"
/// axis_name = "X"
/// communication_type = "USB"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct DoverAxisConfig {
    /// Path to Dover Motion device configuration file
    pub device_path: String,

    /// Axis name (e.g., "X", "Y", "Z")
    pub axis_name: String,

    /// Communication type ("USB", "Ethernet", etc.)
    pub communication_type: String,
}

/// Factory for creating Dover Motion axis driver instances.
pub struct DoverAxisFactory;

/// Static capabilities for Dover axis drivers
static DOVER_CAPABILITIES: &[Capability] = &[
    Capability::Movable,
    Capability::Parameterized,
    Capability::TriggerOnPosition,
];

impl DriverFactory for DoverAxisFactory {
    fn driver_type(&self) -> &'static str {
        "dover_axis"
    }

    fn name(&self) -> &'static str {
        "Dover Motion Axis (SmartStage/DOF-5)"
    }

    fn capabilities(&self) -> &'static [Capability] {
        DOVER_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: DoverAxisConfig = config.clone().try_into()?;

        if cfg.axis_name.is_empty() {
            return Err(anyhow!("Dover axis_name cannot be empty"));
        }

        if cfg.device_path.is_empty() {
            return Err(anyhow!("Dover device_path cannot be empty"));
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: DoverAxisConfig = config.try_into().context("Invalid Dover axis config")?;

            #[cfg(feature = "dover-hardware")]
            {
                let driver = Arc::new(
                    DoverAxisDriver::new_async(
                        &cfg.device_path,
                        &cfg.axis_name,
                        &cfg.communication_type,
                    )
                    .await?,
                );

                Ok(DeviceComponents {
                    movable: Some(driver.clone()),
                    parameterized: Some(driver.clone()),
                    trigger_on_position: Some(driver),
                    ..Default::default()
                })
            }

            #[cfg(not(feature = "dover-hardware"))]
            {
                tracing::warn!(
                    "Dover hardware feature disabled - using mock driver for axis '{}'",
                    cfg.axis_name
                );

                let driver = Arc::new(DoverMockDriver::new(&cfg.axis_name));

                Ok(DeviceComponents {
                    movable: Some(driver.clone()),
                    parameterized: Some(driver.clone()),
                    trigger_on_position: Some(driver),
                    ..Default::default()
                })
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_factory_driver_type() {
        let factory = DoverAxisFactory;
        assert_eq!(factory.driver_type(), "dover_axis");
        assert_eq!(factory.name(), "Dover Motion Axis (SmartStage/DOF-5)");
    }

    #[test]
    fn test_factory_capabilities() {
        let factory = DoverAxisFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Movable));
        assert!(caps.contains(&Capability::Parameterized));
        assert!(caps.contains(&Capability::TriggerOnPosition));
    }

    #[test]
    fn test_factory_validate_config() {
        let factory = DoverAxisFactory;

        // Valid config
        let valid_config = toml::toml! {
            device_path = "C:\\ProgramData\\Dover Motion\\SmartStage.xml"
            axis_name = "X"
            communication_type = "USB"
        };
        assert!(factory.validate(&toml::Value::Table(valid_config)).is_ok());

        // Missing axis_name
        let missing_axis = toml::toml! {
            device_path = "C:\\ProgramData\\Dover Motion\\SmartStage.xml"
            communication_type = "USB"
        };
        assert!(factory.validate(&toml::Value::Table(missing_axis)).is_err());

        // Empty device_path
        let empty_path = toml::toml! {
            device_path = ""
            axis_name = "X"
            communication_type = "USB"
        };
        assert!(factory.validate(&toml::Value::Table(empty_path)).is_err());
    }
}
