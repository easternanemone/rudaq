//! V4 Serial Hardware Adapter
//!
//! Lightweight wrapper around the existing SerialAdapter for V4 actors.
//! Provides async serial communication for instruments like Newport 1830-C.

use crate::adapters::SerialAdapter;
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Builder for constructing SerialAdapterV4 with custom configuration
///
/// Provides a safe, fluent interface for configuring serial adapters
/// while preserving sensible defaults.
///
/// # Example
/// ```no_run
/// use std::time::Duration;
/// use v4_daq::hardware::SerialAdapterV4Builder;
///
/// let adapter = SerialAdapterV4Builder::new("/dev/ttyUSB0".to_string(), 9600)
///     .with_timeout(Duration::from_millis(500))
///     .build();
/// ```
pub struct SerialAdapterV4Builder {
    port_name: String,
    baud_rate: u32,
    timeout: Duration,
    line_terminator: String,
    response_delimiter: char,
}

impl SerialAdapterV4Builder {
    /// Create a new builder with required parameters
    ///
    /// # Arguments
    /// * `port_name` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `baud_rate` - Communication speed (e.g., 9600, 115200)
    ///
    /// Default configuration:
    /// * timeout: 1 second
    /// * line_terminator: "\r\n"
    /// * response_delimiter: '\n'
    pub fn new(port_name: String, baud_rate: u32) -> Self {
        Self {
            port_name,
            baud_rate,
            timeout: Duration::from_secs(1),
            line_terminator: "\r\n".to_string(),
            response_delimiter: '\n',
        }
    }

    /// Set the read timeout duration
    ///
    /// Default: 1 second
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the line terminator string for commands
    ///
    /// Default: "\r\n" (CRLF)
    pub fn with_line_terminator(mut self, terminator: String) -> Self {
        self.line_terminator = terminator;
        self
    }

    /// Set the response delimiter character
    ///
    /// Default: '\n' (newline)
    pub fn with_response_delimiter(mut self, delimiter: char) -> Self {
        self.response_delimiter = delimiter;
        self
    }

