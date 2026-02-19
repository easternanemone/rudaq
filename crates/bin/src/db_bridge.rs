//! Bridge between hardware-crate config types and db-crate record types.
//!
//! Conversions live here (in `bin`) because `db` cannot depend on `hardware`
//! — Phase 2 will add `hardware → db`, which would create a cycle if reversed.
//!
//! Export functions (`db_to_hardware_config`, `db_to_hardware_toml`) are used
//! by CLI subcommands (`rust-daq config export`).

use db::config_store::{json_to_toml, toml_to_json, DbDriver, DbInstrument};
use hardware::registry::{DeviceConfig, DeviceRegistry, DriverConfig, HardwareConfig};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Import direction: HardwareConfig → DB records
// ---------------------------------------------------------------------------

/// Convert a parsed `HardwareConfig` into DB-native instrument records.
pub fn devices_to_db(config: &HardwareConfig) -> Vec<DbInstrument> {
    config
        .devices
        .iter()
        .map(|d| DbInstrument {
            device_id: d.id.clone(),
            name: d.name.clone(),
            driver_type: d.driver.driver_type.clone(),
            config: toml_to_json(&d.driver.config),
            enabled: d.enabled,
        })
        .collect()
}

/// Extract unique driver definitions from a `HardwareConfig`.
///
/// Since the TOML config doesn't carry driver metadata (capabilities, etc.),
/// we use the driver_type as a placeholder name. Driver records are later
/// enriched by the plugin/factory system.
pub fn drivers_from_config(config: &HardwareConfig) -> Vec<DbDriver> {
    drivers_from_config_with_sources(config, None, &HashMap::new())
}

/// Extract driver definitions from config, enriched by registered factory
/// introspection when available.
///
/// Enrichment source precedence:
/// 1. Factory metadata (`name`, `capabilities`, `available_commands`)
/// 2. TOML fallback (`driver_type` placeholders with empty lists)
pub fn drivers_from_config_with_registry(
    config: &HardwareConfig,
    registry: &DeviceRegistry,
) -> Vec<DbDriver> {
    drivers_from_config_with_sources(config, Some(registry), &HashMap::new())
}

