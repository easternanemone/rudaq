#![cfg(feature = "generic_driver")]
//! Mock Serial Port Tests for GenericSerialDriver
//!
//! Tests verifying command formatting and response parsing using mock serial ports.
//! These tests validate GenericSerialDriver behavior without requiring real hardware.
//!
//! Run with: `cargo nextest run -p daq-scripting --test generic_driver_mock_tests`

use daq_hardware::capabilities::Movable;
use daq_hardware::config::load_device_config;
use daq_hardware::drivers::generic_serial::{DynSerial, GenericSerialDriver, SharedPort};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// =============================================================================
// Mock Serial Port Infrastructure
// =============================================================================

/// A mock serial port that records writes and returns preset responses.
/// Reuses pattern from daq-hardware/tests/ell14_migration.rs
struct MockSerial {
    write_buf: Arc<std::sync::Mutex<Vec<u8>>>,
    read_buf: Arc<std::sync::Mutex<Cursor<Vec<u8>>>>,
}

impl MockSerial {
    fn new() -> Self {
        Self {
            write_buf: Arc::new(std::sync::Mutex::new(Vec::new())),
            read_buf: Arc::new(std::sync::Mutex::new(Cursor::new(Vec::new()))),
        }
    }

    /// Get the last written command as a string
    fn get_written(&self) -> String {
        let buf = self.write_buf.lock().unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    /// Set the next response to return
    fn set_response(&self, response: &str) {
        let mut buf = self.read_buf.lock().unwrap();
        *buf = Cursor::new(response.as_bytes().to_vec());
    }

    /// Clear written data
    #[allow(dead_code)]
    fn clear(&self) {
        let mut buf = self.write_buf.lock().unwrap();
        buf.clear();
    }
}

impl Clone for MockSerial {
    fn clone(&self) -> Self {
        Self {
            write_buf: Arc::clone(&self.write_buf),
            read_buf: Arc::clone(&self.read_buf),
        }
    }
}

impl tokio::io::AsyncRead for MockSerial {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let mut read_buf = self.read_buf.lock().unwrap();
        let data = read_buf.get_ref();
        let pos = read_buf.position() as usize;
        let remaining = &data[pos..];
        let to_copy = std::cmp::min(remaining.len(), buf.remaining());
        buf.put_slice(&remaining[..to_copy]);
        read_buf.set_position((pos + to_copy) as u64);
        std::task::Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncWrite for MockSerial {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut write_buf = self.write_buf.lock().unwrap();
        write_buf.extend_from_slice(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl Unpin for MockSerial {}

/// Create a mock port and its handle for testing
fn create_mock_port() -> (SharedPort, MockSerial) {
    let mock = MockSerial::new();
    let boxed: DynSerial = Box::new(mock.clone());
    let port: SharedPort = Arc::new(Mutex::new(boxed));
    (port, mock)
}

/// Get path to ELL14 config file
fn ell14_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("config/devices/ell14.toml")
}

// =============================================================================
// Tests
// =============================================================================

/// Test that move_abs formats the command correctly for ELL14
/// Note: This test verifies command formatting via format_command since move_abs
/// requires full conversion context setup which is tested in daq-hardware integration tests.
#[tokio::test(flavor = "multi_thread")]
async fn test_move_abs_formats_command_correctly() {
    let (port, _mock) = create_mock_port();

    let config_path = ell14_config_path();
    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found at {:?}", config_path);
        return;
    }

    let config = load_device_config(&config_path).expect("Failed to load config");
    let driver = GenericSerialDriver::new(config, port, "2").expect("Failed to create driver");

    // Test command formatting directly (avoids conversion context requirements)
    // 17920 pulses = 45 degrees (at 398.22 pulses/degree)
    let mut params = std::collections::HashMap::new();
    params.insert("position_pulses".to_string(), 17920.0f64);

    let cmd = driver
        .format_command("move_absolute", &params)
        .await
        .expect("format_command failed");

    // Should contain address prefix and hex-encoded position
    // 17920 in hex = 4600
    assert!(
        cmd.starts_with("2"),
        "Command should start with address '2', got: {}",
        cmd
    );
    assert!(
        cmd.to_lowercase().contains("ma"),
        "Command should contain 'ma' for move_absolute, got: {}",
        cmd
    );
}

/// Test that position() parses the response correctly
#[tokio::test(flavor = "multi_thread")]
async fn test_position_parses_response_correctly() {
    let (port, mock) = create_mock_port();
    // 17744 pulses ≈ 44.56 degrees (at 398.22 pulses/degree)
    // 17744 in hex = 0x4550
    mock.set_response("2PO00004550");

    let config_path = ell14_config_path();
    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found at {:?}", config_path);
        return;
    }

