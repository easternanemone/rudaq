#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_imports,
    unused_mut,
    missing_docs
)]
#![cfg(feature = "server")]

use anyhow::Result;
use daq_driver_pvcam::PvcamDriver;
use daq_proto::daq::hardware_service_server::HardwareService;
use daq_proto::daq::{
    GetParameterRequest, SetParameterRequest, StartStreamRequest, StopStreamRequest,
    StreamFramesRequest, StreamQuality,
};
use daq_server::grpc::hardware_service::HardwareServiceImpl;
use rust_daq::hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::StreamExt;
use tonic::Request;

#[tokio::test]
async fn test_grpc_camera_control_stream() -> Result<()> {
    // 1. Setup: Register PvcamDriver (Mock Mode)
    let mut registry = DeviceRegistry::new();

    // Register directly using DriverType::Pvcam (assuming new_async supports mock internally)
    // Note: We need to use DriverType enum which might require feature flags in daq-hardware.
    // However, integrations usually instantiate driver directly or rely on registry logic.
    // Let's use direct registry insertion if possible or standard config registration.

    // Since we are in integration test context, we use the standard registry flow.
    // Need to ensure DriverType::Pvcam is available (requires feature 'pvcam' or 'all_hardware').
    // The registry instantiation logic will create PvcamDriver.

    // DriverType::Pvcam is gated by #[cfg(feature = "pvcam")].
    // all_hardware includes pvcam (mock driver).
    // So ensuring 'rust-daq' compiles with 'server' (and defaults 'all_hardware') should be enough.

    registry
        .register(DeviceConfig {
            id: "prime_bsi".to_string(),
            name: "Prime BSI Express".to_string(),
            driver: DriverType::Pvcam {
                camera_name: "MockCamera".to_string(),
            },
        })
        .await?;

    let registry = Arc::new(registry);
    let service = HardwareServiceImpl::new(registry.clone());

    // 2. Verify Parameter Control (Exposure)
    // Exposure default is 100.0 ms

    // Set Exposure to 50.0 ms via gRPC
    // Note: PvcamDriver exposure is "acquisition.exposure_ms" (f64)
    let request = Request::new(SetParameterRequest {
        device_id: "prime_bsi".to_string(),
        parameter_name: "acquisition.exposure_ms".to_string(),
        value: "50.0".to_string(),
    });
    let response = service.set_parameter(request).await?;
    let set_resp = response.into_inner();
    assert!(set_resp.success, "Failed to set exposure");
    assert_eq!(set_resp.actual_value, "50.0"); // JSON serializes f64 as "50.0"

    // Verify Readback
    let request = Request::new(GetParameterRequest {
        device_id: "prime_bsi".to_string(),
        parameter_name: "acquisition.exposure_ms".to_string(),
    });
    let response = service.get_parameter(request).await?;
    let val_str = response.into_inner().value;
    assert_eq!(val_str.parse::<f64>()?, 50.0);

    // 3. Verify Streaming
    // Start Stream via gRPC
    let request = Request::new(StartStreamRequest {
        device_id: "prime_bsi".to_string(),
        frame_count: None, // Continuous
    });
    service.start_stream(request).await?;

    // Subscribe to Frames
    let request = Request::new(StreamFramesRequest {
        device_id: "prime_bsi".to_string(),
        max_fps: 0,
        quality: StreamQuality::Full.into(),
    });
    let mut stream = service.stream_frames(request).await?.into_inner();

    // Verify getting frames
    // Collect 3 frames
    for i in 0..3 {
        let frame_res = tokio::time::timeout(Duration::from_secs(2), stream.next()).await;

        match frame_res {
            Ok(Some(Ok(frame))) => {
                // Verify Metadata
                assert_eq!(frame.device_id, "prime_bsi");
                assert_eq!(frame.width, 2048); // Mock default
                assert_eq!(frame.height, 2048);
                // Verify exposure in frame metadata (bd-183h)
                if let Some(exp) = frame.exposure_ms {
                    assert_eq!(exp, 50.0, "Frame metadata mismatch exposure");
                }
                println!("Received frame {} with ts: {}", i, frame.timestamp_ns);
            }
            Ok(Some(Err(e))) => panic!("Stream error: {}", e),
            Ok(None) => panic!("Stream ended prematurely"),
            Err(_) => panic!("Timeout waiting for frame {}", i),
        }
    }

    // Stop Stream
    let request = Request::new(StopStreamRequest {
        device_id: "prime_bsi".to_string(),
    });
    let response = service.stop_stream(request).await?;
    let stop_resp = response.into_inner();
    assert!(stop_resp.success);
    // Frame count should be at least 3 (might be more due to async)
    assert!(stop_resp.frames_captured >= 3);

    Ok(())
}
