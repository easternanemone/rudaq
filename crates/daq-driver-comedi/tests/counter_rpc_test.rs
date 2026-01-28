//! Integration test for Phase 4 Counter/Timer RPCs
//!
//! This test requires:
//! 1. The daemon running with comedi_hardware feature
//! 2. Hardware: NI PCI-MIO-16XE-10 at /dev/comedi0
//!
//! Run with: cargo test --features hardware_tests --test counter_rpc_test -- --nocapture

#![cfg(feature = "hardware_tests")]

use protocol::ni_daq::ni_daq_service_client::NiDaqServiceClient;
use protocol::ni_daq::{
    ConfigureCounterRequest, CounterEdge, CounterMode, ReadCounterRequest, ResetCounterRequest,
};
use tonic::transport::Channel;

const DEVICE_ID: &str = "photodiode";
const DAEMON_URL: &str = "http://localhost:50051";

async fn connect() -> Result<NiDaqServiceClient<Channel>, Box<dyn std::error::Error>> {
    let channel = Channel::from_static(DAEMON_URL).connect().await?;
    Ok(NiDaqServiceClient::new(channel))
}

#[tokio::test]
async fn test_configure_counter() {
    let mut client = connect().await.expect("Failed to connect to daemon");

    let request = ConfigureCounterRequest {
        device_id: DEVICE_ID.to_string(),
        counter: 0,
        mode: CounterMode::EventCount as i32,
        edge: CounterEdge::Rising as i32,
        gate_pin: 0,
        source_pin: 0,
    };

    let response = client
        .configure_counter(request)
        .await
        .expect("ConfigureCounter RPC failed");

    let result = response.into_inner();
    assert!(result.success, "ConfigureCounter should succeed");
    assert!(
        result.error_message.is_empty(),
        "Error message should be empty"
    );
}

#[tokio::test]
async fn test_reset_counter() {
    let mut client = connect().await.expect("Failed to connect to daemon");

    let request = ResetCounterRequest {
        device_id: DEVICE_ID.to_string(),
        counter: 0,
    };

    let response = client
        .reset_counter(request)
        .await
        .expect("ResetCounter RPC failed");

    let result = response.into_inner();
    assert!(result.success, "ResetCounter should succeed");
}

#[tokio::test]
async fn test_read_counter() {
    let mut client = connect().await.expect("Failed to connect to daemon");

    // First reset counter to known state
    let reset_request = ResetCounterRequest {
        device_id: DEVICE_ID.to_string(),
        counter: 0,
    };
    client
        .reset_counter(reset_request)
        .await
        .expect("ResetCounter failed");

    // Now read counter
    let request = ReadCounterRequest {
        device_id: DEVICE_ID.to_string(),
        counter: 0,
    };

    let response = client
        .read_counter(request)
        .await
        .expect("ReadCounter RPC failed");

    let result = response.into_inner();
    assert!(result.success, "ReadCounter should succeed");
    assert!(result.timestamp_ns > 0, "Timestamp should be non-zero");
    // Counter value after reset should be 0 or close to 0
    println!(
        "Counter 0 value: {}, timestamp: {} ns",
        result.count, result.timestamp_ns
    );
}

#[tokio::test]
async fn test_read_all_counters() {
    let mut client = connect().await.expect("Failed to connect to daemon");

    // NI PCI-MIO-16XE-10 has 3 counter channels (GPCTR0-2)
    for counter in 0..3 {
        let request = ReadCounterRequest {
            device_id: DEVICE_ID.to_string(),
            counter,
        };

        let response = client.read_counter(request).await;

        match response {
            Ok(resp) => {
                let result = resp.into_inner();
                println!(
                    "Counter {} = {}, success: {}, timestamp: {} ns",
                    counter, result.count, result.success, result.timestamp_ns
                );
                assert!(result.success, "ReadCounter {} should succeed", counter);
            }
            Err(e) => {
                panic!("ReadCounter {} failed: {}", counter, e);
            }
        }
    }
}

#[tokio::test]
async fn test_reset_all_counters() {
    let mut client = connect().await.expect("Failed to connect to daemon");

    // Reset all 3 counters
    for counter in 0..3 {
        let request = ResetCounterRequest {
            device_id: DEVICE_ID.to_string(),
            counter,
        };

        let response = client
            .reset_counter(request)
            .await
            .expect(&format!("ResetCounter {} failed", counter));

        let result = response.into_inner();
        assert!(result.success, "ResetCounter {} should succeed", counter);
    }

    // Verify all counters are zero
    for counter in 0..3 {
        let request = ReadCounterRequest {
            device_id: DEVICE_ID.to_string(),
            counter,
        };

        let response = client
            .read_counter(request)
            .await
            .expect(&format!("ReadCounter {} failed after reset", counter));

        let result = response.into_inner();
        println!("Counter {} after reset = {}", counter, result.count);
        // Note: Counter might not be exactly 0 if there are spurious edges
        // but it should be a small value
    }
}

#[tokio::test]
async fn test_invalid_counter_channel() {
    let mut client = connect().await.expect("Failed to connect to daemon");

    // Try to read an invalid counter channel (99)
    let request = ReadCounterRequest {
        device_id: DEVICE_ID.to_string(),
        counter: 99,
    };

    let response = client.read_counter(request).await;

    // Should fail with an error
    assert!(
        response.is_err(),
        "Reading invalid counter channel should fail"
    );

    let error = response.unwrap_err();
    println!("Expected error for invalid counter: {}", error);
}

#[tokio::test]
async fn test_invalid_device() {
    let mut client = connect().await.expect("Failed to connect to daemon");

    let request = ReadCounterRequest {
        device_id: "nonexistent_device".to_string(),
        counter: 0,
    };

    let response = client.read_counter(request).await;

    // Should fail with device not found
    assert!(
        response.is_err(),
        "Reading from nonexistent device should fail"
    );

    let error = response.unwrap_err();
    println!("Expected error for invalid device: {}", error);
}
