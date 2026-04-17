use crate::grpc::proto::run_engine_service_server::RunEngineServiceServer;
use crate::grpc::proto::{DaemonInfoRequest, DaemonInfoResponse, SystemStatus};
#[cfg(feature = "scripting")]
use crate::grpc::proto::{
    ListExecutionsRequest, ListExecutionsResponse, ListScriptsRequest, ListScriptsResponse,
    ScriptInfo, ScriptStatus, StartRequest, StartResponse, StatusRequest, StopRequest,
    StopResponse,
    control_service_server::{ControlService, ControlServiceServer},
};
use crate::grpc::run_engine_service::RunEngineServiceImpl;
#[cfg(feature = "serial")]
use crate::grpc::{PluginServiceImpl, PluginServiceServer};
use common::core::Measurement;
#[cfg(feature = "scripting")]
use common::limits;
#[cfg(feature = "scripting")]
use scripting::ScriptEngine; // Trait import
// use common::error::DaqError; // Unused
#[cfg(feature = "scripting")]
use experiment::RunEngine;
use protocol::daq::{UploadRequest, UploadResponse};
#[cfg(feature = "scripting")]
use scripting::RhaiEngine;
#[cfg(feature = "scripting")]
use scripting::ScriptPlanRunner;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "storage_hdf5")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;
#[cfg(feature = "storage_hdf5")]
use tokio::sync::mpsc;
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::service::interceptor::interceptor;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use uuid::Uuid;

#[cfg(test)]
use crate::config::GrpcSettings;
use crate::config::ServerConfig;

mod auth;
mod daq_server;
mod measurement_pipeline;
mod startup;
mod web_ui;

pub use daq_server::DaqServer;
pub use startup::{start_server, start_server_with_hardware};

