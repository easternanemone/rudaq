// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use db::config_store::{DbDriver, DbInstrument, config_hash, toml_to_json};
use db::{DaqDb, DbConfig};
use driver_registry::{create_registry_from_config, register_all_factories};
use hardware::registry::{DeviceRegistry, HardwareConfig};

/// Get the workspace root directory (two levels up from CARGO_MANIFEST_DIR).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .unwrap()
        .parent() // workspace root
        .unwrap()
        .to_path_buf()
}

/// Path to the mock_maitai_lab.toml hardware profile.
pub fn mock_maitai_lab_path() -> PathBuf {
    workspace_root().join("config/profiles/mock_maitai_lab.toml")
}

/// Path to config/devices directory (universal driver TOML configs).
pub fn config_devices_dir() -> PathBuf {
    workspace_root().join("config/devices")
}

/// Parse the mock_maitai_lab.toml hardware config.
pub fn load_mock_maitai_config() -> HardwareConfig {
    HardwareConfig::from_file(&mock_maitai_lab_path())
        .expect("mock_maitai_lab.toml should parse successfully")
}

/// Create a registry with all factories registered (including universal drivers).
pub async fn create_full_registry() -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    register_all_factories(&registry, Some(&config_devices_dir()))
        .await
        .expect("factory registration should succeed");
    registry
}

/// Create a registry populated with all 9 mock_maitai_lab devices.
pub async fn create_populated_registry() -> (DeviceRegistry, HardwareConfig) {
    let hw_config = load_mock_maitai_config();
    let devices_dir = config_devices_dir();
    let registry = create_registry_from_config(&hw_config, Some(devices_dir.as_path()))
        .await
        .expect("registry creation from mock_maitai_lab config should succeed");

    // Register extra factories for reconciler to use
    register_all_factories(&registry, Some(&devices_dir))
        .await
        .ok(); // May double-register, that's fine

    (registry, hw_config)
}

/// Convert HardwareConfig devices to DB instruments (mirrors db_bridge::devices_to_db).
pub fn devices_to_db(config: &HardwareConfig) -> Vec<DbInstrument> {
    config
        .devices
        .iter()
        .map(|d| DbInstrument {
            device_id: d.id.to_string(),
            name: d.name.clone(),
            driver_type: d.driver.driver_type.clone(),
            config: toml_to_json(&d.driver.config),
            enabled: d.enabled,
        })
        .collect()
}

/// Extract unique driver definitions from hardware config.
pub fn drivers_from_config(config: &HardwareConfig) -> Vec<DbDriver> {
    let mut seen = std::collections::HashSet::new();
    config
        .devices
        .iter()
        .filter(|d| seen.insert(d.driver.driver_type.clone()))
        .map(|d| DbDriver {
            driver_type: d.driver.driver_type.clone(),
            name: d.driver.driver_type.clone(),
            capabilities: vec![],
            commands: vec![],
        })
        .collect()
}

/// Shadow write a hardware config to the database (mirrors db_bridge::shadow_write).
pub async fn shadow_write(
    db: &DaqDb,
    config: &HardwareConfig,
) -> Result<(usize, usize), db::error::DbError> {
    let instruments = devices_to_db(config);
    let drivers = drivers_from_config(config);
    let driver_count = db.upsert_drivers(&drivers).await?;
    let report = db.upsert_instruments(&instruments).await?;
    Ok((driver_count, report.instruments_upserted))
}
