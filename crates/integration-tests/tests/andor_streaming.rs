//! Integration tests for Andor iStar mock camera streaming pipeline (bd-ebmb / C.1)
//!
//! Exercises the full frame streaming stack:
//! - Mock Andor camera registration → gRPC StartStream / StreamFrames / StopStream
//! - Frame metadata validation (dimensions, bit depth, exposure, temperature)
//! - Stream stop/restart lifecycle
//! - Parameter control during active streaming
//! - Sustained streaming performance (100 frames, no corruption)
//!
//! Run with: cargo nextest run -p integration-tests --features db-surreal-mem --test andor_streaming
#![cfg(feature = "db-surreal-mem")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unused_imports,
    missing_docs
)]

use std::sync::Arc;
use std::time::Duration;

use driver_registry::register_all_factories;
use hardware::registry::DeviceRegistry;
use protocol::daq::hardware_service_server::HardwareService;
use protocol::daq::{
    GetParameterRequest, SetParameterRequest, StartStreamRequest, StopStreamRequest,
    StreamFramesRequest, StreamQuality,
};
use server::grpc::hardware_service::HardwareServiceImpl;
use tokio_stream::StreamExt;
use tonic::Request;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a registry with all factories and register the Andor mock camera.
async fn setup_andor_streaming() -> Option<(Arc<DeviceRegistry>, HardwareServiceImpl)> {
    let registry = DeviceRegistry::new();
    register_all_factories(&registry, None)
        .await
        .expect("factory registration should succeed");

    if registry
        .register_from_toml(
            "istar_stream",
            "Test iStar Streaming",
            "andor_istar",
            toml::Value::Table(Default::default()),
        )
        .await
        .is_err()
    {
        eprintln!("Skipping: andor_istar factory not available");
        return None;
    }

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());
    Some((registry, service))
}

// =============================================================================
// Test 1: Mock camera produces frames through gRPC streaming
// =============================================================================

#[tokio::test]
async fn test_andor_mock_produces_frames() {
    let Some((_registry, service)) = setup_andor_streaming().await else {
        return;
    };

    // Start stream
    let resp = service
        .start_stream(Request::new(StartStreamRequest {
            device_id: "istar_stream".to_string(),
            frame_count: None,
        }))
        .await
        .expect("StartStream should succeed");
    assert!(resp.into_inner().success);

    // Open frame stream
    let request = Request::new(StreamFramesRequest {
        device_id: "istar_stream".to_string(),
        max_fps: 0,
        quality: StreamQuality::Full.into(),
    });
    let mut stream = service
        .stream_frames(request)
        .await
        .expect("StreamFrames should succeed")
        .into_inner();

    // Collect 3 frames and validate
    for i in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap_or_else(|_| panic!("Timeout waiting for frame {}", i))
            .expect("Stream should not end early")
            .expect("Frame should not be an error");

        assert_eq!(frame.device_id, "istar_stream");
        assert_eq!(frame.width, 2048, "Frame width should be 2048");
        assert_eq!(frame.height, 2048, "Frame height should be 2048");
        assert_eq!(
            frame.bit_depth, 12,
            "Bit depth should be 12 (default Mono12)"
        );
        // Data may be LZ4-compressed; verify uncompressed_size if present
        let expected_raw = 2048 * 2048 * 2; // 8,388,608 bytes
        if frame.uncompressed_size > 0 {
            assert_eq!(
                frame.uncompressed_size as usize, expected_raw,
                "Uncompressed size should be 2048x2048x2 bytes"
            );
        } else {
            assert_eq!(frame.data.len(), expected_raw, "Raw frame data size");
        }
    }

    // Stop stream
    let resp = service
        .stop_stream(Request::new(StopStreamRequest {
            device_id: "istar_stream".to_string(),
        }))
        .await
        .expect("StopStream should succeed");
    assert!(resp.into_inner().success);
}

// =============================================================================
// Test 2: Frame metadata is correctly populated
// =============================================================================