/// Build driver rows from TOML config with optional factory enrichment and
/// fallback to existing DB metadata.
///
/// Precedence:
/// 1. Factory metadata (if registry/factory exists)
/// 2. Existing DB metadata for that driver_type
/// 3. Placeholder TOML fallback (driver_type only)
pub fn drivers_from_config_with_sources(
    config: &HardwareConfig,
    registry: Option<&DeviceRegistry>,
    existing_by_driver_type: &HashMap<String, DbDriver>,
) -> Vec<DbDriver> {
    let mut seen = HashSet::new();
    config
        .devices
        .iter()
        .filter(|d| seen.insert(d.driver.driver_type.clone()))
        .map(|d| {
            if let Some(info) = registry.and_then(|r| r.factory_info(&d.driver.driver_type)) {
                DbDriver {
                    driver_type: d.driver.driver_type.clone(),
                    name: info.name,
                    capabilities: info.capabilities,
                    commands: info.available_commands,
                }
            } else if let Some(existing) = existing_by_driver_type.get(&d.driver.driver_type) {
                existing.clone()
            } else {
                DbDriver {
                    driver_type: d.driver.driver_type.clone(),
                    name: d.driver.driver_type.clone(),
                    capabilities: vec![],
                    commands: vec![],
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Export direction: DB records → HardwareConfig TOML
// ---------------------------------------------------------------------------

/// Reconstruct a `HardwareConfig` from DB instrument records.
pub fn db_to_hardware_config(instruments: &[DbInstrument]) -> HardwareConfig {
    let devices = instruments
        .iter()
        .map(|inst| DeviceConfig {
            id: inst.device_id.clone(),
            name: inst.name.clone(),
            driver: DriverConfig::new(inst.driver_type.clone(), json_to_toml(&inst.config)),
            enabled: inst.enabled,
        })
        .collect();

    HardwareConfig {
        plugin_paths: vec![],
        devices,
    }
}

/// Export DB instruments as a TOML string compatible with `HardwareConfig`.
pub fn db_to_hardware_toml(instruments: &[DbInstrument]) -> Result<String, toml::ser::Error> {
    let hw_config = db_to_hardware_config(instruments);
    toml::to_string_pretty(&hw_config)
}

// ---------------------------------------------------------------------------
// Shadow write helper
// ---------------------------------------------------------------------------

/// Shadow-write a parsed hardware config into the database.
///
/// Non-fatal: returns errors instead of panicking so the caller can log and
/// continue booting from TOML.
pub async fn shadow_write(
    db: &db::DaqDb,
    config: &HardwareConfig,
) -> Result<(usize, usize), db::error::DbError> {
    let instruments = devices_to_db(config);
    let existing_by_driver_type: HashMap<String, DbDriver> = db
        .get_all_drivers()
        .await?
        .into_iter()
        .map(|driver| (driver.driver_type.clone(), driver))
        .collect();
    let drivers = drivers_from_config_with_sources(config, None, &existing_by_driver_type);

    let driver_count = db.upsert_drivers(&drivers).await?;
    let report = db.upsert_instruments(&instruments).await?;

    Ok((driver_count, report.instruments_upserted))
}

/// Shadow-write a parsed hardware config into the database, enriching drivers
/// from registered factory metadata when possible.
pub async fn shadow_write_with_registry(
    db: &db::DaqDb,
    config: &HardwareConfig,
    registry: &DeviceRegistry,
) -> Result<(usize, usize), db::error::DbError> {
    let instruments = devices_to_db(config);
    let existing_by_driver_type: HashMap<String, DbDriver> = db
        .get_all_drivers()
        .await?
        .into_iter()
        .map(|driver| (driver.driver_type.clone(), driver))
        .collect();
    let drivers =
        drivers_from_config_with_sources(config, Some(registry), &existing_by_driver_type);

    let driver_count = db.upsert_drivers(&drivers).await?;
    let report = db.upsert_instruments(&instruments).await?;

    Ok((driver_count, report.instruments_upserted))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use db::DbConfig;

    fn sample_hardware_config() -> HardwareConfig {
        let toml_str = r#"
            [[devices]]
            id = "rotator_2"
            name = "ELL14 Rotator (Address 2)"
            enabled = true

            [devices.driver]
            type = "ell14"
            port = "/dev/serial/by-id/usb-FTDI-port0"
            address = "2"

            [[devices]]
            id = "power_meter"
            name = "Newport 1830-C"

            [devices.driver]
            type = "newport1830_c"
            port = "/dev/ttyS0"
        "#;
        toml::from_str(toml_str).unwrap()
    }

    #[test]
    fn test_devices_to_db() {
        let config = sample_hardware_config();
        let instruments = devices_to_db(&config);

        assert_eq!(instruments.len(), 2);
        assert_eq!(instruments[0].device_id, "rotator_2");
        assert_eq!(instruments[0].driver_type, "ell14");
        assert_eq!(
            instruments[0].config["port"],
            "/dev/serial/by-id/usb-FTDI-port0"
        );
        assert_eq!(instruments[0].config["address"], "2");
        assert!(instruments[0].enabled);

        assert_eq!(instruments[1].device_id, "power_meter");
        assert_eq!(instruments[1].driver_type, "newport1830_c");
    }

    #[test]
    fn test_drivers_from_config() {
        let config = sample_hardware_config();
        let drivers = drivers_from_config(&config);

        assert_eq!(drivers.len(), 2);
        assert_eq!(drivers[0].driver_type, "ell14");
        assert_eq!(drivers[1].driver_type, "newport1830_c");
    }

    #[test]
    fn test_drivers_from_config_preserves_existing_metadata_without_registry() {
        let config = sample_hardware_config();
        let mut existing = HashMap::new();
        existing.insert(
            "ell14".to_string(),
            DbDriver {
                driver_type: "ell14".to_string(),
                name: "ELL14 Rotator".to_string(),
                capabilities: vec!["movable".to_string()],
                commands: vec!["home".to_string(), "stop".to_string()],
            },
        );

        let drivers = drivers_from_config_with_sources(&config, None, &existing);
        let ell14 = drivers
            .iter()
            .find(|driver| driver.driver_type == "ell14")
            .expect("ell14 driver missing");
        let newport = drivers
            .iter()
            .find(|driver| driver.driver_type == "newport1830_c")
            .expect("newport1830_c driver missing");

        assert_eq!(ell14.name, "ELL14 Rotator");
        assert_eq!(ell14.capabilities, vec!["movable"]);
        assert_eq!(ell14.commands, vec!["home", "stop"]);
        assert!(newport.capabilities.is_empty());
        assert!(newport.commands.is_empty());
    }

    #[test]
    fn test_round_trip_toml_to_db_to_toml() {
        let config = sample_hardware_config();
        let instruments = devices_to_db(&config);
        let reconstructed = db_to_hardware_config(&instruments);

        assert_eq!(reconstructed.devices.len(), 2);
        assert_eq!(reconstructed.devices[0].id, "rotator_2");
        assert_eq!(reconstructed.devices[0].driver.driver_type, "ell14");
        assert_eq!(
            reconstructed.devices[0].driver.config.as_table().unwrap()["port"],
            toml::Value::String("/dev/serial/by-id/usb-FTDI-port0".into())
        );
    }

    #[test]
    fn test_export_toml_string() {
        let config = sample_hardware_config();
        let instruments = devices_to_db(&config);
        let toml_str = db_to_hardware_toml(&instruments).unwrap();

        // Verify the exported TOML can be re-parsed
        let reparsed: HardwareConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(reparsed.devices.len(), 2);
        assert_eq!(reparsed.devices[0].id, "rotator_2");
        assert_eq!(reparsed.devices[0].driver.driver_type, "ell14");
    }

    #[tokio::test]
    async fn test_shadow_write_preserves_existing_driver_metadata() {
        let db = db::DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.upsert_drivers(&[DbDriver {
            driver_type: "ell14".to_string(),
            name: "ELL14 Rotator".to_string(),
            capabilities: vec!["movable".to_string(), "homable".to_string()],
            commands: vec!["home".to_string(), "stop".to_string()],
        }])
        .await
        .unwrap();

        let config = sample_hardware_config();
        shadow_write(&db, &config).await.unwrap();

        let drivers = db.get_all_drivers().await.unwrap();
        let ell14 = drivers
            .iter()
            .find(|driver| driver.driver_type == "ell14")
            .expect("ell14 row missing after shadow_write");
        assert_eq!(ell14.name, "ELL14 Rotator");
        assert_eq!(ell14.capabilities, vec!["movable", "homable"]);
        assert_eq!(ell14.commands, vec!["home", "stop"]);
    }
}
