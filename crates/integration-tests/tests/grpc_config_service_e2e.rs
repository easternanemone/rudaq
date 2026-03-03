//! Integration tests for ConfigService gRPC surface (bd-zyc8).
//!
//! Extracted from the surrealdb_daemon_e2e monolith. Focuses on gRPC wire path
//! for hardware configuration CRUD, driver discovery, and metadata enrichment.
//!
//! Run with: cargo nextest run -p integration-tests --features db-surreal-mem --test grpc_config_service_e2e

#![cfg(all(feature = "db-surreal-mem", feature = "server"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unused_imports,
    missing_docs
)]

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_stream::StreamExt;
use tonic::Request;

use db::config_store::{config_hash, toml_to_json, DbDriver, DbInstrument};
use db::{DaqDb, DbConfig};
use hardware::registry::{DeviceRegistry, HardwareConfig};
use server::grpc::config_service::ConfigServiceImpl;
use server::grpc::{
    ConfigService, DeleteInstrumentRequest, ExportConfigRequest, GetDbInfoRequest,
    GetInstrumentRequest, ImportConfigRequest, InstrumentConfig, ListDriversRequest,
    ListInstrumentsRequest, SubscribeConfigRequest, UpsertInstrumentRequest,
};

// ---------------------------------------------------------------------------
// Helpers (mirrors surrealdb_daemon_e2e until extracted to common)
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn mock_maitai_lab_path() -> PathBuf {
    workspace_root().join("config/profiles/mock_maitai_lab.toml")
}

#[allow(dead_code)]
fn config_devices_dir() -> PathBuf {
    workspace_root().join("config/devices")
}

fn load_mock_maitai_config() -> HardwareConfig {
    HardwareConfig::from_file(&mock_maitai_lab_path()).expect("mock_maitai_lab.toml should parse")
}

fn devices_to_db(config: &HardwareConfig) -> Vec<DbInstrument> {
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

fn drivers_from_config(config: &HardwareConfig) -> Vec<DbDriver> {
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

async fn shadow_write(db: &DaqDb, config: &HardwareConfig) -> Result<(), db::error::DbError> {
    db.upsert_drivers(&drivers_from_config(config)).await?;
    db.upsert_instruments(&devices_to_db(config)).await?;
    Ok(())
}

async fn setup_config_service() -> (ConfigServiceImpl, DaqDb) {
    let hw_config = load_mock_maitai_config();
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
    shadow_write(&db, &hw_config).await.unwrap();
    let svc = ConfigServiceImpl::new(db.clone(), None);
    (svc, db)
}

// ---------------------------------------------------------------------------
// ConfigService CRUD Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_instruments_returns_9() {
    let (svc, _db) = setup_config_service().await;

    let resp = svc
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    let instruments = resp.into_inner().instruments;
    assert_eq!(instruments.len(), 9);
}

#[tokio::test]
async fn test_get_instrument_rotator2() {
    let (svc, _db) = setup_config_service().await;

    let resp = svc
        .get_instrument(Request::new(GetInstrumentRequest {
            device_id: "rotator_2".into(),
        }))
        .await
        .unwrap();
    let inst = resp.into_inner();
    assert_eq!(inst.device_id, "rotator_2");
    assert_eq!(inst.driver_type, "universal_thorlabs_ell14");
    assert!(inst.enabled);

    let config: serde_json::Value = serde_json::from_str(&inst.config_json).unwrap();
    assert_eq!(config.get("mock"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(
        config.get("address"),
        Some(&serde_json::Value::String("2".into()))
    );
}

#[tokio::test]
async fn test_upsert_new_device() {
    let (svc, _db) = setup_config_service().await;

    let resp = svc
        .upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "new_rotator".into(),
                name: "New ELL14 Rotator".into(),
                driver_type: "universal_thorlabs_ell14".into(),
                config_json: r#"{"mock":true,"address":"9"}"#.into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();
    assert!(resp.into_inner().success);

    let resp = svc
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    assert_eq!(resp.into_inner().instruments.len(), 10);
}

#[tokio::test]
async fn test_delete_device() {
    let (svc, _db) = setup_config_service().await;

    let resp = svc
        .delete_instrument(Request::new(DeleteInstrumentRequest {
            device_id: "rotator_8".into(),
        }))
        .await
        .unwrap();
    assert!(resp.into_inner().success);

    let resp = svc
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    assert_eq!(resp.into_inner().instruments.len(), 8);
}

#[tokio::test]
async fn test_list_drivers_returns_5() {
    let (svc, _db) = setup_config_service().await;

    let resp = svc
        .list_drivers(Request::new(ListDriversRequest {}))
        .await
        .unwrap();
    let drivers = resp.into_inner().drivers;
    assert_eq!(drivers.len(), 5);
}

#[tokio::test]
async fn test_import_export_roundtrip() {
    let (svc, _db) = setup_config_service().await;

    let export_resp = svc
        .export_config(Request::new(ExportConfigRequest {}))
        .await
        .unwrap();
    let toml_str = export_resp.into_inner().toml_content;
    assert!(!toml_str.is_empty());
    assert!(toml_str.contains("rotator_2"));

    let db2 = DaqDb::init(DbConfig::in_memory()).await.unwrap();
    let svc2 = ConfigServiceImpl::new(db2, None);

    let import_resp = svc2
        .import_config(Request::new(ImportConfigRequest {
            toml_content: toml_str,
        }))
        .await
        .unwrap();
    assert_eq!(import_resp.into_inner().instruments_imported, 9);
}

#[tokio::test]
async fn test_get_db_info() {
    let (svc, _db) = setup_config_service().await;

    let resp = svc
        .get_db_info(Request::new(GetDbInfoRequest {}))
        .await
        .unwrap();
    let info = resp.into_inner();
    assert_eq!(info.instrument_count, 9);
    assert_eq!(info.driver_count, 5);
}

#[tokio::test]
async fn test_subscribe_config_changes() {
    let (svc, db) = setup_config_service().await;

    let resp = svc
        .subscribe_config_changes(Request::new(SubscribeConfigRequest {}))
        .await
        .unwrap();
    let mut stream = resp.into_inner();

    let svc2 = ConfigServiceImpl::new(db.clone(), None);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        svc2.upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "sub_test_device".into(),
                name: "Subscription Test".into(),
                driver_type: "mock_power_meter".into(),
                config_json: "{}".into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();
    });

    let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timeout")
        .expect("stream empty")
        .expect("gRPC error");
    assert!(event.change_type.contains("upsert") || !event.device_id.is_empty());
}

// ---------------------------------------------------------------------------
// Stress Tests (Concurrent Operations)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_concurrent_upserts_converge() {
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

    let mut handles = vec![];
    let driver_types = [
        "mock_power_meter",
        "mock_camera",
        "mock_stage",
        "mock_power_meter",
        "mock_camera",
    ];

    for i in 0..30 {
        let svc = ConfigServiceImpl::new(db.clone(), None);
        let driver_type = driver_types[i % driver_types.len()].to_string();
        handles.push(tokio::spawn(async move {
            svc.upsert_instrument(Request::new(UpsertInstrumentRequest {
                instrument: Some(InstrumentConfig {
                    device_id: format!("concurrent_{i}"),
                    name: format!("Concurrent Device {i}"),
                    driver_type,
                    config_json: format!(r#"{{"index":{i}}}"#),
                    enabled: true,
                }),
            }))
            .await
        }));
    }

    let results = futures::future::join_all(handles).await;
    for res in results {
        assert!(res.unwrap().is_ok());
    }

    let instruments = db.get_all_instruments().await.unwrap();
    assert_eq!(instruments.len(), 30);
}