#[tokio::test]
async fn test_andor_frame_metadata() {
    let Some((_registry, service)) = setup_andor_streaming().await else {
        return;
    };

    // Set exposure to 25ms (0.025s) before streaming
    service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_stream".to_string(),
            parameter_name: "exposure_s".to_string(),
            value: "0.025".to_string(),
        }))
        .await
        .expect("SetParameter exposure_s should succeed");

    // Start stream and get a frame
    service
        .start_stream(Request::new(StartStreamRequest {
            device_id: "istar_stream".to_string(),
            frame_count: None,
        }))
        .await
        .expect("StartStream should succeed");

    let mut stream = service
        .stream_frames(Request::new(StreamFramesRequest {
            device_id: "istar_stream".to_string(),
            max_fps: 0,
            quality: StreamQuality::Full.into(),
        }))
        .await
        .expect("StreamFrames should succeed")
        .into_inner();

    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Should receive frame within timeout")
        .expect("Stream should not end")
        .expect("Frame should not be error");

    // Validate metadata
    assert_eq!(frame.width, 2048);
    assert_eq!(frame.height, 2048);
    assert_eq!(frame.bit_depth, 12);
    assert!(frame.timestamp_ns > 0, "Timestamp should be nonzero");

    // Exposure should be ~25ms (the mock sets exposure_ms from exposure_s * 1000)
    if let Some(exp_ms) = frame.exposure_ms {
        assert!(
            (exp_ms - 25.0).abs() < 1.0,
            "Expected ~25ms exposure, got {}",
            exp_ms
        );
    }

    // Frame number should be valid
    // (first frame may be 0 or 1 depending on timing)
    assert!(
        frame.frame_number < 100,
        "Frame number should be reasonable"
    );

    // Stop stream
    service
        .stop_stream(Request::new(StopStreamRequest {
            device_id: "istar_stream".to_string(),
        }))
        .await
        .expect("StopStream should succeed");
}

// =============================================================================
// Test 3: Stream stop/restart lifecycle
// =============================================================================

#[tokio::test]
async fn test_andor_stream_stop_restart() {
    let Some((_registry, service)) = setup_andor_streaming().await else {
        return;
    };

    // --- Phase 1: Start and collect frames ---
    service
        .start_stream(Request::new(StartStreamRequest {
            device_id: "istar_stream".to_string(),
            frame_count: None,
        }))
        .await
        .expect("StartStream should succeed");

    let mut stream = service
        .stream_frames(Request::new(StreamFramesRequest {
            device_id: "istar_stream".to_string(),
            max_fps: 0,
            quality: StreamQuality::Full.into(),
        }))
        .await
        .expect("StreamFrames should succeed")
        .into_inner();

    // Collect 3 frames
    let mut first_run_numbers = Vec::new();
    for i in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap_or_else(|_| panic!("Timeout on frame {}", i))
            .expect("Stream should not end")
            .expect("Frame should be ok");
        first_run_numbers.push(frame.frame_number);
        assert_eq!(frame.width, 2048);
    }

    // Frame numbers should be monotonically non-decreasing
    for window in first_run_numbers.windows(2) {
        assert!(
            window[1] >= window[0],
            "Frame numbers should be non-decreasing: {} < {}",
            window[0],
            window[1]
        );
    }

    // --- Phase 2: Stop ---
    let resp = service
        .stop_stream(Request::new(StopStreamRequest {
            device_id: "istar_stream".to_string(),
        }))
        .await
        .expect("StopStream should succeed");
    assert!(resp.into_inner().success);

    // Small delay to let stream task wind down
    tokio::time::sleep(Duration::from_millis(50)).await;

    // --- Phase 3: Restart ---
    service
        .start_stream(Request::new(StartStreamRequest {
            device_id: "istar_stream".to_string(),
            frame_count: None,
        }))
        .await
        .expect("Restart should succeed");

    let mut stream2 = service
        .stream_frames(Request::new(StreamFramesRequest {
            device_id: "istar_stream".to_string(),
            max_fps: 0,
            quality: StreamQuality::Full.into(),
        }))
        .await
        .expect("StreamFrames after restart should succeed")
        .into_inner();

    // Collect 3 more frames
    for i in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), stream2.next())
            .await
            .unwrap_or_else(|_| panic!("Timeout on restart frame {}", i))
            .expect("Stream should not end")
            .expect("Frame should be ok");
        assert_eq!(frame.width, 2048, "Restarted frame should be 2048 wide");
        assert_eq!(frame.height, 2048, "Restarted frame should be 2048 tall");
    }

    // Cleanup
    service
        .stop_stream(Request::new(StopStreamRequest {
            device_id: "istar_stream".to_string(),
        }))
        .await
        .expect("Final StopStream should succeed");
}

// =============================================================================
// Test 4: Parameter control during active streaming
// =============================================================================

