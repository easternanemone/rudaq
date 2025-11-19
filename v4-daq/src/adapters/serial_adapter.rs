//! Serial Hardware Adapter for RS-232/USB-Serial instruments
//!
//! Provides clean async serial communication for instruments like Newport 1830-C, ESP300, etc.
//! V4 version with minimal legacy dependencies.

use anyhow::{anyhow, Context, Result};
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

const DEFAULT_SERIAL_TIMEOUT_MS: u64 = 1000;

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
            timeout: Duration::from_millis(DEFAULT_SERIAL_TIMEOUT_MS),
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

        let cmd = format!("{}{}", command, self.line_terminator);
        let port_clone = Arc::clone(port);
        let delimiter = self.response_delimiter;

        tokio::task::spawn_blocking(move || {
            let mut port = port_clone.blocking_lock();
            port.write_all(cmd.as_bytes())?;
            port.flush()?;

            let mut response = String::new();
            let mut buf = [0; 256];

            loop {
                match port.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let chunk = String::from_utf8_lossy(&buf[..n]);
                        response.push_str(&chunk);
                        if response.ends_with(delimiter) {
                            break;
                        }
                    }
                    Ok(_) => break,
                    Err(e) => return Err(anyhow!(e)),
                }
            }

            Ok(response.trim().to_string())
        })
        .await?
    }

    /// Send a command without waiting for response (fire-and-forget)
    #[cfg(feature = "instrument_serial")]
    pub async fn send_command_no_response(&self, command: &str) -> Result<()> {
        let port = self
            .port
            .as_ref()
            .ok_or_else(|| anyhow!("Serial port not connected"))?;

        let cmd = format!("{}{}", command, self.line_terminator);
        let port_clone = Arc::clone(port);

        tokio::task::spawn_blocking(move || {
            let mut port = port_clone.blocking_lock();
            port.write_all(cmd.as_bytes())?;
            port.flush()?;
            Ok(())
        })
        .await?
    }

    /// Connect to the serial port
    #[cfg(feature = "instrument_serial")]
    pub async fn connect(&mut self) -> Result<()> {
        let port_name = self.port_name.clone();
        let baud_rate = self.baud_rate;
        let timeout = self.timeout;

        let port = tokio::task::spawn_blocking(move || {
            let port = serialport::new(&port_name, baud_rate)
                .timeout(timeout)
                .open()
                .with_context(|| format!("Failed to open serial port {}", port_name))?;

            Ok::<Box<dyn SerialPort>, anyhow::Error>(port)
        })
        .await??;

        self.port = Some(Arc::new(Mutex::new(port)));
        debug!("Connected to serial port: {}", self.port_name);
        Ok(())
    }

    /// Disconnect from the serial port
    #[cfg(feature = "instrument_serial")]
    pub async fn disconnect(&mut self) -> Result<()> {
        self.port = None;
        debug!("Disconnected from serial port: {}", self.port_name);
        Ok(())
    }

    /// Check if connected to serial port
    pub fn is_connected(&self) -> bool {
        #[cfg(feature = "instrument_serial")]
        {
            self.port.is_some()
        }
        #[cfg(not(feature = "instrument_serial"))]
        {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_adapter_creation() {
        let adapter = SerialAdapter::new("/dev/ttyUSB0".to_string(), 9600);
        assert_eq!(adapter.port_name, "/dev/ttyUSB0");
        assert_eq!(adapter.baud_rate, 9600);
        assert!(!adapter.is_connected());
    }

    #[test]
    fn test_builder_pattern() {
        let adapter = SerialAdapter::new("/dev/ttyUSB0".to_string(), 9600)
            .with_timeout(Duration::from_millis(500))
            .with_line_terminator("\n".to_string())
            .with_response_delimiter('\r');

        assert_eq!(adapter.timeout, Duration::from_millis(500));
        assert_eq!(adapter.line_terminator, "\n");
        assert_eq!(adapter.response_delimiter, '\r');
    }
}
