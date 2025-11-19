use crate::config::TimeoutSettings;


use crate::hardware::adapter::{AdapterError, HardwareAdapter};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::debug;
use serde_json::json;
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
            timeout: default_serial_timeout(),
            line_terminator: "\r\n".to_string(),
            response_delimiter: '\n',
            #[cfg(feature = "instrument_serial")]
            port: None,
        }
    }
}

fn default_serial_timeout() -> Duration {
    Duration::from_millis(TimeoutSettings::default().serial_read_timeout_ms)
}

#[async_trait]
impl HardwareAdapter for SerialAdapter {
    fn name(&self) -> &str {
        "serial"
    }

    fn default_config(&self) -> serde_json::Value {
        json!({
            "port": self.port_name,
            "baud_rate": self.baud_rate,
            "timeout_ms": self.timeout.as_millis(),
            "line_terminator": self.line_terminator,
            "response_delimiter": self.response_delimiter.to_string(),
        })
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<()> {
        if !config.is_object() {
            return Err(AdapterError::InvalidConfig("Config must be an object".to_string()).into());
        }
        if !config["port"].is_string() {
            return Err(AdapterError::InvalidConfig("Port must be a string".to_string()).into());
        }
        if !config["baud_rate"].is_u64() {
            return Err(AdapterError::InvalidConfig("Baud rate must be a number".to_string()).into());
        }
        Ok(())
    }

    async fn connect(&mut self, config: &serde_json::Value) -> Result<()> {
        self.port_name = config["port"].as_str().unwrap_or(&self.port_name).to_string();
        self.baud_rate = config["baud_rate"].as_u64().unwrap_or(self.baud_rate as u64) as u32;
        self.timeout = Duration::from_millis(
            config["timeout_ms"].as_u64().unwrap_or(self.timeout.as_millis() as u64),
        );
        self.line_terminator = config["line_terminator"]
            .as_str()
            .unwrap_or(&self.line_terminator)
            .to_string();
        self.response_delimiter = config["response_delimiter"]
            .as_str()
            .unwrap_or(&self.response_delimiter.to_string())
            .chars()
            .next()
            .unwrap_or(self.response_delimiter);

        #[cfg(feature = "instrument_serial")]
        {
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
            Err(AdapterError::ConnectionFailed("Serial feature disabled".to_string()).into())
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

    async fn send(&mut self, command: &str) -> Result<()> {
        #[cfg(feature = "instrument_serial")]
        {
            let port = self
                .port
                .as_ref()
                .ok_or(AdapterError::NotConnected)
                .map_err(anyhow::Error::from)?;

            let command_str = format!("{}{}", command, self.line_terminator);
            let port_clone = port.clone();
            let command_clone = command.to_string();

            // Execute blocking serial I/O on dedicated thread
            tokio::task::spawn_blocking(move || {
                use std::io::Write;

                let mut port_guard = port_clone.blocking_lock();

                // Write command
                port_guard
                    .write_all(command_str.as_bytes())
                    .context("Failed to write to serial port")?;

                port_guard.flush().context("Failed to flush serial port")?;

                debug!("Sent serial command: {}", command_clone.trim());
                Ok(())
            })
            .await
            .context("Serial I/O task panicked")?
        }

        #[cfg(not(feature = "instrument_serial"))]
        {
Err(AdapterError::SendFailed("Serial feature disabled".to_string()).into())
        }
    }

    async fn query(&mut self, query: &str) -> Result<String> {
        #[cfg(feature = "instrument_serial")]
        {
            let port = self
                .port
                .as_ref()
                .ok_or(AdapterError::NotConnected)
                .map_err(anyhow::Error::from)?;

            let command_str = format!("{}{}", query, self.line_terminator);
            let command_for_log = query.to_string(); // Clone for logging
            let delimiter = self.response_delimiter;
            let timeout = self.timeout;
            let port_clone = port.clone();

            // Execute blocking serial I/O on dedicated thread
            tokio::task::spawn_blocking(move || -> Result<String> {
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
                            return Err(AdapterError::ConnectionFailed("Unexpected EOF".to_string()).into());
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
        {
            let _ = query;
            Err(AdapterError::QueryFailed("Serial feature disabled".to_string()).into())
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_adapter_creation() {
        let adapter = SerialAdapter::new("/dev/ttyUSB0".to_string(), 9600);
        assert_eq!(adapter.name(), "serial");
        assert_eq!(adapter.port_name, "/dev/ttyUSB0");
        assert_eq!(adapter.baud_rate, 9600);
    }

    #[test]
    fn test_info_string() {
        let adapter = SerialAdapter::new("COM3".to_string(), 115200);
        let config = adapter.default_config();
        assert_eq!(config["port"], "COM3");
        assert_eq!(config["baud_rate"], 115200);
    }
}
