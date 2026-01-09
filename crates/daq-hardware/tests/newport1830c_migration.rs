//! Newport 1830-C Migration Tests
//!
//! This test module validates the config-driven GenericSerialDriver using newport_1830c.toml.
//! Tests the Readable and WavelengthTunable trait implementations.
//!
//! Run with: `cargo test -p daq-hardware --test newport1830c_migration`

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
fn test_newport1830c_config_loads_successfully() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");

    let config = load_device_config(&config_path).expect("Failed to load newport_1830c.toml");

    // Verify basic device info
    assert_eq!(config.device.name, "Newport 1830-C");
    assert_eq!(config.device.protocol, "newport_1830c");
    assert_eq!(config.device.manufacturer, "Newport");

    // Verify connection settings
    assert_eq!(config.connection.baud_rate, 9600);

    // Verify key commands exist
    assert!(config.commands.contains_key("read_power"));
    assert!(config.commands.contains_key("get_wavelength"));
    assert!(config.commands.contains_key("set_wavelength"));

    // Verify responses exist
    assert!(config.responses.contains_key("power"));
    assert!(config.responses.contains_key("wavelength"));

    // Verify trait mappings exist
    assert!(config.trait_mapping.contains_key("Readable"));
    assert!(config.trait_mapping.contains_key("WavelengthTunable"));

    let readable = &config.trait_mapping["Readable"];
    assert!(readable.methods.contains_key("read"));

    let wavelength = &config.trait_mapping["WavelengthTunable"];
    assert!(wavelength.methods.contains_key("set_wavelength"));
    assert!(wavelength.methods.contains_key("get_wavelength"));
}

// =============================================================================
// Test: Command Formatting
// =============================================================================

#[tokio::test]
async fn test_read_power_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("read_power", &HashMap::new())
        .await
        .unwrap();

    // Expected: "D?" (read power query)
    assert_eq!(cmd, "D?", "Read power command format mismatch");
}

#[tokio::test]
async fn test_get_wavelength_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("get_wavelength", &HashMap::new())
        .await
        .unwrap();

    // Expected: "W?" (wavelength query)
    assert_eq!(cmd, "W?", "Get wavelength command format mismatch");
}

#[tokio::test]
async fn test_set_wavelength_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test case: Set wavelength to 780nm
    let mut params = HashMap::new();
    params.insert("wavelength_nm".to_string(), 780.0);

    let cmd = driver.format_command("set_wavelength", &params).await.unwrap();

    // Expected: "W0780" (4-digit format)
    assert_eq!(cmd, "W0780", "Set wavelength command format mismatch");
}

#[tokio::test]
async fn test_set_attenuator_commands() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd_on = driver
        .format_command("set_attenuator_on", &HashMap::new())
        .await
        .unwrap();
    assert_eq!(cmd_on, "A1", "Attenuator on command mismatch");

    let cmd_off = driver
        .format_command("set_attenuator_off", &HashMap::new())
        .await
        .unwrap();
    assert_eq!(cmd_off, "A0", "Attenuator off command mismatch");
}

#[tokio::test]
async fn test_set_filter_commands() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd_slow = driver
        .format_command("set_filter_slow", &HashMap::new())
        .await
        .unwrap();
    assert_eq!(cmd_slow, "F1", "Filter slow command mismatch");

    let cmd_medium = driver
        .format_command("set_filter_medium", &HashMap::new())
        .await
        .unwrap();
    assert_eq!(cmd_medium, "F2", "Filter medium command mismatch");

    let cmd_fast = driver
        .format_command("set_filter_fast", &HashMap::new())
        .await
        .unwrap();
    assert_eq!(cmd_fast, "F3", "Filter fast command mismatch");
}

// =============================================================================
// Test: Response Parsing - Scientific Notation
// =============================================================================

#[test]
fn test_parse_power_response_scientific_positive() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "5E-9" -> 5e-9 W (5 nW)
    let parsed = driver.parse_response("power", "5E-9").unwrap();
    let value = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((value - 5e-9).abs() < 1e-15, "Power value mismatch: got {}", value);
}

#[test]
fn test_parse_power_response_scientific_with_decimal() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "1.234E-6" -> 1.234e-6 W (1.234 uW)
    let parsed = driver.parse_response("power", "1.234E-6").unwrap();
    let value = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((value - 1.234e-6).abs() < 1e-12, "Power value mismatch: got {}", value);
}

#[test]
fn test_parse_power_response_with_leading_sign() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "+.75E-9" -> 0.75e-9 W (0.75 nW)
    let parsed = driver.parse_response("power", "+.75E-9").unwrap();
    let value = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((value - 0.75e-9).abs() < 1e-15, "Power value mismatch: got {}", value);
}

#[test]
fn test_parse_power_response_milliwatt() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "1.5E-3" -> 1.5e-3 W (1.5 mW)
    let parsed = driver.parse_response("power", "1.5E-3").unwrap();
    let value = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((value - 1.5e-3).abs() < 1e-9, "Power value mismatch: got {}", value);
}

#[test]
fn test_parse_wavelength_response() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "0780" -> 780nm
    let parsed = driver.parse_response("wavelength", "0780").unwrap();
    let wavelength = parsed.fields.get("wavelength").unwrap().as_i64().unwrap();
    assert_eq!(wavelength, 780, "Wavelength value mismatch");
}

#[test]
fn test_parse_range_response() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing range "3"
    let parsed = driver.parse_response("range", "3").unwrap();
    let range = parsed.fields.get("range").unwrap().as_i64().unwrap();
    assert_eq!(range, 3, "Range value mismatch");
}

#[test]
fn test_parse_units_response() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing units "1" (dBm)
    let parsed = driver.parse_response("units", "1").unwrap();
    let units = parsed.fields.get("units").unwrap().as_i64().unwrap();
    assert_eq!(units, 1, "Units value mismatch");
}

// =============================================================================
// Test: Factory Creation
// =============================================================================

#[test]
fn test_factory_creates_newport1830c_driver() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");

    let (port, _mock) = create_mock_port();
    let driver = DriverFactory::create_from_file(&config_path, port, "0").unwrap();

    // Should be Newport1830C variant
    assert!(matches!(driver, ConfiguredDriver::Newport1830C(_)));
    assert_eq!(driver.protocol(), "newport_1830c");
    assert_eq!(driver.name(), "Newport 1830-C");
    assert_eq!(driver.address(), "0");
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
    let config_path = workspace_root.join("config/devices/newport_1830c.toml");
    let config = load_device_config(&config_path).unwrap();

    // Verify error codes are defined
    assert!(config.error_codes.contains_key("ERR"));
    assert!(config.error_codes.contains_key("OVER"));
    assert!(config.error_codes.contains_key("UNDER"));

    // Check error descriptions
    assert_eq!(config.error_codes["ERR"].name, "Error");
    assert_eq!(config.error_codes["OVER"].name, "Overrange");
    assert!(config.error_codes["UNDER"].recoverable);
}
