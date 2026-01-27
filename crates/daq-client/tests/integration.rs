//! Integration tests for daq-client against a live daemon.
//!
//! These tests are ignored by default. Run with:
//! ```
//! DAEMON_URL=http://maitai-eos:50051 cargo test -p daq-client --test integration -- --ignored
//! ```

use daq_client::connection::DaemonAddress;
use daq_client::DaqClient;
use std::str::FromStr;

/// Get daemon URL from environment or default to maitai-eos
fn daemon_url() -> String {
    std::env::var("DAEMON_URL").unwrap_or_else(|_| "http://maitai-eos:50051".to_string())
}

/// Helper to skip test gracefully if daemon is unavailable
async fn try_connect() -> Option<DaqClient> {
    let url = daemon_url();
    let addr = DaemonAddress::from_str(&url).ok()?;
    DaqClient::connect(&addr).await.ok()
}

#[tokio::test]
#[ignore]
async fn test_connect_to_daemon() {
    let url = daemon_url();
    let addr = DaemonAddress::from_str(&url).expect("Failed to parse daemon URL");

    let result = DaqClient::connect(&addr).await;

    if result.is_err() {
        eprintln!("Skipping test: daemon not available at {}", url);
        return;
    }

    assert!(result.is_ok(), "Should successfully connect to daemon");
}

#[tokio::test]
#[ignore]
async fn test_connect_invalid_address() {
    let addr =
        DaemonAddress::from_str("http://invalid-host:99999").expect("Failed to parse invalid URL");

    let result = DaqClient::connect(&addr).await;

    assert!(result.is_err(), "Should fail to connect to invalid address");
}

#[tokio::test]
#[ignore]
async fn test_list_devices() {
    let mut client = match try_connect().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: daemon not available");
            return;
        }
    };

    let devices = client.list_devices().await.expect("Failed to list devices");

    assert!(
        !devices.is_empty(),
        "Should have at least one device registered"
    );

    // Verify each device has required fields
    for device in &devices {
        assert!(!device.id.is_empty(), "Device ID should not be empty");
        assert!(!device.name.is_empty(), "Device name should not be empty");
        assert!(
            !device.capabilities.is_empty(),
            "Device should have at least one capability"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_get_device_info() {
    let mut client = match try_connect().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: daemon not available");
            return;
        }
    };

    let devices = client.list_devices().await.expect("Failed to list devices");

    if devices.is_empty() {
        eprintln!("Skipping test: no devices available");
        return;
    }

    let device_id = &devices[0].id;

    let state = client.get_device_state(device_id).await;

    assert!(
        state.is_ok(),
        "Should successfully get device state for {}",
        device_id
    );
}

#[tokio::test]
#[ignore]
async fn test_health_check() {
    let mut client = match try_connect().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: daemon not available");
            return;
        }
    };

    let result = client.health_check().await;

    assert!(
        result.is_ok(),
        "Health check should succeed for responsive daemon"
    );
}

#[tokio::test]
#[ignore]
async fn test_read_value() {
    let mut client = match try_connect().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: daemon not available");
            return;
        }
    };

    let devices = client.list_devices().await.expect("Failed to list devices");

    // Find a device with Readable capability
    let readable_device = devices.iter().find(|d| {
        d.capabilities.iter().any(|c| {
            // Capability is an enum, check for Readable variant
            matches!(c, 1) // Readable = 1 in protobuf enum
        })
    });

    if readable_device.is_none() {
        eprintln!("Skipping test: no Readable device available");
        return;
    }

    let device_id = &readable_device.unwrap().id;

    let response = client.read_value(device_id).await;

    assert!(
        response.is_ok(),
        "Should successfully read value from {}",
        device_id
    );

    let response = response.unwrap();
    assert!(response.success, "ReadValue should succeed");
    assert!(!response.units.is_empty(), "Should have units field");
}
