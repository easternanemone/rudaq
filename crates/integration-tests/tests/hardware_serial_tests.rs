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

use hardware::drivers::mock_serial;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::{Duration, timeout};

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
                .write_all(format!("QUERY{i}\r").as_bytes())
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
                .write_all(format!("CMD{i}\r").as_bytes())
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
        harness.expect_write(format!("CMD{i}\r").as_bytes()).await;
        tokio::time::sleep(Duration::from_millis(10)).await; // Simulate processing delay
        harness
            .send_response(format!("ACK{i}\r\n").as_bytes())
            .unwrap();
    }

    let responses = app_task.await.unwrap();
    assert_eq!(responses, vec!["ACK0", "ACK1", "ACK2", "ACK3", "ACK4"]);
}

// =============================================================================
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
            Ok(Err(e)) => Err(format!("IO error: {e}")),
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
                .write_all(format!("FAST{i}\r").as_bytes())
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
        harness.expect_write(format!("FAST{i}\r").as_bytes()).await;
        harness
            .send_response(format!("OK{i}\r\n").as_bytes())
            .unwrap();
    }

    let responses = app_task.await.unwrap();
    assert_eq!(responses.len(), 10);
    for i in 0..10 {
        assert_eq!(responses[i], format!("OK{i}"));
    }
}
