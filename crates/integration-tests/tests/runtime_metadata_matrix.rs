//! Runtime metadata integration matrix.
//!
//! Covers representative launch profiles across:
//! - no-db + universal-only
//! - no-db + hybrid (universal + camera-native)
//! - db-on + hybrid metadata parity

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use driver_registry::{create_registry_from_config, register_all_factories};
use hardware::registry::HardwareConfig;

#[cfg(feature = "db")]
use db::config_store::{DbDriver, DbInstrument, toml_to_json};
#[cfg(feature = "db")]
use db::{DaqDb, DbConfig};
#[cfg(feature = "db")]
use std::collections::HashMap;
#[cfg(feature = "db")]
use std::collections::HashSet;
#[cfg(feature = "db")]
use tempfile::TempDir;

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
async fn matrix_universal_parameterized_runtime_parity() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;

    for device in registry.list_devices() {
        let parameterized = registry
            .get_parameterized(&device.id)
            .expect("universal ELL14 profile device should expose Parameterized");
        let names = parameterized.parameters().names();
        assert!(
            !names.is_empty(),
            "device '{}' should expose runtime parameters",
            device.id
        );
        assert!(
            device
                .capabilities
                .iter()
                .any(|c| c.as_str() == "parameterized"),
            "device '{}' should advertise Parameterized capability",
            device.id
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

#[cfg(feature = "db")]
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
            device_id: device.id.to_string(),
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

// =========================================================================
// Cross-profile universal parity assertions (bd-lncj.3.3)
// =========================================================================

#[tokio::test]
async fn matrix_universal_panel_kind_populated() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;

    let universal_devices: Vec<_> = registry
        .list_devices()
        .into_iter()
        .filter(|d| d.driver_type.starts_with("universal_"))
        .collect();
    assert!(
        !universal_devices.is_empty(),
        "expected at least one universal_* device in mock_maitai_lab profile"
    );

    for device in &universal_devices {
        assert!(
            device.metadata.panel_kind.is_some(),
            "universal device '{}' ({}) should have panel_kind from manifest [ui]",
            device.id,
            device.driver_type
        );
    }
}

#[tokio::test]
async fn matrix_universal_category_set() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;

    let universal_devices: Vec<_> = registry
        .list_devices()
        .into_iter()
        .filter(|d| d.driver_type.starts_with("universal_"))
        .collect();
    assert!(
        !universal_devices.is_empty(),
        "expected at least one universal_* device in mock_maitai_lab profile"
    );

    for device in &universal_devices {
        assert!(
            device.metadata.category.is_some(),
            "universal device '{}' ({}) should have category",
            device.id,
            device.driver_type
        );
    }
}

#[tokio::test]
async fn matrix_same_driver_type_yields_identical_capabilities() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;
    let devices = registry.list_devices();

    let mut by_driver: std::collections::HashMap<String, Vec<Vec<String>>> =
        std::collections::HashMap::new();
    for d in &devices {
        let caps = normalize_capabilities(&d.capabilities);
        by_driver
            .entry(d.driver_type.clone())
            .or_default()
            .push(caps);
    }
    for (driver, instances) in &by_driver {
        if instances.len() > 1 {
            for (i, caps) in instances.iter().enumerate().skip(1) {
                assert_eq!(
                    &instances[0], caps,
                    "driver '{driver}' instance 0 vs {i} has capability mismatch"
                );
            }
        }
    }
}

#[tokio::test]
async fn matrix_same_driver_type_yields_identical_commands() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;
    let devices = registry.list_devices();

    let mut by_driver: std::collections::HashMap<String, Vec<Vec<String>>> =
        std::collections::HashMap::new();
    for d in &devices {
        let cmds = normalize(&d.metadata.available_commands);
        by_driver
            .entry(d.driver_type.clone())
            .or_default()
            .push(cmds);
    }
    for (driver, instances) in &by_driver {
        if instances.len() > 1 {
            for (i, cmds) in instances.iter().enumerate().skip(1) {
                assert_eq!(
                    &instances[0], cmds,
                    "driver '{driver}' instance 0 vs {i} has command catalog mismatch"
                );
            }
        }
    }
}

