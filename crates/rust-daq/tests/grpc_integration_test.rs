#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    missing_docs
)]
//! Integration tests for gRPC camera and scan streaming paths (bd-fxzu)
//!
//! Tests:
//! 1. Scan progress stream reaches client
//! 2. Registry camera path with MockCamera
//! 3. Frame count tracking

#[cfg(feature = "server")]
mod camera_integration_tests {
    use daq_proto::daq::hardware_service_server::HardwareService;
    use daq_proto::daq::{
        ArmRequest, DeviceStateRequest, ListDevicesRequest, StartStreamRequest, StopStreamRequest,
        StreamFramesRequest, TriggerRequest,
    };
    use daq_server::grpc::hardware_service::HardwareServiceImpl;
    use rust_daq::hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::time::timeout;
    use tokio_stream::StreamExt;
    use tonic::Request;

    /// Create a registry with MockCamera for testing
    async fn create_camera_registry() -> DeviceRegistry {
        let registry = DeviceRegistry::new();

        // Register MockCamera
        registry
            .register(DeviceConfig {
                id: "test_camera".into(),
                name: "Test MockCamera".into(),
                driver: DriverType::MockCamera {
                    width: 640,
                    height: 480,
                },
            })
            .await
            .unwrap();

        registry
    }

    /// Test: Registry camera path - device listing shows camera capabilities
    #[tokio::test]
    async fn test_camera_appears_in_registry_with_correct_capabilities() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(ListDevicesRequest {
            capability_filter: None,
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        assert_eq!(devices.len(), 1);
        let camera = &devices[0];
        assert_eq!(camera.id, "test_camera");
        assert!(camera.is_triggerable, "Camera should be triggerable");
        assert!(camera.is_frame_producer, "Camera should be frame producer");
        assert!(
            camera.is_exposure_controllable,
            "Camera should have exposure control"
        );
    }

    /// Test: List devices with capability filter for triggerable
    #[tokio::test]
    async fn test_filter_devices_by_triggerable_capability() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(ListDevicesRequest {
            capability_filter: Some("triggerable".to_string()),
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "test_camera");
    }

    /// Test: List devices with capability filter for frame_producer
    #[tokio::test]
    async fn test_filter_devices_by_frame_producer_capability() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(ListDevicesRequest {
            capability_filter: Some("frame_producer".to_string()),
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "test_camera");
    }

    /// Test: Get camera device state shows armed and streaming status
    #[tokio::test]
    async fn test_camera_device_state() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let request = Request::new(DeviceStateRequest {
            device_id: "test_camera".to_string(),
        });
        let response = service.get_device_state(request).await.unwrap();
        let state = response.into_inner();

        assert_eq!(state.device_id, "test_camera");
        assert!(state.online);
        // MockCamera starts not armed and not streaming
        assert_eq!(state.armed, Some(false));
        assert_eq!(state.streaming, Some(false));
    }

    /// Test: Arm camera through gRPC
    #[tokio::test]
    async fn test_arm_camera_via_grpc() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Arm the camera
        let arm_request = Request::new(ArmRequest {
            device_id: "test_camera".to_string(),
        });
        let arm_response = service.arm(arm_request).await.unwrap();
        let arm_result = arm_response.into_inner();

        assert!(arm_result.success);
        assert!(arm_result.armed);

        // Verify state changed
        let state_request = Request::new(DeviceStateRequest {
            device_id: "test_camera".to_string(),
        });
        let state_response = service.get_device_state(state_request).await.unwrap();
        let state = state_response.into_inner();

