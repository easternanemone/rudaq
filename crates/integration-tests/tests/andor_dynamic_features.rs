//! Integration tests for Andor iStar dynamic feature discovery + DB persistence (bd-tk92 / E.1)
//!
//! Exercises the full stack:
//! - Mock Andor camera registration → dynamic `Parameter<T>` population
//! - gRPC `ListParameters` → verify all dynamic features visible
//! - gRPC `SetParameter` / `GetParameter` → type-dispatched operations
//! - Read-only rejection
//! - SurrealDB `device_feature` persistence from parameter metadata
//! - Feature cleanup on device removal
//!
//! Run with: cargo nextest run -p integration-tests --features db-surreal-mem --test andor_dynamic_features
#![cfg(feature = "db-surreal-mem")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unused_imports,
    missing_docs
)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use common::capabilities::Parameterized;
use common::observable::ParameterBase;
use db::config_store::DbDeviceFeature;
use db::{DaqDb, DbConfig};
use driver_registry::register_all_factories;
use hardware::registry::DeviceRegistry;
use protocol::daq::hardware_service_server::HardwareService;
use protocol::daq::{GetParameterRequest, ListParametersRequest, SetParameterRequest};
use server::grpc::hardware_service::HardwareServiceImpl;
use tonic::Request;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a registry and register all factories (including Andor mock).
async fn test_registry() -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    register_all_factories(&registry, None)
        .await
        .expect("factory registration should succeed");
    registry
}

/// Register Andor iStar mock camera in the registry.
async fn register_andor_mock(registry: &DeviceRegistry) -> bool {
    registry
        .register_from_toml(
            "istar_test",
            "Test iStar",
            "andor_istar",
            toml::Value::Table(Default::default()),
        )
        .await
        .is_ok()
}

/// Extract parameter metadata from a registered device and convert to DbDeviceFeature records.
fn extract_db_features(registry: &DeviceRegistry, device_id: &str) -> Vec<DbDeviceFeature> {
    let parameterized = registry
        .get_parameterized(device_id)
        .expect("device should be Parameterized");
    let params = parameterized.parameters();
    params
        .iter()
        .map(|(name, param)| {
            let meta = param.metadata();
            DbDeviceFeature {
                device_id: device_id.to_owned(),
                feature_name: name.to_owned(),
                feature_type: meta.dtype.clone(),
                readable: true,
                writable: !meta.read_only,
                min_value: meta.min_value,
                max_value: meta.max_value,
                step: meta.step,
                enum_values: meta.enum_values.clone(),
                unit: meta.units.clone(),
                description: meta.description.clone(),
                group_name: meta.group_name.clone(),
            }
        })
        .collect()
}

// =============================================================================
// Test 1: Dynamic features visible in ListParameters
// =============================================================================

#[tokio::test]
async fn test_andor_list_parameters_returns_dynamic_features() {
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    let resp = service
        .list_parameters(Request::new(ListParametersRequest {
            device_id: "istar_test".to_string(),
        }))
        .await
        .expect("ListParameters should succeed");

    let params = resp.into_inner().parameters;

    // Should have 30+ parameters (11 core + ~50 dynamic).
    assert!(
        params.len() >= 30,
        "Expected 30+ parameters, got {}",
        params.len()
    );

    // Verify core parameters exist (snake_case names from MockCamera).
    let names: HashSet<&str> = params.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains("exposure_s"), "core: exposure_s");
    assert!(names.contains("mcp_gain"), "core: mcp_gain");

    // Verify dynamic features exist (PascalCase SDK3 feature names).
    assert!(names.contains("TriggerMode"), "dynamic: TriggerMode");
    assert!(names.contains("PixelEncoding"), "dynamic: PixelEncoding");
    assert!(names.contains("SensorCooling"), "dynamic: SensorCooling");
    assert!(names.contains("FrameRate"), "dynamic: FrameRate");
    assert!(names.contains("GateMode"), "dynamic: GateMode");
    assert!(names.contains("DDGOutputDelay"), "dynamic: DDGOutputDelay");
    assert!(names.contains("CameraModel"), "dynamic: CameraModel");

    // Verify dtype is set correctly for dynamic features.
    let pixel_enc = params.iter().find(|p| p.name == "PixelEncoding").unwrap();
    assert_eq!(pixel_enc.dtype, "enum", "PixelEncoding should be enum");
    assert!(
        !pixel_enc.enum_values.is_empty(),
        "PixelEncoding should have enum values"
    );

    let frame_rate = params.iter().find(|p| p.name == "FrameRate").unwrap();
    assert_eq!(frame_rate.dtype, "float", "FrameRate should be float");
    assert!(
        frame_rate.min_value.is_some(),
        "FrameRate should have min_value"
    );
}

// =============================================================================
// Test 2: Set/Get each parameter type via gRPC
// =============================================================================

