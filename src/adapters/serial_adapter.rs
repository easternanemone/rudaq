//! Serial Hardware Adapter for RS-232/USB-Serial instruments
//!
//! Provides HardwareAdapter implementation for serial communication,
//! supporting instruments like Newport 1830-C, ESP300, etc.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use daq_core::{AdapterConfig, HardwareAdapter};
use log::debug;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[cfg(feature = "instrument_serial")]
use serialport::SerialPort;

/// Serial adapter for RS-232 communication
///
/// This adapter wraps the serialport crate and provides async I/O
/// using Tokio's blocking task executor for synchronous serial operations.
#[derive(Clone)]
pub struct SerialAdapter {
    /// Port name (e.g., "/dev/ttyUSB0", "COM3")
    port_name: String,

    /// Baud rate (e.g., 9600, 115200)
    baud_rate: u32,

    /// Read timeout
    timeout: Duration,

    /// Line terminator for commands (e.g., "\r\n")
    line_terminator: String,

    /// Response line ending character (e.g., '\n')
    response_delimiter: char,

    /// The actual serial port (behind Arc<Mutex> for async access)
    #[cfg(feature = "instrument_serial")]
    port: Option<Arc<Mutex<Box<dyn SerialPort>>>>,
}

impl SerialAdapter {
    /// Create a new serial adapter with default settings
    ///
    /// # Arguments
    /// * `port_name` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `baud_rate` - Communication speed (e.g., 9600, 115200)
    pub fn new(port_name: String, baud_rate: u32) -> Self {
        Self {
            port_name,
            baud_rate,
            timeout: Duration::from_secs(1),
            line_terminator: "\r\n".to_string(),
            response_delimiter: '\n',
            #[cfg(feature = "instrument_serial")]
            port: None,
        }
    }

    /// Set read timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set line terminator for commands
    pub fn with_line_terminator(mut self, terminator: String) -> Self {
        self.line_terminator = terminator;
        self
    }

    /// Set response delimiter character
    pub fn with_response_delimiter(mut self, delimiter: char) -> Self {
        self.response_delimiter = delimiter;
        self
    }

    /// Send a command and read the response asynchronously
    ///
    /// This method executes serial I/O on a blocking thread to avoid
    /// blocking the Tokio runtime.
    #[cfg(feature = "instrument_serial")]
    pub async fn send_command(&self, command: &str) -> Result<String> {
        let port = self
            .port
            .as_ref()
            .ok_or_else(|| anyhow!("Serial port not connected"))?;

        let command_str = format!("{}{}", command, self.line_terminator);
        let command_for_log = command.to_string(); // Clone for logging
        let delimiter = self.response_delimiter;
        let timeout = self.timeout;
        let port_clone = port.clone();

        // Execute blocking serial I/O on dedicated thread
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};

            let mut port_guard = port_clone.blocking_lock();

            // Write command
            port_guard
                .write_all(command_str.as_bytes())
                .context("Failed to write to serial port")?;

            port_guard.flush().context("Failed to flush serial port")?;

            debug!("Sent serial command: {}", command_for_log.trim());

            // Read response line-by-line until delimiter
            let mut response = String::new();
            let mut buffer = [0u8; 1];
            let start = std::time::Instant::now();

            loop {
                if start.elapsed() > timeout {
                    return Err(anyhow!("Serial read timeout after {:?}", timeout));
                }

                match port_guard.read(&mut buffer) {
                    Ok(1) => {
                        let ch = buffer[0] as char;
                        response.push(ch);

                        if ch == delimiter {
                            break;
                        }
                    }
                    Ok(0) => {
                        // EOF - shouldn't happen with serial ports
                        return Err(anyhow!("Unexpected EOF from serial port"));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                        // Port timeout is shorter than our overall timeout
                        continue;
                    }
                    Err(e) => {
                        return Err(anyhow!("Serial read error: {}", e));
                    }
                    Ok(_) => unreachable!("Read into single-byte buffer returned >1"),
                }
            }

            let response = response.trim().to_string();
            debug!("Received serial response: {}", response);
            Ok(response)
        })
        .await
        .context("Serial I/O task panicked")?
    }

    #[cfg(not(feature = "instrument_serial"))]
    pub async fn send_command(&self, _command: &str) -> Result<String> {
        Err(anyhow!(
            "Serial support not enabled. Rebuild with --features instrument_serial"
        ))
    }
}

#[async_trait]
impl HardwareAdapter for SerialAdapter {
    async fn connect(&mut self, config: &AdapterConfig) -> Result<()> {
        #[cfg(feature = "instrument_serial")]
        {
            // Override settings from config if provided
            if let Some(baud) = config.params.get("baud_rate").and_then(|v| v.as_u64()) {
                self.baud_rate = baud as u32;
            }

            if let Some(timeout_ms) = config.params.get("timeout_ms").and_then(|v| v.as_u64()) {
                self.timeout = Duration::from_millis(timeout_ms);
            }

            // Open serial port using serialport crate
            let port = serialport::new(&self.port_name, self.baud_rate)
                .timeout(Duration::from_millis(100)) // Internal read timeout
                .open()
                .with_context(|| {
                    format!(
                        "Failed to open serial port '{}' at {} baud",
                        self.port_name, self.baud_rate
                    )
                })?;

            self.port = Some(Arc::new(Mutex::new(port)));

            debug!(
                "Serial port '{}' opened at {} baud",
                self.port_name, self.baud_rate
            );
            Ok(())
        }

        #[cfg(not(feature = "instrument_serial"))]
        {
            let _ = config;
            Err(anyhow!(
                "Serial support not enabled. Rebuild with --features instrument_serial"
            ))
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        #[cfg(feature = "instrument_serial")]
        {
            if self.port.is_some() {
                self.port = None;
                debug!("Serial port '{}' closed", self.port_name);
            }
        }
        Ok(())
    }

    async fn reset(&mut self) -> Result<()> {
        // For serial adapters, reset means disconnect and reconnect
        self.disconnect().await?;

        // Small delay to let hardware reset
        tokio::time::sleep(Duration::from_millis(500)).await;

        self.connect(&AdapterConfig::default()).await
    }

    fn is_connected(&self) -> bool {
        #[cfg(feature = "instrument_serial")]
        {
            self.port.is_some()
        }

        #[cfg(not(feature = "instrument_serial"))]
        {
            false
        }
    }

    fn adapter_type(&self) -> &str {
        "serial"
    }

    fn info(&self) -> String {
        format!(
            "SerialAdapter({} @ {} baud)",
            self.port_name, self.baud_rate
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_adapter_creation() {
        let adapter = SerialAdapter::new("/dev/ttyUSB0".to_string(), 9600);
        assert_eq!(adapter.adapter_type(), "serial");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.port_name, "/dev/ttyUSB0");
        assert_eq!(adapter.baud_rate, 9600);
    }

    #[test]
    fn test_serial_adapter_builder() {
        let adapter = SerialAdapter::new("/dev/ttyUSB0".to_string(), 9600)
            .with_timeout(Duration::from_millis(500))
            .with_line_terminator("\n".to_string())
            .with_response_delimiter('\r');

        assert_eq!(adapter.timeout, Duration::from_millis(500));
        assert_eq!(adapter.line_terminator, "\n");
        assert_eq!(adapter.response_delimiter, '\r');
    }

    #[test]
    fn test_info_string() {
        let adapter = SerialAdapter::new("COM3".to_string(), 115200);
        let info = adapter.info();
        assert!(info.contains("COM3"));
        assert!(info.contains("115200"));
    }
}