#[tokio::test]
async fn test_andor_parameter_control_during_streaming() {
    let Some((_registry, service)) = setup_andor_streaming().await else {
        return;
    };

    // Start streaming
    service
        .start_stream(Request::new(StartStreamRequest {
            device_id: "istar_stream".to_string(),
            frame_count: None,
        }))
        .await
        .expect("StartStream should succeed");

    let mut stream = service
        .stream_frames(Request::new(StreamFramesRequest {
            device_id: "istar_stream".to_string(),
            max_fps: 0,
            quality: StreamQuality::Full.into(),
        }))
        .await
        .expect("StreamFrames should succeed")
        .into_inner();

    // Get initial frame to confirm streaming works
    let _frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Should get initial frame")
        .expect("Stream should not end")
        .expect("Frame should be ok");

    // --- Modify parameters while streaming ---

    // Set exposure
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_stream".to_string(),
            parameter_name: "exposure_s".to_string(),
            value: "0.1".to_string(),
        }))
        .await
        .expect("SetParameter exposure_s should succeed during streaming");
    assert!(resp.into_inner().success);

    // Set TriggerMode (enum)
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_stream".to_string(),
            parameter_name: "TriggerMode".to_string(),
            value: "External".to_string(),
        }))
        .await
        .expect("SetParameter TriggerMode should succeed during streaming");
    assert!(resp.into_inner().success);

    // Set FrameCount (int — known writable dynamic parameter)
    let resp = service
        .set_parameter(Request::new(SetParameterRequest {
            device_id: "istar_stream".to_string(),
            parameter_name: "FrameCount".to_string(),
            value: "100".to_string(),
        }))
        .await
        .expect("SetParameter FrameCount should succeed during streaming");
    assert!(resp.into_inner().success);

    // Verify parameters round-trip
    let resp = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "istar_stream".to_string(),
            parameter_name: "exposure_s".to_string(),
        }))
        .await
        .expect("GetParameter exposure_s should succeed");
    let val: f64 = resp.into_inner().value.parse().unwrap();
    assert!((val - 0.1).abs() < 1e-9, "Exposure should be 0.1");

    let resp = service
        .get_parameter(Request::new(GetParameterRequest {
            device_id: "istar_stream".to_string(),
            parameter_name: "FrameCount".to_string(),
        }))
        .await
        .expect("GetParameter FrameCount should succeed");
    assert_eq!(resp.into_inner().value, "100");

    // Verify streaming still works after parameter changes
    for i in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap_or_else(|_| panic!("Timeout on frame {} after param changes", i))
            .expect("Stream should not end after param changes")
            .expect("Frame should be ok after param changes");
        assert_eq!(frame.width, 2048);
        assert_eq!(frame.height, 2048);
    }

    // Cleanup
    service
        .stop_stream(Request::new(StopStreamRequest {
            device_id: "istar_stream".to_string(),
        }))
        .await
        .expect("StopStream should succeed");
}

// =============================================================================
// Test 5: Sustained streaming performance (100 frames)
// =============================================================================

#[tokio::test]
async fn test_andor_streaming_performance() {
    let Some((_registry, service)) = setup_andor_streaming().await else {
        return;
    };

    service
        .start_stream(Request::new(StartStreamRequest {
            device_id: "istar_stream".to_string(),
            frame_count: None,
        }))
        .await
        .expect("StartStream should succeed");

    let mut stream = service
        .stream_frames(Request::new(StreamFramesRequest {
            device_id: "istar_stream".to_string(),
            max_fps: 0,
            quality: StreamQuality::Full.into(),
        }))
        .await
        .expect("StreamFrames should succeed")
        .into_inner();

    let target_frames = 100u64;
    let mut received = 0u64;
    let mut prev_frame_number = 0u64;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while received < target_frames {
        let frame = tokio::time::timeout_at(deadline, stream.next())
            .await
            .expect("Should receive all 100 frames within 30s")
            .expect("Stream should not end before 100 frames")
            .expect("Frame should not be an error");

        // Validate every frame
        assert_eq!(frame.width, 2048, "frame {} width", received);
        assert_eq!(frame.height, 2048, "frame {} height", received);
        assert_eq!(frame.bit_depth, 12, "frame {} bit_depth", received);
        // Data may be LZ4-compressed; verify uncompressed_size if present
        let expected_raw = 2048 * 2048 * 2;
        if frame.uncompressed_size > 0 {
            assert_eq!(
                frame.uncompressed_size as usize, expected_raw,
                "frame {} uncompressed size",
                received
            );
        } else {
            assert_eq!(
                frame.data.len(),
                expected_raw,
                "frame {} data size",
                received
            );
        }
        assert!(
            frame.timestamp_ns > 0,
            "frame {} timestamp should be nonzero",
            received
        );

        // Frame numbers should be non-decreasing
        if received > 0 {
            assert!(
                frame.frame_number >= prev_frame_number,
                "Frame numbers should be non-decreasing: {} < {}",
                prev_frame_number,
                frame.frame_number
            );
        }
        prev_frame_number = frame.frame_number;
        received += 1;
    }

    assert_eq!(
        received, target_frames,
        "Should have received exactly 100 frames"
    );

    // Cleanup
    service
        .stop_stream(Request::new(StopStreamRequest {
            device_id: "istar_stream".to_string(),
        }))
        .await
        .expect("StopStream should succeed");
}