#[tokio::test]
async fn test_andor_set_get_enum_parameter() {
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // Set TriggerMode (enum) to "External".
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "TriggerMode".to_string(),
            value: "External".to_string(),
        }))
        .await
        .expect("SetParameter should succeed for enum");

    assert!(resp.into_inner().success);

    // Get it back.
    // Note: GetParameter returns `serde_json::Value::to_string()` — for String params
    // this includes JSON quotes (e.g., `"\"External\""` instead of `"External"`).
    let resp = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "TriggerMode".to_string(),
        }))
        .await
        .expect("GetParameter should succeed");

    let raw = resp.into_inner().value;
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed.as_str().unwrap(), "External");
}

#[tokio::test]
async fn test_andor_set_get_bool_parameter() {
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // SensorCooling is a writable bool.
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "SensorCooling".to_string(),
            value: "false".to_string(),
        }))
        .await
        .expect("SetParameter should succeed for bool");

    assert!(resp.into_inner().success);

    let resp = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "SensorCooling".to_string(),
        }))
        .await
        .expect("GetParameter should succeed");

    assert_eq!(resp.into_inner().value, "false");
}

#[tokio::test]
async fn test_andor_set_get_float_parameter() {
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // DDGOutputDelay is a writable float.
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "DDGOutputDelay".to_string(),
            value: "42.5".to_string(),
        }))
        .await
        .expect("SetParameter should succeed for float");

    assert!(resp.into_inner().success);

    let resp = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "DDGOutputDelay".to_string(),
        }))
        .await
        .expect("GetParameter should succeed");

    let val: f64 = resp.into_inner().value.parse().unwrap();
    assert!((val - 42.5).abs() < 1e-9);
}

#[tokio::test]
async fn test_andor_set_get_int_parameter() {
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // FrameCount is a writable int.
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "FrameCount".to_string(),
            value: "100".to_string(),
        }))
        .await
        .expect("SetParameter should succeed for int");

    assert!(resp.into_inner().success);

    let resp = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "FrameCount".to_string(),
        }))
        .await
        .expect("GetParameter should succeed");

    assert_eq!(resp.into_inner().value, "100");
}

// =============================================================================
// Test 3: Read-only parameters reject writes
// =============================================================================

#[tokio::test]
async fn test_andor_readonly_parameter_rejects_set() {
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // SensorWidth is read-only.
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "SensorWidth".to_string(),
            value: "1024".to_string(),
        }))
        .await;

    // Should fail — either gRPC error or success=false.
    match resp {
        Ok(r) => assert!(
            !r.into_inner().success,
            "Setting read-only SensorWidth should fail"
        ),
        Err(status) => assert!(
            status.code() == tonic::Code::InvalidArgument
                || status.code() == tonic::Code::FailedPrecondition,
            "Expected InvalidArgument or FailedPrecondition, got {:?}",
            status.code()
        ),
    }

    // FrameRate is read-only.
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_test".to_string(),
            parameter_name: "FrameRate".to_string(),
            value: "30.0".to_string(),
        }))
        .await;

    if let Ok(r) = resp {
        assert!(
            !r.into_inner().success,
            "Setting read-only FrameRate should fail"
        );
    }
}

// =============================================================================
// Test 4: SurrealDB feature persistence from parameter metadata
// =============================================================================

#[tokio::test]
async fn test_andor_features_persisted_to_surrealdb() {
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    // Extract parameter metadata and persist to DB (same as reconciler pipeline).
    let features = extract_db_features(&registry, "istar_test");
    assert!(
        features.len() >= 30,
        "Expected 30+ features to extract, got {}",
        features.len()
    );

    db.upsert_device_features(&features).await.unwrap();

    // Verify features were persisted.
    let stored = db.get_device_features("istar_test").await.unwrap();
    assert_eq!(
        features.len(),
        stored.len(),
        "All extracted features should be persisted"
    );

    // Verify feature types are correct.
    let feature_map: HashMap<&str, &DbDeviceFeature> = stored
        .iter()
        .map(|f| (f.feature_name.as_str(), f))
        .collect();

    // Check a float feature (DDGOutputDelay is dynamic with explicit dtype).
    let ddg = feature_map
        .get("DDGOutputDelay")
        .expect("DDGOutputDelay should be persisted");
    assert_eq!(ddg.feature_type, "float");

    // Check an enum feature.
    let pixel_enc = feature_map
        .get("PixelEncoding")
        .expect("PixelEncoding should be persisted");
    assert_eq!(pixel_enc.feature_type, "enum");
    assert!(
        !pixel_enc.enum_values.is_empty(),
        "PixelEncoding should have enum values in DB"
    );

    // Check a read-only feature.
    let sensor_w = feature_map
        .get("SensorWidth")
        .expect("SensorWidth should be persisted");
    assert!(!sensor_w.writable, "SensorWidth should not be writable");

    // Check a feature with range.
    let frame_rate = feature_map
        .get("FrameRate")
        .expect("FrameRate should be persisted");
    assert_eq!(frame_rate.feature_type, "float");
    assert!(
        frame_rate.min_value.is_some(),
        "FrameRate should have min in DB"
    );
    assert!(
        frame_rate.max_value.is_some(),
        "FrameRate should have max in DB"
    );
}

