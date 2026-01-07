#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
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
use daq_proto::daq::hardware_service_server::HardwareService;
use daq_proto::daq::{GetParameterRequest, SetParameterRequest, StreamParameterChangesRequest};
use daq_server::grpc::hardware_service::HardwareServiceImpl;
use rust_daq::hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};
use std::sync::Arc;
use tokio_stream::StreamExt;
use tonic::Request;

// =============================================================================
// Test 1: Basic Integration Test
// =============================================================================

#[tokio::test]
async fn test_basic_parameter_integration() -> Result<()> {
    // Setup: Create registry with MockCamera
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "mock_camera".to_string(),
            name: "Mock Camera".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    // Wrap in Arc<RwLock> for HardwareService
    let registry = Arc::new(registry);
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
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "mock_camera".to_string(),
            name: "Mock Camera".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // Subscribe to parameter changes (no filter)
    let request = Request::new(StreamParameterChangesRequest {
        device_id: None,
        parameter_names: vec![],
    });
    let response = service.stream_parameter_changes(request).await?;
    let mut stream = response.into_inner();

    // Give stream time to initialize
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

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
// Test 3: Real Driver Test (MaiTai with Mock Serial)
// =============================================================================

#[tokio::test]
#[cfg(feature = "tokio_serial")]
async fn test_maitai_parameter_integration() -> Result<()> {
    use rust_daq::hardware::capabilities::Parameterized;
    use rust_daq::hardware::maitai::MaiTaiDriver;
    use std::io::Write;
    // use tempfile::NamedTempFile; // Unused if test skipped
    #[allow(unused_imports)]
    use tempfile::NamedTempFile;

    // Create a mock serial port using pty (Unix-like systems only)
    #[cfg(unix)]
    {
        #[allow(unused_imports)]
        use std::os::unix::io::AsRawFd;

        // Create pseudo-terminal pair
        let pty = nix::pty::openpty(None, None).expect("Failed to create pty");
        let master = pty.master;
        let slave = pty.slave;

        // Get slave path
        let slave_path = nix::unistd::ttyname(&slave).expect("Failed to get slave path");
        let slave_path_str = slave_path.to_str().unwrap();

        // Spawn background task to handle serial protocol
        // bd-d7uw: Use into_raw_fd() to transfer ownership, preventing double-close
        use std::os::unix::io::IntoRawFd;
        let master_fd = master.into_raw_fd();
        // bd-9thk: Use spawn_blocking for blocking PTY I/O to avoid blocking the async runtime
        tokio::task::spawn_blocking(move || {
            use std::os::unix::io::FromRawFd;
            // SAFETY: master_fd ownership was transferred via into_raw_fd(), so no double-close
            let mut file = unsafe { std::fs::File::from_raw_fd(master_fd) };

            loop {
                let mut buf = [0u8; 256];
                if let Ok(n) = std::io::Read::read(&mut file, &mut buf) {
                    if n == 0 {
                        break;
                    }
                    let cmd = String::from_utf8_lossy(&buf[..n]);

                    // Simulate MaiTai responses
                    if cmd.contains("WAVELENGTH:") {
                        // Echo command (MaiTai behavior)
                        let _ = file.write_all(b"OK\n");
                    } else if cmd.contains("WAVELENGTH?") {
                        let _ = file.write_all(b"800nm\n");
                    }
                }
            }
        });

        // Small delay for pty to be ready
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Create MaiTai driver with mock serial port
        // bd-9thk: Handle macOS PTY limitation (Not a typewriter/ENOTTY)
        let driver = match MaiTaiDriver::new_async(slave_path_str).await {
            Ok(d) => d,
            Err(e) => {
                // Use Debug format to check the full error chain (context + root cause)
                let msg = format!("{:?}", e);
                if msg.contains("Not a typewriter")
                    || msg.contains("Inappropriate ioctl for device")
                {
                    println!(
                        "Skipping test: PTY not supported by tokio-serial on this OS (ENOTTY)"
                    );
                    return Ok(());
                }
                return Err(e);
            }
        };

        // Create registry and register driver
        #[allow(unused_mut, unused_variables)]
        let mut registry = DeviceRegistry::new();

        // Get parameters from driver
        let params = driver.parameters();

        // Register driver manually (since we can't use DriverType::MaiTai directly)
        // This would normally be done by the registry, but we're testing the parameter system

        // Set wavelength via parameter system
        if let Some(wavelength_param) = params.get("wavelength_nm") {
            wavelength_param.set_json(serde_json::json!(850.0))?;

            // Verify parameter updated
            let value = wavelength_param.get_json()?;
            assert_eq!(value.as_f64().unwrap(), 850.0);
        } else {
            panic!("wavelength_nm parameter not found in MaiTai driver");
        }

        // Note: We can't fully test serial command sending without more complex mocking
        // but we've verified the parameter system integration
    }

    #[cfg(not(unix))]
    {
        // Skip test on non-Unix systems
        println!("Skipping MaiTai serial test on non-Unix platform");
    }

    Ok(())
}

// =============================================================================
// Test 4: Negative Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_parameter_name() -> Result<()> {
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "mock_camera".to_string(),
            name: "Mock Camera".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    let registry = Arc::new(registry);
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
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "mock_camera".to_string(),
            name: "Mock Camera".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    let registry = Arc::new(registry);
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
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "mock_camera".to_string(),
            name: "Mock Camera".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    let registry = Arc::new(registry);
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
    let registry = Arc::new(registry);
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
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "mock_camera".to_string(),
            name: "Mock Camera".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    let registry = Arc::new(registry);
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
            let exposure_value = 0.033 + (i % 100) as f64 / 1000.0; // Vary exposure
            let request = Request::new(SetParameterRequest {
                device_id: "mock_camera".to_string(),
                parameter_name: "exposure_s".to_string(),
                value: format!("{}", exposure_value),
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
            panic!("Task failed: {}", e);
        }
        Err(_) => {
            panic!("DEADLOCK DETECTED: Test timed out after 30 seconds");
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
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "camera1".to_string(),
            name: "Camera 1".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;
    registry
        .register(DeviceConfig {
            id: "camera2".to_string(),
            name: "Camera 2".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    let registry = Arc::new(registry);
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
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "camera1".to_string(),
            name: "Camera 1".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;
    registry
        .register(DeviceConfig {
            id: "camera2".to_string(),
            name: "Camera 2".to_string(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
        .await?;

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // Subscribe to parameter changes for camera1 only
    let request = Request::new(StreamParameterChangesRequest {
        device_id: Some("camera1".to_string()),
        parameter_names: vec![],
    });
    let response = service.stream_parameter_changes(request).await?;
    let mut stream = response.into_inner();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

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