use auth::{build_cors_layer, build_tls_config, validate_auth};
#[cfg(test)]
use measurement_pipeline::ImageSequenceFault;
use measurement_pipeline::{
    annotate_measurement_data_integrity, build_image_measurement, encode_measurement_frame,
    load_echelle_profile_for_device, log_data_integrity_fault, metadata_string,
    spectrum_payload_from_parts,
};
use web_ui::WebUiLayer;
// `DaqServer` struct, its `impl` blocks, and the `ControlService` gRPC
// trait implementation moved to the `daq_server` sibling module (C1 step
// 4a). The struct is re-exported above. `build_grpc_server!` macro plus
// `start_server` / `start_server_with_hardware` live in the `startup`
// sibling module (C1 step 3), `WebUiLayer` and friends in `web_ui`
// (C1 step 2), auth/TLS/CORS helpers in `auth`, and measurement payload
// helpers in `measurement_pipeline`.

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_image_measurement(source: &str, frame_number: Option<u64>) -> Measurement {
        Measurement::Image {
            name: source.to_string(),
            width: 2,
            height: 2,
            buffer: common::core::PixelBuffer::U16(vec![1, 2, 3, 4]),
            unit: "counts".to_string(),
            metadata: common::core::ImageMetadata {
                frame_number,
                ..Default::default()
            },
            timestamp: Utc::now(),
        }
    }

    /// Create a test DaqServer with a mock RunEngine (bd-si2c)
    #[cfg(feature = "scripting")]
    fn create_test_server() -> DaqServer {
        let registry = hardware::registry::DeviceRegistry::new();
        let run_engine = std::sync::Arc::new(experiment::RunEngine::new(registry));
        let token = CancellationToken::new();
        #[cfg(feature = "storage_hdf5")]
        {
            DaqServer::new(None, run_engine, token).expect("failed to create test DaqServer")
        }
        #[cfg(not(feature = "storage_hdf5"))]
        {
            DaqServer::new(run_engine, token).expect("failed to create test DaqServer")
        }
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_upload_valid_script() {
        let server = create_test_server();
        let request = Request::new(UploadRequest {
            script_content: "let x = 42;".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });

        let response = server.upload_script(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!(!resp.script_id.is_empty());
        assert_eq!(resp.error_message, "");
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_upload_invalid_script() {
        let server = create_test_server();
        let request = Request::new(UploadRequest {
            script_content: "this is not valid rhai syntax {{{".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });

        let response = server.upload_script(request).await.unwrap();
        let resp = response.into_inner();

        assert!(!resp.success);
        assert!(resp.script_id.is_empty());
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_start_nonexistent_script() {
        let server = create_test_server();
        let request = Request::new(StartRequest {
            script_id: "nonexistent-id".to_string(),
            parameters: HashMap::new(),
        });

        let result = server.start_script(request).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_script_execution_lifecycle() {
        let server = create_test_server();

        // Upload script
        let upload_req = Request::new(UploadRequest {
            script_content: "let x = 1 + 1;".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });
        let upload_resp = server.upload_script(upload_req).await.unwrap().into_inner();
        assert!(upload_resp.success);

        // Start execution
        let start_req = Request::new(StartRequest {
            script_id: upload_resp.script_id,
            parameters: HashMap::new(),
        });
        let start_resp = server.start_script(start_req).await.unwrap().into_inner();
        assert!(start_resp.started);

        // Wait for completion
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Check status
        let status_req = Request::new(StatusRequest {
            execution_id: start_resp.execution_id,
        });
        let status_resp = server
            .get_script_status(status_req)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status_resp.state, "COMPLETED");
        assert_eq!(status_resp.error_message, "");
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_basic() {
        use tokio_stream::StreamExt;

        let server = create_test_server();

        // Get sender to simulate hardware
        let data_sender = server.data_sender();

        // Start streaming with no filters
        let request = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Spawn task to send mock data
        tokio::spawn(async move {
            for i in 0..5 {
                let _ = data_sender.send(Measurement::Scalar {
                    name: "test_channel".to_string(),
                    value: f64::from(i),
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        // Collect measurements
        let mut received = Vec::new();
        while let Some(result) = stream.next().await {
            let data_point = result.unwrap();
            received.push(data_point);
            if received.len() >= 5 {
                break;
            }
        }

        // Verify we got all 5 measurements
        assert_eq!(received.len(), 5);
        assert_eq!(received[0].channel, "test_channel");
        assert_eq!(received[0].value, 0.0);
        assert_eq!(received[4].value, 4.0);
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_channel_filter() {
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();

        // Request only "channel_a" measurements
        let request = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec!["channel_a".to_string()],
            max_rate_hz: 0,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Send mixed data
        tokio::spawn(async move {
            for i in 0..10 {
                let channel = if i % 2 == 0 { "channel_a" } else { "channel_b" };
                let _ = data_sender.send(Measurement::Scalar {
                    name: channel.to_string(),
                    value: f64::from(i),
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });

        // Collect filtered measurements
        let mut received = Vec::new();
        while let Some(result) = stream.next().await {
            let data_point = result.unwrap();
            received.push(data_point);
            if received.len() >= 5 {
                break;
            }
        }

        // Verify only channel_a was received
        assert_eq!(received.len(), 5);
        for data_point in &received {
            assert_eq!(data_point.channel, "channel_a");
        }

        // Verify values are even (0, 2, 4, 6, 8)
        assert_eq!(received[0].value, 0.0);
        assert_eq!(received[1].value, 2.0);
        assert_eq!(received[4].value, 8.0);
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_rate_limiting() {
        use std::time::Instant;
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();

        // Request max 10 Hz rate
        let request = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 10,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Send data faster than rate limit
        tokio::spawn(async move {
            for i in 0..20 {
                let _ = data_sender.send(Measurement::Scalar {
                    name: "test".to_string(),
                    value: f64::from(i),
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });

        // Measure time to receive 5 measurements
        let start = Instant::now();
        let mut count = 0;
        while let Some(result) = stream.next().await {
            result.unwrap();
            count += 1;
            if count >= 5 {
                break;
            }
        }
        let elapsed = start.elapsed();

        // At 10 Hz, 5 measurements should take ~400-500ms
        // (first is immediate, then 4 x 100ms intervals)
        assert!(
            elapsed.as_millis() >= 400,
            "Rate limiting not working: took {elapsed:?}"
        );
        assert!(
            elapsed.as_millis() < 700,
            "Rate limiting too slow: took {elapsed:?}"
        );
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_multiple_clients() {
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();

        // Start two concurrent streams
        let request1 = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });
        let request2 = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });

        let response1 = server.stream_measurements(request1).await.unwrap();
        let response2 = server.stream_measurements(request2).await.unwrap();

        let mut stream1 = response1.into_inner();
        let mut stream2 = response2.into_inner();

        // Send test data
        tokio::spawn(async move {
            for i in 0..3 {
                let _ = data_sender.send(Measurement::Scalar {
                    name: "shared".to_string(),
                    value: f64::from(i),
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        // Both clients should receive the same data
        let mut client1_data = Vec::new();
        let mut client2_data = Vec::new();

        for _ in 0..3 {
            if let Some(result) = stream1.next().await {
                client1_data.push(result.unwrap().value);
            }
            if let Some(result) = stream2.next().await {
                client2_data.push(result.unwrap().value);
            }
        }

        assert_eq!(client1_data.len(), 3);
        assert_eq!(client2_data.len(), 3);
        assert_eq!(client1_data, client2_data);
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_spectra_filters_by_device_id_metadata() {
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();
        let request = Request::new(crate::grpc::proto::SpectrumStreamRequest {
            channels: vec!["camera-a".to_string()],
            max_rate_hz: 0,
        });

        let response = server.stream_spectra(request).await.unwrap();
        let mut stream = response.into_inner();

        tokio::spawn(async move {
            let _ = data_sender.send(Measurement::Spectrum {
                name: "echelle_order_0".to_string(),
                frequencies: vec![500.0, 501.0],
                amplitudes: vec![1.0, 2.0],
                frequency_unit: Some("nm".to_string()),
                amplitude_unit: Some("counts".to_string()),
                metadata: Some(serde_json::json!({
                    "device_id": "camera-a",
                    "relative_index": 0
                })),
                timestamp: Utc::now(),
            });
        });

        let payload = stream
            .next()
            .await
            .expect("expected a streamed spectrum")
            .expect("expected streamed spectrum payload");

        assert_eq!(payload.name, "echelle_order_0");
        assert_eq!(payload.device_id, "camera-a");
        assert_eq!(payload.order_index, 0);
    }

    #[test]
    fn test_auth_rejects_missing_token() {
        let settings = GrpcSettings {
            auth_enabled: true,
            auth_token: Some("secret".to_string()),
            ..Default::default()
        };

        let request = Request::new(());
        let result = validate_auth(&settings, &request);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn test_auth_rejects_invalid_token() {
        let settings = GrpcSettings {
            auth_enabled: true,
            auth_token: Some("secret".to_string()),
            ..Default::default()
        };

        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer wrong".parse().unwrap());

        let result = validate_auth(&settings, &request);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn test_auth_allows_matching_token() {
        let settings = GrpcSettings {
            auth_enabled: true,
            auth_token: Some("secret".to_string()),
            ..Default::default()
        };

        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("x-api-key", "secret".parse().unwrap());

        let result = validate_auth(&settings, &request);

        assert!(result.is_ok());
    }

    #[test]
    fn test_build_image_measurement_preserves_frame_metadata() {
        let frame = common::data::Frame::from_u16(4, 2, &[1, 2, 3, 4, 5, 6, 7, 8])
            .with_frame_number(42)
            .with_exposure(12.5)
            .with_roi_offset(8, 16)
            .with_metadata(common::data::FrameMetadata {
                temperature_c: Some(-15.0),
                binning: Some((2, 4)),
                ..Default::default()
            });

        let measurement = build_image_measurement("camera-a", &frame);
        let Measurement::Image {
            name,
            width,
            height,
            buffer,
            metadata,
            ..
        } = measurement
        else {
            panic!("expected image measurement");
        };

        assert_eq!(name, "camera-a");
        assert_eq!((width, height), (4, 2));
        assert!(matches!(buffer, common::core::PixelBuffer::U16(_)));
        assert_eq!(metadata.exposure_ms, Some(12.5));
        assert_eq!(metadata.temperature_c, Some(-15.0));
        assert_eq!(metadata.binning, Some((2, 4)));
        assert_eq!(metadata.roi_origin, Some((8, 16)));
        assert_eq!(metadata.frame_number, Some(42));
        assert_eq!(metadata.sequence_gap_from_previous, None);
    }

    #[test]
    fn test_spectrum_payload_from_parts_defaults_without_metadata() {
        let payload = spectrum_payload_from_parts(
            "spectrum-a".to_string(),
            vec![500.0, 501.0],
            vec![1.0, 2.0],
            Some("nm".to_string()),
            Some("counts".to_string()),
            None,
            123,
        );

        assert_eq!(payload.name, "spectrum-a");
        assert_eq!(payload.wavelengths, vec![500.0, 501.0]);
        assert_eq!(payload.intensities, vec![1.0, 2.0]);
        assert_eq!(payload.wavelength_unit, "nm");
        assert_eq!(payload.intensity_unit, "counts");
        assert_eq!(payload.order_index, 0);
        assert!(!payload.merged);
        assert!(payload.ivar.is_empty());
        assert!(payload.quality_flags.is_empty());
        assert_eq!(payload.timestamp_ns, 123);
        assert!(payload.metadata_json.is_empty());
    }

    #[test]
    fn test_spectrum_payload_from_parts_parses_echelle_metadata() {
        let payload = spectrum_payload_from_parts(
            "echelle_merged_preview".to_string(),
            vec![500.0, 501.0],
            vec![1.0, 2.0],
            Some("nm".to_string()),
            Some("counts".to_string()),
            Some(serde_json::json!({
                "kind": "echelle_merged",
                "profile_id": "profile-42",
                "quality_flags": ["cosmic_masked", "blaze_corrected"],
                "snr_estimate": 18.5,
                "ivar": [0.25, 0.5]
            })),
            456,
        );

        assert!(payload.merged);
        assert_eq!(payload.order_index, -1);
        assert_eq!(payload.calibration_profile_id, "profile-42");
        assert_eq!(
            payload.quality_flags,
            vec!["cosmic_masked".to_string(), "blaze_corrected".to_string()]
        );
        assert_eq!(payload.snr_estimate, 18.5);
        assert_eq!(payload.ivar, vec![0.25, 0.5]);
        assert!(
            payload
                .metadata_json
                .contains("\"profile_id\":\"profile-42\"")
        );
    }

    #[test]
    fn test_annotate_measurement_data_integrity_marks_gap() {
        let mut last_frame_numbers = HashMap::new();
        let mut first = test_image_measurement("camera-a", Some(7));
        let mut second = test_image_measurement("camera-a", Some(10));

        assert!(annotate_measurement_data_integrity(&mut first, &mut last_frame_numbers).is_none());

        let (source, fault) =
            annotate_measurement_data_integrity(&mut second, &mut last_frame_numbers)
                .expect("expected sequence gap fault");
        let Measurement::Image { metadata, .. } = second else {
            panic!("expected image measurement");
        };

        assert_eq!(source, "camera-a");
        assert_eq!(
            fault,
            ImageSequenceFault {
                previous_frame_number: 7,
                current_frame_number: 10,
                missing_frames: 2,
            }
        );
        assert_eq!(metadata.sequence_gap_from_previous, Some(2));
        assert_eq!(last_frame_numbers.get("camera-a"), Some(&10));
    }

    #[test]
    fn test_annotate_measurement_data_integrity_ignores_regressions() {
        let mut last_frame_numbers = HashMap::from([("camera-a".to_string(), 10)]);
        let mut measurement = test_image_measurement("camera-a", Some(9));

        assert!(
            annotate_measurement_data_integrity(&mut measurement, &mut last_frame_numbers)
                .is_none()
        );
        let Measurement::Image { metadata, .. } = measurement else {
            panic!("expected image measurement");
        };
        assert_eq!(metadata.sequence_gap_from_previous, None);
        assert_eq!(last_frame_numbers.get("camera-a"), Some(&10));
    }
}
