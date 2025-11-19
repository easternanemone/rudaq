//! Integration tests for SharedSerialPort exclusive access mechanism
//!
//! Tests concurrent V2/V4 actor access to shared serial ports with ownership tracking

use std::sync::Arc;
use std::time::Duration;
use v4_daq::hardware::{SerialPortConfig, SerialParity, SharedSerialPort};

#[tokio::test]
async fn test_new_port_is_available() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = SharedSerialPort::new(config);
    assert!(port.is_available());
    assert!(port.current_owner().is_none());
}

#[tokio::test]
async fn test_port_properties() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 115200,
        ..Default::default()
    };
    let port = SharedSerialPort::new(config);
    assert_eq!(port.path(), "/dev/ttyUSB0");
    assert_eq!(port.baud_rate(), 115200);
}

#[tokio::test]
async fn test_acquire_release_single_actor() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = SharedSerialPort::new(config);

    // Acquire
    let guard = port
        .acquire("actor_1", Duration::from_secs(1))
        .await
        .expect("Failed to acquire port");

    assert!(!port.is_available());
    assert_eq!(port.current_owner(), Some("actor_1".to_string()));
    assert_eq!(guard.actor_id(), "actor_1");

    // Release (drop guard)
    drop(guard);

    assert!(port.is_available());
    assert!(port.current_owner().is_none());
}

#[tokio::test]
async fn test_exclusive_access_two_actors() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = SharedSerialPort::new(config);

    // First actor acquires
    let guard1 = port
        .acquire("actor_1", Duration::from_millis(100))
        .await
        .expect("Actor 1 failed to acquire");

    // Second actor tries to acquire - should fail
    let result = port
        .acquire("actor_2", Duration::from_millis(100))
        .await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("already in use"));

    // First actor releases
    drop(guard1);

    // Now second actor can acquire
    let guard2 = port
        .acquire("actor_2", Duration::from_millis(100))
        .await
        .expect("Actor 2 failed to acquire after release");

    assert_eq!(port.current_owner(), Some("actor_2".to_string()));
    drop(guard2);
}

#[tokio::test]
async fn test_timeout_on_acquire() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = Arc::new(SharedSerialPort::new(config));

    // First actor holds the port for a while
    let guard1 = port
        .acquire("actor_1", Duration::from_secs(10))
        .await
        .expect("Actor 1 failed to acquire");

    // Spawn a second actor that tries to acquire with short timeout
    let port_clone = port.clone();
    let result = port_clone
        .acquire("actor_2", Duration::from_millis(100))
        .await;

    // Should fail due to port being in use and short timeout
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Timeout") || err_msg.contains("already in use"));

    drop(guard1);
}

#[tokio::test]
async fn test_guard_write_read() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = SharedSerialPort::new(config);

    let mut guard = port
        .acquire("actor_1", Duration::from_secs(1))
        .await
        .expect("Failed to acquire port");

    // Test write
    guard.write(b"*IDN?\r\n").await.expect("Write failed");

    // Test write_all
    guard
        .write_all(b"*RST\r\n")
        .await
        .expect("Write all failed");

    // Test read (will return 0 in mock)
    let mut buf = [0u8; 256];
    let n = guard.read(&mut buf).await.expect("Read failed");
    assert_eq!(n, 0); // Mock always returns 0
}

#[tokio::test]
async fn test_multiple_sequential_acquisitions() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = SharedSerialPort::new(config);

    // Sequential acquisitions should work
    for i in 0..5 {
        let actor_id = format!("actor_{}", i);
        let guard = port
            .acquire(&actor_id, Duration::from_millis(100))
            .await
            .expect(&format!("Failed to acquire for {}", actor_id));

        assert_eq!(port.current_owner(), Some(actor_id.clone()));
        drop(guard);
        assert!(port.is_available());
    }
}

#[tokio::test]
async fn test_concurrent_acquisitions() {
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = Arc::new(SharedSerialPort::new(config));

    let mut handles = vec![];

    // Spawn 5 concurrent tasks trying to acquire
    for i in 0..5 {
        let port_clone = port.clone();
        let handle = tokio::spawn(async move {
            let actor_id = format!("actor_{}", i);
            match port_clone.acquire(&actor_id, Duration::from_millis(500)).await {
                Ok(guard) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    drop(guard);
                    Ok(actor_id)
                }
                Err(e) => Err(e),
            }
        });
        handles.push(handle);
    }

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(handles).await;

    // Some should succeed, some might fail due to timeouts
    let successes: Vec<_> = results
        .iter()
        .filter_map(|r| r.as_ref().ok().and_then(|inner| inner.as_ref().ok()))
        .collect();

    // At least one should succeed
    assert!(!successes.is_empty());
}

#[tokio::test]
async fn test_parity_configurations() {
    // Test all parity modes
    let modes = vec![
        SerialParity::None,
        SerialParity::Even,
        SerialParity::Odd,
    ];

    for parity in modes {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            parity,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);
        assert!(port.is_available());
    }
}

#[tokio::test]
async fn test_various_baud_rates() {
    // Test common baud rates
    let baud_rates = vec![9600, 19200, 38400, 57600, 115200];

    for baud_rate in baud_rates {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);
        assert_eq!(port.baud_rate(), baud_rate);
    }
}

#[tokio::test]
async fn test_v2_v4_simulated_workload() {
    // Simulate V2 and V4 actors alternately accessing the same port
    let config = SerialPortConfig {
        path: "/dev/ttyUSB0".to_string(),
        baud_rate: 9600,
        ..Default::default()
    };
    let port = Arc::new(SharedSerialPort::new(config));

    // V2 actor (tokio task)
    let port_v2 = port.clone();
    let v2_handle = tokio::spawn(async move {
        for i in 0..2 {
            let mut guard = port_v2
                .acquire(&format!("v2_actor_{}", i), Duration::from_secs(2))
                .await
                .expect("V2: Failed to acquire");

            // Simulate some work (hold port for 50ms)
            guard.write(b"*IDN?\r\n").await.ok();
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(guard);

            // Allow time for other actor to acquire (100ms gap)
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    // V4 actor (simulated Kameo) - offset start
    let port_v4 = port.clone();
    let v4_handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(75)).await;

        for i in 0..2 {
            let mut guard = port_v4
                .acquire(&format!("v4_actor_{}", i), Duration::from_secs(2))
                .await
                .expect("V4: Failed to acquire");

            // Simulate some work (hold port for 50ms)
            guard.write_all(b"*RST\r\n").await.ok();
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(guard);

            // Allow time for other actor to acquire (100ms gap)
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    // Wait for both
    let (v2_result, v4_result) = tokio::join!(v2_handle, v4_handle);
    assert!(v2_result.is_ok(), "V2 workload failed");
    assert!(v4_result.is_ok(), "V4 workload failed");
}