#[tokio::test]
async fn matrix_cross_profile_driver_type_parity() {
    let ell14_registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let maitai_registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;

    let ell14_device = ell14_registry.list_devices().into_iter().next().unwrap();
    let maitai_ell14 = maitai_registry
        .list_devices()
        .into_iter()
        .find(|d| d.driver_type == ell14_device.driver_type)
        .expect("same driver type should exist across compared profiles");

    assert_eq!(
        normalize_capabilities(&ell14_device.capabilities),
        normalize_capabilities(&maitai_ell14.capabilities),
        "same driver type across profiles must expose identical capabilities"
    );
    assert_eq!(
        normalize(&ell14_device.metadata.available_commands),
        normalize(&maitai_ell14.metadata.available_commands),
        "same driver type across profiles must expose identical command catalog"
    );
    assert_eq!(
        ell14_device.metadata.panel_kind, maitai_ell14.metadata.panel_kind,
        "same driver type across profiles must expose identical panel_kind"
    );
}

#[tokio::test]
async fn matrix_universal_parameterized_parity_across_profiles() {
    let ell14_registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let maitai_registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;

    let ell14_device = ell14_registry.list_devices().into_iter().next().unwrap();

    let ell14_params = ell14_registry
        .get_parameterized(&ell14_device.id)
        .expect("ELL14 device should be Parameterized");
    let ell14_names: Vec<String> = {
        let mut n: Vec<String> = ell14_params
            .parameters()
            .names()
            .into_iter()
            .map(String::from)
            .collect();
        n.sort();
        n
    };

    let maitai_ell14 = maitai_registry
        .list_devices()
        .into_iter()
        .find(|d| d.driver_type == ell14_device.driver_type)
        .expect("same driver type should exist across compared profiles");
    let maitai_params = maitai_registry
        .get_parameterized(&maitai_ell14.id)
        .expect("same driver type in other profile should also be Parameterized");
    let maitai_names: Vec<String> = {
        let mut n: Vec<String> = maitai_params
            .parameters()
            .names()
            .into_iter()
            .map(String::from)
            .collect();
        n.sort();
        n
    };
    assert_eq!(
        ell14_names, maitai_names,
        "Parameterized names must be identical for same driver type across profiles"
    );
}

#[tokio::test]
async fn matrix_universal_manifest_features_match_parameterized() {
    let registry = create_profile_registry("config/profiles/mock_ell14.toml").await;
    let devices = registry.list_devices();
    let device = &devices[0];

    let param_feature_names: Vec<String> = {
        let mut names: Vec<_> = device
            .metadata
            .manifest_features
            .iter()
            .filter(|f| f.feature_type != "command")
            .map(|f| f.name.clone())
            .collect();
        names.sort();
        names
    };

    let parameterized = registry
        .get_parameterized(&device.id)
        .expect("device should be Parameterized");
    let runtime_names: Vec<String> = {
        let mut n: Vec<String> = parameterized
            .parameters()
            .names()
            .into_iter()
            .map(String::from)
            .collect();
        n.sort();
        n
    };

    assert_eq!(
        param_feature_names, runtime_names,
        "manifest feature parameter names must match runtime Parameterized names"
    );
}

#[tokio::test]
async fn matrix_native_camera_has_no_manifest_features() {
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;
    let camera = registry
        .list_devices()
        .into_iter()
        .find(|d| d.driver_type == "mock_camera")
        .expect("hybrid profile should include mock_camera");

    assert!(
        camera.metadata.manifest_features.is_empty(),
        "native camera should not have manifest-derived features"
    );
    assert!(
        camera.metadata.ui_schema_json.is_none(),
        "native camera should not have universal UI schema"
    );
}