        assert_eq!(state.armed, Some(true));
    }

    /// Test: Trigger camera through gRPC (must arm first)
    #[tokio::test]
    async fn test_trigger_camera_via_grpc() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Arm first
        let arm_request = Request::new(ArmRequest {
            device_id: "test_camera".to_string(),
        });
        service.arm(arm_request).await.unwrap();

        // Now trigger
        let trigger_request = Request::new(TriggerRequest {
            device_id: "test_camera".to_string(),
        });
        let trigger_response = service.trigger(trigger_request).await.unwrap();
        let trigger_result = trigger_response.into_inner();

        assert!(trigger_result.success);
        assert!(trigger_result.trigger_timestamp_ns > 0);
    }

    /// Test: Trigger without arming fails
    #[tokio::test]
    async fn test_trigger_without_arm_fails() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Try to trigger without arming - should fail with FAILED_PRECONDITION status
        let trigger_request = Request::new(TriggerRequest {
            device_id: "test_camera".to_string(),
        });
        let trigger_result = service.trigger(trigger_request).await;

        // With the new consistent error handling, this should return a Status error
        assert!(trigger_result.is_err());
        let status = trigger_result.unwrap_err();
        // "not armed" is a precondition failure
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(status.message().to_lowercase().contains("not armed"));
    }

    /// Test: Start and stop frame streaming via gRPC
    #[tokio::test]
    async fn test_start_stop_stream_via_grpc() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Start streaming
        let start_request = Request::new(StartStreamRequest {
            device_id: "test_camera".to_string(),
            frame_count: None, // Continuous streaming
        });
        let start_response = service.start_stream(start_request).await.unwrap();
        let start_result = start_response.into_inner();

        assert!(start_result.success);

        // Verify streaming state
        let state_request = Request::new(DeviceStateRequest {
            device_id: "test_camera".to_string(),
        });
        let state_response = service.get_device_state(state_request).await.unwrap();
        let state = state_response.into_inner();
        assert_eq!(state.streaming, Some(true));

        // Stop streaming
        let stop_request = Request::new(StopStreamRequest {
            device_id: "test_camera".to_string(),
        });
        let stop_response = service.stop_stream(stop_request).await.unwrap();
        let stop_result = stop_response.into_inner();

        assert!(stop_result.success);

        // Verify streaming stopped
        let state_request2 = Request::new(DeviceStateRequest {
            device_id: "test_camera".to_string(),
        });
        let state_response2 = service.get_device_state(state_request2).await.unwrap();
        let state2 = state_response2.into_inner();
        assert_eq!(state2.streaming, Some(false));
    }

    /// Test: Frame count tracking through gRPC
    ///
    /// Note: MockCamera's frame_count increments on trigger(), not during streaming.
    /// This test verifies frame count is returned through gRPC.
    #[tokio::test]
    async fn test_frame_count_tracking_via_grpc() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        // Arm and trigger to increment frame count
        let arm_request = Request::new(ArmRequest {
            device_id: "test_camera".to_string(),
        });
        service.arm(arm_request).await.unwrap();

        // Trigger multiple times to get frame count
        for _ in 0..3 {
            let trigger_request = Request::new(TriggerRequest {
                device_id: "test_camera".to_string(),
            });
            service.trigger(trigger_request).await.unwrap();
        }

        // Stop stream returns frame count (even if not streaming)
        let stop_request = Request::new(StopStreamRequest {
            device_id: "test_camera".to_string(),
        });
        let stop_response = service.stop_stream(stop_request).await.unwrap();
        let stop_result = stop_response.into_inner();

        // Frame count should be 3 from our triggers
        assert_eq!(stop_result.frames_captured, 3);
    }

    /// Test: Camera not found returns appropriate error
    #[tokio::test]
    async fn test_camera_not_found_error() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        let arm_request = Request::new(ArmRequest {
            device_id: "nonexistent_camera".to_string(),
        });
        let result = service.arm(arm_request).await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    /// Test: stream_frames rate limiting and metrics exposure
    #[tokio::test]
    async fn test_stream_frames_rate_limiting_and_metrics() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        service
            .start_stream(Request::new(StartStreamRequest {
                device_id: "test_camera".to_string(),
                frame_count: None,
            }))
            .await
            .unwrap();

        let request = Request::new(StreamFramesRequest {
            device_id: "test_camera".to_string(),
            max_fps: 10,
        });
        let mut stream = service.stream_frames(request).await.unwrap().into_inner();

        let start = Instant::now();
        let mut frames = Vec::new();
        let mut last_metrics = None;

        while start.elapsed() < Duration::from_secs(3) {
            match timeout(Duration::from_millis(300), stream.next()).await {
                Ok(Some(Ok(frame))) => {
                    last_metrics = frame.metrics.clone();
                    frames.push(frame);
                }
                Ok(Some(Err(err))) => panic!("stream error: {}", err),
                Ok(None) => break,
                Err(_) => {}
            }
        }

        let elapsed = start.elapsed().as_secs_f64().max(0.1);
        let fps = frames.len() as f64 / elapsed;
        assert!(fps <= 14.0, "rate limiter should cap fps, got {}", fps);
        assert!(fps >= 6.0, "expected some frames, got {}", fps);

        let metrics = last_metrics.expect("streaming metrics should be present");
        assert!(metrics.frames_sent >= frames.len() as u64);
        assert!(metrics.current_fps > 0.0);
        assert!(metrics.avg_latency_ms >= 0.0);
        assert!(
            metrics.frames_dropped > 0,
            "expected dropped/limited frames reported"
        );

        service
            .stop_stream(Request::new(StopStreamRequest {
                device_id: "test_camera".to_string(),
            }))
            .await
            .unwrap();
    }

    /// Stress test: 60s sustained streaming (ignored by default).
    #[tokio::test]
    #[ignore]
    async fn test_stream_frames_sustained_60s() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(Arc::new(registry));

        service
            .start_stream(Request::new(StartStreamRequest {
                device_id: "test_camera".to_string(),
                frame_count: None,
            }))
            .await
            .unwrap();

        let request = Request::new(StreamFramesRequest {
            device_id: "test_camera".to_string(),
            max_fps: 10,
        });
        let mut stream = service.stream_frames(request).await.unwrap().into_inner();

        let start = Instant::now();
        let mut last_metrics = None;
        let mut frames_received = 0u64;

        while start.elapsed() < Duration::from_secs(60) {
            match timeout(Duration::from_millis(500), stream.next()).await {
                Ok(Some(Ok(frame))) => {
                    last_metrics = frame.metrics.clone();
                    frames_received = frames_received.saturating_add(1);
                }
                Ok(Some(Err(err))) => panic!("stream error: {}", err),
                Ok(None) => break,
                Err(_) => {}
            }
        }

        assert!(frames_received > 0, "expected frames over 60s window");
        let metrics = last_metrics.expect("streaming metrics should be present");
        assert!(metrics.current_fps > 0.0);

        service
            .stop_stream(Request::new(StopStreamRequest {
                device_id: "test_camera".to_string(),
            }))
            .await
            .unwrap();
    }
}

