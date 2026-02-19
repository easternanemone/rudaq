//! Runtime metadata integration matrix.
//!
//! Covers representative launch profiles across:
//! - no-db + universal-only
//! - no-db + hybrid (universal + camera-native)
//! - db-on + hybrid metadata parity

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use hardware::registry::{create_registry_from_config, register_all_factories, HardwareConfig};

#[cfg(feature = "db-surreal-mem")]
use db::config_store::{toml_to_json, DbDriver, DbInstrument};
#[cfg(feature = "db-surreal-mem")]
use db::{DaqDb, DbConfig};
#[cfg(feature = "db-surreal-mem")]
use std::collections::HashMap;
#[cfg(feature = "db-surreal-mem")]
use std::collections::HashSet;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("integration-tests crate should be under /crates")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}

fn devices_dir() -> PathBuf {
    workspace_root().join("config/devices")
}

fn profile_path(path: &str) -> PathBuf {
    workspace_root().join(path)
}

fn load_profile(path: &str) -> HardwareConfig {
    HardwareConfig::from_file(&profile_path(path)).expect("profile should parse")
}

async fn create_profile_registry(path: &str) -> hardware::registry::DeviceRegistry {
    let config = load_profile(path);
    let registry = create_registry_from_config(&config, Some(&devices_dir()))
        .await
        .expect("registry should build from profile");
    register_all_factories(&registry, Some(&devices_dir()))
        .await
        .expect("factory registration should succeed");
    registry
}

#[cfg(feature = "db-surreal-mem")]
fn normalize(values: &[String]) -> Vec<String> {
    let mut out = values.to_vec();
    out.sort();
    out.dedup();
    out
}

#[tokio::test]
async fn matrix_no_db_universal_only_metadata_ui() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let devices = registry.list_devices();

    assert_eq!(devices.len(), 3, "expected 3 mock ELL14 devices");
    for device in devices {
        assert!(
            device.driver_type.starts_with("universal_"),
            "device '{}' should use universal driver, got '{}'",
            device.id,
            device.driver_type
        );
        assert!(
            !device.metadata.available_commands.is_empty(),
            "device '{}' should expose available_commands",
            device.id
        );
        assert!(
            device.metadata.ui_schema_json.is_some(),
            "device '{}' should expose ui_schema_json from manifest [ui]",
            device.id
        );
    }
}

#[tokio::test]
async fn matrix_no_db_hybrid_camera_native_parity() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;
    let devices = registry.list_devices();

    assert_eq!(
        devices.len(),
        9,
        "mock maitai profile should register 9 devices"
    );

    let camera = devices
        .iter()
        .find(|device| device.driver_type == "mock_camera");
    assert!(
        camera.is_some(),
        "hybrid profile should include native camera driver"
    );
    let camera = camera.expect("camera must exist");
    assert!(
        camera.metadata.ui_schema_json.is_none(),
        "native camera should not require universal UI schema"
    );

    let universal_devices: Vec<_> = devices
        .iter()
        .filter(|device| device.driver_type.starts_with("universal_"))
        .collect();
    assert!(
        !universal_devices.is_empty(),
        "hybrid profile should include universal devices"
    );
    let mut ui_schema_count = 0usize;
    for device in universal_devices {
        if device.metadata.ui_schema_json.is_some() {
            ui_schema_count += 1;
        }
        assert!(
            !device.metadata.available_commands.is_empty(),
            "universal device '{}' should expose command catalog",
            device.id
        );
    }
    assert!(
        ui_schema_count > 0,
        "expected at least one universal device to expose ui schema metadata"
    );
}

#[cfg(feature = "db-surreal-mem")]
#[tokio::test]
async fn matrix_db_on_hybrid_driver_metadata_parity() {
    let config = load_profile("config/profiles/mock_maitai_lab.toml");
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;
    let db = DaqDb::init(DbConfig::in_memory())
        .await
        .expect("in-memory db should initialize");

    let mut seen_driver_types = HashSet::new();
    let mut drivers = Vec::<DbDriver>::new();
    for device in &config.devices {
        if !seen_driver_types.insert(device.driver.driver_type.clone()) {
            continue;
        }
        let factory = registry
            .factory_info(&device.driver.driver_type)
            .expect("profile driver type should have registered factory");
        drivers.push(DbDriver {
            driver_type: device.driver.driver_type.clone(),
            name: factory.name,
            capabilities: factory.capabilities,
            commands: factory.available_commands,
        });
    }

    let instruments: Vec<DbInstrument> = config
        .devices
        .iter()
        .map(|device| DbInstrument {
            device_id: device.id.clone(),
            name: device.name.clone(),
            driver_type: device.driver.driver_type.clone(),
            config: toml_to_json(&device.driver.config),
            enabled: device.enabled,
        })
        .collect();

    db.upsert_drivers(&drivers)
        .await
        .expect("driver metadata upsert should succeed");
    let report = db
        .upsert_instruments(&instruments)
        .await
        .expect("instrument upsert should succeed");
    assert_eq!(
        report.instruments_upserted,
        instruments.len(),
        "all instruments should be persisted"
    );

    let stored: HashMap<String, DbDriver> = db
        .get_all_drivers()
        .await
        .expect("driver read should succeed")
        .into_iter()
        .map(|driver| (driver.driver_type.clone(), driver))
        .collect();

    for expected in &drivers {
        let actual = stored
            .get(&expected.driver_type)
            .expect("persisted driver row should exist");
        assert_eq!(
            normalize(&actual.capabilities),
            normalize(&expected.capabilities),
            "capabilities mismatch for driver '{}'",
            expected.driver_type
        );
        assert_eq!(
            normalize(&actual.commands),
            normalize(&expected.commands),
            "commands mismatch for driver '{}'",
            expected.driver_type
        );
    }
}
