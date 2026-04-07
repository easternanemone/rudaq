#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    missing_docs
)]
//! Comprehensive gRPC Parameter Integration Tests (bd-0hk1)
//!
//! This test suite verifies the complete end-to-end flow:
//! gRPC → ParameterSet → Parameter → Hardware callback
//!
//! Test Coverage:
//! 1. Basic integration: set_parameter RPC triggers hardware callback
//! 2. Parameter change notifications: broadcast stream works
//! 3. Real driver integration: MaiTai with mock serial
//! 4. Negative tests: invalid parameters, out of range values
//! 5. Concurrency safety: concurrent reads/writes don't deadlock
//!
//! Requires: `--features server` to compile

#![cfg(feature = "server")]

use anyhow::Result;
use hardware::registry::{DeviceRegistry, register_mock_factories};
use protocol::daq::hardware_service_server::HardwareService;
use protocol::daq::{
    GetParameterRequest, ListParametersRequest, SetParameterRequest, StreamParameterChangesRequest,
};
use server::grpc::hardware_service::HardwareServiceImpl;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tonic::Request;

// =============================================================================
// Test 1: Basic Integration Test
// =============================================================================

#[tokio::test]
async fn test_basic_parameter_integration() -> Result<()> {
    // Setup: Create registry with MockCamera
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    // Wrap in Arc<RwLock> for HardwareService
    let registry = registry;
    let service = HardwareServiceImpl::new(registry.clone());

    // Get initial exposure value
    let request = Request::new(GetParameterRequest {
        device_id: "mock_camera".to_string(),
        parameter_name: "exposure_s".to_string(),
    });
    let response = service.get_parameter(request).await?;
    let initial_value: f64 = response.into_inner().value.parse()?;
    assert_eq!(initial_value, 0.033); // Default exposure

    // Set new exposure via gRPC
    let request = Request::new(SetParameterRequest {
        device_id: "mock_camera".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "0.1".to_string(), // 100ms exposure
    });
    let response = service.set_parameter(request).await?;
    let set_response = response.into_inner();
    assert!(set_response.success);
    assert_eq!(set_response.actual_value, "0.1");

    // Verify parameter was updated
    let request = Request::new(GetParameterRequest {
        device_id: "mock_camera".to_string(),
        parameter_name: "exposure_s".to_string(),
    });
    let response = service.get_parameter(request).await?;
    let new_value: f64 = response.into_inner().value.parse()?;
    assert_eq!(new_value, 0.1);

    // Verify hardware callback was invoked (MockCamera tracks internal state)
    let exposure_ctrl = registry.get_exposure_control("mock_camera").unwrap();
    let actual_exposure = exposure_ctrl.get_exposure().await?;
    assert_eq!(actual_exposure, 0.1);

    Ok(())
}

// =============================================================================
// Test 2: Parameter Change Notification Test
// =============================================================================

#[tokio::test]
async fn test_parameter_change_notifications() -> Result<()> {
    // Setup: Create registry with MockCamera
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry.clone());

    // Subscribe to parameter changes (no filter)
    let request = Request::new(StreamParameterChangesRequest {
        device_id: None,
        parameter_names: vec![],
    });
    let response = service.stream_parameter_changes(request).await?;
    let mut stream = response.into_inner();

    // Give stream time to initialize
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Set parameter via gRPC
    let request = Request::new(SetParameterRequest {
        device_id: "mock_camera".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "0.25".to_string(),
    });
    service.set_parameter(request).await?;

    // Receive notification
    let change = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("timeout waiting for parameter change");

    assert!(change.is_some());
    let change_data = change.unwrap()?;
    assert_eq!(change_data.device_id, "mock_camera");
    assert_eq!(change_data.name, "exposure_s");
    assert_eq!(change_data.old_value, "0.033"); // Default
    assert_eq!(change_data.new_value, "0.25");

    Ok(())
}

// =============================================================================
// Test 3: Negative Tests (renumbered after MaiTai driver deletion)
// =============================================================================

#[tokio::test]
async fn test_invalid_parameter_name() -> Result<()> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry);

    // Try to set non-existent parameter
    let request = Request::new(SetParameterRequest {
        device_id: "mock_camera".to_string(),
        parameter_name: "invalid_param".to_string(),
        value: "123".to_string(),
    });

    let result = service.set_parameter(request).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);

    Ok(())
}

#[tokio::test]
async fn test_out_of_range_value() -> Result<()> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry);

    // Try to set exposure outside valid range (0.001 - 10.0 seconds)
    let request = Request::new(SetParameterRequest {
        device_id: "mock_camera".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "100.0".to_string(), // Too large
    });

    let result = service.set_parameter(request).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);

    Ok(())
}

