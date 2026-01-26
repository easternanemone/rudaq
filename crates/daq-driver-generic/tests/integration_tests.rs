//! Integration tests for daq-driver-generic using mock serial ports.
//!
//! These tests verify the GenericSerialDriver works correctly with simulated
//! device interactions, including command/response transactions, response parsing,
//! unit conversions, and trait method execution.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

// =============================================================================
// Mock Serial Port Implementation
// =============================================================================

/// The client-facing side of the mock serial port.
/// Implements AsyncRead + AsyncWrite for use with GenericSerialDriver.
#[derive(Debug)]
pub struct MockSerialPort {
    /// Channel to send written data to the harness
    writes_tx: UnboundedSender<Vec<u8>>,
    /// Channel to receive data from the harness to be read
    reads_rx: UnboundedReceiver<Vec<u8>>,
    /// Buffer for data received from the harness but not yet read
    read_buffer: VecDeque<u8>,
}

/// The test-facing side for controlling the mock serial port.
#[derive(Debug)]
pub struct MockDeviceHarness {
    /// Channel to receive data written by the client
    writes_rx: UnboundedReceiver<Vec<u8>>,
    /// Channel to send data to the client for it to read
    reads_tx: UnboundedSender<Vec<u8>>,
    /// Buffer for data received from the client
    write_buffer: Vec<u8>,
}

/// Creates a new connected pair of MockSerialPort and MockDeviceHarness.
pub fn new_mock_serial() -> (MockSerialPort, MockDeviceHarness) {
    let (client_to_harness_tx, client_to_harness_rx) = mpsc::unbounded_channel();
    let (harness_to_client_tx, harness_to_client_rx) = mpsc::unbounded_channel();

    let port = MockSerialPort {
        writes_tx: client_to_harness_tx,
        reads_rx: harness_to_client_rx,
        read_buffer: VecDeque::new(),
    };

    let harness = MockDeviceHarness {
        writes_rx: client_to_harness_rx,
        reads_tx: harness_to_client_tx,
        write_buffer: Vec::new(),
    };

    (port, harness)
}

impl AsyncRead for MockSerialPort {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // If we have data in our internal buffer, use that first
        if !self.read_buffer.is_empty() {
            let available = self.read_buffer.len();
            let to_read = std::cmp::min(buf.remaining(), available);
            let chunk: Vec<u8> = self.read_buffer.drain(..to_read).collect();
            buf.put_slice(&chunk);
            return Poll::Ready(Ok(()));
        }

