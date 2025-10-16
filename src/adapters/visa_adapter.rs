//! VISA Hardware Adapter for GPIB/USB/Ethernet instruments
//!
//! Provides HardwareAdapter implementation for VISA communication protocol,
//! supporting instruments via GPIB, VXI, USB, Ethernet, etc.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use daq_core::{AdapterConfig, HardwareAdapter};
use std::time::Duration;

#[cfg(feature = "instrument_visa")]
use anyhow::Context;
#[cfg(feature = "instrument_visa")]
use log::debug;
#[cfg(feature = "instrument_visa")]
use std::sync::Arc;
#[cfg(feature = "instrument_visa")]
use tokio::sync::Mutex;

#[cfg(feature = "instrument_visa")]
use visa_rs::{DefaultRM, Instrument, VISA};

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

    /// The actual VISA instrument (behind Arc<Mutex> for async access)
    #[cfg(feature = "instrument_visa")]
    instrument: Option<Arc<Mutex<Box<dyn Instrument>>>>,
}

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
            #[cfg(feature = "instrument_visa")]
            instrument: None,
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
    #[cfg(feature = "instrument_visa")]
    pub async fn send_command(&self, command: &str) -> Result<String> {
        let instrument = self
            .instrument
            .as_ref()
            .ok_or_else(|| anyhow!("VISA instrument not connected"))?;

        let command_str = format!("{}{}", command, self.line_terminator);
        let command_for_log = command.to_string();
        let instrument_clone = instrument.clone();
        let timeout = self.timeout;

        // Execute blocking VISA I/O on dedicated thread
        tokio::task::spawn_blocking(move || {
            let mut instr_guard = instrument_clone.blocking_lock();

            // Set timeout
            instr_guard
                .set_timeout(timeout.as_millis() as u32)
                .context("Failed to set VISA timeout")?;

            // For query commands (ending with ?), use query method
            if command_for_log.trim().ends_with('?') {
                // SCPI query - expect response
                let response = instr_guard
                    .query(&command_str)
                    .with_context(|| format!("VISA query failed for: {}", command_for_log))?;

                let response = response.trim().to_string();
                debug!("VISA query '{}' -> '{}'", command_for_log.trim(), response);
                Ok(response)
            } else {
                // SCPI command - write only
                instr_guard
                    .write(&command_str)
                    .with_context(|| format!("VISA write failed for: {}", command_for_log))?;

                debug!("VISA command sent: {}", command_for_log.trim());
                Ok(String::new())
            }
        })
        .await
        .context("VISA I/O task panicked")?
    }

    #[cfg(not(feature = "instrument_visa"))]
    pub async fn send_command(&self, _command: &str) -> Result<String> {
        Err(anyhow!(
            "VISA support not enabled. Rebuild with --features instrument_visa"
        ))
    }

    /// Send a SCPI write command (no response expected)
    #[cfg(feature = "instrument_visa")]
    pub async fn send_write(&self, command: &str) -> Result<()> {
        let instrument = self
            .instrument
            .as_ref()
            .ok_or_else(|| anyhow!("VISA instrument not connected"))?;

        let command_str = format!("{}{}", command, self.line_terminator);
        let command_for_log = command.to_string();
        let instrument_clone = instrument.clone();
        let timeout = self.timeout;

        tokio::task::spawn_blocking(move || {
            let mut instr_guard = instrument_clone.blocking_lock();

            instr_guard
                .set_timeout(timeout.as_millis() as u32)
                .context("Failed to set VISA timeout")?;

            instr_guard
                .write(&command_str)
                .with_context(|| format!("VISA write failed for: {}", command_for_log))?;

            debug!("VISA write sent: {}", command_for_log.trim());
            Ok(())
        })
        .await
        .context("VISA write task panicked")?
    }

    #[cfg(not(feature = "instrument_visa"))]
    pub async fn send_write(&self, _command: &str) -> Result<()> {
        Err(anyhow!(
            "VISA support not enabled. Rebuild with --features instrument_visa"
        ))
    }
}

// Clone removed - adapters should be wrapped in Arc for shared ownership
// Cloning would create a new, unconnected instance which is misleading

#[async_trait]
impl HardwareAdapter for VisaAdapter {
    async fn connect(&mut self, config: &AdapterConfig) -> Result<()> {
        #[cfg(feature = "instrument_visa")]
        {
            // Override settings from config if provided
            if let Some(timeout_ms) = config.params.get("timeout_ms").and_then(|v| v.as_u64()) {
                self.timeout = Duration::from_millis(timeout_ms);
            }

            if let Some(terminator) = config.params.get("line_terminator").and_then(|v| v.as_str())
            {
                self.line_terminator = terminator.to_string();
            }

            // Open VISA resource
            let resource_str = self.resource_string.clone();
            let timeout_ms = self.timeout.as_millis() as u32;

            let instrument = tokio::task::spawn_blocking(move || {
                let rm = DefaultRM::new().context("Failed to create VISA resource manager")?;

                let instr = rm
                    .open(&resource_str, timeout_ms, 0)
                    .with_context(|| format!("Failed to open VISA resource: {}", resource_str))?;

                Ok::<Box<dyn Instrument>, anyhow::Error>(instr)
            })
            .await
            .context("VISA open task panicked")??;

            self.instrument = Some(Arc::new(Mutex::new(instrument)));

            debug!(
                "VISA resource '{}' opened with {}ms timeout",
                self.resource_string,
                self.timeout.as_millis()
            );
            Ok(())
        }

        #[cfg(not(feature = "instrument_visa"))]
        {
            let _ = config;
            Err(anyhow!(
                "VISA support not enabled. Rebuild with --features instrument_visa"
            ))
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        #[cfg(feature = "instrument_visa")]
        {
            if self.instrument.is_some() {
                self.instrument = None;
                debug!("VISA resource '{}' closed", self.resource_string);
            }
        }
        Ok(())
    }

    async fn reset(&mut self) -> Result<()> {
        #[cfg(feature = "instrument_visa")]
        {
            // For VISA instruments, send *RST command before disconnect/reconnect
            if let Some(ref instrument) = self.instrument {
                let instrument_clone = instrument.clone();
                let timeout = self.timeout;

                tokio::task::spawn_blocking(move || {
                    let mut instr_guard = instrument_clone.blocking_lock();
                    instr_guard
                        .set_timeout(timeout.as_millis() as u32)
                        .context("Failed to set timeout")?;
                    instr_guard
                        .write("*RST\n")
                        .context("Failed to send *RST command")?;
                    Ok::<(), anyhow::Error>(())
                })
                .await
                .context("VISA reset task panicked")??;

                debug!("Sent *RST to VISA instrument");
            }
        }

        // Disconnect and reconnect after delay
        self.disconnect().await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        self.connect(&AdapterConfig::default()).await
    }

    fn is_connected(&self) -> bool {
        #[cfg(feature = "instrument_visa")]
        {
            self.instrument.is_some()
        }

        #[cfg(not(feature = "instrument_visa"))]
        {
            false
        }
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
