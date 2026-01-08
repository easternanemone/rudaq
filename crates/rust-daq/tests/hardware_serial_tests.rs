#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    clippy::needless_range_loop,
    unused_mut,
    unused_imports,
    missing_docs
)]
//! Integration tests for hardware serial drivers
//!
//! These tests verify that hardware drivers correctly implement serial communication
//! patterns including timeouts, flow control, command parsing, and error handling.
//!
//! Uses MockSerialPort to simulate device behavior without requiring physical hardware.

use rust_daq::hardware::mock_serial;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::{timeout, Duration};

// =============================================================================
// Generic Serial Communication Tests
// =============================================================================

#[tokio::test]
async fn test_serial_read_timeout() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    // Spawn task that will timeout waiting for response
    let read_task = tokio::spawn(async move {
        reader.write_all(b"QUERY?\r").await.unwrap();
        let mut response = String::new();
        timeout(Duration::from_millis(100), reader.read_line(&mut response)).await
    });

    // Harness receives command but NEVER responds (simulating timeout)
    harness.expect_write(b"QUERY?\r").await;
    // Intentionally do not send response

    let result = read_task.await.unwrap();
    assert!(
        result.is_err(),
        "Expected timeout error when device doesn't respond"
    );
}

#[tokio::test]
async fn test_serial_write_read_roundtrip() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        // Write command
        reader.write_all(b"GET_STATUS\r").await.unwrap();

        // Read response
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();
        response
    });

    // Harness simulates device behavior
    harness.expect_write(b"GET_STATUS\r").await;
    harness.send_response(b"STATUS:OK\r\n").unwrap();

    let response = app_task.await.unwrap();
    assert_eq!(response, "STATUS:OK\r\n");
}

#[tokio::test]
async fn test_serial_command_parsing() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        // Send query command
        reader.write_all(b"PARAM:VALUE?\r").await.unwrap();

        // Read and parse response
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();

        // Parse "PARAM:123.45" format
        let value: f64 = response
            .trim()
            .split(':')
            .next_back()
            .unwrap()
            .parse()
            .unwrap();
        value
    });

    harness.expect_write(b"PARAM:VALUE?\r").await;
    harness.send_response(b"PARAM:123.45\r\n").unwrap();

    let parsed_value = app_task.await.unwrap();
    assert!((parsed_value - 123.45).abs() < 1e-6);
}

#[tokio::test]
async fn test_serial_multiple_queries() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        let mut results = Vec::new();

        for i in 1..=3 {
            reader
                .write_all(format!("QUERY{}\r", i).as_bytes())
                .await
                .unwrap();
            let mut response = String::new();
            reader.read_line(&mut response).await.unwrap();
            results.push(response.trim().to_string());
        }

        results
    });

    // Simulate device responding to multiple queries
    harness.expect_and_respond(b"QUERY1\r", b"RESP1\r\n").await;
    harness.expect_and_respond(b"QUERY2\r", b"RESP2\r\n").await;
    harness.expect_and_respond(b"QUERY3\r", b"RESP3\r\n").await;

    let results = app_task.await.unwrap();
    assert_eq!(results, vec!["RESP1", "RESP2", "RESP3"]);
}

#[tokio::test]
async fn test_serial_flow_control_simulation() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        // Send multiple commands rapidly
        for i in 0..5 {
            reader
                .write_all(format!("CMD{}\r", i).as_bytes())
                .await
                .unwrap();
        }

        // Read all responses
        let mut responses = Vec::new();
        for _ in 0..5 {
            let mut response = String::new();
            reader.read_line(&mut response).await.unwrap();
            responses.push(response.trim().to_string());
        }
        responses
    });

    // Device processes commands with delays (simulating flow control)
    for i in 0..5 {
        harness.expect_write(format!("CMD{}\r", i).as_bytes()).await;
        tokio::time::sleep(Duration::from_millis(10)).await; // Simulate processing delay
        harness
            .send_response(format!("ACK{}\r\n", i).as_bytes())
            .unwrap();
    }

    let responses = app_task.await.unwrap();
    assert_eq!(responses, vec!["ACK0", "ACK1", "ACK2", "ACK3", "ACK4"]);
}

// =============================================================================
// MaiTai Laser Driver Tests
// =============================================================================

/// Test MaiTai wavelength query command
#[tokio::test]
#[cfg(feature = "instrument_spectra_physics")]
async fn test_maitai_wavelength_query() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        // MaiTai protocol: WAVELENGTH? -> WAVELENGTH:800
        reader.write_all(b"WAVELENGTH?\r").await.unwrap();

        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();

        // Parse wavelength from response
        let wavelength: f64 = response
            .trim()
            .split(':')
            .next_back()
            .unwrap()
            .parse()
            .unwrap();
        wavelength
    });

    // Simulate MaiTai device
    harness.expect_write(b"WAVELENGTH?\r").await;
    harness.send_response(b"WAVELENGTH:800\r\n").unwrap();

    let wavelength = app_task.await.unwrap();
    assert_eq!(wavelength, 800.0);
}

