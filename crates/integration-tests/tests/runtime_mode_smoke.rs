//! Runtime-mode smoke tests (bd-9n9k.7).
//!
//! Validates that each runtime profile produces the expected driver
//! classification and metadata.  Mirrors the three CI matrix entries:
//!
//! - **universal-smoke**: all devices come from universal drivers, command
//!   catalogs populated, no deprecated driver types loaded.
//! - **hybrid-db-mem-smoke**: hybrid profile with in-memory DB backend;
//!   validates driver classification, metadata, and DB init/persist.
//! - **hybrid-db-rocksdb-smoke** (feature-gated): RocksDB-backed DB
//!   initialises and survives a roundtrip with data intact.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use hardware::registry::{create_registry_from_config, DeviceRegistry, HardwareConfig};
use tokio::sync::OnceCell;

static MOCK_ELL14_REGISTRY: OnceCell<DeviceRegistry> = OnceCell::const_new();
static MOCK_MAITAI_LAB_REGISTRY: OnceCell<DeviceRegistry> = OnceCell::const_new();

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

fn load_profile(path: &str) -> HardwareConfig {
    HardwareConfig::from_file(&workspace_root().join(path)).expect("profile should parse")
}

async fn build_profile_registry(path: &str) -> DeviceRegistry {
    let config = load_profile(path);
    // create_registry_from_config already calls register_all_factories internally
    create_registry_from_config(&config, Some(&devices_dir()))
        .await
        .expect("registry should build from profile")
}

async fn cached_profile_registry(
    cell: &'static OnceCell<DeviceRegistry>,
    path: &'static str,
) -> &'static DeviceRegistry {
    cell.get_or_init(|| async move { build_profile_registry(path).await })
        .await
}

async fn mock_ell14_registry() -> &'static DeviceRegistry {
    cached_profile_registry(&MOCK_ELL14_REGISTRY, "config/profiles/mock_ell14.toml").await
}

async fn mock_maitai_lab_registry() -> &'static DeviceRegistry {
    cached_profile_registry(
        &MOCK_MAITAI_LAB_REGISTRY,
        "config/profiles/mock_maitai_lab.toml",
    )
    .await
}

// -------------------------------------------------------------------------
// universal-smoke
// -------------------------------------------------------------------------

#[tokio::test]
async fn smoke_universal_only_all_devices_universal() {
    let registry = mock_ell14_registry().await;
    let devices = registry.list_devices();

    assert!(
        !devices.is_empty(),
        "profile should register at least 1 device"
    );
    for device in &devices {
        assert!(
            device.driver_type.starts_with("universal_"),
            "device '{}' should use universal driver, got '{}'",
            device.id,
            device.driver_type
        );
    }
}

#[tokio::test]
async fn smoke_universal_only_command_catalogs_populated() {
    let registry = mock_ell14_registry().await;

    for device in registry.list_devices() {
        assert!(
            !device.metadata.available_commands.is_empty(),
            "device '{}' ({}) should expose available_commands",
            device.id,
            device.driver_type
        );
    }
}

#[tokio::test]
async fn smoke_universal_only_no_deprecated_driver_types() {
    // Deprecated driver types are the legacy serial-crate drivers that have
    // been superseded by universal TOML manifests.
    let deprecated_prefixes = ["ell14", "esp300", "newport_", "spectra_physics", "maitai"];

    let registry = mock_ell14_registry().await;
    for device in registry.list_devices() {
        for prefix in &deprecated_prefixes {
            assert!(
                !device.driver_type.starts_with(prefix),
                "device '{}' uses deprecated driver type '{}' (prefix '{}')",
                device.id,
                device.driver_type,
                prefix
            );
        }
    }
}

// -------------------------------------------------------------------------
// native-exception-smoke
// -------------------------------------------------------------------------

#[tokio::test]
async fn smoke_hybrid_native_camera_present() {
    let registry = mock_maitai_lab_registry().await;
    let devices = registry.list_devices();

    let has_native_camera = devices.iter().any(|d| d.driver_type == "mock_camera");
    assert!(
        has_native_camera,
        "hybrid profile should include native camera driver (mock_camera)"
    );
}

#[tokio::test]
async fn smoke_hybrid_universal_devices_present() {
    let registry = mock_maitai_lab_registry().await;
    let devices = registry.list_devices();

    let universal_count = devices
        .iter()
        .filter(|d| d.driver_type.starts_with("universal_"))
        .count();
    assert!(
        universal_count >= 3,
        "hybrid profile should include at least 3 universal devices, found {universal_count}"
    );
}

#[tokio::test]
async fn smoke_hybrid_driver_classification() {
    let registry = mock_maitai_lab_registry().await;
    let devices = registry.list_devices();

    for device in &devices {
        let is_universal = device.driver_type.starts_with("universal_");
        let is_native_exception = matches!(
            device.driver_type.as_str(),
            "mock_camera" | "pvcam" | "andor_sdk3" | "comedi" | "dover_motion"
        );
        let is_mock = device.driver_type.starts_with("mock_");

        assert!(
            is_universal || is_native_exception || is_mock,
            "device '{}' has unexpected driver type '{}' — should be universal, native exception, or mock",
            device.id,
            device.driver_type
        );
    }
}