        // Otherwise, poll the channel for a new chunk
        match self.reads_rx.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => {
                self.read_buffer.extend(chunk);
                let available = self.read_buffer.len();
                let to_read = std::cmp::min(buf.remaining(), available);
                let data: Vec<u8> = self.read_buffer.drain(..to_read).collect();
                buf.put_slice(&data);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())), // EOF
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for MockSerialPort {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.writes_tx.send(buf.to_vec()) {
            Ok(_) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "mock device harness disconnected",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl Unpin for MockSerialPort {}

impl MockDeviceHarness {
    /// Sends a response to the client (simulates device responding).
    pub fn send_response(&self, data: &[u8]) -> Result<(), &'static str> {
        self.reads_tx
            .send(data.to_vec())
            .map_err(|_| "Failed to send response: client port disconnected")
    }

    /// Waits for the client to write data and captures it.
    pub async fn expect_write(&mut self, expected: &[u8]) {
        let timeout_duration = Duration::from_secs(2);

        while self.write_buffer.len() < expected.len() {
            match timeout(timeout_duration, self.writes_rx.recv()).await {
                Ok(Some(chunk)) => self.write_buffer.extend_from_slice(&chunk),
                Ok(None) => panic!("Client-side port closed while expecting a write."),
                Err(_) => {
                    panic!(
                        "Timeout waiting for write. Expected `{:?}` ({} bytes), got `{:?}` ({} bytes).",
                        String::from_utf8_lossy(expected),
                        expected.len(),
                        String::from_utf8_lossy(&self.write_buffer),
                        self.write_buffer.len()
                    );
                }
            }
        }

        let actual = &self.write_buffer[..expected.len()];
        assert_eq!(
            actual,
            expected,
            "Mismatch in expected write. Expected `{:?}`, got `{:?}`.",
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(actual)
        );

        self.write_buffer.drain(..expected.len());
    }

    /// Expects a write and sends a response in one operation.
    pub async fn expect_and_respond(&mut self, expected: &[u8], response: &[u8]) {
        self.expect_write(expected).await;
        self.send_response(response)
            .expect("Failed to send response");
    }

    /// Drains any pending writes without asserting.
    pub async fn drain_writes(&mut self) {
        let short_timeout = Duration::from_millis(50);
        while let Ok(Some(chunk)) = timeout(short_timeout, self.writes_rx.recv()).await {
            self.write_buffer.extend_from_slice(&chunk);
        }
        self.write_buffer.clear();
    }
}

// =============================================================================
// Test Configuration Helpers
// =============================================================================

/// Create a SharedPort from a MockSerialPort.
fn create_shared_port(
    port: MockSerialPort,
) -> Arc<Mutex<Box<dyn daq_driver_generic::SerialPortIO>>> {
    Arc::new(Mutex::new(Box::new(port)))
}

/// Minimal test configuration for ELL14-like device.
const MINIMAL_CONFIG: &str = r#"
[device]
name = "Test Device"
protocol = "test_protocol"

[connection]
type = "serial"
timeout_ms = 1000
terminator_tx = ""
terminator_rx = ""

[parameters.pulses_per_degree]
type = "float"
default = 398.2222

[commands.get_position]
template = "${address}gp"
response = "position"

[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
parameters = { position_pulses = "int32" }
response = "position"

[commands.get_status]
template = "${address}gs"
response = "status"

[commands.stop]
template = "${address}st"
expects_response = false

[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{1,8})$"

[responses.position.fields.addr]
type = "string"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true

[responses.status]
pattern = "^(?P<addr>[0-9A-Fa-f])GS(?P<code>[0-9A-Fa-f]{2})$"

[responses.status.fields.addr]
type = "string"

[responses.status.fields.code]
type = "hex_u32"

[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"

[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "degrees"

[trait_mapping.Movable.position]
command = "get_position"
output_conversion = "pulses_to_degrees"
output_field = "pulses"

[trait_mapping.Movable.stop]
command = "stop"

[trait_mapping.Movable.wait_settled]
poll_command = "get_status"
success_condition = "code == 0"
poll_interval_ms = 50
timeout_ms = 5000
"#;

/// Readable device configuration for sensor-like devices.
const READABLE_CONFIG: &str = r#"
[device]
name = "Test Power Meter"
protocol = "power_meter"
capabilities = ["Readable"]

[connection]
type = "serial"
timeout_ms = 1000
terminator_tx = "\r\n"
terminator_rx = "\r\n"

[parameters]

[commands.read_power]
template = "MEAS:POW?"
response = "power"

[responses.power]
pattern = "^(?P<value>[+-]?\\d+\\.?\\d*(?:[eE][+-]?\\d+)?)$"

[responses.power.fields.value]
type = "float"

[conversions]

[trait_mapping.Readable.read]
command = "read_power"
output_field = "value"
"#;

/// Configuration with error codes for testing error detection.
const ERROR_CONFIG: &str = r#"
[device]
name = "Test Device with Errors"
protocol = "test_errors"

[connection]
type = "serial"
timeout_ms = 500

[parameters]

[commands.test_cmd]
template = "${address}TEST"
response = "test_response"

[responses.test_response]
pattern = "^(?P<result>.+)$"

[responses.test_response.fields.result]
type = "string"

[error_codes."GS02"]
name = "MechanicalTimeout"
description = "Motor did not reach position"
recoverable = true
severity = "error"

[error_codes."GS03"]
name = "SensorError"
description = "Position sensor malfunction"
recoverable = false
severity = "critical"

[conversions]

[trait_mapping]
"#;

// =============================================================================
// Integration Tests
// =============================================================================

mod mock_serial_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_mock_serial_basic_write_read() {
        let (mut port, mut harness) = new_mock_serial();

        // Client writes
        let write_task = tokio::spawn(async move {
            port.write_all(b"HELLO").await.unwrap();
            port
        });

        // Harness receives
        harness.expect_write(b"HELLO").await;
        write_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_mock_serial_command_response() {
        let (mut port, mut harness) = new_mock_serial();

        let app_task = tokio::spawn(async move {
            port.write_all(b"QUERY").await.unwrap();
            let mut buf = [0u8; 32];
            let n = port.read(&mut buf).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });

        harness.expect_write(b"QUERY").await;
        harness.send_response(b"RESPONSE").unwrap();

        let response = app_task.await.unwrap();
        assert_eq!(response, "RESPONSE");
    }

    #[tokio::test]
    async fn test_mock_serial_multiple_exchanges() {
        let (mut port, mut harness) = new_mock_serial();

        let app_task = tokio::spawn(async move {
            // First exchange
            port.write_all(b"CMD1").await.unwrap();
            let mut buf = [0u8; 32];
            let n = port.read(&mut buf).await.unwrap();
            let r1 = String::from_utf8_lossy(&buf[..n]).to_string();

            // Second exchange
            port.write_all(b"CMD2").await.unwrap();
            let n = port.read(&mut buf).await.unwrap();
            let r2 = String::from_utf8_lossy(&buf[..n]).to_string();

            (r1, r2)
        });

        harness.expect_and_respond(b"CMD1", b"ACK1").await;
        harness.expect_and_respond(b"CMD2", b"ACK2").await;

        let (r1, r2) = app_task.await.unwrap();
        assert_eq!(r1, "ACK1");
        assert_eq!(r2, "ACK2");
    }
}

mod driver_tests {
    use super::*;
    use daq_driver_generic::GenericSerialDriver;
    use daq_plugin_api::config::InstrumentConfig;

    fn load_config(toml_str: &str) -> InstrumentConfig {
        toml::from_str(toml_str).expect("Failed to parse test config")
    }

    #[tokio::test]
    async fn test_driver_creation() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2");
        assert!(driver.is_ok());

        let driver = driver.unwrap();
        assert_eq!(driver.address(), "2");
    }

    #[tokio::test]
    async fn test_format_command_simple() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Simple command without extra params
        let cmd = driver
            .format_command("get_position", &HashMap::new())
            .await
            .unwrap();
        assert_eq!(cmd, "2gp");
    }

    #[tokio::test]
    async fn test_format_command_with_hex() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Command with hex-formatted parameter
        let mut params = HashMap::new();
        params.insert("position_pulses".to_string(), 17920.0); // 0x4600

        let cmd = driver
            .format_command("move_absolute", &params)
            .await
            .unwrap();
        assert_eq!(cmd, "2ma00004600");
    }

    #[tokio::test]
    async fn test_format_command_negative_hex() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "A").unwrap();

        // Negative value should produce two's complement hex
        let mut params = HashMap::new();
        params.insert("position_pulses".to_string(), -256.0); // 0xFFFFFF00

        let cmd = driver
            .format_command("move_absolute", &params)
            .await
            .unwrap();
        assert_eq!(cmd, "AmaFFFFFF00");
    }

    #[tokio::test]
    async fn test_parse_position_response() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Parse position response
        let parsed = driver.parse_response("position", "2PO00004600").unwrap();
        assert_eq!(parsed.raw, "2PO00004600");

        let addr = parsed.fields.get("addr").unwrap().as_string();
        assert_eq!(addr, "2");

        let pulses = parsed.fields.get("pulses").unwrap().as_i64().unwrap();
        assert_eq!(pulses, 17920); // 0x4600
    }

    #[tokio::test]
    async fn test_parse_status_response() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Parse status OK
        let parsed = driver.parse_response("status", "2GS00").unwrap();
        let code = parsed.fields.get("code").unwrap().as_i64().unwrap();
        assert_eq!(code, 0);

        // Parse status error (mechanical timeout = 0x02)
        let parsed = driver.parse_response("status", "2GS02").unwrap();
        let code = parsed.fields.get("code").unwrap().as_i64().unwrap();
        assert_eq!(code, 2);
    }

    #[tokio::test]
    async fn test_apply_conversion_degrees_to_pulses() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // 45 degrees * 398.2222 ≈ 17920 pulses
        let pulses = driver
            .apply_conversion("degrees_to_pulses", "degrees", 45.0)
            .await
            .unwrap();
        assert!((pulses - 17920.0).abs() < 1.0);
    }

    #[tokio::test]
    async fn test_apply_conversion_pulses_to_degrees() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // 17920 pulses / 398.2222 ≈ 45 degrees
        let degrees = driver
            .apply_conversion("pulses_to_degrees", "pulses", 17920.0)
            .await
            .unwrap();
        assert!((degrees - 45.0).abs() < 0.1);
    }

    #[tokio::test]
    async fn test_transaction_with_mock() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, mut harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Spawn task to simulate device response
        let harness_task = tokio::spawn(async move {
            harness.expect_write(b"2gp").await;
            harness.send_response(b"2PO00004600").unwrap();
            harness
        });

        // Driver sends command and reads response
        let response = driver.transaction("2gp").await.unwrap();
        assert_eq!(response, "2PO00004600");

        harness_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_send_command_no_response() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, mut harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        let harness_task = tokio::spawn(async move {
            harness.expect_write(b"2st").await;
            harness
        });

        // Stop command doesn't expect response
        driver.send_command("2st").await.unwrap();

        harness_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_get_and_set_parameter() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Check default value
        let ppd = driver.get_parameter("pulses_per_degree").await;
        assert!(ppd.is_some());
        assert!((ppd.unwrap() - 398.2222).abs() < 0.001);

        // Set new value
        driver.set_parameter("custom_param", 123.456).await;
        let value = driver.get_parameter("custom_param").await;
        assert!(value.is_some());
        assert!((value.unwrap() - 123.456).abs() < 0.001);
    }
}