#[tokio::test]
async fn matrix_camera_native_exception_driver_type_contract() {
    let config = load_profile("config/profiles/mock_maitai_lab.toml");
    let camera = config
        .devices
        .iter()
        .find(|d| d.driver.driver_type == "mock_camera")
        .expect("mock_maitai_lab profile should include camera device");
    assert_eq!(
        camera.driver.driver_type, "mock_camera",
        "camera native exception should remain on mock_camera, not universal_*"
    );
}

#[tokio::test]
async fn matrix_comedi_and_dover_driver_types_remain_native_exceptions() {
    let config = load_profile("config/maitai_universal.toml");
    let native_driver_types: Vec<&str> = config
        .devices
        .iter()
        .map(|d| d.driver.driver_type.as_str())
        .filter(|t| t.starts_with("comedi_") || *t == "dover_axis")
        .collect();

    assert!(
        !native_driver_types.is_empty(),
        "maitai_universal profile should include native DAQ/motion exception drivers"
    );
    assert!(
        native_driver_types
            .iter()
            .all(|t| !t.starts_with("universal_")),
        "comedi/dover driver types must remain native exceptions, found: {native_driver_types:?}"
    );
}

#[cfg(feature = "db")]
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
            device_id: device.id.to_string(),
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

// =============================================================================
// SpectrumReadable (1D detector) parity tests (bd-lncj.1.2 / bd-lncj.2.2)
// =============================================================================

#[tokio::test]
async fn matrix_universal_spectrometer_has_spectrum_readable() {
    let registry = create_profile_registry("config/profiles/mock_spectrometer.toml").await;
    let devices = registry.list_devices();
    let spec = devices
        .iter()
        .find(|d| d.id == "spectrometer")
        .expect("mock_spectrometer profile should contain 'spectrometer' device");

    let cap_strs: Vec<&str> = spec.capabilities.iter().map(|c| c.as_str()).collect();
    assert!(
        cap_strs.contains(&"spectrum_readable"),
        "spectrometer should advertise SpectrumReadable capability, got: {cap_strs:?}"
    );
    assert!(
        cap_strs.contains(&"readable"),
        "spectrometer should also advertise Readable capability"
    );
}

#[tokio::test]
async fn matrix_universal_spectrometer_read_spectrum_returns_data() {
    let registry = create_profile_registry("config/profiles/mock_spectrometer.toml").await;
    let spec_readable = registry
        .get_spectrum_readable("spectrometer")
        .expect("spectrometer should provide SpectrumReadable");

    let data = spec_readable
        .read_spectrum()
        .await
        .expect("read_spectrum should succeed");
    assert!(
        !data.values.is_empty(),
        "spectrum data should contain at least one value"
    );
    assert_eq!(data.value_units, "counts");
    assert_eq!(data.axis_units.as_deref(), Some("nm"));
}

#[tokio::test]
async fn matrix_universal_spectrometer_spectrum_length() {
    let registry = create_profile_registry("config/profiles/mock_spectrometer.toml").await;
    let spec_readable = registry
        .get_spectrum_readable("spectrometer")
        .expect("spectrometer should provide SpectrumReadable");

    assert_eq!(
        spec_readable.spectrum_length(),
        1024,
        "spectrum_length should match manifest config"
    );
}

#[tokio::test]
async fn matrix_universal_spectrometer_category_is_detector() {
    let registry = create_profile_registry("config/profiles/mock_spectrometer.toml").await;
    let devices = registry.list_devices();
    let spec = devices
        .iter()
        .find(|d| d.id == "spectrometer")
        .expect("spectrometer should exist");

    assert_eq!(
        spec.metadata.category,
        Some(common::capabilities::DeviceCategory::Detector),
        "spectrometer category should be Detector"
    );
    assert_eq!(
        spec.metadata.panel_kind.as_deref(),
        Some("detector"),
        "spectrometer panel_kind should be 'detector'"
    );
}

// =============================================================================
// File-backed DB persistence: metadata survives "restart" (bd-9n9k.4)
// =============================================================================

