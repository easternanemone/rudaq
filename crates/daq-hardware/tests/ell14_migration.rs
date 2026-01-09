//! ELL14 Migration Tests
//!
//! This test module compares the behavior of the existing hand-coded Ell14Driver
//! with the new config-driven GenericSerialDriver using ell14.toml.
//!
//! The goal is to verify that both drivers:
//! - Generate identical command strings
//! - Parse responses identically
//! - Apply conversions (degrees <-> pulses) correctly
//!
//! Run with: `cargo test -p daq-hardware --test ell14_migration --features driver-thorlabs`

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

/// A mock serial port that records writes and returns preset responses
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

    /// Get the last written command as a string
    #[allow(dead_code)]
    fn get_written(&self) -> String {
        let buf = self.write_buf.try_lock().unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    /// Set the next response to return
    #[allow(dead_code)]
    fn set_response(&self, response: &str) {
        let mut buf = self.read_buf.try_lock().unwrap();
        *buf = Cursor::new(response.as_bytes().to_vec());
    }

    /// Clear written data
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
fn test_ell14_config_loads_successfully() {
    // Find config file relative to workspace root
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");

    let config = load_device_config(&config_path).expect("Failed to load ell14.toml");

    // Verify basic device info
    assert_eq!(config.device.name, "Thorlabs ELL14");
    assert_eq!(config.device.protocol, "elliptec");
    assert_eq!(config.device.manufacturer, "Thorlabs");

    // Verify connection settings
    assert_eq!(config.connection.baud_rate, 9600);

    // Verify key commands exist
    assert!(config.commands.contains_key("move_absolute"));
    assert!(config.commands.contains_key("get_position"));
    assert!(config.commands.contains_key("get_status"));
    assert!(config.commands.contains_key("stop"));

    // Verify responses exist
    assert!(config.responses.contains_key("position"));
    assert!(config.responses.contains_key("status"));

    // Verify conversions exist
    assert!(config.conversions.contains_key("degrees_to_pulses"));
    assert!(config.conversions.contains_key("pulses_to_degrees"));

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
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Test case: 45.0 degrees with 398.2222 pulses/degree = 17920 pulses = 0x4600
    let mut params = HashMap::new();
    params.insert("position_pulses".to_string(), 17920.0);

    let cmd = driver.format_command("move_absolute", &params).await.unwrap();

    // Expected: "2ma00004600" (address 2, move_absolute, 8-char hex 0x4600)
    assert_eq!(cmd, "2ma00004600", "Move absolute command format mismatch");
}

#[tokio::test]
async fn test_move_absolute_with_conversion() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Apply degrees_to_pulses conversion
    // 45.0 degrees * 398.2222 pulses/degree = 17920 pulses (rounded)
    let pulses = driver
        .apply_conversion("degrees_to_pulses", "degrees", 45.0)
        .await
        .unwrap();

    assert!(
        (pulses - 17920.0).abs() < 1.0,
        "Conversion result {} doesn't match expected 17920",
        pulses
    );
}

#[tokio::test]
async fn test_move_relative_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "3").unwrap();

    // Test negative relative move: -10 degrees
    // -10.0 * 398.2222 = -3982 pulses = 0xFFFFF072 (two's complement)
    let mut params = HashMap::new();
    params.insert("distance_pulses".to_string(), -3982.0);

    let cmd = driver.format_command("move_relative", &params).await.unwrap();

    // Negative value should be in two's complement hex
    assert_eq!(cmd, "3mrFFFFF072", "Relative move command format mismatch");
}

#[tokio::test]
async fn test_get_position_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "8").unwrap();

    let cmd = driver
        .format_command("get_position", &HashMap::new())
        .await
        .unwrap();

    assert_eq!(cmd, "8gp", "Get position command format mismatch");
}

#[tokio::test]
async fn test_stop_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    let cmd = driver.format_command("stop", &HashMap::new()).await.unwrap();

    assert_eq!(cmd, "2st", "Stop command format mismatch");
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
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Test parsing "2PO00004600" -> pulses = 0x4600 = 17920
    let parsed = driver.parse_response("position", "2PO00004600").unwrap();

    assert_eq!(parsed.fields.get("addr").unwrap().as_string(), "2");

    let pulses = parsed.fields.get("pulses").unwrap().as_i64().unwrap();
    assert_eq!(pulses, 17920, "Pulses value mismatch");
}