#[tokio::test]
async fn test_type_mismatch() -> Result<()> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry);

    // Try to set string value to f64 parameter
    let request = Request::new(SetParameterRequest {
        device_id: "mock_camera".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "\"not_a_number\"".to_string(),
    });

    let result = service.set_parameter(request).await;

    // Should fail during parameter validation/deserialization
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_device_not_found() -> Result<()> {
    let registry = DeviceRegistry::new();
    let registry = registry;
    let service = HardwareServiceImpl::new(registry);

    let request = Request::new(SetParameterRequest {
        device_id: "nonexistent_device".to_string(),
        parameter_name: "some_param".to_string(),
        value: "123".to_string(),
    });

    let result = service.set_parameter(request).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);

    Ok(())
}

// =============================================================================
// Test 5: CRITICAL - Concurrency Test
// =============================================================================

#[tokio::test]
async fn test_concurrent_parameter_access_no_deadlock() -> Result<()> {
    // Setup: Create registry with MockCamera
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = Arc::new(HardwareServiceImpl::new(registry.clone()));

    // Spawn background task: loops calling driver.get_exposure().await
    let registry_clone = registry.clone();
    let read_task = tokio::spawn(async move {
        for i in 0..1000 {
            if let Some(exposure_ctrl) = registry_clone.get_exposure_control("mock_camera") {
                // This acquires: Driver Mutex → reads Parameter
                let _ = exposure_ctrl.get_exposure().await;
            }

            // Yield occasionally to allow interleaving
            if i % 10 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });

    // Main thread: loops calling set_parameter RPC (1000 iterations)
    let service_clone = service.clone();
    let write_task = tokio::spawn(async move {
        for i in 0..1000 {
            let exposure_value = 0.033 + f64::from(i % 100) / 1000.0; // Vary exposure
            let request = Request::new(SetParameterRequest {
                device_id: "mock_camera".to_string(),
                parameter_name: "exposure_s".to_string(),
                value: format!("{exposure_value}"),
            });

            // This acquires: Registry RwLock → Parameter Lock → Driver Mutex (via callback)
            let _ = service_clone.set_parameter(request).await;

            // Yield occasionally to allow interleaving
            if i % 10 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });

    // Wait for both tasks to complete (with timeout to detect deadlock)
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async move {
        tokio::try_join!(read_task, write_task).map(|_| ())
    })
    .await;

    match result {
        Ok(Ok(())) => {
            println!("✓ Concurrency test passed: 1000 iterations completed without deadlock");
        }
        Ok(Err(e)) => {
            panic!("Task failed: {e}");
        }
        Err(err) => {
            panic!("DEADLOCK DETECTED: Test timed out after 30 seconds: {err}");
        }
    }

    Ok(())
}

// =============================================================================
// Test 6: Multiple Devices Parameter Isolation
// =============================================================================

#[tokio::test]
async fn test_multiple_devices_parameter_isolation() -> Result<()> {
    // Setup: Create registry with two MockCameras
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "camera1",
            "Camera 1",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;
    registry
        .register_from_toml(
            "camera2",
            "Camera 2",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry.clone());

    // Set different exposures for each camera
    let request1 = Request::new(SetParameterRequest {
        device_id: "camera1".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "0.1".to_string(),
    });
    service.set_parameter(request1).await?;

    let request2 = Request::new(SetParameterRequest {
        device_id: "camera2".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "0.5".to_string(),
    });
    service.set_parameter(request2).await?;

    // Verify isolation: camera1 still has 0.1
    let request = Request::new(GetParameterRequest {
        device_id: "camera1".to_string(),
        parameter_name: "exposure_s".to_string(),
    });
    let response = service.get_parameter(request).await?;
    assert_eq!(response.into_inner().value, "0.1");

    // Verify camera2 has 0.5
    let request = Request::new(GetParameterRequest {
        device_id: "camera2".to_string(),
        parameter_name: "exposure_s".to_string(),
    });
    let response = service.get_parameter(request).await?;
    assert_eq!(response.into_inner().value, "0.5");

    Ok(())
}

// =============================================================================
// Test 7: Filtered Parameter Change Notifications
// =============================================================================

#[tokio::test]
async fn test_filtered_parameter_notifications() -> Result<()> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "camera1",
            "Camera 1",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;
    registry
        .register_from_toml(
            "camera2",
            "Camera 2",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry.clone());

    // Subscribe to parameter changes for camera1 only
    let request = Request::new(StreamParameterChangesRequest {
        device_id: Some("camera1".to_string()),
        parameter_names: vec![],
    });
    let response = service.stream_parameter_changes(request).await?;
    let mut stream = response.into_inner();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Change parameter on camera2 (should be filtered out)
    let request = Request::new(SetParameterRequest {
        device_id: "camera2".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "0.2".to_string(),
    });
    service.set_parameter(request).await?;

    // Change parameter on camera1 (should pass filter)
    let request = Request::new(SetParameterRequest {
        device_id: "camera1".to_string(),
        parameter_name: "exposure_s".to_string(),
        value: "0.3".to_string(),
    });
    service.set_parameter(request).await?;

    // Should receive only camera1 change
    let change = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("timeout waiting for parameter change");

    assert!(change.is_some());
    let change_data = change.unwrap()?;
    assert_eq!(change_data.device_id, "camera1");
    assert_eq!(change_data.new_value, "0.3");

    Ok(())
}

