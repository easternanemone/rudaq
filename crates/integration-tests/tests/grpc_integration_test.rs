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
//! Integration tests for gRPC camera streaming paths (bd-fxzu)
//!
//! Tests:
//! 1. Registry camera path with MockCamera
//! 2. Frame count tracking
//! 3. Frame streaming rate limiting

#[cfg(feature = "server")]
mod camera_integration_tests {
    use hardware::registry::{DeviceRegistry, register_mock_factories};
    use protocol::daq::hardware_service_server::HardwareService;
    use protocol::daq::{
        ArmRequest, DeviceStateRequest, ListDevicesRequest, StartStreamRequest, StopStreamRequest,
        StreamFramesRequest, StreamQuality, TriggerRequest,
    };
    use server::grpc::hardware_service::HardwareServiceImpl;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::time::timeout;
    use tokio_stream::StreamExt;
    use tonic::Request;

    /// Create a registry with MockCamera for testing
    async fn create_camera_registry() -> DeviceRegistry {
        let registry = DeviceRegistry::new();
        register_mock_factories(&registry);

        // Register MockCamera
        registry
            .register_from_toml(
                "test_camera",
                "Test MockCamera",
                "mock_camera",
                toml::toml! {
                    width = 640
                    height = 480
                }
                .into(),
            )
            .await
            .unwrap();

        registry
    }

    /// Test: Registry camera path - device listing shows camera capabilities
    #[tokio::test]
    async fn test_camera_appears_in_registry_with_correct_capabilities() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(registry);

        let request = Request::new(ListDevicesRequest {
            capability_filter: None,
        });
        let response = service.list_devices(request).await.unwrap();
        let devices = response.into_inner().devices;

        assert_eq!(devices.len(), 1);
        let camera = &devices[0];
        assert_eq!(camera.id, "test_camera");
        assert!(
            camera.capabilities.contains(&"triggerable".to_string()),
            "Camera should be triggerable"
        );
        assert!(
            camera.capabilities.contains(&"frame_producer".to_string()),
            "Camera should be frame producer"
        );
        assert!(
            camera
                .capabilities
                .contains(&"exposure_controllable".to_string()),
            "Camera should have exposure control"
        );
    }

    /// Test: List devices with capability filter for triggerable
    #[tokio::test]
    async fn test_filter_devices_by_triggerable_capability() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
        let service = HardwareServiceImpl::new(registry);

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
            quality: StreamQuality::Full.into(),
        });
        let mut stream = service.stream_frames(request).await.unwrap().into_inner();

        let start = Instant::now();
        let mut frames = Vec::new();
        let mut last_metrics = None;

        // Use a longer initial timeout (2s) to handle mock camera startup latency,
        // then shorter timeouts (300ms) for subsequent frames. This prevents flaky
        // failures on loaded CI machines where the first frame takes longer to arrive.
        let mut first_frame = true;
        while start.elapsed() < Duration::from_secs(5) {
            let frame_timeout = if first_frame {
                Duration::from_secs(2)
            } else {
                Duration::from_millis(300)
            };
            match timeout(frame_timeout, stream.next()).await {
                Ok(Some(Ok(frame))) => {
                    first_frame = false;
                    last_metrics = frame.metrics.clone();
                    frames.push(frame);
                }
                Ok(Some(Err(err))) => panic!("stream error: {err}"),
                Ok(None) => break,
                Err(_) if !first_frame => break,
                Err(_) => {
                    // First frame timed out — this is the flaky case.
                    // Continue trying rather than giving up immediately.
                }
            }
        }

        let elapsed = start.elapsed().as_secs_f64().max(0.1);
        #[allow(clippy::cast_precision_loss)]
        // SAFETY: test/benchmark values are bounded
        let fps = frames.len() as f64 / elapsed;
        assert!(fps <= 14.0, "rate limiter should cap fps, got {fps}");
        // At least some frames should arrive (relaxed for CI variability)
        assert!(
            !frames.is_empty(),
            "expected at least some frames, got none"
        );

        if let Some(metrics) = last_metrics {
            assert!(metrics.frames_sent >= frames.len() as u64);
            assert!(metrics.avg_latency_ms >= 0.0);
        }

        service
            .stop_stream(Request::new(StopStreamRequest {
                device_id: "test_camera".to_string(),
            }))
            .await
            .unwrap();
    }

    /// Stress test: 60s sustained streaming (ignored by default).
    #[tokio::test]
    #[ignore = "long-running stress test; run manually with --ignored"]
    async fn test_stream_frames_sustained_60s() {
        let registry = create_camera_registry().await;
        let service = HardwareServiceImpl::new(registry);

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
            quality: StreamQuality::Full.into(),
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
                Ok(Some(Err(err))) => panic!("stream error: {err}"),
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