    /// Build the SerialAdapterV4 with the configured settings
    pub fn build(self) -> SerialAdapterV4 {
        let inner = SerialAdapter::new(self.port_name, self.baud_rate)
            .with_timeout(self.timeout)
            .with_line_terminator(self.line_terminator)
            .with_response_delimiter(self.response_delimiter);

        SerialAdapterV4 {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

/// V4 Serial adapter for RS-232/USB-Serial instruments
///
/// This wraps the existing SerialAdapter to provide a V4-compatible
/// interface for Kameo actors. It maintains async I/O using Tokio's
/// blocking task executor.
#[derive(Clone)]
pub struct SerialAdapterV4 {
    /// The underlying serial adapter
    inner: Arc<Mutex<SerialAdapter>>,
}

impl SerialAdapterV4 {
    /// Create a new V4 serial adapter with default configuration
    ///
    /// # Arguments
    /// * `port_name` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `baud_rate` - Communication speed (e.g., 9600, 115200)
    ///
    /// # Example
    /// ```no_run
    /// use v4_daq::hardware::SerialAdapterV4;
    ///
    /// let adapter = SerialAdapterV4::new("/dev/ttyUSB0".to_string(), 9600);
    /// ```
    pub fn new(port_name: String, baud_rate: u32) -> Self {
        SerialAdapterV4Builder::new(port_name, baud_rate).build()
    }

    /// Send a command and read the response
    ///
    /// # Arguments
    /// * `command` - The command string to send (line terminator added automatically)
    ///
    /// # Returns
    /// The trimmed response string
    ///
    /// # Example (Newport 1830-C)
    /// ```no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use v4_daq::hardware::SerialAdapterV4;
    /// let adapter = SerialAdapterV4::new("/dev/ttyUSB0".to_string(), 9600);
    ///
    /// // Set wavelength to 780 nm
    /// adapter.send_command("PM:Lambda 780").await?;
    ///
    /// // Read power
    /// let response = adapter.send_command("PM:Power?").await?;
    /// let power: f64 = response.parse()?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "instrument_serial")]
    pub async fn send_command(&self, command: &str) -> Result<String> {
        self.inner
            .lock()
            .await
            .send_command(command)
            .await
            .context("Serial command failed")
    }

    /// Send a command without waiting for response
    ///
    /// Useful for fire-and-forget commands that don't return a response.
    #[cfg(feature = "instrument_serial")]
    pub async fn send_command_no_response(&self, command: &str) -> Result<()> {
        self.inner
            .lock()
            .await
            .send_command_no_response(command)
            .await
            .context("Serial command failed")
    }

    /// Send query and strip command echo from response
    ///
    /// Some instruments (e.g., MaiTai laser) echo the command in their response:
    /// - Query: "WAVELENGTH?"
    /// - Response: "WAVELENGTH:800"
    ///
    /// This method extracts the value after the separator character.
    ///
    /// # Arguments
    /// * `command` - SCPI command to send
    /// * `separator` - Character that separates echo from value (commonly ':' or '=')
    ///
    /// # Returns
    /// The value portion of the response (after separator), or full response if no separator
    ///
    /// # Example (MaiTai)
    /// ```no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use v4_daq::hardware::SerialAdapterV4;
    /// let adapter = SerialAdapterV4::new("/dev/ttyUSB0".to_string(), 9600);
    ///
    /// // Query returns "WAVELENGTH:800"
    /// let value = adapter.query_with_echo_strip("WAVELENGTH?", ':').await?;
    /// // value = "800"
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "instrument_serial")]
    pub async fn query_with_echo_strip(&self, command: &str, separator: char) -> Result<String> {
        let response = self.send_command(command).await?;

        // Some instruments echo command: "WAVELENGTH:800"
        // Extract value after separator
        let value = response
            .split(separator)
            .last()
            .unwrap_or(&response)
            .trim()
            .to_string();

        Ok(value)
    }

    /// Check if the adapter is connected to a serial port
    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.is_connected()
    }

    /// Connect to the serial port
    #[cfg(feature = "instrument_serial")]
    pub async fn connect(&self) -> Result<()> {
        let mut adapter = self.inner.lock().await;
        adapter
            .connect()
            .await
            .context("Failed to connect serial adapter")
    }

    /// Connect to the serial port (no-op when feature not enabled)
    #[cfg(not(feature = "instrument_serial"))]
    pub async fn connect(&self) -> Result<()> {
        Err(anyhow::anyhow!(
            "instrument_serial feature not enabled"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_adapter_creation() {
        let adapter = SerialAdapterV4::new("/dev/ttyUSB0".to_string(), 9600);
        // Should not panic
        drop(adapter);
    }

    #[test]
    fn test_serial_adapter_builder_defaults() {
        let adapter = SerialAdapterV4Builder::new("/dev/ttyUSB0".to_string(), 9600).build();
        // Should create successfully with defaults
        drop(adapter);
    }

    #[test]
    fn test_serial_adapter_builder_with_custom_timeout() {
        let adapter = SerialAdapterV4Builder::new("/dev/ttyUSB0".to_string(), 9600)
            .with_timeout(Duration::from_millis(500))
            .build();
        // Should create successfully with custom timeout
        drop(adapter);
    }

    #[test]
    fn test_serial_adapter_builder_with_custom_terminators() {
        let adapter = SerialAdapterV4Builder::new("/dev/ttyUSB0".to_string(), 9600)
            .with_line_terminator("\n".to_string())
            .with_response_delimiter('\r')
            .build();
        // Should create successfully with custom terminators
        drop(adapter);
    }

    #[test]
    fn test_serial_adapter_builder_fluent_api() {
        let adapter = SerialAdapterV4Builder::new("/dev/ttyUSB0".to_string(), 115200)
            .with_timeout(Duration::from_millis(2000))
            .with_line_terminator("\r\n".to_string())
            .with_response_delimiter('\n')
            .build();
        // Should chain methods successfully
        drop(adapter);
    }
}