// =============================================================================
// Test 8: Parameter<String> Roundtrip — No Double-Quoting (bd-0lacj)
// =============================================================================

#[tokio::test]
async fn test_string_parameter_roundtrip_no_double_quoting() -> Result<()> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry.clone());

    // Verify user_label is listed with dtype "string"
    let list_resp = service
        .list_parameters(Request::new(ListParametersRequest {
            device_id: "mock_camera".to_string(),
        }))
        .await?;
    let label_desc = list_resp
        .into_inner()
        .parameters
        .into_iter()
        .find(|p| p.name == "user_label")
        .expect("user_label parameter should be listed");
    assert_eq!(label_desc.dtype, "string");

    // Get the default string value — must NOT be double-quoted
    let response = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
        }))
        .await?;
    let value = response.into_inner().value;
    assert_eq!(
        value, "default",
        "string value should not have extra quotes"
    );
    assert!(
        !value.starts_with('"'),
        "string value must not be double-quoted: got {value:?}"
    );

    // Set a new string value and verify roundtrip
    let set_resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
            value: "my_camera_1".to_string(),
        }))
        .await?;
    let set_inner = set_resp.into_inner();
    assert!(set_inner.success);
    assert_eq!(
        set_inner.actual_value, "my_camera_1",
        "set response should echo back the raw string"
    );

    // Read it back and confirm no double-quoting
    let response = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
        }))
        .await?;
    let roundtripped = response.into_inner().value;
    assert_eq!(roundtripped, "my_camera_1");
    assert!(
        !roundtripped.starts_with('"'),
        "roundtripped string must not be double-quoted: got {roundtripped:?}"
    );

    Ok(())
}

// =============================================================================
// Test 9: JSON-Shaped String Preserved as String (bd-0lacj)
// =============================================================================

#[tokio::test]
async fn test_json_shaped_string_preserved() -> Result<()> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry.clone());

    // Set a value that looks like JSON to a string-typed parameter.
    // The dtype-aware coercion (bd-4w33o) must treat it as a raw string,
    // NOT parse it as JSON and re-serialize.
    let json_shaped = r#"[{"x":0,"y":100}]"#;
    let set_resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
            value: json_shaped.to_string(),
        }))
        .await?;
    let set_inner = set_resp.into_inner();
    assert!(set_inner.success);
    assert_eq!(
        set_inner.actual_value, json_shaped,
        "JSON-shaped string should be preserved verbatim"
    );

    // Read it back — must be identical, not re-serialized JSON
    let response = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
        }))
        .await?;
    let value = response.into_inner().value;
    assert_eq!(
        value, json_shaped,
        "JSON-shaped string should roundtrip verbatim"
    );

    Ok(())
}

// =============================================================================
// Test 10: String Parameter dtype-Aware Coercion (bd-0lacj)
// =============================================================================

#[tokio::test]
async fn test_string_parameter_dtype_coercion() -> Result<()> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    let registry = registry;
    let service = HardwareServiceImpl::new(registry.clone());

    // Set a numeric-looking value to a string parameter.
    // Because dtype="string", the value "42" should be stored as the string
    // "42", not parsed as integer 42 and re-serialized.
    let set_resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
            value: "42".to_string(),
        }))
        .await?;
    let set_inner = set_resp.into_inner();
    assert!(set_inner.success);
    assert_eq!(
        set_inner.actual_value, "42",
        "numeric string should be stored as-is"
    );

    // Read it back — the value should still be the string "42"
    let response = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
        }))
        .await?;
    let value = response.into_inner().value;
    assert_eq!(
        value, "42",
        "numeric string should roundtrip as the string 42"
    );

    // Also verify a boolean-looking value is treated as string
    let set_resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
            value: "true".to_string(),
        }))
        .await?;
    assert!(set_resp.into_inner().success);

    let response = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "mock_camera".to_string(),
            parameter_name: "user_label".to_string(),
        }))
        .await?;
    let value = response.into_inner().value;
    assert_eq!(
        value, "true",
        "boolean-looking string should be preserved as string 'true'"
    );

    Ok(())
}
