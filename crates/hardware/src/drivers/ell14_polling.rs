//! Polling tests for ELL14 Driver
use super::ell14::{Ell14Driver, ElliptecState, SharedPort};
use crate::drivers::mock_serial;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;

#[tokio::test]
async fn test_polling_task_broadcasts_state_updates() {
    // Create a mock driver
    let (port, mut harness) = mock_serial::new();
    let shared_port: SharedPort = Arc::new(Mutex::new(Box::new(port)));
    let driver = Ell14Driver::with_test_port(shared_port, "0", 398.2222);
    let mut rx = driver.subscribe();

    // Start polling (store handle to abort after test)
    let _polling_handle = driver.start_polling(Duration::from_millis(10));

    // Simulate device responses
    tokio::spawn(async move {
        for i in 0..5 {
            // Respond to position query
            harness.expect_write(b"0gp").await;
            harness
                .send_response(format!("0PO{:08X}", i * 100).as_bytes())
                .unwrap();

            // Respond to status query
            harness.expect_write(b"0gs").await;
            harness.send_response(b"0GS01").unwrap();

            // Respond to error query
            harness.expect_write(b"0ge").await;
            harness.send_response(b"0GE00").unwrap();
        }
    });

    // Wait for a few state updates
    for i in 0..5 {
        let state: ElliptecState = timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        let expected_pos = (i * 100) as f64 / 398.2222;
        assert!((state.position - expected_pos).abs() < 0.01);
        assert_eq!(state.status, 1);
        assert_eq!(state.error_code, None);
    }
}

#[tokio::test]
async fn test_polling_task_handles_errors() {
    // Create a mock driver
    let (port, mut harness) = mock_serial::new();
    let shared_port: SharedPort = Arc::new(Mutex::new(Box::new(port)));
    let driver = Ell14Driver::with_test_port(shared_port, "0", 398.2222);
    let mut rx = driver.subscribe();

    // Start polling (store handle to abort after test)
    let _polling_handle = driver.start_polling(Duration::from_millis(10));

    // Simulate device responses
    tokio::spawn(async move {
        // Respond to position query
        harness.expect_write(b"0gp").await;
        harness.send_response(b"0PO00000000").unwrap();

        // Respond to status query
        harness.expect_write(b"0gs").await;
        harness.send_response(b"0GS01").unwrap();

        // Respond to error query
        harness.expect_write(b"0ge").await;
        harness.send_response(b"0GE05").unwrap();
    });

    // Wait for the state update
    let state = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(state.error_code, Some(5));
}