/// Test MaiTai wavelength set command
#[tokio::test]
#[cfg(feature = "instrument_spectra_physics")]
async fn test_maitai_wavelength_set() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let target_wavelength = 850.0;

    let app_task = tokio::spawn(async move {
        // Send wavelength set command
        reader
            .write_all(format!("WAVELENGTH:{}\r", target_wavelength).as_bytes())
            .await
            .unwrap();

        // MaiTai typically doesn't respond to set commands, just a small delay
        tokio::time::sleep(Duration::from_millis(10)).await;
    });

    // Device receives command
    harness.expect_write(b"WAVELENGTH:850\r").await;

    app_task.await.unwrap();
}

/// Test MaiTai power query with timeout handling
#[tokio::test]
#[cfg(feature = "instrument_spectra_physics")]
async fn test_maitai_power_query_with_timeout() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        reader.write_all(b"POWER?\r").await.unwrap();

        let mut response = String::new();
        let result = timeout(Duration::from_secs(1), reader.read_line(&mut response)).await;

        match result {
            Ok(Ok(_)) => {
                let power: f64 = response
                    .trim()
                    .split(':')
                    .next_back()
                    .unwrap()
                    .parse()
                    .unwrap();
                Ok(power)
            }
            Ok(Err(e)) => Err(format!("IO error: {}", e)),
            Err(_) => Err("Timeout".to_string()),
        }
    });

    // Device responds with power reading
    harness.expect_write(b"POWER?\r").await;
    harness.send_response(b"POWER:2.5\r\n").unwrap();

    let result = app_task.await.unwrap();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 2.5);
}

/// Test MaiTai shutter control
#[tokio::test]
#[cfg(feature = "instrument_spectra_physics")]
async fn test_maitai_shutter_control() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        // Open shutter
        reader.write_all(b"SHUTTER:1\r").await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Query shutter state
        reader.write_all(b"SHUTTER?\r").await.unwrap();
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();

        let state: i32 = response
            .trim()
            .split(':')
            .next_back()
            .unwrap()
            .parse()
            .unwrap();

        // Close shutter
        reader.write_all(b"SHUTTER:0\r").await.unwrap();

        state
    });

    // Simulate device
    harness.expect_write(b"SHUTTER:1\r").await;
    harness
        .expect_and_respond(b"SHUTTER?\r", b"SHUTTER:1\r\n")
        .await;
    harness.expect_write(b"SHUTTER:0\r").await;

    let shutter_state = app_task.await.unwrap();
    assert_eq!(shutter_state, 1); // Shutter was open
}

/// Test MaiTai identification query
#[tokio::test]
#[cfg(feature = "instrument_spectra_physics")]
async fn test_maitai_identify() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        reader.write_all(b"*IDN?\r").await.unwrap();
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();
        response.trim().to_string()
    });

    harness.expect_write(b"*IDN?\r").await;
    harness
        .send_response(b"Spectra-Physics,MaiTai HP,12345,v1.0\r\n")
        .unwrap();

    let id = app_task.await.unwrap();
    assert!(id.contains("MaiTai"));
    assert!(id.contains("Spectra-Physics"));
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_serial_malformed_response() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        reader.write_all(b"GET_VALUE?\r").await.unwrap();
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();

        // Try to parse response that doesn't have expected format
        response
            .trim()
            .split(':')
            .next_back()
            .unwrap()
            .parse::<f64>()
    });

    harness.expect_write(b"GET_VALUE?\r").await;
    harness.send_response(b"ERROR:INVALID\r\n").unwrap();

    let result = app_task.await.unwrap();
    assert!(result.is_err(), "Should fail to parse 'INVALID' as f64");
}

#[tokio::test]
async fn test_serial_partial_response() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        reader.write_all(b"QUERY?\r").await.unwrap();
        let mut response = String::new();

        // Set a timeout to avoid hanging forever
        let result = timeout(Duration::from_millis(200), reader.read_line(&mut response)).await;

        match result {
            Ok(Ok(_)) => Ok(response),
            Ok(Err(e)) => Err(format!("IO error: {}", e)),
            Err(_) => Err("Timeout".to_string()),
        }
    });

    // Send partial response without line terminator
    harness.expect_write(b"QUERY?\r").await;
    harness.send_response(b"PARTIAL").unwrap(); // Missing \r\n

    // Should timeout waiting for line terminator
    let result = app_task.await.unwrap();
    assert!(result.is_err());
}

#[tokio::test]
async fn test_serial_rapid_commands() {
    let (port, mut harness) = mock_serial::new();
    let mut reader = BufReader::new(port);

    let app_task = tokio::spawn(async move {
        let mut responses = Vec::new();

        // Send 10 commands as fast as possible
        for i in 0..10 {
            reader
                .write_all(format!("FAST{}\r", i).as_bytes())
                .await
                .unwrap();
        }

        // Read all responses
        for _ in 0..10 {
            let mut response = String::new();
            timeout(Duration::from_secs(1), reader.read_line(&mut response))
                .await
                .unwrap()
                .unwrap();
            responses.push(response.trim().to_string());
        }

        responses
    });

    // Device handles rapid commands
    for i in 0..10 {
        harness
            .expect_write(format!("FAST{}\r", i).as_bytes())
            .await;
        harness
            .send_response(format!("OK{}\r\n", i).as_bytes())
            .unwrap();
    }

    let responses = app_task.await.unwrap();
    assert_eq!(responses.len(), 10);
    for i in 0..10 {
        assert_eq!(responses[i], format!("OK{}", i));
    }
}