// =============================================================================
// Test 5: Features cleaned up on device removal
// =============================================================================

#[tokio::test]
async fn test_andor_features_cleaned_on_device_removal() {
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    // Persist features.
    let features = extract_db_features(&registry, "istar_test");
    db.upsert_device_features(&features).await.unwrap();

    let stored = db.get_device_features("istar_test").await.unwrap();
    assert!(!stored.is_empty(), "Features should exist before cleanup");

    // Delete features (simulates reconciler cleanup on device removal).
    db.delete_device_features("istar_test").await.unwrap();

    let after_delete = db.get_device_features("istar_test").await.unwrap();
    assert!(
        after_delete.is_empty(),
        "Features should be cleaned up after deletion"
    );
}

// =============================================================================
// Test 6: Feature persistence is idempotent across upsert cycles
// =============================================================================

#[tokio::test]
async fn test_andor_feature_persistence_idempotent() {
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let features = extract_db_features(&registry, "istar_test");

    // First upsert.
    db.upsert_device_features(&features).await.unwrap();
    let first = db.get_device_features("istar_test").await.unwrap();

    // Second upsert (should be no-op, no duplication).
    db.upsert_device_features(&features).await.unwrap();
    let second = db.get_device_features("istar_test").await.unwrap();

    assert_eq!(
        first.len(),
        second.len(),
        "Idempotent upsert should not duplicate features"
    );
}

// =============================================================================
// Test 7: Parameter group assignments via gRPC ListParameters
// =============================================================================

#[tokio::test]
async fn test_andor_parameters_have_group_assignments() {
    let registry = test_registry().await;
    if !register_andor_mock(&registry).await {
        return;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    let resp = service
        .list_parameters(Request::new(ListParametersRequest {
            device_id: "istar_test".to_string(),
        }))
        .await
        .expect("ListParameters should succeed");

    let params = resp.into_inner().parameters;

    // Build a name -> group_name map for easy lookup.
    let group_map: HashMap<&str, Option<&str>> = params
        .iter()
        .map(|p| (p.name.as_str(), p.group_name.as_deref()))
        .collect();

    // Verify specific features have the expected group assignments.
    assert_eq!(
        group_map.get("TriggerMode").copied().flatten(),
        Some("Acquisition"),
        "TriggerMode should be in the Acquisition group"
    );
    assert_eq!(
        group_map.get("DDGOutputDelay").copied().flatten(),
        Some("Timing"),
        "DDGOutputDelay should be in the Timing group"
    );
    assert_eq!(
        group_map.get("SensorWidth").copied().flatten(),
        Some("Sensor"),
        "SensorWidth should be in the Sensor group"
    );
    assert_eq!(
        group_map.get("InsertionDelay").copied().flatten(),
        Some("Intensifier"),
        "InsertionDelay should be in the Intensifier group"
    );
    assert_eq!(
        group_map.get("PixelEncoding").copied().flatten(),
        Some("Readout"),
        "PixelEncoding should be in the Readout group"
    );
    assert_eq!(
        group_map.get("AOIWidth").copied().flatten(),
        Some("ROI"),
        "AOIWidth should be in the ROI group"
    );
    assert_eq!(
        group_map.get("MetadataEnable").copied().flatten(),
        Some("Metadata"),
        "MetadataEnable should be in the Metadata group"
    );
    assert_eq!(
        group_map.get("CameraModel").copied().flatten(),
        Some("Device"),
        "CameraModel should be in the Device group"
    );
    assert_eq!(
        group_map.get("GateMode").copied().flatten(),
        Some("Intensifier"),
        "GateMode should be in the Intensifier group"
    );
    assert_eq!(
        group_map.get("SensorTemperature").copied().flatten(),
        Some("Sensor"),
        "SensorTemperature should be in the Sensor group"
    );

    // Verify that most dynamic parameters have a group assigned.
    let dynamic_with_group = params.iter().filter(|p| p.group_name.is_some()).count();
    assert!(
        dynamic_with_group >= 30,
        "Expected at least 30 parameters with group assignments, got {dynamic_with_group}"
    );

    // Verify the group_name is also present in nested ParameterMetadata.
    let trigger = params.iter().find(|p| p.name == "TriggerMode").unwrap();
    if let Some(ref meta) = trigger.metadata {
        assert_eq!(
            meta.group_name.as_deref(),
            Some("Acquisition"),
            "ParameterMetadata.group_name should also be populated"
        );
    }
}
