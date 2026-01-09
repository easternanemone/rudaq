//! ESP300 Migration Tests
//!
//! This test module validates the config-driven GenericSerialDriver using esp300.toml.
//!
//! Run with: `cargo test -p daq-hardware --test esp300_migration`

use daq_hardware::config::load_device_config;
use daq_hardware::drivers::generic_serial::{GenericSerialDriver, SharedPort};
use daq_hardware::factory::{ConfiguredDriver, DriverFactory};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

// =============================================================================
// Mock Serial Port for Testing
// =============================================================================

struct MockSerial {
    write_buf: Arc<Mutex<Vec<u8>>>,
    read_buf: Arc<Mutex<Cursor<Vec<u8>>>>,
}

impl MockSerial {
    fn new() -> Self {
        Self {
            write_buf: Arc::new(Mutex::new(Vec::new())),
            read_buf: Arc::new(Mutex::new(Cursor::new(Vec::new()))),
        }
    }

    #[allow(dead_code)]
    fn get_written(&self) -> String {
        let buf = self.write_buf.try_lock().unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    #[allow(dead_code)]
    fn set_response(&self, response: &str) {
        let mut buf = self.read_buf.try_lock().unwrap();
        *buf = Cursor::new(response.as_bytes().to_vec());
    }

    #[allow(dead_code)]
    fn clear(&self) {
        let mut buf = self.write_buf.try_lock().unwrap();
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
        let mut read_buf = self.read_buf.try_lock().unwrap();
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
        let mut write_buf = self.write_buf.try_lock().unwrap();
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

fn create_mock_port() -> (SharedPort, MockSerial) {
    let mock = MockSerial::new();
    let port: SharedPort = Arc::new(Mutex::new(Box::new(mock.clone())));
    (port, mock)
}

// =============================================================================
// Test: Config Loading
// =============================================================================

#[test]
fn test_esp300_config_loads_successfully() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");

    let config = load_device_config(&config_path).expect("Failed to load esp300.toml");

    // Verify basic device info
    assert_eq!(config.device.name, "Newport ESP300");
    assert_eq!(config.device.protocol, "esp300");
    assert_eq!(config.device.manufacturer, "Newport");

    // Verify connection settings
    assert_eq!(config.connection.baud_rate, 19200);

    // Verify key commands exist
    assert!(config.commands.contains_key("move_absolute"));
    assert!(config.commands.contains_key("move_relative"));
    assert!(config.commands.contains_key("get_position"));
    assert!(config.commands.contains_key("stop"));
    assert!(config.commands.contains_key("home"));

    // Verify responses exist
    assert!(config.responses.contains_key("position"));
    assert!(config.responses.contains_key("motion_status"));

    // Verify trait mappings exist
    assert!(config.trait_mapping.contains_key("Movable"));
    let movable = &config.trait_mapping["Movable"];
    assert!(movable.methods.contains_key("move_abs"));
    assert!(movable.methods.contains_key("position"));
    assert!(movable.methods.contains_key("stop"));
}

// =============================================================================
// Test: Command Formatting
// =============================================================================

#[tokio::test]
async fn test_move_absolute_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "1").unwrap();

    // Test case: Move axis 1 to position 25.5mm
    let mut params = HashMap::new();
    params.insert("position".to_string(), 25.5);

    let cmd = driver.format_command("move_absolute", &params).await.unwrap();

    // Expected: "1PA25.5" (axis 1, PA command, position)
    assert_eq!(cmd, "1PA25.5", "Move absolute command format mismatch");
}

#[tokio::test]
async fn test_move_relative_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Test case: Move axis 2 relative -10.0mm
    let mut params = HashMap::new();
    params.insert("distance".to_string(), -10.0);

    let cmd = driver.format_command("move_relative", &params).await.unwrap();

    // Expected: "2PR-10" (axis 2, PR command, distance)
    assert_eq!(cmd, "2PR-10", "Move relative command format mismatch");
}

#[tokio::test]
async fn test_get_position_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "3").unwrap();

    let cmd = driver
        .format_command("get_position", &HashMap::new())
        .await
        .unwrap();

    // Expected: "3TP?" (axis 3, TP? query)
    assert_eq!(cmd, "3TP?", "Get position command format mismatch");
}

#[tokio::test]
async fn test_stop_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "1").unwrap();

    let cmd = driver.format_command("stop", &HashMap::new()).await.unwrap();

    assert_eq!(cmd, "1ST", "Stop command format mismatch");
}

#[tokio::test]
async fn test_home_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    let cmd = driver.format_command("home", &HashMap::new()).await.unwrap();

    assert_eq!(cmd, "2OR", "Home command format mismatch");
}

// =============================================================================
// Test: Response Parsing
// =============================================================================

#[test]
fn test_parse_position_response() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "1").unwrap();

    // Test parsing "12.345" -> 12.345mm
    let parsed = driver.parse_response("position", "12.345").unwrap();
    let value = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((value - 12.345).abs() < 0.001, "Position value mismatch");
}

#[test]
fn test_parse_negative_position_response() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "1").unwrap();

    // Test parsing "-5.678" -> -5.678mm
    let parsed = driver.parse_response("position", "-5.678").unwrap();
    let value = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((value - (-5.678)).abs() < 0.001, "Negative position value mismatch");
}

#[test]
fn test_parse_motion_status_stationary() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "1").unwrap();

    // Motion done status: "0" = stationary
    let parsed = driver.parse_response("motion_status", "0").unwrap();
    let status = parsed.fields.get("status").unwrap().as_i64().unwrap();
    assert_eq!(status, 0, "Status should be 0 (stationary)");
}

#[test]
fn test_parse_motion_status_moving() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "1").unwrap();

    // Motion done status: "1" = moving
    let parsed = driver.parse_response("motion_status", "1").unwrap();
    let status = parsed.fields.get("status").unwrap().as_i64().unwrap();
    assert_eq!(status, 1, "Status should be 1 (moving)");
}

// =============================================================================
// Test: Factory Creation
// =============================================================================

#[test]
fn test_factory_creates_esp300_driver() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");

    let (port, _mock) = create_mock_port();
    let driver = DriverFactory::create_from_file(&config_path, port, "1").unwrap();

    // Should be Esp300 variant
    assert!(matches!(driver, ConfiguredDriver::Esp300(_)));
    assert_eq!(driver.protocol(), "esp300");
    assert_eq!(driver.name(), "Newport ESP300");
    assert_eq!(driver.address(), "1");
}

// =============================================================================
// Test: Error Code Mapping
// =============================================================================

#[test]
fn test_error_codes_in_config() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/esp300.toml");
    let config = load_device_config(&config_path).unwrap();

    // Verify error codes are defined
    assert!(config.error_codes.contains_key("0"));
    assert!(config.error_codes.contains_key("1"));
    assert!(config.error_codes.contains_key("2"));

    // Check error descriptions
    assert_eq!(config.error_codes["0"].name, "OK");
    assert_eq!(config.error_codes["1"].name, "AxisNotFound");
    assert!(!config.error_codes["1"].recoverable);
}
