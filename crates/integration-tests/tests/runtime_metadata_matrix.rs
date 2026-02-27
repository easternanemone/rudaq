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

fn normalize(values: &[String]) -> Vec<String> {
    let mut out = values.to_vec();
    out.sort();
    out.dedup();
    out
}

fn normalize_capabilities(values: &[common::driver::Capability]) -> Vec<String> {
    let mut out: Vec<String> = values.iter().map(|cap| cap.as_str().to_string()).collect();
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

// =========================================================================
// Metadata Enrichment Assertions (bd-zyc8)
// =========================================================================

#[tokio::test]
async fn matrix_universal_factory_info_matches_device_commands() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;

    for device in registry.list_devices() {
        let factory = registry
            .factory_info(&device.driver_type)
            .expect("factory info must exist for universal device");

        assert_eq!(
            normalize(&factory.available_commands),
            normalize(&device.metadata.available_commands),
            "command catalog enrichment failed for device '{}' ({})",
            device.id,
            device.driver_type
        );
    }
}

#[tokio::test]
async fn matrix_universal_device_capabilities_match_factory_info() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;

    for device in registry.list_devices() {
        let factory = registry
            .factory_info(&device.driver_type)
            .expect("factory info must exist for universal device");

        assert_eq!(
            normalize(&factory.capabilities),
            normalize_capabilities(&device.capabilities),
            "capability enrichment failed for device '{}' ({})",
            device.id,
            device.driver_type
        );
    }
}

#[tokio::test]
async fn matrix_hybrid_universal_factory_info_present_for_universal_devices() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;

    let universal_devices: Vec<_> = registry
        .list_devices()
        .into_iter()
        .filter(|d| d.driver_type.starts_with("universal_"))
        .collect();

    for device in universal_devices {
        let factory = registry
            .factory_info(&device.driver_type)
            .expect("factory info must exist for universal device in hybrid profile");

        assert!(
            !factory.available_commands.is_empty(),
            "factory command catalog must be populated for device '{}'",
            device.id
        );
    }
}

// =========================================================================
// Manifest feature metadata parity tests (bd-9n9k.4)
// =========================================================================

#[tokio::test]
async fn matrix_universal_manifest_features_populated() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let devices = registry.list_devices();

    for device in &devices {
        assert!(
            !device.metadata.manifest_features.is_empty(),
            "universal device '{}' should have manifest_features populated",
            device.id
        );

        // Check that parameters from TOML manifest flow through
        let param_features: Vec<_> = device
            .metadata
            .manifest_features
            .iter()
            .filter(|f| f.feature_type != "command")
            .collect();
        assert!(
            !param_features.is_empty(),
            "device '{}' should have parameter-type manifest features",
            device.id
        );

        // Check that commands from TOML manifest flow through
        let command_features: Vec<_> = device
            .metadata
            .manifest_features
            .iter()
            .filter(|f| f.feature_type == "command")
            .collect();
        assert!(
            !command_features.is_empty(),
            "device '{}' should have command-type manifest features",
            device.id
        );
    }
}

#[tokio::test]
async fn matrix_universal_parameter_metadata_rich() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let devices = registry.list_devices();
    let device = &devices[0]; // Any ELL14 device

    // The ELL14 manifest defines position_deg with range, unit, and description
    let position = device
        .metadata
        .manifest_features
        .iter()
        .find(|f| f.name == "position_deg");
    assert!(
        position.is_some(),
        "ELL14 should expose position_deg parameter"
    );
    let position = position.expect("position_deg must exist");
    assert_eq!(position.feature_type, "float");
    assert_eq!(position.unit.as_deref(), Some("degrees"));
    assert!(position.min_value.is_some(), "position_deg should have min");
    assert!(position.max_value.is_some(), "position_deg should have max");
    assert!(
        position.description.is_some(),
        "position_deg should have description"
    );
    assert!(position.readable);
    assert!(
        position.writable,
        "position_deg should be writable (read_only not set in manifest)"
    );
}

#[tokio::test]
async fn matrix_universal_command_descriptions_flow_through() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let devices = registry.list_devices();
    let device = &devices[0];

    let move_cmd = device
        .metadata
        .manifest_features
        .iter()
        .find(|f| f.name == "move_absolute" && f.feature_type == "command");
    assert!(
        move_cmd.is_some(),
        "ELL14 should expose move_absolute command as a manifest feature"
    );
    let move_cmd = move_cmd.expect("move_absolute must exist");
    assert!(
        move_cmd.description.is_some(),
        "move_absolute command should have a description"
    );
    assert!(move_cmd.writable, "commands should be writable");
}

#[tokio::test]
async fn matrix_native_devices_no_manifest_features() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;
    let devices = registry.list_devices();

    // Native drivers (mock_camera) should have empty manifest_features
    let camera = devices
        .iter()
        .find(|d| d.driver_type == "mock_camera")
        .expect("hybrid profile should include mock_camera");
    assert!(
        camera.metadata.manifest_features.is_empty(),
        "native driver should have empty manifest_features (uses Parameterized instead)"
    );
}

#[cfg(feature = "db-surreal-mem")]
#[tokio::test]
async fn matrix_db_manifest_features_persisted() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let db = DaqDb::init(DbConfig::in_memory())
        .await
        .expect("in-memory db should initialize");

    let devices = registry.list_devices();
    let device = &devices[0];

    // Simulate what the reconciler does for manifest-driven devices
    let features: Vec<db::config_store::DbDeviceFeature> = device
        .metadata
        .manifest_features
        .iter()
        .map(|mf| db::config_store::DbDeviceFeature {
            device_id: device.id.clone(),
            feature_name: mf.name.clone(),
            feature_type: mf.feature_type.clone(),
            readable: mf.readable,
            writable: mf.writable,
            min_value: mf.min_value,
            max_value: mf.max_value,
            step: None,
            enum_values: Vec::new(),
            unit: mf.unit.clone(),
            description: mf.description.clone(),
            group_name: None,
        })
        .collect();

    assert!(
        !features.is_empty(),
        "should have features to persist for ELL14"
    );

    let count = db
        .upsert_device_features(&features)
        .await
        .expect("feature upsert should succeed");
    assert!(count > 0, "should persist at least one feature");

    // Verify round-trip
    let stored = db
        .get_device_features(&device.id)
        .await
        .expect("feature read should succeed");
    assert_eq!(
        stored.len(),
        features.len(),
        "stored features should match input count"
    );

    // Verify a specific feature round-tripped correctly
    let stored_position = stored.iter().find(|f| f.feature_name == "position_deg");
    assert!(
        stored_position.is_some(),
        "position_deg should survive round-trip"
    );
    let stored_position = stored_position.expect("position_deg must exist in DB");
    assert_eq!(stored_position.feature_type, "float");
    assert_eq!(stored_position.unit.as_deref(), Some("degrees"));
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
