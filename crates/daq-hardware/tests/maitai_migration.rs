//! MaiTai Migration Tests
//!
//! This test module validates the config-driven GenericSerialDriver using maitai.toml.
//! Tests the Readable, WavelengthTunable, and ShutterControl trait implementations.
//!
//! Run with: `cargo test -p daq-hardware --test maitai_migration`

use daq_hardware::config::load_device_config;
use daq_hardware::config::schema::{CapabilityType, DeviceCategory, FlowControlSetting};
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
fn test_maitai_config_loads_successfully() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");

    let config = load_device_config(&config_path).expect("Failed to load maitai.toml");

    // Verify basic device info
    assert_eq!(config.device.name, "Spectra-Physics MaiTai");
    assert_eq!(config.device.protocol, "maitai");
    assert_eq!(config.device.manufacturer, "Spectra-Physics");
    assert_eq!(config.device.category, DeviceCategory::Source); // laser is Source category

    // Verify connection settings
    assert_eq!(config.connection.baud_rate, 9600);
    assert_eq!(
        config.connection.flow_control,
        FlowControlSetting::Software
    ); // XON/XOFF

    // Verify key commands exist
    assert!(config.commands.contains_key("set_wavelength"));
    assert!(config.commands.contains_key("get_wavelength"));
    assert!(config.commands.contains_key("open_shutter"));
    assert!(config.commands.contains_key("close_shutter"));
    assert!(config.commands.contains_key("get_shutter"));
    assert!(config.commands.contains_key("get_power"));
    assert!(config.commands.contains_key("emission_on"));
    assert!(config.commands.contains_key("emission_off"));

    // Verify responses exist
    assert!(config.responses.contains_key("wavelength"));
    assert!(config.responses.contains_key("shutter_state"));
    assert!(config.responses.contains_key("power"));

    // Verify trait mappings exist
    assert!(config.trait_mapping.contains_key("Readable"));
    assert!(config.trait_mapping.contains_key("WavelengthTunable"));
    assert!(config.trait_mapping.contains_key("ShutterControl"));

    let readable = &config.trait_mapping["Readable"];
    assert!(readable.methods.contains_key("read"));

    let wavelength = &config.trait_mapping["WavelengthTunable"];
    assert!(wavelength.methods.contains_key("set_wavelength"));
    assert!(wavelength.methods.contains_key("get_wavelength"));

    let shutter = &config.trait_mapping["ShutterControl"];
    assert!(shutter.methods.contains_key("open_shutter"));
    assert!(shutter.methods.contains_key("close_shutter"));
    assert!(shutter.methods.contains_key("is_shutter_open"));
}

// =============================================================================
// Test: Command Formatting - Wavelength
// =============================================================================

#[tokio::test]
async fn test_set_wavelength_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test case: Set wavelength to 820nm
    let mut params = HashMap::new();
    params.insert("wavelength".to_string(), 820.0);

    let cmd = driver.format_command("set_wavelength", &params).await.unwrap();

    // Expected: "WAVELENGTH:820"
    assert_eq!(cmd, "WAVELENGTH:820", "Set wavelength command format mismatch");
}

#[tokio::test]
async fn test_get_wavelength_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("get_wavelength", &HashMap::new())
        .await
        .unwrap();

    // Expected: "WAVELENGTH?"
    assert_eq!(cmd, "WAVELENGTH?", "Get wavelength command format mismatch");
}

// =============================================================================
// Test: Command Formatting - Shutter
// =============================================================================

#[tokio::test]
async fn test_open_shutter_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("open_shutter", &HashMap::new())
        .await
        .unwrap();

    // Expected: "SHUTter:1"
    assert_eq!(cmd, "SHUTter:1", "Open shutter command format mismatch");
}

#[tokio::test]
async fn test_close_shutter_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("close_shutter", &HashMap::new())
        .await
        .unwrap();

    // Expected: "SHUTter:0"
    assert_eq!(cmd, "SHUTter:0", "Close shutter command format mismatch");
}

#[tokio::test]
async fn test_get_shutter_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("get_shutter", &HashMap::new())
        .await
        .unwrap();

    // Expected: "SHUTTER?"
    assert_eq!(cmd, "SHUTTER?", "Get shutter command format mismatch");
}

// =============================================================================
// Test: Command Formatting - Emission
// =============================================================================

#[tokio::test]
async fn test_emission_on_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("emission_on", &HashMap::new())
        .await
        .unwrap();

    // Expected: "ON"
    assert_eq!(cmd, "ON", "Emission on command format mismatch");
}

#[tokio::test]
async fn test_emission_off_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("emission_off", &HashMap::new())
        .await
        .unwrap();

    // Expected: "OFF"
    assert_eq!(cmd, "OFF", "Emission off command format mismatch");
}

// =============================================================================
// Test: Command Formatting - Power & Identity
// =============================================================================

#[tokio::test]
async fn test_get_power_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("get_power", &HashMap::new())
        .await
        .unwrap();

    // Expected: "POWER?"
    assert_eq!(cmd, "POWER?", "Get power command format mismatch");
}

