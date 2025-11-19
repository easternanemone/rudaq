//! VISA Hardware Adapter for GPIB/USB/Ethernet instruments
//!
//! Provides HardwareAdapter implementation for VISA communication protocol,
//! supporting instruments via GPIB, VXI, USB, Ethernet, etc.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use daq_core::{AdapterConfig, HardwareAdapter};
use std::time::Duration;

/// VISA adapter for instrument communication
///
/// This adapter wraps the visa-rs crate and provides async I/O
/// using Tokio's blocking task executor for synchronous VISA operations.
///
/// Supports resource strings like:
/// - "GPIB0::1::INSTR" (GPIB interface)
/// - "USB0::0x1234::0x5678::SERIAL::INSTR" (USB)
/// - "TCPIP0::192.168.1.100::INSTR" (Ethernet/LXI)
pub struct VisaAdapter {
    /// VISA resource string (e.g., "GPIB0::1::INSTR")
    pub(crate) resource_string: String,

    /// Read/write timeout
    pub(crate) timeout: Duration,

    /// Line terminator for commands (typically "\n" for SCPI)
    pub(crate) line_terminator: String,
}

const VISA_DEPRECATED: &str =
    "VISA integration has been deprecated in rust_daq; use serial drivers instead.";

impl VisaAdapter {
    /// Create a new VISA adapter with default settings
    ///
    /// # Arguments
    /// * `resource_string` - VISA resource identifier (e.g., "GPIB0::1::INSTR")
    pub fn new(resource_string: String) -> Self {
        Self {
            resource_string,
            timeout: Duration::from_secs(5),
            line_terminator: "\n".to_string(),
        }
    }

    /// Set read/write timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set line terminator for commands
    pub fn with_line_terminator(mut self, terminator: String) -> Self {
        self.line_terminator = terminator;
        self
    }

    /// Send a SCPI command and read the response asynchronously
    ///
    /// This method executes VISA I/O on a blocking thread to avoid
    /// blocking the Tokio runtime.
    ///
    /// # Arguments
    /// * `command` - SCPI command string (without terminator)
    ///
    /// # Returns
    /// Response string from the instrument (trimmed)
    pub async fn send_command(&self, _command: &str) -> Result<String> {
        Err(anyhow!(VISA_DEPRECATED))
    }

    /// Send a SCPI write command (no response expected)
    pub async fn send_write(&self, _command: &str) -> Result<()> {
        Err(anyhow!(VISA_DEPRECATED))
    }
}

// Clone removed - adapters should be wrapped in Arc for shared ownership
// Cloning would create a new, unconnected instance which is misleading

#[async_trait]
impl HardwareAdapter for VisaAdapter {
    async fn connect(&mut self, config: &AdapterConfig) -> Result<()> {
        let _ = config;
        Err(anyhow!(VISA_DEPRECATED))
    }

    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn reset(&mut self) -> Result<()> {
        Err(anyhow!(VISA_DEPRECATED))
    }

    fn is_connected(&self) -> bool {
        false
    }

    fn adapter_type(&self) -> &str {
        "visa"
    }

    fn info(&self) -> String {
        format!(
            "VisaAdapter({} @ {}ms timeout)",
            self.resource_string,
            self.timeout.as_millis()
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
    fn test_visa_adapter_creation() {
        let adapter = VisaAdapter::new("GPIB0::1::INSTR".to_string());
        assert_eq!(adapter.adapter_type(), "visa");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.resource_string, "GPIB0::1::INSTR");
        assert_eq!(adapter.timeout, Duration::from_secs(5));
    }

    #[test]
    fn test_visa_adapter_builder() {
        let adapter = VisaAdapter::new("USB0::0x1234::0x5678::SERIAL::INSTR".to_string())
            .with_timeout(Duration::from_millis(2000))
            .with_line_terminator("\r\n".to_string());

        assert_eq!(adapter.timeout, Duration::from_millis(2000));
        assert_eq!(adapter.line_terminator, "\r\n");
    }

    #[test]
    fn test_info_string() {
        let adapter = VisaAdapter::new("TCPIP0::192.168.1.100::INSTR".to_string())
            .with_timeout(Duration::from_millis(3000));
        let info = adapter.info();
        assert!(info.contains("TCPIP0::192.168.1.100::INSTR"));
        assert!(info.contains("3000ms"));
    }

    #[test]
    fn test_gpib_resource_string() {
        let adapter = VisaAdapter::new("GPIB0::5::INSTR".to_string());
        assert_eq!(adapter.resource_string, "GPIB0::5::INSTR");
    }

    #[test]
    fn test_usb_resource_string() {
        let adapter = VisaAdapter::new("USB0::0x1AB1::0x04CE::DS1ZA123456789::INSTR".to_string());
        assert_eq!(
            adapter.resource_string,
            "USB0::0x1AB1::0x04CE::DS1ZA123456789::INSTR"
        );
    }

    #[test]
    fn test_tcpip_resource_string() {
        let adapter = VisaAdapter::new("TCPIP0::192.168.0.10::inst0::INSTR".to_string());
        assert_eq!(adapter.resource_string, "TCPIP0::192.168.0.10::inst0::INSTR");
    }
}
