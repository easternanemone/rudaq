//! Migration tests for Phase 4: Production Hardening features.
//!
//! Tests:
//! - Per-command timeout configuration
//! - Retry logic with exponential backoff
//! - Error code detection and mapping
//! - Initialization sequence execution

use daq_hardware::config::load_device_config_from_str;
use daq_hardware::config::schema::{ErrorSeverity, RetryConfig};
use daq_hardware::drivers::generic_serial::GenericSerialDriver;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;

// =============================================================================
// Test Helpers
// =============================================================================

/// Mock serial port for testing
struct MockPort {
    write_buf: Arc<Mutex<Vec<u8>>>,
    read_buf: Arc<Mutex<Cursor<Vec<u8>>>>,
}

impl MockPort {
    fn new() -> Self {
        Self {
            write_buf: Arc::new(Mutex::new(Vec::new())),
            read_buf: Arc::new(Mutex::new(Cursor::new(Vec::new()))),
        }
    }

    #[allow(dead_code)]
    fn set_response(&self, response: &str) {
        let mut buf = self.read_buf.try_lock().unwrap();
        *buf = Cursor::new(response.as_bytes().to_vec());
    }
}

impl AsyncRead for MockPort {
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

impl AsyncWrite for MockPort {
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

impl Unpin for MockPort {}

// Use the driver's SharedPort type
use daq_hardware::drivers::generic_serial::{DynSerial, SharedPort};

fn create_mock_port() -> SharedPort {
    Arc::new(Mutex::new(Box::new(MockPort::new()) as DynSerial))
}

// =============================================================================
// Schema Tests: Per-Command Timeout
// =============================================================================

const CONFIG_WITH_COMMAND_TIMEOUT: &str = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
timeout_ms = 1000

[commands.fast_command]
template = "FAST"
timeout_ms = 100

[commands.slow_command]
template = "SLOW"
timeout_ms = 5000

[commands.default_timeout]
template = "DEFAULT"
"#;

#[test]
fn test_per_command_timeout_parsing() {
    let config = load_device_config_from_str(CONFIG_WITH_COMMAND_TIMEOUT).unwrap();

    // Fast command has 100ms timeout
    let fast = config.commands.get("fast_command").unwrap();
    assert_eq!(fast.timeout_ms, Some(100));

    // Slow command has 5000ms timeout
    let slow = config.commands.get("slow_command").unwrap();
    assert_eq!(slow.timeout_ms, Some(5000));

    // Default timeout is None (uses connection timeout)
    let default = config.commands.get("default_timeout").unwrap();
    assert_eq!(default.timeout_ms, None);
}

// =============================================================================
// Schema Tests: Retry Configuration
// =============================================================================

const CONFIG_WITH_RETRY: &str = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
timeout_ms = 1000

[default_retry]
max_retries = 5
initial_delay_ms = 200
max_delay_ms = 10000
backoff_multiplier = 2.5
retry_on_errors = ["TIMEOUT", "BUSY"]
no_retry_on_errors = ["FATAL"]

[commands.with_retry]
template = "CMD1"

[commands.with_retry.retry]
max_retries = 2
initial_delay_ms = 50

[commands.no_retry]
template = "CMD2"

[commands.no_retry.retry]
max_retries = 0
"#;

#[test]
fn test_default_retry_config_parsing() {
    let config = load_device_config_from_str(CONFIG_WITH_RETRY).unwrap();

    let default_retry = config.default_retry.as_ref().unwrap();
    assert_eq!(default_retry.max_retries, 5);
    assert_eq!(default_retry.initial_delay_ms, 200);
    assert_eq!(default_retry.max_delay_ms, 10000);
    assert!((default_retry.backoff_multiplier - 2.5).abs() < 0.001);
    assert_eq!(default_retry.retry_on_errors, vec!["TIMEOUT", "BUSY"]);
    assert_eq!(default_retry.no_retry_on_errors, vec!["FATAL"]);
}

#[test]
fn test_command_specific_retry_config() {
    let config = load_device_config_from_str(CONFIG_WITH_RETRY).unwrap();

    // Command with custom retry
    let with_retry = config.commands.get("with_retry").unwrap();
    let retry = with_retry.retry.as_ref().unwrap();
    assert_eq!(retry.max_retries, 2);
    assert_eq!(retry.initial_delay_ms, 50);

    // Command with no retry
    let no_retry = config.commands.get("no_retry").unwrap();
    let retry = no_retry.retry.as_ref().unwrap();
    assert_eq!(retry.max_retries, 0);
}

#[test]
fn test_retry_config_defaults() {
    let default = RetryConfig::default();
    assert_eq!(default.max_retries, 3);
    assert_eq!(default.initial_delay_ms, 100);
    assert_eq!(default.max_delay_ms, 5000);
    assert!((default.backoff_multiplier - 2.0).abs() < 0.001);
    assert!(default.retry_on_errors.is_empty());
    assert!(default.no_retry_on_errors.is_empty());
}

// =============================================================================
// Schema Tests: Error Code Configuration
// =============================================================================

const CONFIG_WITH_ERROR_CODES: &str = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
timeout_ms = 1000

[error_codes.E01]
name = "Timeout"
description = "Command timed out"
recoverable = true
severity = "warning"

[error_codes.E02]
name = "Mechanical Error"
description = "Mechanical failure detected"
recoverable = false
severity = "critical"

[error_codes.E02.recovery_action]
command = "reset"
auto_recover = false
manual_instructions = "Power cycle the device"

[error_codes.E03]
name = "Busy"
description = "Device is busy"
recoverable = true
severity = "info"

[error_codes.E03.recovery_action]
command = "abort"
auto_recover = true
delay_ms = 500
"#;

#[test]
fn test_error_code_parsing() {
    let config = load_device_config_from_str(CONFIG_WITH_ERROR_CODES).unwrap();

    // E01 - Timeout error
    let e01 = config.error_codes.get("E01").unwrap();
    assert_eq!(e01.name, "Timeout");
    assert!(e01.recoverable);
    assert_eq!(e01.severity, ErrorSeverity::Warning);
    assert!(e01.recovery_action.is_none());

    // E02 - Critical error with manual recovery
    let e02 = config.error_codes.get("E02").unwrap();
    assert_eq!(e02.name, "Mechanical Error");
    assert!(!e02.recoverable);
    assert_eq!(e02.severity, ErrorSeverity::Critical);
    let recovery = e02.recovery_action.as_ref().unwrap();
    assert_eq!(recovery.command.as_ref().unwrap(), "reset");
    assert!(!recovery.auto_recover);
    assert_eq!(
        recovery.manual_instructions.as_ref().unwrap(),
        "Power cycle the device"
    );

    // E03 - Info error with auto recovery
    let e03 = config.error_codes.get("E03").unwrap();
    assert_eq!(e03.severity, ErrorSeverity::Info);
    let recovery = e03.recovery_action.as_ref().unwrap();
    assert!(recovery.auto_recover);
    assert_eq!(recovery.delay_ms, 500);
}

#[test]
fn test_error_severity_serialization() {
    // Test all severity levels parse correctly
    let config_str = r#"
[device]
name = "Test"
protocol = "test"

[connection]
type = "serial"

[error_codes.INFO]
name = "Info"
severity = "info"

[error_codes.WARNING]
name = "Warning"
severity = "warning"

[error_codes.ERROR]
name = "Error"
severity = "error"

[error_codes.CRITICAL]
name = "Critical"
severity = "critical"

[error_codes.FATAL]
name = "Fatal"
severity = "fatal"
"#;

    let config = load_device_config_from_str(config_str).unwrap();

    assert_eq!(
        config.error_codes.get("INFO").unwrap().severity,
        ErrorSeverity::Info
    );
    assert_eq!(
        config.error_codes.get("WARNING").unwrap().severity,
        ErrorSeverity::Warning
    );
    assert_eq!(
        config.error_codes.get("ERROR").unwrap().severity,
        ErrorSeverity::Error
    );
    assert_eq!(
        config.error_codes.get("CRITICAL").unwrap().severity,
        ErrorSeverity::Critical
    );
    assert_eq!(
        config.error_codes.get("FATAL").unwrap().severity,
        ErrorSeverity::Fatal
    );
}

// =============================================================================
// Schema Tests: Init Sequence
// =============================================================================

const CONFIG_WITH_INIT_SEQUENCE: &str = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
timeout_ms = 1000

[commands.identify]
template = "*IDN?"
response = "identity"

[commands.reset]
template = "*RST"
expects_response = false

[commands.set_mode]
template = "MODE ${mode}"
parameters = { mode = "int32" }

[responses.identity]
pattern = "^(?P<manufacturer>.*),(?P<model>.*)$"

[responses.identity.fields.manufacturer]
type = "string"

[responses.identity.fields.model]
type = "string"

[[init_sequence]]
command = "identify"
description = "Query device identity"
required = true

[[init_sequence]]
command = "reset"
description = "Reset to default state"
required = true
delay_ms = 500

[[init_sequence]]
command = "set_mode"
description = "Set operating mode"
required = false
params = { mode = 1 }
expect = "OK"
"#;

#[test]
fn test_init_sequence_parsing() {
    let config = load_device_config_from_str(CONFIG_WITH_INIT_SEQUENCE).unwrap();

    assert_eq!(config.init_sequence.len(), 3);

    // Step 1: identify
    let step1 = &config.init_sequence[0];
    assert_eq!(step1.command, "identify");
    assert_eq!(step1.description, "Query device identity");
    assert!(step1.required);
    assert_eq!(step1.delay_ms, 0);

    // Step 2: reset with delay
    let step2 = &config.init_sequence[1];
    assert_eq!(step2.command, "reset");
    assert!(step2.required);
    assert_eq!(step2.delay_ms, 500);

    // Step 3: set_mode (optional, with params and expect)
    let step3 = &config.init_sequence[2];
    assert_eq!(step3.command, "set_mode");
    assert!(!step3.required);
    assert_eq!(step3.params.get("mode").unwrap(), &serde_json::json!(1));
    assert_eq!(step3.expect.as_ref().unwrap(), "OK");
}

// =============================================================================
// Driver Tests: Error Detection
// =============================================================================

const CONFIG_WITH_ERROR_DETECTION: &str = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
timeout_ms = 500

[commands.test_cmd]
template = "TEST"

[error_codes.GS02]
name = "Mechanical Timeout"
description = "Motor did not reach position"
recoverable = true
severity = "error"

[error_codes.GS03]
name = "Sensor Error"
description = "Position sensor malfunction"
recoverable = false
severity = "critical"
"#;

#[tokio::test]
async fn test_error_code_detection() {
    let config = load_device_config_from_str(CONFIG_WITH_ERROR_DETECTION).unwrap();
    let port = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "1").unwrap();