mod conversion_tests {
    use super::*;
    use daq_driver_generic::GenericSerialDriver;
    use daq_plugin_api::config::InstrumentConfig;

    fn load_config(toml_str: &str) -> InstrumentConfig {
        toml::from_str(toml_str).expect("Failed to parse test config")
    }

    #[tokio::test]
    async fn test_conversion_roundtrip() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Convert degrees -> pulses -> degrees should be ~identity
        let original_degrees = 90.0;
        let pulses = driver
            .apply_conversion("degrees_to_pulses", "degrees", original_degrees)
            .await
            .unwrap();
        let back_to_degrees = driver
            .apply_conversion("pulses_to_degrees", "pulses", pulses)
            .await
            .unwrap();

        assert!(
            (back_to_degrees - original_degrees).abs() < 0.01,
            "Roundtrip failed: {} -> {} -> {}",
            original_degrees,
            pulses,
            back_to_degrees
        );
    }

    #[tokio::test]
    async fn test_conversion_zero() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // 0 degrees = 0 pulses
        let pulses = driver
            .apply_conversion("degrees_to_pulses", "degrees", 0.0)
            .await
            .unwrap();
        assert!((pulses - 0.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_conversion_full_rotation() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // 360 degrees = 360 * 398.2222 = 143360 pulses (ELL14 full rotation)
        let pulses = driver
            .apply_conversion("degrees_to_pulses", "degrees", 360.0)
            .await
            .unwrap();
        assert!((pulses - 143360.0).abs() < 1.0);
    }
}

