//! gRPC Service Integration Tests (bd-jglc)
//!
//! These tests verify gRPC service layer functionality using direct
//! HardwareServiceImpl calls rather than network roundtrips.
//!
//! Tests verify:
//! - Service method implementations
//! - Request/response structure
//! - Error handling and propagation
//! - Device listing and filtering
//!
//! # Test Categories
//!
//! 1. **Device Listing** - ListDevices RPC with various filters
//! 2. **Device Control** - Move, Read, and other device operations
//! 3. **Error Handling** - Verify error codes propagate correctly
//! 4. **Concurrent Access** - Multiple operations on same device

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
    unused_imports,
    missing_docs
)]
// Only run when server feature is enabled
#![cfg(feature = "server")]

use std::sync::Arc;
use std::time::Duration;

use daq_server::grpc::hardware_service::HardwareServiceImpl;
use protocol::daq::hardware_service_server::HardwareService;
use protocol::daq::{ListDevicesRequest, MoveRequest, ReadValueRequest};
use rust_daq::hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};
use tokio::time::timeout;
use tonic::Request;

/// Create a registry with mock devices for testing
async fn create_test_registry() -> DeviceRegistry {
    let registry = DeviceRegistry::new();

    // Register a mock stage
    registry
        .register(DeviceConfig {
            id: "test_stage".into(),
            name: "Test Stage".into(),
            driver: DriverType::MockStage {
                initial_position: 0.0,
            },
        })
        .await
        .expect("Failed to register mock stage");

    // Register a mock camera
    registry
        .register(DeviceConfig {
            id: "test_camera".into(),
            name: "Test Camera".into(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await
        .expect("Failed to register mock camera");

    // Register a mock power meter
    registry
        .register(DeviceConfig {
            id: "test_power".into(),
            name: "Test Power Meter".into(),
            driver: DriverType::MockPowerMeter {
                reading: 1e-3, // 1 mW
            },
        })
        .await
        .expect("Failed to register mock power meter");

    registry
}

// =============================================================================
// Device Listing Tests
// =============================================================================

/// Test: ListDevices returns all registered devices
#[tokio::test]
async fn test_list_devices() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    let request = Request::new(ListDevicesRequest {
        capability_filter: None,
    });

    let response = timeout(Duration::from_secs(5), service.list_devices(request))
        .await
        .expect("Request timed out")
        .expect("ListDevices failed");

    let devices = response.into_inner().devices;
    assert_eq!(devices.len(), 3, "Should have 3 registered devices");

    // Verify device IDs
    let ids: Vec<_> = devices.iter().map(|d| d.id.as_str()).collect();
    assert!(ids.contains(&"test_stage"));
    assert!(ids.contains(&"test_camera"));
    assert!(ids.contains(&"test_power"));
}

/// Test: ListDevices with capability filter for movable devices
#[tokio::test]
async fn test_list_devices_filter_movable() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    let request = Request::new(ListDevicesRequest {
        capability_filter: Some("movable".to_string()),
    });

    let response = timeout(Duration::from_secs(5), service.list_devices(request))
        .await
        .expect("Request timed out")
        .expect("ListDevices failed");

    let devices = response.into_inner().devices;
    assert_eq!(devices.len(), 1, "Only stage should be movable");
    assert_eq!(devices[0].id, "test_stage");
}

/// Test: ListDevices with capability filter for readable devices
#[tokio::test]
async fn test_list_devices_filter_readable() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    let request = Request::new(ListDevicesRequest {
        capability_filter: Some("readable".to_string()),
    });

    let response = timeout(Duration::from_secs(5), service.list_devices(request))
        .await
        .expect("Request timed out")
        .expect("ListDevices failed");

    let devices = response.into_inner().devices;
    // Power meter should be readable
    assert!(
        devices.iter().any(|d| d.id == "test_power"),
        "Power meter should be in readable devices"
    );
}

// =============================================================================
// Device Control Tests
// =============================================================================

/// Test: Move a stage device
#[tokio::test]
async fn test_move_stage() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    // Move the stage to position 42.5
    let request = Request::new(MoveRequest {
        device_id: "test_stage".to_string(),
        value: 42.5,
        wait_for_completion: Some(true),
        timeout_ms: Some(5000),
    });

    let response = timeout(Duration::from_secs(5), service.move_absolute(request))
        .await
        .expect("Request timed out")
        .expect("Move failed");

    let move_response = response.into_inner();
    assert!(move_response.success, "Move should succeed");
}