#[tokio::test]
async fn test_identify_command_format() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    let cmd = driver
        .format_command("identify", &HashMap::new())
        .await
        .unwrap();

    // Expected: "*IDN?"
    assert_eq!(cmd, "*IDN?", "Identify command format mismatch");
}

// =============================================================================
// Test: Response Parsing - Wavelength
// =============================================================================

#[test]
fn test_parse_wavelength_response() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "820nm" -> 820.0
    let parsed = driver.parse_response("wavelength", "820nm").unwrap();
    let wavelength = parsed.fields.get("wavelength").unwrap().as_f64().unwrap();
    assert!((wavelength - 820.0).abs() < 0.1, "Wavelength value mismatch: got {}", wavelength);
}

#[test]
fn test_parse_wavelength_response_uppercase() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "820NM" -> 820.0
    let parsed = driver.parse_response("wavelength", "820NM").unwrap();
    let wavelength = parsed.fields.get("wavelength").unwrap().as_f64().unwrap();
    assert!((wavelength - 820.0).abs() < 0.1, "Wavelength value mismatch");
}

#[test]
fn test_parse_wavelength_response_with_decimal() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "799.5nm" -> 799.5
    let parsed = driver.parse_response("wavelength", "799.5nm").unwrap();
    let wavelength = parsed.fields.get("wavelength").unwrap().as_f64().unwrap();
    assert!((wavelength - 799.5).abs() < 0.1, "Wavelength value mismatch");
}

// =============================================================================
// Test: Response Parsing - Shutter State
// =============================================================================

#[test]
fn test_parse_shutter_open() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "1" -> shutter open
    let parsed = driver.parse_response("shutter_state", "1").unwrap();
    let state = parsed.fields.get("state").unwrap().as_i64().unwrap();
    assert_eq!(state, 1, "Shutter should be open (1)");
}

#[test]
fn test_parse_shutter_closed() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "0" -> shutter closed
    let parsed = driver.parse_response("shutter_state", "0").unwrap();
    let state = parsed.fields.get("state").unwrap().as_i64().unwrap();
    assert_eq!(state, 0, "Shutter should be closed (0)");
}

// =============================================================================
// Test: Response Parsing - Power
// =============================================================================

#[test]
fn test_parse_power_response_watts() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "3.00W" -> 3.00
    let parsed = driver.parse_response("power", "3.00W").unwrap();
    let power = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((power - 3.0).abs() < 0.01, "Power value mismatch: got {}", power);
}

#[test]
fn test_parse_power_response_milliwatts() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "100mW" -> 100
    let parsed = driver.parse_response("power", "100mW").unwrap();
    let power = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((power - 100.0).abs() < 0.01, "Power value mismatch");
}

#[test]
fn test_parse_power_response_percent() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    let (port, _mock) = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "0").unwrap();

    // Test parsing "50%" -> 50
    let parsed = driver.parse_response("power", "50%").unwrap();
    let power = parsed.fields.get("value").unwrap().as_f64().unwrap();
    assert!((power - 50.0).abs() < 0.01, "Power value mismatch");
}

// =============================================================================
// Test: Factory Creation
// =============================================================================

#[test]
fn test_factory_creates_maitai_driver() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");

    let (port, _mock) = create_mock_port();
    let driver = DriverFactory::create_from_file(&config_path, port, "0").unwrap();

    // Should be MaiTai variant
    assert!(matches!(driver, ConfiguredDriver::MaiTai(_)));
    assert_eq!(driver.protocol(), "maitai");
    assert_eq!(driver.name(), "Spectra-Physics MaiTai");
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
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    // Verify error codes are defined
    assert!(config.error_codes.contains_key("ERR"));

    // Check error descriptions
    assert_eq!(config.error_codes["ERR"].name, "Error");
    assert!(config.error_codes["ERR"].recoverable);
}

// =============================================================================
// Test: Capabilities Verification
// =============================================================================

#[test]
fn test_maitai_capabilities() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    // Verify all expected capabilities are declared
    assert!(config.device.capabilities.contains(&CapabilityType::Readable));
    assert!(config.device.capabilities.contains(&CapabilityType::WavelengthTunable));
    assert!(config.device.capabilities.contains(&CapabilityType::ShutterControl));
    assert!(config.device.capabilities.contains(&CapabilityType::Parameterized));
}

// =============================================================================
// Test: Parameters Verification
// =============================================================================

#[test]
fn test_maitai_parameters() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config_path = workspace_root.join("config/devices/maitai.toml");
    let config = load_device_config(&config_path).unwrap();

    // Verify wavelength parameters
    assert!(config.parameters.contains_key("wavelength_nm"));
    assert!(config.parameters.contains_key("wavelength_min"));
    assert!(config.parameters.contains_key("wavelength_max"));

    // Verify shutter parameter
    assert!(config.parameters.contains_key("shutter_open"));

    // Check wavelength range defaults
    let min = config.parameters.get("wavelength_min").unwrap();
    let max = config.parameters.get("wavelength_max").unwrap();
    assert_eq!(min.default.as_f64().unwrap(), 690.0);
    assert_eq!(max.default.as_f64().unwrap(), 1040.0);
}