mod error_detection_tests {
    use super::*;
    use daq_driver_generic::GenericSerialDriver;
    use daq_plugin_api::config::InstrumentConfig;

    fn load_config(toml_str: &str) -> InstrumentConfig {
        toml::from_str(toml_str).expect("Failed to parse test config")
    }

    #[tokio::test]
    async fn test_no_error_in_normal_response() {
        let config = load_config(ERROR_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "1").unwrap();

        // Normal response should not trigger error detection
        let error = driver.check_for_error("1PO00001234");
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn test_error_code_detected() {
        let config = load_config(ERROR_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "1").unwrap();

        // Response containing error code GS02
        let error = driver.check_for_error("1GS02");
        assert!(error.is_some());

        let error = error.unwrap();
        assert_eq!(error.code, "GS02");
        assert_eq!(error.name, "MechanicalTimeout");
        assert!(error.recoverable);
        // ErrorSeverity is an enum, compare by variant name
        assert_eq!(format!("{:?}", error.severity), "Error");
    }

    #[tokio::test]
    async fn test_critical_error_detected() {
        let config = load_config(ERROR_CONFIG);
        let (port, _harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "1").unwrap();

        // Response containing critical error code GS03
        let error = driver.check_for_error("1GS03");
        assert!(error.is_some());

        let error = error.unwrap();
        assert_eq!(error.code, "GS03");
        assert_eq!(error.name, "SensorError");
        assert!(!error.recoverable);
        // ErrorSeverity is an enum, compare by variant name
        assert_eq!(format!("{:?}", error.severity), "Critical");
    }
}

mod response_value_tests {
    use daq_driver_generic::ResponseValue;

    #[test]
    fn test_response_value_float_conversions() {
        let val = ResponseValue::Float(123.456);
        assert!((val.as_f64().unwrap() - 123.456).abs() < 0.001);
        assert_eq!(val.as_i64().unwrap(), 123);
        assert_eq!(val.as_string(), "123.456");
    }