/// Test: Read from power meter
#[tokio::test]
async fn test_read_power_meter() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    let request = Request::new(ReadValueRequest {
        device_id: "test_power".to_string(),
    });

    let response = timeout(Duration::from_secs(5), service.read_value(request))
        .await
        .expect("Request timed out")
        .expect("Read failed");

    let read_response = response.into_inner();
    // MockPowerMeter returns a value - verify it's a number
    assert!(
        read_response.value.is_finite(),
        "Should return a finite value"
    );
}

// =============================================================================
// Error Handling Tests
// =============================================================================

/// Test: Move to non-existent device returns NOT_FOUND error
#[tokio::test]
async fn test_move_nonexistent_device() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    let request = Request::new(MoveRequest {
        device_id: "nonexistent_device".to_string(),
        value: 0.0,
        wait_for_completion: None,
        timeout_ms: None,
    });

    let result = timeout(Duration::from_secs(5), service.move_absolute(request))
        .await
        .expect("Request timed out");

    assert!(result.is_err(), "Should fail for nonexistent device");
    let status = result.unwrap_err();
    assert_eq!(
        status.code(),
        tonic::Code::NotFound,
        "Should return NOT_FOUND for missing device"
    );
}

/// Test: Read from non-existent device returns NOT_FOUND error
#[tokio::test]
async fn test_read_nonexistent_device() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    let request = Request::new(ReadValueRequest {
        device_id: "nonexistent_device".to_string(),
    });

    let result = timeout(Duration::from_secs(5), service.read_value(request))
        .await
        .expect("Request timed out");

    assert!(result.is_err(), "Should fail for nonexistent device");
    let status = result.unwrap_err();
    assert_eq!(
        status.code(),
        tonic::Code::NotFound,
        "Should return NOT_FOUND for missing device"
    );
}

/// Test: Move on non-movable device returns appropriate error
#[tokio::test]
async fn test_move_non_movable_device() {
    let registry = Arc::new(create_test_registry().await);
    let service = HardwareServiceImpl::new(registry);

    // Try to move the power meter (which is not movable)
    let request = Request::new(MoveRequest {
        device_id: "test_power".to_string(),
        value: 0.0,
        wait_for_completion: None,
        timeout_ms: None,
    });

    let result = timeout(Duration::from_secs(5), service.move_absolute(request))
        .await
        .expect("Request timed out");

    assert!(result.is_err(), "Should fail for non-movable device");
    // The specific error code may vary (FailedPrecondition or Unimplemented)
    let status = result.unwrap_err();
    assert!(
        matches!(
            status.code(),
            tonic::Code::FailedPrecondition | tonic::Code::Unimplemented | tonic::Code::NotFound
        ),
        "Should return appropriate error for non-movable device, got: {:?}",
        status.code()
    );
}

// =============================================================================
// Concurrent Access Tests
// =============================================================================

/// Test: Multiple concurrent list_devices calls
#[tokio::test]
async fn test_concurrent_list_devices() {
    let registry = Arc::new(create_test_registry().await);
    let service = Arc::new(HardwareServiceImpl::new(registry));

    // Spawn 10 concurrent requests
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let service = Arc::clone(&service);
            tokio::spawn(async move {
                let request = Request::new(ListDevicesRequest {
                    capability_filter: None,
                });
                service.list_devices(request).await
            })
        })
        .collect();

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(handles).await;

    for result in results {
        let response = result.expect("Task panicked").expect("Request failed");
        assert_eq!(
            response.into_inner().devices.len(),
            3,
            "Each request should see 3 devices"
        );
    }
}

/// Test: Concurrent move operations on same device
#[tokio::test]
async fn test_concurrent_moves() {
    let registry = Arc::new(create_test_registry().await);
    let service = Arc::new(HardwareServiceImpl::new(registry));

    // Spawn 5 concurrent move requests to the same stage
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let service = Arc::clone(&service);
            let position = i as f64 * 10.0;
            tokio::spawn(async move {
                let request = Request::new(MoveRequest {
                    device_id: "test_stage".to_string(),
                    value: position,
                    wait_for_completion: Some(true),
                    timeout_ms: Some(5000),
                });
                service.move_absolute(request).await
            })
        })
        .collect();

    // All moves should complete (though order is non-deterministic)
    let results: Vec<_> = futures::future::join_all(handles).await;

    let mut success_count = 0;
    for result in results {
        if let Ok(Ok(response)) = result {
            if response.into_inner().success {
                success_count += 1;
            }
        }
    }

    // At least some moves should succeed (concurrent access may serialize them)
    assert!(success_count > 0, "At least one move should succeed");
}