    // No error in OK response
    assert!(driver.check_for_error("1PO00001234").is_none());

    // GS02 error detected
    let error = driver.check_for_error("1GS02").unwrap();
    assert_eq!(error.code, "GS02");
    assert_eq!(error.name, "Mechanical Timeout");
    assert!(error.recoverable);
    assert_eq!(error.severity, ErrorSeverity::Error);

    // GS03 error detected
    let error = driver.check_for_error("1GS03").unwrap();
    assert_eq!(error.code, "GS03");
    assert_eq!(error.name, "Sensor Error");
    assert!(!error.recoverable);
    assert_eq!(error.severity, ErrorSeverity::Critical);
}

// =============================================================================
// Integration Tests: Full Config
// =============================================================================

const FULL_PRODUCTION_CONFIG: &str = r#"
[device]
name = "Production Test Device"
protocol = "test"
category = "stage"
capabilities = ["Movable"]

[connection]
type = "serial"
baud_rate = 9600
timeout_ms = 1000

[default_retry]
max_retries = 3
initial_delay_ms = 100
max_delay_ms = 2000
backoff_multiplier = 2.0
no_retry_on_errors = ["E_FATAL"]

[parameters.pulses_per_degree]
type = "float"
default = 398.2222

[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
parameters = { position_pulses = "int32" }
timeout_ms = 10000
response = "position"

[commands.move_absolute.retry]
max_retries = 5
initial_delay_ms = 200

[commands.get_position]
template = "${address}gp"
response = "position"

[commands.get_status]
template = "${address}gs"
response = "status"
timeout_ms = 100

[commands.home]
template = "${address}ho"
expects_response = false
timeout_ms = 30000

[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{1,8})$"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true

[responses.status]
pattern = "^(?P<addr>[0-9A-Fa-f])GS(?P<code>[0-9A-Fa-f]{2})$"

[responses.status.fields.code]
type = "hex_u8"

[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"

[error_codes.01]
name = "Communication Timeout"
description = "No response from device"
recoverable = true
severity = "warning"

[error_codes.02]
name = "Mechanical Timeout"
description = "Motor did not reach position"
recoverable = true
severity = "error"

[error_codes.03]
name = "Sensor Error"
description = "Position sensor malfunction"
recoverable = false
severity = "critical"

[error_codes.E_FATAL]
name = "Fatal Error"
description = "Device in unrecoverable state"
recoverable = false
severity = "fatal"

[error_codes.03.recovery_action]
command = "home"
auto_recover = false
manual_instructions = "Home the device before retrying"

[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "position"

[trait_mapping.Movable.position]
command = "get_position"
output_conversion = "pulses_to_degrees"
output_field = "pulses"

[[init_sequence]]
command = "get_status"
description = "Check device status"
required = true

[[init_sequence]]
command = "home"
description = "Home the device"
required = false
delay_ms = 1000
"#;

#[test]
fn test_full_production_config_parsing() {
    let config = load_device_config_from_str(FULL_PRODUCTION_CONFIG).unwrap();

    // Device identity
    assert_eq!(config.device.name, "Production Test Device");

    // Default retry
    let default_retry = config.default_retry.as_ref().unwrap();
    assert_eq!(default_retry.max_retries, 3);
    assert_eq!(default_retry.no_retry_on_errors, vec!["E_FATAL"]);

    // Command-specific timeout and retry
    let move_cmd = config.commands.get("move_absolute").unwrap();
    assert_eq!(move_cmd.timeout_ms, Some(10000));
    let move_retry = move_cmd.retry.as_ref().unwrap();
    assert_eq!(move_retry.max_retries, 5);

    // Error codes
    assert_eq!(config.error_codes.len(), 4);
    let e03 = config.error_codes.get("03").unwrap();
    assert!(e03.recovery_action.is_some());

    // Init sequence
    assert_eq!(config.init_sequence.len(), 2);
    assert!(!config.init_sequence[1].required);
}

#[tokio::test]
async fn test_driver_creation_with_production_config() {
    let config = load_device_config_from_str(FULL_PRODUCTION_CONFIG).unwrap();
    let port = create_mock_port();
    let driver = GenericSerialDriver::new(config, port, "2").unwrap();

    // Test command formatting still works
    let mut params = HashMap::new();
    params.insert("position_pulses".to_string(), 17920.0);
    let cmd = driver
        .format_command("move_absolute", &params)
        .await
        .unwrap();
    assert_eq!(cmd, "2ma00004600");

    // Test pulses_per_degree parameter
    let ppd = driver.get_pulses_per_degree().await;
    assert!((ppd - 398.2222).abs() < 0.001);
}
