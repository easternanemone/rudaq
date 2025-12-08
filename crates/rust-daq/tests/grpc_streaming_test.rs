#![cfg(not(target_arch = "wasm32"))]
//! Integration test for gRPC streaming functionality
//!
//! Tests the stream_measurements RPC to verify real-time data streaming works correctly.

#[cfg(feature = "networking")]
mod streaming_tests {
    use chrono::Utc;
    use rust_daq::core::Measurement;
    use rust_daq::grpc::proto::MeasurementRequest;
    use rust_daq::grpc::{ControlService, DaqServer};
    use tokio_stream::StreamExt;
    use tonic::Request;

    #[tokio::test]
    async fn test_stream_receives_broadcast_data() {
        // Create server
        let server = DaqServer::default();

        // Get sender for hardware simulation
        let data_sender = server.data_sender();

        // Start streaming
        let request = Request::new(MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Simulate hardware sending data
        tokio::spawn(async move {
            for i in 0..3 {
                let _ = data_sender.send(Measurement::Scalar {
                    name: "test_channel".to_string(),
                    value: i as f64 * 10.0,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        // Collect streamed measurements
        let mut measurements = Vec::new();
        while let Some(result) = stream.next().await {
            let data_point = result.unwrap();
            measurements.push(data_point);
            if measurements.len() >= 3 {
                break;
            }
        }

        // Verify we got all measurements
        assert_eq!(measurements.len(), 3);
        assert_eq!(measurements[0].value, 0.0);
        assert_eq!(measurements[1].value, 10.0);
        assert_eq!(measurements[2].value, 20.0);
    }

    #[tokio::test]
    async fn test_channel_filtering() {
        let server = DaqServer::default();
        let data_sender = server.data_sender();

        // Request only "temperature" channel
        let request = Request::new(MeasurementRequest {
            channels: vec!["temperature".to_string()],
            max_rate_hz: 0,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Send mixed channel data
        tokio::spawn(async move {
            let channels = vec!["temperature", "pressure", "temperature", "voltage"];
            for (i, &channel) in channels.iter().enumerate() {
                let _ = data_sender.send(Measurement::Scalar {
                    name: channel.to_string(),
                    value: i as f64,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });

        // Collect filtered measurements
        let mut measurements = Vec::new();
        while let Some(result) = stream.next().await {
            let dp = result.unwrap();
            measurements.push(dp);
            if measurements.len() >= 2 {
                break;
            }
        }

        // Should only receive temperature measurements (indices 0 and 2)
        assert_eq!(measurements.len(), 2);
        assert_eq!(measurements[0].channel, "temperature");
        assert_eq!(measurements[0].value, 0.0);
        assert_eq!(measurements[1].channel, "temperature");
        assert_eq!(measurements[1].value, 2.0);
    }

    #[tokio::test]
    async fn test_multiple_concurrent_clients() {
        let server = DaqServer::default();
        let data_sender = server.data_sender();

        // Create two concurrent clients
        let request1 = Request::new(MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });
        let request2 = Request::new(MeasurementRequest {
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
                    value: i as f64 * 100.0,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        // Both clients collect data
        let mut client1_values = Vec::new();
        let mut client2_values = Vec::new();

        for _ in 0..3 {
            if let Some(result) = stream1.next().await {
                client1_values.push(result.unwrap().value);
            }
            if let Some(result) = stream2.next().await {
                client2_values.push(result.unwrap().value);
            }
        }

        // Both clients should receive identical data (broadcast pattern)
        assert_eq!(client1_values, vec![0.0, 100.0, 200.0]);
        assert_eq!(client2_values, vec![0.0, 100.0, 200.0]);
    }
}
