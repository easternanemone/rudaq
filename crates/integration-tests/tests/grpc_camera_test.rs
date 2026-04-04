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
use driver_pvcam::PvcamDriver;
use hardware::registry::DeviceRegistry;
use protocol::daq::hardware_service_server::HardwareService;
use protocol::daq::{
    GetParameterRequest, SetParameterRequest, StartStreamRequest, StopStreamRequest,
    StreamFramesRequest, StreamQuality,
};
use server::grpc::hardware_service::HardwareServiceImpl;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::StreamExt;
use tonic::Request;

#[tokio::test]
async fn test_grpc_camera_control_stream() -> Result<()> {
    // 1. Setup: Register PvcamDriver (Mock Mode)
    let registry = DeviceRegistry::new();

    #[cfg(feature = "pvcam")]
    {
        use driver_pvcam::PvcamFactory;
        registry.register_factory(Box::new(PvcamFactory));

        registry
            .register_from_toml(
                "prime_bsi",
                "Prime BSI Express",
                "pvcam",
                toml::toml! {
                    camera_name = "MockCamera"
                }
                .into(),
            )
            .await?;
    }

    #[cfg(not(feature = "pvcam"))]
    {
        // Skip test if pvcam feature not enabled
        println!("Skipping test: pvcam feature not enabled");
        return Ok(());
    }

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
            Ok(Some(Err(e))) => panic!("Stream error: {e}"),
            Ok(None) => panic!("Stream ended prematurely"),
            Err(err) => panic!("Timeout waiting for frame {i}: {err}"),
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