/// Verifies that driver metadata, instruments, and device features survive a
/// file-backed DB close + reopen cycle, simulating a daemon restart.
#[cfg(feature = "db")]
#[tokio::test]
async fn matrix_db_file_persistence_survives_restart() {
    let tmp = TempDir::new().expect("temp dir");
    let db_path = tmp.path().join("restart_test.db");
    let db_path_str = db_path.display().to_string();

    let config = load_profile("config/profiles/mock_maitai_lab.toml");
    let registry = create_profile_registry("config/profiles/mock_maitai_lab.toml").await;

    // --- Phase 1: Write metadata to file-backed DB ---

    let mut seen_driver_types = HashSet::new();
    let mut drivers = Vec::<DbDriver>::new();
    for device in &config.devices {
        if !seen_driver_types.insert(device.driver.driver_type.clone()) {
            continue;
        }
        let factory = registry
            .factory_info(&device.driver.driver_type)
            .expect("factory should exist");
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
            device_id: device.id.to_string(),
            name: device.name.clone(),
            driver_type: device.driver.driver_type.clone(),
            config: toml_to_json(&device.driver.config),
            enabled: device.enabled,
        })
        .collect();

    let expected_driver_count = drivers.len() as u64;
    let expected_instrument_count = instruments.len() as u64;

    // Also persist device features for the first universal device
    let first_universal = registry
        .list_devices()
        .into_iter()
        .find(|d| !d.metadata.manifest_features.is_empty());
    let expected_features: Vec<db::config_store::DbDeviceFeature> =
        if let Some(ref device) = first_universal {
            device
                .metadata
                .manifest_features
                .iter()
                .map(|mf| db::config_store::DbDeviceFeature {
                    device_id: device.id.to_string(),
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
                .collect()
        } else {
            Vec::new()
        };

    {
        let db = DaqDb::init(DbConfig::file(&db_path_str))
            .await
            .expect("file DB init");

        db.upsert_drivers(&drivers).await.expect("upsert drivers");
        db.upsert_instruments(&instruments)
            .await
            .expect("upsert instruments");
        if !expected_features.is_empty() {
            db.upsert_device_features(&expected_features)
                .await
                .expect("upsert features");
        }

        // Verify data is present before close
        let info = db.info().await.expect("info");
        assert_eq!(info.driver_count, expected_driver_count);
        assert_eq!(info.instrument_count, expected_instrument_count);
        // DB drops here, closing the connection
    }

    // --- Phase 2: Reopen DB and verify metadata survived ---

    let db2 = DaqDb::init(DbConfig::file(&db_path_str))
        .await
        .expect("file DB re-init should succeed");

    let info2 = db2.info().await.expect("info after reopen");
    assert_eq!(
        info2.driver_count, expected_driver_count,
        "driver count should survive restart"
    );
    assert_eq!(
        info2.instrument_count, expected_instrument_count,
        "instrument count should survive restart"
    );

    // Verify specific driver metadata round-tripped
    let stored_drivers: HashMap<String, DbDriver> = db2
        .get_all_drivers()
        .await
        .expect("drivers after reopen")
        .into_iter()
        .map(|d| (d.driver_type.clone(), d))
        .collect();

    for expected in &drivers {
        let actual = stored_drivers
            .get(&expected.driver_type)
            .unwrap_or_else(|| panic!("driver '{}' should survive restart", expected.driver_type));
        assert_eq!(
            normalize(&actual.capabilities),
            normalize(&expected.capabilities),
            "capabilities should survive restart for '{}'",
            expected.driver_type
        );
        assert_eq!(
            normalize(&actual.commands),
            normalize(&expected.commands),
            "commands should survive restart for '{}'",
            expected.driver_type
        );
    }

    // Verify device features round-tripped
    if let Some(ref device) = first_universal {
        let stored_features = db2
            .get_device_features(&device.id)
            .await
            .expect("features after reopen");
        assert_eq!(
            stored_features.len(),
            expected_features.len(),
            "feature count should survive restart for '{}'",
            device.id
        );
    }
}
