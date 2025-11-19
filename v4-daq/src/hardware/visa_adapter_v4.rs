//! V4 VISA Hardware Adapter
//!
//! Builder-based VISA adapter for SCPI instruments (oscilloscopes, power supplies, etc.).
//! Wraps visa-rs library with async interface for V4 actors.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[cfg(feature = "instrument_visa")]
use visa_rs::{DefaultRM, Instrument, VISA_SUCCESS};

/// Builder for constructing VisaAdapterV4 with custom configuration
///
/// Provides a safe, fluent interface for configuring VISA adapters
/// while preserving sensible defaults.
///
/// # Example
/// ```no_run
/// use std::time::Duration;
/// use v4_daq::hardware::VisaAdapterV4Builder;
///
/// let adapter = VisaAdapterV4Builder::new("TCPIP0::192.168.1.100::INSTR".to_string())
///     .with_timeout(Duration::from_millis(2000))
///     .with_read_terminator("\n".to_string())
///     .build()
///     .await?;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct VisaAdapterV4Builder {
    resource_name: String,
    timeout: Duration,
    read_terminator: String,
    write_terminator: String,
}

impl VisaAdapterV4Builder {
    /// Create a new builder with resource name
    ///
    /// # Arguments
    /// * `resource_name` - VISA resource string (e.g., "TCPIP0::192.168.1.100::INSTR")
    pub fn new(resource_name: String) -> Self {
        Self {
            resource_name,
            timeout: Duration::from_secs(1),
            read_terminator: "\n".to_string(),
            write_terminator: "\n".to_string(),
        }
    }

    /// Set timeout for VISA operations
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set read terminator character(s)
    pub fn with_read_terminator(mut self, terminator: String) -> Self {
        self.read_terminator = terminator;
        self
    }

    /// Set write terminator character(s)
    pub fn with_write_terminator(mut self, terminator: String) -> Self {
        self.write_terminator = terminator;
        self
    }

    /// Build the VisaAdapterV4 instance
    ///
    /// # Errors
    /// Returns error if VISA resource cannot be opened
    #[cfg(feature = "instrument_visa")]
    pub async fn build(self) -> Result<VisaAdapterV4> {
        VisaAdapterV4::new(
            self.resource_name,
            self.timeout,
            self.read_terminator,
            self.write_terminator,
        )
        .await
    }

    /// Build the VisaAdapterV4 instance (mock mode without VISA feature)
    #[cfg(not(feature = "instrument_visa"))]
    pub async fn build(self) -> Result<VisaAdapterV4> {
        Ok(VisaAdapterV4 {
            resource_name: self.resource_name,
            timeout: self.timeout,
            read_terminator: self.read_terminator,
            write_terminator: self.write_terminator,
        })
    }
}

/// V4 VISA Adapter for SCPI instruments
///
/// Async wrapper around visa-rs for use with V4 actor system.
///
/// ## Features
/// - Builder pattern with sensible defaults
/// - Configurable timeouts
/// - Configurable terminators
/// - Thread-safe with Arc<Mutex<>>
///
/// ## Hardware Support
/// - VISA over TCP/IP (LAN instruments)
/// - VISA over USB (USB instruments)
/// - VISA over GPIB (legacy instruments)
///
/// ## Example Usage
/// ```no_run
/// use v4_daq::hardware::VisaAdapterV4;
/// use std::time::Duration;
///
/// # async fn example() -> anyhow::Result<()> {
/// let adapter = VisaAdapterV4::new(
///     "TCPIP0::192.168.1.100::INSTR".to_string(),
///     Duration::from_secs(2),
///     "\n".to_string(),
///     "\n".to_string(),
/// ).await?;
///
/// let idn = adapter.query("*IDN?").await?;
/// println!("Instrument: {}", idn);
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "instrument_visa")]
pub struct VisaAdapterV4 {
    inner: Arc<Mutex<Instrument>>,
    resource_name: String,
    timeout: Duration,
    read_terminator: String,
    write_terminator: String,
}

/// Mock VISA adapter (when instrument_visa feature is disabled)
#[cfg(not(feature = "instrument_visa"))]
pub struct VisaAdapterV4 {
    resource_name: String,
    timeout: Duration,
    read_terminator: String,
    write_terminator: String,
}

#[cfg(feature = "instrument_visa")]
impl VisaAdapterV4 {
    /// Create new VISA adapter
    ///
    /// # Arguments
    /// * `resource_name` - VISA resource string
    /// * `timeout` - Timeout for VISA operations
    /// * `read_terminator` - String to append to reads
    /// * `write_terminator` - String to append to writes
    ///
    /// # Errors
    /// Returns error if VISA resource cannot be opened
    pub async fn new(
        resource_name: String,
        timeout: Duration,
        read_terminator: String,
        write_terminator: String,
    ) -> Result<Self> {
        // Initialize VISA resource manager
        let rm = DefaultRM::new()
            .with_context(|| "Failed to initialize VISA resource manager")?;

        // Open instrument
        let mut instr = rm
            .open(&resource_name)
            .with_context(|| format!("Failed to open VISA resource: {}", resource_name))?;

        // Set timeout (convert to milliseconds)
        let timeout_ms = timeout.as_millis() as u32;
        instr
            .set_timeout(timeout_ms)
            .with_context(|| format!("Failed to set VISA timeout to {}ms", timeout_ms))?;

        Ok(Self {
            inner: Arc::new(Mutex::new(instr)),
            resource_name,
            timeout,
            read_terminator,
            write_terminator,
        })
    }