    #[test]
    fn test_response_value_int_conversions() {
        let val = ResponseValue::Int(-42);
        assert!((val.as_f64().unwrap() - (-42.0)).abs() < 0.001);
        assert_eq!(val.as_i64().unwrap(), -42);
        assert_eq!(val.as_string(), "-42");
    }

    #[test]
    fn test_response_value_uint_conversions() {
        let val = ResponseValue::Uint(255);
        assert!((val.as_f64().unwrap() - 255.0).abs() < 0.001);
        assert_eq!(val.as_i64().unwrap(), 255);
        assert_eq!(val.as_string(), "255");
    }

    #[test]
    fn test_response_value_string() {
        let val = ResponseValue::String("hello".to_string());
        assert!(val.as_f64().is_none());
        assert!(val.as_i64().is_none());
        assert_eq!(val.as_string(), "hello");
    }

    #[test]
    fn test_response_value_bool() {
        let val = ResponseValue::Bool(true);
        assert!(val.as_f64().is_none());
        assert!(val.as_i64().is_none());
        assert_eq!(val.as_string(), "true");
    }
}

mod trait_execution_tests {
    use super::*;
    use daq_driver_generic::GenericSerialDriver;
    use daq_plugin_api::config::InstrumentConfig;

    fn load_config(toml_str: &str) -> InstrumentConfig {
        toml::from_str(toml_str).expect("Failed to parse test config")
    }

    #[tokio::test]
    async fn test_execute_position_query() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, mut harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        // Simulate device response in background
        let harness_task = tokio::spawn(async move {
            // Wait for position query command
            harness.expect_write(b"2gp").await;
            // Respond with position (17920 pulses = 45 degrees)
            harness.send_response(b"2PO00004600").unwrap();
            harness
        });

        // Execute trait method
        let result = driver
            .execute_trait_method("Movable", "position", None)
            .await
            .unwrap();

        assert!(result.is_some());
        let degrees = result.unwrap();
        // Should be ~45 degrees (17920 / 398.2222)
        assert!((degrees - 45.0).abs() < 0.1);

        harness_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_execute_move_command() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, mut harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        let harness_task = tokio::spawn(async move {
            // Expect move command: 45 degrees -> 17920 pulses -> 0x4600
            harness.expect_write(b"2ma00004600").await;
            // Respond with new position
            harness.send_response(b"2PO00004600").unwrap();
            harness
        });

        // Execute move_abs with 45 degrees
        let result = driver
            .execute_trait_method("Movable", "move_abs", Some(45.0))
            .await;

        assert!(result.is_ok(), "execute_trait_method failed: {:?}", result);

        harness_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_execute_stop_command() {
        let config = load_config(MINIMAL_CONFIG);
        let (port, mut harness) = new_mock_serial();
        let shared_port = create_shared_port(port);

        let driver = GenericSerialDriver::new(config, shared_port, "2").unwrap();

        let harness_task = tokio::spawn(async move {
            // Stop command doesn't have a response
            harness.expect_write(b"2st").await;
            harness
        });

        // Execute stop
        let result = driver
            .execute_trait_method("Movable", "stop", None)
            .await;

        assert!(result.is_ok());

        harness_task.await.unwrap();
    }
}

mod config_validation_tests {
    use super::*;
    use daq_plugin_api::config::InstrumentConfig;

    #[test]
    fn test_minimal_config_valid() {
        let config: Result<InstrumentConfig, _> = toml::from_str(MINIMAL_CONFIG);
        assert!(config.is_ok(), "Minimal config should be valid");
    }

    #[test]
    fn test_readable_config_valid() {
        let config: Result<InstrumentConfig, _> = toml::from_str(READABLE_CONFIG);
        assert!(config.is_ok(), "Readable config should be valid");
    }

    #[test]
    fn test_error_config_valid() {
        let config: Result<InstrumentConfig, _> = toml::from_str(ERROR_CONFIG);
        assert!(config.is_ok(), "Error config should be valid");
    }

    #[test]
    fn test_invalid_config_fails() {
        let invalid_config = r#"
[device]
# Missing required 'name' field
protocol = "test"

[connection]
type = "serial"
"#;

        let config: Result<InstrumentConfig, _> = toml::from_str(invalid_config);
        assert!(config.is_err(), "Invalid config should fail to parse");
    }
}