#[cfg(feature = "server")]
mod scan_integration_tests {
    use daq_proto::daq::scan_service_server::ScanService;
    use daq_proto::daq::{
        AxisConfig, CreateScanRequest, GetScanStatusRequest, ScanConfig, ScanState, ScanType,
        StartScanRequest, StreamScanProgressRequest,
    };
    use daq_server::grpc::scan_service::ScanServiceImpl;
    use rust_daq::hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};
    use serial_test::serial;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tokio_stream::StreamExt;
    use tonic::Request;

    /// Create a registry with movable and readable devices for scan testing
    async fn create_scan_registry() -> DeviceRegistry {
        let mut registry = DeviceRegistry::new();

        // Register MockStage for axis movement
        registry
            .register(DeviceConfig {
                id: "test_stage".into(),
                name: "Test Stage".into(),
                driver: DriverType::MockStage {
                    initial_position: 0.0,
                },
            })
            .await
            .unwrap();

        // Register MockPowerMeter for data acquisition
        registry
            .register(DeviceConfig {
                id: "test_meter".into(),
                name: "Test Power Meter".into(),
                driver: DriverType::MockPowerMeter { reading: 1.0 },
            })
            .await
            .unwrap();

        registry
    }

    /// Test: Create scan validates configuration
    #[tokio::test]
    #[serial]
    async fn test_create_scan() {
        let registry = create_scan_registry().await;
        let service = ScanServiceImpl::new(Arc::new(registry));

        let config = ScanConfig {
            axes: vec![AxisConfig {
                device_id: "test_stage".to_string(),
                start_position: 0.0,
                end_position: 10.0,
                num_points: 5,
            }],
            scan_type: ScanType::LineScan.into(),
            acquire_device_ids: vec!["test_meter".to_string()],
            dwell_time_ms: 10.0,
            triggers_per_point: 1,
            ..Default::default()
        };

        let request = Request::new(CreateScanRequest {
            config: Some(config),
        });
        let response = service.create_scan(request).await.unwrap();
        let result = response.into_inner();

        assert!(result.success);
        assert!(!result.scan_id.is_empty());
        assert_eq!(result.total_points, 5);
    }

    /// Test: Create scan fails with invalid device
    #[tokio::test]
    #[serial]
    async fn test_create_scan_invalid_device() {
        let registry = create_scan_registry().await;
        let service = ScanServiceImpl::new(Arc::new(registry));

        let config = ScanConfig {
            axes: vec![AxisConfig {
                device_id: "nonexistent_stage".to_string(),
                start_position: 0.0,
                end_position: 10.0,
                num_points: 5,
            }],
            scan_type: ScanType::LineScan.into(),
            ..Default::default()
        };

        let request = Request::new(CreateScanRequest {
            config: Some(config),
        });
        let response = service.create_scan(request).await.unwrap();
        let result = response.into_inner();

        assert!(!result.success);
        assert!(result.error_message.contains("not found"));
    }

    /// Test: Start scan and verify progress stream receives updates (bd-fxzu test #1)
    #[tokio::test]
    #[serial]
    async fn test_scan_progress_stream_reaches_client() {
        let registry = create_scan_registry().await;
        let service = ScanServiceImpl::new(Arc::new(registry));

        // Create scan with minimal motion (MockStage is 10mm/sec with 50ms settle)
        // 3 points at 0.1mm increments means ~10ms move + 50ms settle per point
        let config = ScanConfig {
            axes: vec![AxisConfig {
                device_id: "test_stage".to_string(),
                start_position: 0.0,
                end_position: 0.2, // Very small range for fast test
                num_points: 3,     // Minimal points
            }],
            scan_type: ScanType::LineScan.into(),
            acquire_device_ids: vec!["test_meter".to_string()],
            dwell_time_ms: 1.0, // Very short dwell for fast test
            triggers_per_point: 1,
            ..Default::default()
        };

        let create_request = Request::new(CreateScanRequest {
            config: Some(config),
        });
        let create_response = service.create_scan(create_request).await.unwrap();
        let scan_id = create_response.into_inner().scan_id;

        // Get progress stream BEFORE starting scan
        let stream_request = Request::new(StreamScanProgressRequest {
            scan_id: scan_id.clone(),
            include_data: true, // Include acquired data points in progress updates
        });
        let stream_response = service.stream_scan_progress(stream_request).await.unwrap();
        let mut stream = stream_response.into_inner();

        // Start scan
        let start_request = Request::new(StartScanRequest {
            scan_id: scan_id.clone(),
        });
        let start_response = service.start_scan(start_request).await.unwrap();
        assert!(start_response.into_inner().success);

        // Collect progress updates from stream with timeout per message
        // Note: Stream won't close naturally since ScanExecution keeps the sender alive
        let mut progress_updates = Vec::new();
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(500), stream.next()).await {
                Ok(Some(result)) => {
                    let progress = result.unwrap();
                    progress_updates.push(progress);

                    // Stop when we have enough updates (3 points in scan)
                    if progress_updates.len() >= 3 {
                        break;
                    }
                }
                Ok(None) => {
                    // Stream closed
                    break;
                }
                Err(_) => {
                    // Timeout waiting for next message - scan likely complete
                    break;
                }
            }
        }

        // Verify we received progress updates
        assert!(
            !progress_updates.is_empty(),
            "Should receive at least one progress update"
        );

        // Verify progress updates have correct scan_id
        for progress in &progress_updates {
            assert_eq!(progress.scan_id, scan_id);
            assert!(progress.total_points > 0);
        }

        // Verify we got updates from different points
        let unique_points: std::collections::HashSet<_> =
            progress_updates.iter().map(|p| p.point_index).collect();
        assert!(
            unique_points.len() > 1,
            "Should receive updates from multiple scan points"
        );

        // Verify data points are included in progress
        for progress in &progress_updates {
            assert!(
                !progress.data_points.is_empty(),
                "Progress should include data points from acquisition"
            );
        }
    }

    /// Test: Get scan status during and after execution
    #[tokio::test]
    #[serial]
    async fn test_get_scan_status() {
        let registry = create_scan_registry().await;
        let service = ScanServiceImpl::new(Arc::new(registry));

        // Create scan
        let config = ScanConfig {
            axes: vec![AxisConfig {
                device_id: "test_stage".to_string(),
                start_position: 0.0,
                end_position: 2.0,
                num_points: 3,
            }],
            scan_type: ScanType::LineScan.into(),
            acquire_device_ids: vec!["test_meter".to_string()],
            dwell_time_ms: 1.0,
            triggers_per_point: 1,
            ..Default::default()
        };

        let create_request = Request::new(CreateScanRequest {
            config: Some(config),
        });
        let create_response = service.create_scan(create_request).await.unwrap();
        let scan_id = create_response.into_inner().scan_id;

        // Check status before starting
        let status_request = Request::new(GetScanStatusRequest {
            scan_id: scan_id.clone(),
        });
        let status_response = service.get_scan_status(status_request).await.unwrap();
        let status = status_response.into_inner();

        assert_eq!(status.scan_id, scan_id);
        assert_eq!(status.state, i32::from(ScanState::ScanCreated));
        assert_eq!(status.total_points, 3);

        // Start scan and wait for completion
        let start_request = Request::new(StartScanRequest {
            scan_id: scan_id.clone(),
        });
        service.start_scan(start_request).await.unwrap();

        // Wait for scan to complete
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Check final status
        let final_status_request = Request::new(GetScanStatusRequest {
            scan_id: scan_id.clone(),
        });
        let final_status_response = service.get_scan_status(final_status_request).await.unwrap();
        let final_status = final_status_response.into_inner();

        assert_eq!(final_status.state, i32::from(ScanState::ScanCompleted));
        assert_eq!(final_status.current_point, 3);
        assert!((final_status.progress_percent - 100.0).abs() < 0.1);
    }
}