    /// Send query and read response
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string (e.g., "*IDN?", "MEAS:VOLT:DC?")
    ///
    /// # Errors
    /// - VISA communication error
    /// - Timeout
    /// - Malformed response
    pub async fn query(&self, cmd: &str) -> Result<String> {
        let mut instr = self.inner.lock().await;

        // Write command with terminator
        let write_cmd = format!("{}{}", cmd, self.write_terminator);
        instr
            .write_all(write_cmd.as_bytes())
            .with_context(|| format!("Failed to write VISA command: {}", cmd))?;

        // Read response
        let mut buf = [0u8; 4096];
        let (_, ret) = instr
            .read(&mut buf)
            .with_context(|| format!("Failed to read VISA response for: {}", cmd))?;

        if ret != VISA_SUCCESS {
            anyhow::bail!("VISA read error: status code {}", ret);
        }

        // Convert to string and trim terminator
        let response = String::from_utf8_lossy(&buf)
            .trim_end_matches('\0')
            .trim_end_matches(&self.read_terminator)
            .trim()
            .to_string();

        Ok(response)
    }

    /// Send query with explicit timeout
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string
    /// * `timeout` - Override default timeout
    ///
    /// # Errors
    /// - VISA communication error
    /// - Timeout exceeded
    pub async fn query_with_timeout(&self, cmd: &str, timeout: Duration) -> Result<String> {
        let mut instr = self.inner.lock().await;

        // Save original timeout
        let timeout_ms = timeout.as_millis() as u32;
        instr
            .set_timeout(timeout_ms)
            .with_context(|| "Failed to set temporary VISA timeout")?;

        // Write command
        let write_cmd = format!("{}{}", cmd, self.write_terminator);
        instr
            .write_all(write_cmd.as_bytes())
            .with_context(|| format!("Failed to write VISA command: {}", cmd))?;

        // Read response
        let mut buf = [0u8; 4096];
        let (_, ret) = instr
            .read(&mut buf)
            .with_context(|| format!("Failed to read VISA response for: {}", cmd))?;

        if ret != VISA_SUCCESS {
            anyhow::bail!("VISA read error: status code {}", ret);
        }

        // Restore original timeout
        let original_timeout_ms = self.timeout.as_millis() as u32;
        instr
            .set_timeout(original_timeout_ms)
            .with_context(|| "Failed to restore original VISA timeout")?;

        let response = String::from_utf8_lossy(&buf)
            .trim_end_matches('\0')
            .trim_end_matches(&self.read_terminator)
            .trim()
            .to_string();

        Ok(response)
    }

    /// Send command without expecting response
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string (e.g., "*RST", "*CLS")
    ///
    /// # Errors
    /// - VISA communication error
    pub async fn write(&self, cmd: &str) -> Result<()> {
        let mut instr = self.inner.lock().await;

        let write_cmd = format!("{}{}", cmd, self.write_terminator);
        instr
            .write_all(write_cmd.as_bytes())
            .with_context(|| format!("Failed to write VISA command: {}", cmd))?;

        Ok(())
    }

    /// Check if VISA connection is active
    pub async fn is_connected(&self) -> bool {
        // Try a simple query to check connectivity
        self.query("*IDN?").await.is_ok()
    }

    /// Get current timeout setting
    pub fn get_timeout(&self) -> Duration {
        self.timeout
    }

    /// Get resource name
    pub fn resource_name(&self) -> &str {
        &self.resource_name
    }
}

#[cfg(not(feature = "instrument_visa"))]
impl VisaAdapterV4 {
    /// Mock implementation (returns error)
    pub async fn query(&self, _cmd: &str) -> Result<String> {
        anyhow::bail!("VISA support not compiled (enable instrument_visa feature)")
    }

    /// Mock implementation (returns error)
    pub async fn query_with_timeout(&self, _cmd: &str, _timeout: Duration) -> Result<String> {
        anyhow::bail!("VISA support not compiled (enable instrument_visa feature)")
    }

    /// Mock implementation (returns error)
    pub async fn write(&self, _cmd: &str) -> Result<()> {
        anyhow::bail!("VISA support not compiled (enable instrument_visa feature)")
    }

    /// Mock implementation (always returns false)
    pub async fn is_connected(&self) -> bool {
        false
    }

    /// Get current timeout setting
    pub fn get_timeout(&self) -> Duration {
        self.timeout
    }

    /// Get resource name
    pub fn resource_name(&self) -> &str {
        &self.resource_name
    }
}

impl Clone for VisaAdapterV4 {
    fn clone(&self) -> Self {
        #[cfg(feature = "instrument_visa")]
        {
            Self {
                inner: Arc::clone(&self.inner),
                resource_name: self.resource_name.clone(),
                timeout: self.timeout,
                read_terminator: self.read_terminator.clone(),
                write_terminator: self.write_terminator.clone(),
            }
        }

        #[cfg(not(feature = "instrument_visa"))]
        {
            Self {
                resource_name: self.resource_name.clone(),
                timeout: self.timeout,
                read_terminator: self.read_terminator.clone(),
                write_terminator: self.write_terminator.clone(),
            }
        }
    }
}