#[tokio::test]
async fn test_position_response_with_conversion() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Parse response and apply conversion
    let parsed = driver.parse_response("position", "2PO00004600").unwrap();
    let pulses = parsed.fields.get("pulses").unwrap().as_f64().unwrap();

    // Apply pulses_to_degrees conversion
    let degrees = driver
        .apply_conversion("pulses_to_degrees", "pulses", pulses)
        .await
        .unwrap();

    assert!(
        (degrees - 45.0).abs() < 0.1,
        "Converted position {} doesn't match expected 45.0",
        degrees
    );
}

#[test]
fn test_parse_status_response_ok() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Status OK: "2GS00"
    let parsed = driver.parse_response("status", "2GS00").unwrap();
    let code = parsed.fields.get("code").unwrap().as_i64().unwrap();
    assert_eq!(code, 0, "Status code should be 0 (OK)");
}

#[test]
fn test_parse_status_response_error() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Mechanical timeout: "2GS02"
    let parsed = driver.parse_response("status", "2GS02").unwrap();
    let code = parsed.fields.get("code").unwrap().as_i64().unwrap();
    assert_eq!(code, 2, "Status code should be 2 (mechanical timeout)");
}

#[test]
fn test_parse_jog_step_response() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Jog step: "2GJ00001F4" = 500 pulses
    let parsed = driver.parse_response("jog_step", "2GJ00001F4").unwrap();
    let pulses = parsed.fields.get("pulses").unwrap().as_i64().unwrap();
    assert_eq!(pulses, 500, "Jog step pulses mismatch");
}

// =============================================================================
// Test: Unit Conversions
// =============================================================================

#[tokio::test]
async fn test_degrees_to_pulses_conversion() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Test various angles
    let test_cases = [
        (0.0, 0.0),
        (45.0, 17920.0), // 45 * 398.2222 ≈ 17920
        (90.0, 35840.0), // 90 * 398.2222 ≈ 35840
        (180.0, 71680.0),
        (360.0, 143360.0),
    ];

    for (degrees, expected_pulses) in test_cases {
        let pulses = driver
            .apply_conversion("degrees_to_pulses", "degrees", degrees)
            .await
            .unwrap();
        assert!(
            (pulses - expected_pulses).abs() < 2.0,
            "{}° should be ~{} pulses, got {}",
            degrees,
            expected_pulses,
            pulses
        );
    }
}

#[tokio::test]
async fn test_pulses_to_degrees_conversion() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Test reverse conversion
    let test_cases = [
        (0.0, 0.0),
        (17920.0, 45.0),
        (35840.0, 90.0),
        (71680.0, 180.0),
        (143360.0, 360.0),
    ];

    for (pulses, expected_degrees) in test_cases {
        let degrees = driver
            .apply_conversion("pulses_to_degrees", "pulses", pulses)
            .await
            .unwrap();
        assert!(
            (degrees - expected_degrees).abs() < 0.1,
            "{} pulses should be ~{}°, got {}",
            pulses,
            expected_degrees,
            degrees
        );
    }
}

// =============================================================================
// Test: Factory Creation
// =============================================================================

#[test]
fn test_factory_creates_ell14_driver() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");

    let (port, _mock) = create_mock_port();
    let driver = DriverFactory::create_from_file(&config_path, port, "2").unwrap();

    // Should be Ell14 variant
    assert!(matches!(driver, ConfiguredDriver::Ell14(_)));
    assert_eq!(driver.protocol(), "elliptec");
    assert_eq!(driver.name(), "Thorlabs ELL14");
    assert_eq!(driver.address(), "2");
}

// =============================================================================
// Test: Signed Hex Handling
// =============================================================================

#[test]
fn test_parse_negative_position() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Negative position: 0xFFFFF072 = -3982 (two's complement)
    let parsed = driver.parse_response("position", "2POFFFFF072").unwrap();
    let pulses = parsed.fields.get("pulses").unwrap().as_i64().unwrap();

    assert_eq!(pulses, -3982, "Negative pulses should be -3982");
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
    let config_path = workspace_root.join("config/devices/ell14.toml");
    let config = load_device_config(&config_path).unwrap();

    // Verify error codes are defined
    assert!(config.error_codes.contains_key("0x00")); // OK
    assert!(config.error_codes.contains_key("0x02")); // MechanicalTimeout
    assert!(config.error_codes.contains_key("0x08")); // ThermalError

    // Check error descriptions
    assert_eq!(config.error_codes["0x00"].name, "OK");
    assert_eq!(config.error_codes["0x02"].name, "MechanicalTimeout");
    assert!(!config.error_codes["0x08"].recoverable);
}