// -------------------------------------------------------------------------
// hybrid-db-smoke (requires db-surreal-mem feature)
// -------------------------------------------------------------------------

#[cfg(feature = "db-surreal-mem")]
mod db_smoke {
    use super::*;
    use db::config_store::{toml_to_json, DbDriver, DbInstrument};
    use db::{DaqDb, DbConfig};
    use std::collections::HashSet;

    #[tokio::test]
    async fn smoke_db_init_and_driver_persist() {
        let db = DaqDb::init(DbConfig::in_memory())
            .await
            .expect("in-memory DB should initialize");

        let config = load_profile("config/profiles/mock_ell14.toml");
        let registry = mock_ell14_registry().await;

        let mut seen = HashSet::new();
        let mut drivers = Vec::new();
        for device in &config.devices {
            if !seen.insert(device.driver.driver_type.clone()) {
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
            .map(|d| DbInstrument {
                device_id: d.id.clone(),
                name: d.name.clone(),
                driver_type: d.driver.driver_type.clone(),
                config: toml_to_json(&d.driver.config),
                enabled: d.enabled,
            })
            .collect();

        db.upsert_drivers(&drivers).await.expect("driver upsert");
        let report = db
            .upsert_instruments(&instruments)
            .await
            .expect("instrument upsert");
        assert_eq!(report.instruments_upserted, instruments.len());

        // Verify data persisted
        let stored_drivers = db.get_all_drivers().await.expect("driver read");
        assert!(
            !stored_drivers.is_empty(),
            "drivers should be persisted in DB"
        );

        let stored_instruments = db.get_all_instruments().await.expect("instrument read");
        assert_eq!(
            stored_instruments.len(),
            instruments.len(),
            "all instruments should be persisted"
        );
    }

    #[tokio::test]
    async fn smoke_db_config_service_healthy() {
        let db = DaqDb::init(DbConfig::in_memory())
            .await
            .expect("in-memory DB should initialize");
        let info = db.info().await;
        assert!(info.healthy, "DB should report healthy after init");
    }
}

// -------------------------------------------------------------------------
// hybrid-db-rocksdb-smoke (requires db-surreal-rocksdb feature)
// -------------------------------------------------------------------------

#[cfg(feature = "db-surreal-rocksdb")]
mod db_rocksdb_smoke {
    use db::config_store::{DbDriver, DbInstrument};
    use db::{DaqDb, DbConfig};

    /// Validates RocksDB engine initialises, reports healthy, and can
    /// round-trip driver + instrument records.
    ///
    /// NOTE: SurrealDB's embedded RocksDB engine holds a process-level
    /// file lock that is not released on `DaqDb` drop, so in-process
    /// reopen tests are not possible.  Cross-process restart persistence
    /// is an inherent property of the RocksDB engine and is validated by
    /// the engine's own test suite.
    #[tokio::test]
    async fn smoke_rocksdb_init_and_roundtrip() {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let db_path = tmpdir.path().join("test.db");

        let db = DaqDb::init(DbConfig::rocksdb(&db_path))
            .await
            .expect("RocksDB init");

        // Verify engine reports correctly
        let info = db.info().await;
        assert!(info.healthy, "DB should be healthy after init");
        assert!(
            info.engine.starts_with("rocksdb"),
            "engine should be rocksdb, got: {}",
            info.engine
        );

        // Write driver + instrument records
        let drivers = vec![DbDriver {
            driver_type: "universal_thorlabs_ell14".into(),
            name: "Thorlabs ELL14".into(),
            capabilities: vec!["movable".into()],
            commands: vec!["move_absolute".into(), "get_position".into()],
        }];
        db.upsert_drivers(&drivers).await.expect("driver upsert");

        let instruments = vec![DbInstrument {
            device_id: "test_rotator".into(),
            name: "Test Rotator".into(),
            driver_type: "universal_thorlabs_ell14".into(),
            config: serde_json::json!({"port": "/dev/ttyUSB0", "address": "2"}),
            enabled: true,
        }];
        db.upsert_instruments(&instruments)
            .await
            .expect("instrument upsert");

        // Read back and verify
        let stored_drivers = db.get_all_drivers().await.expect("driver read");
        assert_eq!(stored_drivers.len(), 1, "should have 1 driver");
        assert_eq!(stored_drivers[0].driver_type, "universal_thorlabs_ell14");
        assert_eq!(stored_drivers[0].commands.len(), 2);

        let stored_instruments = db.get_all_instruments().await.expect("instrument read");
        assert_eq!(stored_instruments.len(), 1, "should have 1 instrument");
        assert_eq!(stored_instruments[0].device_id, "test_rotator");
        assert!(stored_instruments[0].enabled);

        // Verify schema version and counts via info
        let info = db.info().await;
        assert!(info.driver_count >= 1, "should report ≥1 driver");
        assert!(info.instrument_count >= 1, "should report ≥1 instrument");
    }
}