    let config = load_device_config(&config_path).expect("Failed to load config");
    let driver = GenericSerialDriver::new(config, port, "2").expect("Failed to create driver");

    let pos = driver.position().await.expect("position() failed");

    // 17744 pulses / 398.22 pulses_per_degree ≈ 44.56 degrees
    let expected = 17744.0 / 398.22;
    assert!(
        (pos - expected).abs() < 0.5,
        "Expected position ~{:.1}, got {:.1}",
        expected,
        pos
    );
}

/// Test transaction roundtrip - send command and get response
#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_roundtrip() {
    let (port, mock) = create_mock_port();
    mock.set_response("2PO00001000");

    let config_path = ell14_config_path();
    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found at {:?}", config_path);
        return;
    }

    let config = load_device_config(&config_path).expect("Failed to load config");
    let driver = GenericSerialDriver::new(config, port, "2").expect("Failed to create driver");

    // Send raw get_position command
    let response = driver.transaction("2gp").await.expect("transaction failed");

    // Verify we got the response
    assert!(
        response.contains("PO") || response.contains("po"),
        "Expected position response containing 'PO', got: {}",
        response
    );

    // Verify command was written
    let written = mock.get_written();
    assert!(
        written.contains("2gp"),
        "Expected '2gp' command, got: {}",
        written
    );
}

/// Test send_command (fire-and-forget, no response expected)
#[tokio::test(flavor = "multi_thread")]
async fn test_send_command_no_response() {
    let (port, mock) = create_mock_port();
    // No response needed for stop command
    mock.set_response("");

    let config_path = ell14_config_path();
    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found at {:?}", config_path);
        return;
    }

    let config = load_device_config(&config_path).expect("Failed to load config");
    let driver = GenericSerialDriver::new(config, port, "2").expect("Failed to create driver");

    // Send stop command (no response expected)
    driver
        .send_command("2st")
        .await
        .expect("send_command failed");

    let written = mock.get_written();
    assert!(
        written.contains("2st"),
        "Expected '2st' command, got: {}",
        written
    );
}

/// Test format_command interpolation with parameters
#[tokio::test(flavor = "multi_thread")]
async fn test_format_command_interpolation() {
    let (port, _mock) = create_mock_port();

    let config_path = ell14_config_path();
    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found at {:?}", config_path);
        return;
    }

    let config = load_device_config(&config_path).expect("Failed to load config");
    let driver = GenericSerialDriver::new(config, port, "2").expect("Failed to create driver");

    // Format move_absolute command with position parameter
    let mut params = std::collections::HashMap::new();
    params.insert("position_pulses".to_string(), 17920.0f64);

    let cmd = driver
        .format_command("move_absolute", &params)
        .await
        .expect("format_command failed");

    // Should have address prefix and hex-encoded position
    assert!(
        cmd.starts_with("2"),
        "Command should start with address '2', got: {}",
        cmd
    );
    // 17920 in hex = 4600
    assert!(
        cmd.contains("4600") || cmd.contains("4600"),
        "Command should contain hex position '4600', got: {}",
        cmd
    );
}
