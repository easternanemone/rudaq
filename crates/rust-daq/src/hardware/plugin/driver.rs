//! Generic instrument driver for YAML-defined plugins.
//!
//! This module provides `GenericDriver`, a runtime-configurable driver that
//! interprets YAML plugin definitions to control serial or TCP instruments without
//! recompilation.

use anyhow::{anyhow, Result};
use rand::Rng;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use strfmt::strfmt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};
use tokio_serial::SerialStream;

use crate::hardware::plugin::schema::{CommandSequence, InstrumentConfig, ValueType};
use crate::observable::ParameterSet; // NEW: For Parameterized trait implementation

// =============================================================================
// Connection Enum - Unified abstraction over Serial, TCP, and Mock
// =============================================================================

/// Unified connection type supporting serial, TCP, and mock communication.
///
/// Serial and TCP variants implement AsyncRead + AsyncWrite, while Mock
/// variant bypasses actual I/O for testing without hardware.
pub enum Connection {
    /// Serial port connection (RS-232, USB-Serial, etc.)
    Serial(SerialStream),
    /// TCP/IP network connection
    Tcp(TcpStream),
    /// Mock connection for testing without hardware
    Mock,
}

impl Connection {
    /// Writes data to the connection.
    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            Connection::Serial(s) => s.write_all(buf).await,
            Connection::Tcp(t) => t.write_all(buf).await,
            Connection::Mock => Ok(()),
        }
    }

    /// Reads data from the connection into the buffer.
    /// Returns the number of bytes read.
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Connection::Serial(s) => s.read(buf).await,
            Connection::Tcp(t) => t.read(buf).await,
            Connection::Mock => Ok(0),
        }
    }

    /// Reads exactly `n` bytes from the connection.
    /// This is essential for binary frame data where we know the exact size.
    async fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        match self {
            Connection::Serial(s) => {
                s.read_exact(buf).await?;
                Ok(())
            }
            Connection::Tcp(t) => {
                t.read_exact(buf).await?;
                Ok(())
            }
            Connection::Mock => {
                // Fill with zeros for mock mode
                buf.fill(0);
                Ok(())
            }
        }
    }
}

// =============================================================================
// Parameterized Trait Implementation (bd-plb6)
// =============================================================================

impl crate::hardware::capabilities::Parameterized for GenericDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.parameters
    }
}

/// A generic instrument driver that interprets commands and responses based on a YAML configuration.
///
/// This driver uses interior mutability (`Mutex`) for the connection, allowing
/// all methods to take `&self` instead of `&mut self`. This enables sharing the
/// driver via `Arc<GenericDriver>` for capability handles.
///
/// # Supported Transports
///
/// - **Serial**: RS-232, USB-Serial adapters (driver_type: serial_scpi, serial_raw)
/// - **TCP/IP**: Network instruments (driver_type: tcp_scpi, tcp_raw)
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
/// let driver = Arc::new(GenericDriver::new_serial(config, serial_port)?);
/// let power = driver.read_named_f64("power", false).await?;
/// ```
pub struct GenericDriver {
    /// The instrument configuration loaded from YAML.
    pub config: InstrumentConfig,

    /// Connection wrapped in Mutex for interior mutability.
    /// This allows `&self` methods while still enabling writes.
    connection: Mutex<Connection>,

    /// Runtime state storage for capability values.
    state: RwLock<HashMap<String, Value>>,

    /// Compiled regex patterns for error detection.
    error_patterns: Vec<Regex>,

    /// Termination bytes for command/response framing.
    termination_bytes: Vec<u8>,

    /// Broadcast channel for streaming frames to multiple subscribers.
    frame_broadcaster: tokio::sync::broadcast::Sender<std::sync::Arc<crate::hardware::Frame>>,

    /// Frame counter for tracking acquisition progress.
    frame_counter: std::sync::atomic::AtomicU64,

    /// Streaming state flag.
    is_streaming: std::sync::atomic::AtomicBool,

    /// Parameter registry for exposing settable parameters (bd-plb6)
    ///
    /// Populated from YAML config.capabilities.settable during initialization.
    /// Enables generic parameter access via Parameterized trait.
    parameters: ParameterSet,
}

impl GenericDriver {
    /// Creates a new GenericDriver from an InstrumentConfig and a serial port.
    ///
    /// # Arguments
    /// * `config` - The instrument configuration loaded from YAML
    /// * `port` - An open tokio_serial::SerialStream
    ///
    /// # Returns
    /// A new GenericDriver instance, or an error if regex compilation fails.
    #[deprecated(since = "0.2.0", note = "use new_serial instead")]
    pub fn new(config: InstrumentConfig, port: SerialStream) -> Result<Self> {
        Self::new_serial(config, port)
    }

    /// Creates a new GenericDriver from an InstrumentConfig and a serial port.
    ///
    /// # Arguments
    /// * `config` - The instrument configuration loaded from YAML
    /// * `port` - An open tokio_serial::SerialStream
    ///
    /// # Returns
    /// A new GenericDriver instance, or an error if regex compilation fails.
    pub fn new_serial(config: InstrumentConfig, port: SerialStream) -> Result<Self> {
        Self::new_with_connection(config, Connection::Serial(port))
    }

    /// Creates a new GenericDriver from an InstrumentConfig and a TCP stream.
    ///
    /// # Arguments
    /// * `config` - The instrument configuration loaded from YAML
    /// * `stream` - An open tokio::net::TcpStream
    ///
    /// # Returns
    /// A new GenericDriver instance, or an error if regex compilation fails.
    pub fn new_tcp(config: InstrumentConfig, stream: TcpStream) -> Result<Self> {
        Self::new_with_connection(config, Connection::Tcp(stream))
    }

    /// Creates a new GenericDriver in mock mode (no hardware required).
    ///
    /// This constructor allows testing and development without physical hardware.
    /// All operations will use mock data and in-memory state tracking.
    ///
    /// # Arguments
    /// * `config` - The instrument configuration loaded from YAML
    ///
    /// # Returns
    /// A new GenericDriver instance in mock mode, or an error if regex compilation fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    /// let driver = Arc::new(GenericDriver::new_mock(config)?);
    /// let power = driver.read_named_f64("power", true).await?; // Always pass true for is_mocking
    /// ```
    pub fn new_mock(config: InstrumentConfig) -> Result<Self> {
        Self::new_with_connection(config, Connection::Mock)
    }

    /// Internal constructor that handles any connection type.
    fn new_with_connection(config: InstrumentConfig, connection: Connection) -> Result<Self> {
        let error_patterns = config
            .error_patterns
            .iter()
            .map(|pattern| Regex::new(pattern))
            .collect::<Result<Vec<Regex>, regex::Error>>()
            .map_err(|e| anyhow!("Failed to compile error regex: {}", e))?;

        let termination_bytes = config.protocol.termination.as_bytes().to_vec();

        // Create broadcast channel with capacity for 100 frames
        let (frame_tx, _) = tokio::sync::broadcast::channel(100);

        Ok(Self {
            config,
            connection: Mutex::new(connection),
            state: RwLock::new(HashMap::new()),
            error_patterns,
            termination_bytes,
            frame_broadcaster: frame_tx,
            frame_counter: std::sync::atomic::AtomicU64::new(0),
            is_streaming: std::sync::atomic::AtomicBool::new(false),
            parameters: ParameterSet::new(),
        })
    }

    /// Sends a command to the instrument and reads its response.
    ///
    /// Uses interior mutability via Mutex to allow `&self` signature.
    async fn execute_command(&self, command: &str) -> Result<String> {
        let cmd_bytes = command.as_bytes();
        let timeout_duration = Duration::from_millis(self.config.protocol.timeout_ms);
        let command_delay = Duration::from_millis(self.config.protocol.command_delay_ms);

        // Combine command and termination bytes for a single write operation
        let mut full_command_bytes = Vec::new();
        full_command_bytes.extend_from_slice(cmd_bytes);
        full_command_bytes.extend_from_slice(&self.termination_bytes);

        // Lock the connection for the duration of this command/response cycle
        let mut conn = self.connection.lock().await;

        tracing::debug!(
            "Sending command: {:?}",
            String::from_utf8_lossy(&full_command_bytes)
        );

        // Send command
        tokio::time::timeout(timeout_duration, conn.write_all(&full_command_bytes)).await??;

        // Apply command delay
        if command_delay.as_millis() > 0 {
            tokio::time::sleep(command_delay).await;
        }

        // Read response
        let mut response_bytes = Vec::new();
        let mut buf = [0u8; 128];
        let mut deadline = tokio::time::Instant::now() + timeout_duration;

        loop {
            let n = tokio::time::timeout_at(deadline, conn.read(&mut buf)).await??;
            if n == 0 {
                tracing::debug!("Received EOF from port.");
                break;
            }
            response_bytes.extend_from_slice(&buf[..n]);

            if response_bytes.ends_with(&self.termination_bytes) {
                tracing::debug!("Received termination bytes. Breaking read loop.");
                break;
            }
            deadline = tokio::time::Instant::now() + timeout_duration;
        }

        // Connection lock is released here when `conn` goes out of scope

        let response_str = String::from_utf8(response_bytes)?;
        tracing::debug!("Received response: {:?}", response_str);

        // Check for error patterns
        for pattern in &self.error_patterns {
            if pattern.is_match(&response_str) {
                return Err(anyhow!("Instrument reported error: {}", response_str));
            }
        }

        Ok(response_str)
    }

    /// Executes a sequence of commands, typically for on_connect or on_disconnect.
    pub async fn execute_command_sequence(&self, sequence: &[CommandSequence]) -> Result<()> {
        for cmd_seq in sequence {
            let _ = self.execute_command(&cmd_seq.cmd).await?;
            if cmd_seq.wait_ms > 0 {
                tokio::time::sleep(Duration::from_millis(cmd_seq.wait_ms)).await;
            }
        }
        Ok(())
    }

    /// Sends a command and reads a fixed number of bytes as binary response.
    ///
    /// This is essential for frame acquisition where the response is raw binary
    /// pixel data, not text with termination characters.
    ///
    /// # Arguments
    /// * `command` - Command to send (e.g., "GET_FRAME")
    /// * `expected_bytes` - Number of bytes to read (e.g., width * height * 2 for u16)
    ///
    /// # Returns
    /// Vec<u8> containing the raw binary data
    async fn execute_binary_command(
        &self,
        command: &str,
        expected_bytes: usize,
    ) -> Result<Vec<u8>> {
        let timeout_duration = Duration::from_millis(self.config.protocol.timeout_ms);
        let command_delay = Duration::from_millis(self.config.protocol.command_delay_ms);

        // Combine command and termination bytes
        let mut full_command_bytes = Vec::new();
        full_command_bytes.extend_from_slice(command.as_bytes());
        full_command_bytes.extend_from_slice(&self.termination_bytes);

        // Lock the connection
        let mut conn = self.connection.lock().await;

        tracing::debug!(
            "Sending binary command: {:?}, expecting {} bytes",
            command,
            expected_bytes
        );

        // Send command
        tokio::time::timeout(timeout_duration, conn.write_all(&full_command_bytes)).await??;

        // Apply command delay
        if command_delay.as_millis() > 0 {
            tokio::time::sleep(command_delay).await;
        }

        // Read exact number of bytes
        let mut response_bytes = vec![0u8; expected_bytes];
        tokio::time::timeout(timeout_duration, conn.read_exact(&mut response_bytes)).await??;

        tracing::debug!("Received {} bytes of binary data", response_bytes.len());

        Ok(response_bytes)
    }

    /// Acquires a single frame from the instrument.
    ///
    /// Sends the frame_cmd and reads width * height * bytes_per_pixel bytes.
    /// Assumes 16-bit pixels (2 bytes per pixel) which is standard for scientific cameras.
    async fn acquire_frame(
        &self,
        frame_producer: &crate::hardware::plugin::schema::FrameProducerCapability,
    ) -> Result<crate::hardware::Frame> {
        let width = frame_producer.width;
        let height = frame_producer.height;
        let bytes_per_pixel = 2usize; // 16-bit pixels
        let frame_size = (width * height) as usize * bytes_per_pixel;

        // Execute frame command and read binary response
        let raw_bytes = self
            .execute_binary_command(&frame_producer.frame_cmd, frame_size)
            .await?;

        // Convert raw bytes to u16 pixels (little-endian)
        let mut buffer = vec![0u16; (width * height) as usize];
        for (i, chunk) in raw_bytes.chunks_exact(2).enumerate() {
            buffer[i] = u16::from_le_bytes([chunk[0], chunk[1]]);
        }

        Ok(crate::hardware::Frame::new(width, height, buffer))
    }

    /// Tries to parse a value from a response string using a given pattern.
    /// Returns a serde_json::Value.
    fn parse_response(
        &self,
        response: &str,
        pattern: &str,
        value_type: ValueType,
    ) -> Result<Value> {
        // Convert friendly pattern (e.g., "TEMP:{val:f}") to regex pattern
        // Strategy: First replace placeholders with temporary markers,
        // then escape special chars, then replace markers with regex groups

        // Step 1: Replace placeholders with unique markers
        let with_markers = pattern
            .replace("{val:f}", "\x00FLOAT\x00")
            .replace("{val:i}", "\x00INT\x00")
            .replace("{val}", "\x00VAL\x00");

        // Step 2: Escape regex special characters in the user's pattern
        let escaped = regex::escape(&with_markers);

        // Step 3: Replace markers with proper regex capture groups
        let regex_pattern_str = escaped
            .replace("\x00FLOAT\x00", "(?P<val>[+-]?([0-9]*[.])?[0-9]+)")
            .replace("\x00INT\x00", "(?P<val>[+-]?\\d+)")
            .replace("\x00VAL\x00", "(?P<val>.*?)");

        let full_regex_pattern = format!("^{}$", regex_pattern_str); // Ensure full string match
        let regex = Regex::new(&full_regex_pattern).map_err(|e| {
            anyhow!(
                "Failed to compile parsing regex '{}': {}",
                full_regex_pattern,
                e
            )
        })?;

        let captures = regex.captures(response).ok_or_else(|| {
            anyhow!(
                "Response '{}' did not match pattern '{}' (regex: {})",
                response,
                pattern,
                full_regex_pattern
            )
        })?;

        let captured_val_str = captures
            .name("val")
            .ok_or_else(|| {
                anyhow!(
                    "Pattern '{}' did not capture 'val' group in response '{}'",
                    pattern,
                    response
                )
            })?
            .as_str();

        match value_type {
            ValueType::Float => Ok(Value::from(captured_val_str.parse::<f64>()?)),
            ValueType::Int => Ok(Value::from(captured_val_str.parse::<i64>()?)),
            ValueType::String | ValueType::Enum => Ok(Value::from(captured_val_str.to_string())),
            ValueType::Bool => {
                let lower = captured_val_str.to_lowercase();
                Ok(Value::from(
                    lower == "on" || lower == "true" || lower == "1",
                ))
            }
        }
    }

    /// Returns a mock value if mock data is configured, otherwise an error indicating no mock.
    fn get_mock_value(
        mock_data: &Option<crate::hardware::plugin::schema::MockData>,
    ) -> Result<Value> {
        if let Some(mock) = mock_data {
            let mut rng = rand::thread_rng();
            let jitter_amount = rng.gen_range(-mock.jitter..=mock.jitter);
            Ok(Value::from(mock.default + jitter_amount))
        } else {
            Err(anyhow!("No mock data configured for this capability"))
        }
    }

    /// Reads a specific named readable capability, optionally using mock data.
    ///
    /// This method allows the GenericDriver to perform a "read" operation for a specific
    /// named capability defined in its configuration.
    ///
    /// # Arguments
    /// * `capability_name` - The `name` of the `ReadableCapability` to read, as defined in the YAML.
    /// * `is_mocking` - If true, mock data will be returned if available, bypassing serial communication.
    ///
    /// # Returns
    /// - `Ok(f64)` - The read or mocked float value.
    /// - `Err` - If the capability is not found, mock data is unavailable, or a communication/parsing error occurs.
    pub async fn read_named_f64(&self, capability_name: &str, is_mocking: bool) -> Result<f64> {
        let readable_cap = self
            .config
            .capabilities
            .readable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Readable capability '{}' not found in config",
                    capability_name
                )
            })?;

        if is_mocking {
            return GenericDriver::get_mock_value(&readable_cap.mock)?
                .as_f64()
                .ok_or_else(|| anyhow!("Mock value for '{}' is not a float", capability_name));
        }

        let response = self.execute_command(&readable_cap.command).await?;
        let parsed_value =
            self.parse_response(&response, &readable_cap.pattern, ValueType::Float)?;

        parsed_value
            .as_f64()
            .ok_or_else(|| anyhow!("Parsed value for '{}' is not a float", capability_name))
    }

    /// Sets a specific named settable capability, optionally for mocking.
    pub async fn set_named_value(
        &self,
        capability_name: &str,
        value: Value,
        is_mocking: bool,
    ) -> Result<()> {
        let settable_cap = self
            .config
            .capabilities
            .settable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Settable capability '{}' not found in config",
                    capability_name
                )
            })?;

        if is_mocking {
            self.state
                .write()
                .await
                .insert(capability_name.to_string(), value);
            return Ok(());
        }

        // Prepare context for strfmt
        let mut fmt_context = HashMap::new();
        fmt_context.insert("val".to_string(), value.to_string());

        let command = strfmt(&settable_cap.set_cmd, &fmt_context)
            .map_err(|e| anyhow!("Failed to format command for '{}': {}", capability_name, e))?;

        self.execute_command(&command).await?;
        self.state
            .write()
            .await
            .insert(capability_name.to_string(), value);
        Ok(())
    }

    /// Gets a specific named settable capability, optionally for mocking.
    pub async fn get_named_value(&self, capability_name: &str, is_mocking: bool) -> Result<Value> {
        let settable_cap = self
            .config
            .capabilities
            .settable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Settable capability '{}' not found in config",
                    capability_name
                )
            })?;

        if is_mocking {
            if let Some(mock_val) = GenericDriver::get_mock_value(&settable_cap.mock).ok() {
                return Ok(mock_val);
            } else {
                let state_read = self.state.read().await;
                if let Some(val) = state_read.get(capability_name) {
                    return Ok(val.clone());
                }
                return Err(anyhow!(
                    "No mock data or current state for settable '{}'",
                    capability_name
                ));
            }
        }

        if let Some(get_cmd) = &settable_cap.get_cmd {
            let response = self.execute_command(get_cmd).await?;
            let parsed_value = self.parse_response(
                &response,
                &settable_cap.pattern,
                settable_cap.value_type.clone(),
            )?;
            self.state
                .write()
                .await
                .insert(capability_name.to_string(), parsed_value.clone());
            Ok(parsed_value)
        } else {
            let state_read = self.state.read().await;
            state_read.get(capability_name).cloned().ok_or_else(|| {
                anyhow!(
                    "Settable '{}' has no get_cmd and no current state.",
                    capability_name
                )
            })
        }
    }

    /// Turns on a specific named switchable capability, optionally for mocking.
    pub async fn turn_on_named(&self, capability_name: &str, is_mocking: bool) -> Result<()> {
        let switchable_cap = self
            .config
            .capabilities
            .switchable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Switchable capability '{}' not found in config",
                    capability_name
                )
            })?;

        if is_mocking {
            self.state
                .write()
                .await
                .insert(capability_name.to_string(), Value::Bool(true));
            return Ok(());
        }

        self.execute_command(&switchable_cap.on_cmd).await?;
        self.state
            .write()
            .await
            .insert(capability_name.to_string(), Value::Bool(true));
        Ok(())
    }

    /// Turns off a specific named switchable capability, optionally for mocking.
    pub async fn turn_off_named(&self, capability_name: &str, is_mocking: bool) -> Result<()> {
        let switchable_cap = self
            .config
            .capabilities
            .switchable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Switchable capability '{}' not found in config",
                    capability_name
                )
            })?;

        if is_mocking {
            self.state
                .write()
                .await
                .insert(capability_name.to_string(), Value::Bool(false));
            return Ok(());
        }

        self.execute_command(&switchable_cap.off_cmd).await?;
        self.state
            .write()
            .await
            .insert(capability_name.to_string(), Value::Bool(false));
        Ok(())
    }

    /// Queries the on/off state of a specific named switchable capability, optionally for mocking.
    pub async fn is_named_on(&self, capability_name: &str, is_mocking: bool) -> Result<bool> {
        let switchable_cap = self
            .config
            .capabilities
            .switchable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Switchable capability '{}' not found in config",
                    capability_name
                )
            })?;

        if is_mocking {
            let state_read = self.state.read().await;
            return state_read
                .get(capability_name)
                .and_then(|v| v.as_bool())
                .ok_or_else(|| anyhow!("No mock state for switchable '{}'", capability_name));
        }

        if let Some(status_cmd) = &switchable_cap.status_cmd {
            let response = self.execute_command(status_cmd).await?;
            let parsed_value = self.parse_response(
                &response,
                switchable_cap
                    .pattern
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or("{}"),
                ValueType::Bool,
            )?;
            self.state
                .write()
                .await
                .insert(capability_name.to_string(), parsed_value.clone());
            parsed_value
                .as_bool()
                .ok_or_else(|| anyhow!("Parsed status for '{}' is not a boolean", capability_name))
        } else {
            Err(anyhow!(
                "Switchable '{}' has no status_cmd to query state",
                capability_name
            ))
        }
    }

    /// Executes a specific named actionable capability, optionally for mocking.
    pub async fn execute_named_action(
        &self,
        capability_name: &str,
        is_mocking: bool,
    ) -> Result<()> {
        let actionable_cap = self
            .config
            .capabilities
            .actionable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Actionable capability '{}' not found in config",
                    capability_name
                )
            })?;

        if is_mocking {
            return Ok(());
        }

        self.execute_command(&actionable_cap.cmd).await?;
        if actionable_cap.wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(actionable_cap.wait_ms)).await;
        }
        Ok(())
    }

    /// Gets a specific named loggable capability, optionally for mocking.
    /// Loggable values are typically static and read once, then cached.
    pub async fn get_named_loggable(
        &self,
        capability_name: &str,
        is_mocking: bool,
    ) -> Result<String> {
        let loggable_cap = self
            .config
            .capabilities
            .loggable
            .iter()
            .find(|cap| cap.name == capability_name)
            .ok_or_else(|| {
                anyhow!(
                    "Loggable capability '{}' not found in config",
                    capability_name
                )
            })?;

        // Try to get from state first (assume cached after first read)
        {
            let state_read = self.state.read().await;
            if let Some(val) = state_read.get(capability_name) {
                if let Some(s) = val.as_str() {
                    return Ok(s.to_string());
                }
            }
        }

        if is_mocking {
            return GenericDriver::get_mock_value(&loggable_cap.mock)?
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Mock value for '{}' is not a string", capability_name));
        }

        let response = self.execute_command(&loggable_cap.cmd).await?;
        let parsed_value =
            self.parse_response(&response, &loggable_cap.pattern, ValueType::String)?;

        self.state
            .write()
            .await
            .insert(capability_name.to_string(), parsed_value.clone());
        parsed_value
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Parsed value for '{}' is not a string", capability_name))
    }

    // =========================================================================
    // Movable Capability Methods (for axis control)
    // =========================================================================

    /// Moves a specific named axis to an absolute position.
    ///
    /// # Arguments
    /// * `axis_name` - The name of the axis (as defined in YAML movable.axes)
    /// * `position` - The target position in device-native units
    /// * `is_mocking` - If true, only updates internal state without serial communication
    pub async fn move_axis_abs(
        &self,
        axis_name: &str,
        position: f64,
        is_mocking: bool,
    ) -> Result<()> {
        let movable = self
            .config
            .capabilities
            .movable
            .as_ref()
            .ok_or_else(|| anyhow!("No movable capability configured"))?;

        // Validate axis exists
        let _axis = movable
            .axes
            .iter()
            .find(|a| a.name == axis_name)
            .ok_or_else(|| anyhow!("Axis '{}' not found in movable config", axis_name))?;

        if is_mocking {
            self.state.write().await.insert(
                format!("axis_{}_position", axis_name),
                Value::from(position),
            );
            return Ok(());
        }

        // Format and send command
        let mut fmt_context = HashMap::new();
        fmt_context.insert("axis".to_string(), axis_name.to_string());
        fmt_context.insert("val".to_string(), position.to_string());

        let command = strfmt(&movable.set_cmd, &fmt_context)
            .map_err(|e| anyhow!("Failed to format move command: {}", e))?;

        self.execute_command(&command).await?;
        self.state.write().await.insert(
            format!("axis_{}_position", axis_name),
            Value::from(position),
        );
        Ok(())
    }

    /// Moves a specific named axis by a relative distance.
    ///
    /// # Arguments
    /// * `axis_name` - The name of the axis
    /// * `distance` - The distance to move (positive or negative)
    /// * `is_mocking` - If true, only updates internal state
    pub async fn move_axis_rel(
        &self,
        axis_name: &str,
        distance: f64,
        is_mocking: bool,
    ) -> Result<()> {
        let current = self.get_axis_position(axis_name, is_mocking).await?;
        self.move_axis_abs(axis_name, current + distance, is_mocking)
            .await
    }

    /// Gets the current position of a specific named axis.
    ///
    /// # Arguments
    /// * `axis_name` - The name of the axis
    /// * `is_mocking` - If true, returns cached position without serial communication
    pub async fn get_axis_position(&self, axis_name: &str, is_mocking: bool) -> Result<f64> {
        let movable = self
            .config
            .capabilities
            .movable
            .as_ref()
            .ok_or_else(|| anyhow!("No movable capability configured"))?;

        // Validate axis exists
        let _axis = movable
            .axes
            .iter()
            .find(|a| a.name == axis_name)
            .ok_or_else(|| anyhow!("Axis '{}' not found in movable config", axis_name))?;

        if is_mocking {
            let state_read = self.state.read().await;
            return state_read
                .get(&format!("axis_{}_position", axis_name))
                .and_then(|v| v.as_f64())
                .ok_or_else(|| anyhow!("No mock position state for axis '{}'", axis_name));
        }

        // Format and send get command
        let mut fmt_context = HashMap::new();
        fmt_context.insert("axis".to_string(), axis_name.to_string());

        let command = strfmt(&movable.get_cmd, &fmt_context)
            .map_err(|e| anyhow!("Failed to format get position command: {}", e))?;

        let response = self.execute_command(&command).await?;
        let parsed_value =
            self.parse_response(&response, &movable.get_pattern, ValueType::Float)?;

        let position = parsed_value
            .as_f64()
            .ok_or_else(|| anyhow!("Parsed position for axis '{}' is not a float", axis_name))?;

        self.state.write().await.insert(
            format!("axis_{}_position", axis_name),
            Value::from(position),
        );
        Ok(position)
    }

    /// Waits for axis motion to settle (polls until position is stable).
    ///
    /// # Arguments
    /// * `axis_name` - The name of the axis
    /// * `is_mocking` - If true, returns immediately
    /// * `timeout` - Maximum time to wait for settling
    pub async fn wait_axis_settled(
        &self,
        axis_name: &str,
        is_mocking: bool,
        timeout: Duration,
    ) -> Result<()> {
        if is_mocking {
            // In mock mode, motion is instant
            tokio::time::sleep(Duration::from_millis(10)).await;
            return Ok(());
        }

        let deadline = tokio::time::Instant::now() + timeout;
        let settle_threshold = 0.001; // Position must be stable within this tolerance
        let poll_interval = Duration::from_millis(50);

        let mut last_position = self.get_axis_position(axis_name, false).await?;

        loop {
            tokio::time::sleep(poll_interval).await;

            if tokio::time::Instant::now() > deadline {
                return Err(anyhow!(
                    "Timeout waiting for axis '{}' to settle",
                    axis_name
                ));
            }

            let current_position = self.get_axis_position(axis_name, false).await?;
            if (current_position - last_position).abs() < settle_threshold {
                return Ok(());
            }
            last_position = current_position;
        }
    }

    // =========================================================================
    // Exposure Control Capability Methods (for camera exposure)
    // =========================================================================

    /// Sets the exposure/integration time in seconds.
    ///
    /// # Arguments
    /// * `seconds` - Exposure time in seconds
    /// * `is_mocking` - If true, only updates internal state without serial communication
    pub async fn set_exposure(&self, seconds: f64, is_mocking: bool) -> Result<()> {
        let exposure_cap = self
            .config
            .capabilities
            .exposure_control
            .as_ref()
            .ok_or_else(|| anyhow!("No exposure control capability configured"))?;

        // Validate range if specified
        if let Some(min) = exposure_cap.min_seconds {
            if seconds < min {
                return Err(anyhow!("Exposure {} s is below minimum {} s", seconds, min));
            }
        }
        if let Some(max) = exposure_cap.max_seconds {
            if seconds > max {
                return Err(anyhow!("Exposure {} s exceeds maximum {} s", seconds, max));
            }
        }

        if is_mocking {
            self.state
                .write()
                .await
                .insert("exposure_seconds".to_string(), Value::from(seconds));
            return Ok(());
        }

        // Format and send command
        let mut fmt_context = HashMap::new();
        fmt_context.insert("val".to_string(), seconds.to_string());

        let command = strfmt(&exposure_cap.set_cmd, &fmt_context)
            .map_err(|e| anyhow!("Failed to format exposure set command: {}", e))?;

        self.execute_command(&command).await?;
        self.state
            .write()
            .await
            .insert("exposure_seconds".to_string(), Value::from(seconds));
        Ok(())
    }

    /// Gets the current exposure/integration time in seconds.
    ///
    /// # Arguments
    /// * `is_mocking` - If true, returns cached exposure without serial communication
    pub async fn get_exposure(&self, is_mocking: bool) -> Result<f64> {
        let exposure_cap = self
            .config
            .capabilities
            .exposure_control
            .as_ref()
            .ok_or_else(|| anyhow!("No exposure control capability configured"))?;

        if is_mocking {
            if let Some(mock_val) = GenericDriver::get_mock_value(&exposure_cap.mock).ok() {
                return mock_val
                    .as_f64()
                    .ok_or_else(|| anyhow!("Mock exposure value is not a float"));
            } else {
                let state_read = self.state.read().await;
                return state_read
                    .get("exposure_seconds")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow!("No mock exposure state"));
            }
        }

        let response = self.execute_command(&exposure_cap.get_cmd).await?;
        let parsed_value =
            self.parse_response(&response, &exposure_cap.get_pattern, ValueType::Float)?;

        let exposure = parsed_value
            .as_f64()
            .ok_or_else(|| anyhow!("Parsed exposure is not a float"))?;

        self.state
            .write()
            .await
            .insert("exposure_seconds".to_string(), Value::from(exposure));
        Ok(exposure)
    }

    // =========================================================================
    // Triggerable Capability Methods (for external triggering)
    // =========================================================================

    /// Arms the device for external or software triggering.
    ///
    /// # Arguments
    /// * `is_mocking` - If true, only updates internal state without serial communication
    pub async fn arm_trigger(&self, is_mocking: bool) -> Result<()> {
        let triggerable = self
            .config
            .capabilities
            .triggerable
            .as_ref()
            .ok_or_else(|| anyhow!("No triggerable capability configured"))?;

        if is_mocking {
            self.state
                .write()
                .await
                .insert("trigger_armed".to_string(), Value::from(true));
            return Ok(());
        }

        self.execute_command(&triggerable.arm_cmd).await?;
        self.state
            .write()
            .await
            .insert("trigger_armed".to_string(), Value::from(true));
        Ok(())
    }

    /// Sends a software trigger to the device.
    ///
    /// # Arguments
    /// * `is_mocking` - If true, only updates internal state without serial communication
    pub async fn send_trigger(&self, is_mocking: bool) -> Result<()> {
        let triggerable = self
            .config
            .capabilities
            .triggerable
            .as_ref()
            .ok_or_else(|| anyhow!("No triggerable capability configured"))?;

        if is_mocking {
            let armed = self
                .state
                .read()
                .await
                .get("trigger_armed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if !armed {
                return Err(anyhow!("Device not armed for trigger (mock mode)"));
            }

            self.state
                .write()
                .await
                .insert("trigger_armed".to_string(), Value::from(false));
            return Ok(());
        }

        self.execute_command(&triggerable.trigger_cmd).await?;
        self.state
            .write()
            .await
            .insert("trigger_armed".to_string(), Value::from(false));
        Ok(())
    }

    /// Checks if the device is currently armed for triggering.
    ///
    /// # Arguments
    /// * `is_mocking` - If true, returns cached state without serial communication
    pub async fn is_trigger_armed(&self, is_mocking: bool) -> Result<bool> {
        let triggerable = self
            .config
            .capabilities
            .triggerable
            .as_ref()
            .ok_or_else(|| anyhow!("No triggerable capability configured"))?;

        if is_mocking {
            let armed = self
                .state
                .read()
                .await
                .get("trigger_armed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            return Ok(armed);
        }

        // If no status command is configured, return error
        let status_cmd = triggerable
            .status_cmd
            .as_ref()
            .ok_or_else(|| anyhow!("No status command configured for triggerable capability"))?;

        let response = self.execute_command(status_cmd).await?;

        // If pattern and armed_value are provided, parse response
        if let (Some(pattern), Some(armed_value)) =
            (&triggerable.status_pattern, &triggerable.armed_value)
        {
            let parsed = self.parse_response(&response, pattern, ValueType::String)?;
            let status_str = parsed
                .as_str()
                .ok_or_else(|| anyhow!("Parsed status is not a string"))?;
            Ok(status_str == armed_value)
        } else {
            // If no pattern specified, check if response contains "armed" or "1"
            let response_lower = response.to_lowercase();
            Ok(response_lower.contains("armed") || response_lower.contains("1"))
        }
    }

    // =========================================================================
    // Scriptable Capability Methods (for Rhai scripting)
    // =========================================================================

    /// Gets a named scriptable capability by name.
    ///
    /// Returns the script configuration, including the source code, for external
    /// execution via a Rhai engine.
    ///
    /// # Arguments
    /// * `script_name` - The name of the scriptable capability
    ///
    /// # Returns
    /// A reference to the ScriptableCapability if found.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let script_cap = driver.get_scriptable("safe_shutdown")?;
    /// println!("Script timeout: {}ms", script_cap.timeout_ms);
    /// // Execute script_cap.script with RhaiEngine
    /// ```
    pub fn get_scriptable(
        &self,
        script_name: &str,
    ) -> Result<&crate::hardware::plugin::schema::ScriptableCapability> {
        self.config
            .capabilities
            .scriptable
            .iter()
            .find(|s| s.name == script_name)
            .ok_or_else(|| {
                anyhow!(
                    "Scriptable capability '{}' not found in config",
                    script_name
                )
            })
    }

    /// Lists all available scriptable capability names.
    pub fn list_scriptables(&self) -> Vec<&str> {
        self.config
            .capabilities
            .scriptable
            .iter()
            .map(|s| s.name.as_str())
            .collect()
    }

    /// Validates that a script can be parsed by Rhai.
    ///
    /// This performs a syntax check on the script without executing it.
    ///
    /// # Arguments
    /// * `script_name` - The name of the scriptable capability to validate
    ///
    /// # Returns
    /// Ok(()) if the script is syntactically valid, Err otherwise.
    pub fn validate_script(&self, script_name: &str) -> Result<()> {
        let script_cap = self.get_scriptable(script_name)?;

        // Create a temporary Rhai engine for validation
        let engine = rhai::Engine::new();
        engine
            .compile(&script_cap.script)
            .map_err(|e| anyhow!("Script '{}' has syntax error: {}", script_name, e))?;

        Ok(())
    }

    /// Creates a Rhai scope pre-populated with driver context.
    ///
    /// This creates a scope with read-only access to driver configuration
    /// that can be used when executing scripts.
    ///
    /// # Returns
    /// A Rhai Scope containing:
    /// - `driver_id` - The driver's metadata ID
    /// - `driver_name` - The driver's display name
    /// - `is_mocking` - Whether in mock mode
    pub fn create_script_scope(&self, is_mocking: bool) -> rhai::Scope<'static> {
        let mut scope = rhai::Scope::new();
        scope.push_constant("driver_id", self.config.metadata.id.clone());
        scope.push_constant("driver_name", self.config.metadata.name.clone());
        scope.push_constant("is_mocking", is_mocking);
        scope
    }

    // =========================================================================
    // FrameProducer Capability Methods
    // =========================================================================

    /// Starts continuous frame streaming.
    ///
    /// # Arguments
    /// * `is_mocking` - If true, generates synthetic frames instead of hardware acquisition
    pub async fn start_frame_stream(&self, is_mocking: bool) -> Result<()> {
        let frame_producer = self
            .config
            .capabilities
            .frame_producer
            .as_ref()
            .ok_or_else(|| anyhow!("No frame_producer capability configured"))?;

        // Check if already streaming
        if self.is_streaming.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(anyhow!("Frame streaming is already active"));
        }

        if !is_mocking {
            self.execute_command(&frame_producer.start_cmd).await?;
        }

        self.is_streaming
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.frame_counter
            .store(0, std::sync::atomic::Ordering::SeqCst);

        // Spawn background task to generate/acquire frames
        let driver_clone = std::sync::Arc::new(self.clone_for_streaming());
        let is_mock = is_mocking;
        tokio::spawn(async move {
            if let Err(e) = driver_clone.frame_streaming_loop(is_mock, None).await {
                tracing::error!("Frame streaming loop error: {}", e);
            }
        });

        Ok(())
    }

    /// Starts finite frame acquisition with a maximum frame count.
    ///
    /// # Arguments
    /// * `frame_limit` - Maximum number of frames to acquire (None = continuous)
    /// * `is_mocking` - If true, generates synthetic frames instead of hardware acquisition
    pub async fn start_frame_stream_finite(
        &self,
        frame_limit: Option<u32>,
        is_mocking: bool,
    ) -> Result<()> {
        match frame_limit {
            Some(0) | None => self.start_frame_stream(is_mocking).await,
            Some(n) => {
                let frame_producer = self
                    .config
                    .capabilities
                    .frame_producer
                    .as_ref()
                    .ok_or_else(|| anyhow!("No frame_producer capability configured"))?;

                if self.is_streaming.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(anyhow!("Frame streaming is already active"));
                }

                if !is_mocking {
                    self.execute_command(&frame_producer.start_cmd).await?;
                }

                self.is_streaming
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                self.frame_counter
                    .store(0, std::sync::atomic::Ordering::SeqCst);

                let driver_clone = std::sync::Arc::new(self.clone_for_streaming());
                let is_mock = is_mocking;
                tokio::spawn(async move {
                    if let Err(e) = driver_clone.frame_streaming_loop(is_mock, Some(n)).await {
                        tracing::error!("Frame streaming loop error: {}", e);
                    }
                });

                Ok(())
            }
        }
    }

    /// Stops frame streaming.
    ///
    /// # Arguments
    /// * `is_mocking` - If true, only updates internal state without hardware communication
    pub async fn stop_frame_stream(&self, is_mocking: bool) -> Result<()> {
        let frame_producer = self
            .config
            .capabilities
            .frame_producer
            .as_ref()
            .ok_or_else(|| anyhow!("No frame_producer capability configured"))?;

        if !self.is_streaming.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(()); // Already stopped
        }

        self.is_streaming
            .store(false, std::sync::atomic::Ordering::SeqCst);

        if !is_mocking {
            self.execute_command(&frame_producer.stop_cmd).await?;
        }

        Ok(())
    }

    /// Returns the configured frame resolution.
    pub fn frame_resolution(&self) -> (u32, u32) {
        self.config
            .capabilities
            .frame_producer
            .as_ref()
            .map(|fp| (fp.width, fp.height))
            .unwrap_or((0, 0))
    }

    /// Subscribes to the frame stream.
    pub async fn subscribe_frames(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<std::sync::Arc<crate::hardware::Frame>>> {
        if self.config.capabilities.frame_producer.is_some() {
            Some(self.frame_broadcaster.subscribe())
        } else {
            None
        }
    }

    /// Checks if currently streaming frames.
    pub async fn is_frame_streaming(&self, _is_mocking: bool) -> Result<bool> {
        Ok(self.is_streaming.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// Returns the number of frames captured since streaming started.
    pub fn frame_count(&self) -> u64 {
        self.frame_counter.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Internal method to create a lightweight clone for the streaming task.
    ///
    /// Note: This is a workaround since we can't easily pass `&self` to a spawned task.
    /// In production, consider using Arc<GenericDriver> throughout.
    fn clone_for_streaming(&self) -> Self {
        Self {
            config: self.config.clone(),
            connection: Mutex::new(Connection::Mock), // Streaming task uses separate connection
            state: RwLock::new(HashMap::new()),
            error_patterns: self.error_patterns.clone(),
            termination_bytes: self.termination_bytes.clone(),
            frame_broadcaster: self.frame_broadcaster.clone(),
            frame_counter: std::sync::atomic::AtomicU64::new(0),
            is_streaming: std::sync::atomic::AtomicBool::new(false),
            parameters: ParameterSet::new(), // Not used in streaming task
        }
    }

    /// Background frame streaming loop.
    async fn frame_streaming_loop(&self, is_mocking: bool, frame_limit: Option<u32>) -> Result<()> {
        let frame_producer = self
            .config
            .capabilities
            .frame_producer
            .as_ref()
            .ok_or_else(|| anyhow!("No frame_producer capability configured"))?;

        let (width, height) = (frame_producer.width, frame_producer.height);
        let mut frame_count = 0u32;

        while self.is_streaming.load(std::sync::atomic::Ordering::SeqCst) {
            // Check frame limit
            if let Some(limit) = frame_limit {
                if frame_count >= limit {
                    self.is_streaming
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
            }

            // Generate or acquire frame
            let frame = if is_mocking {
                self.generate_mock_frame(width, height, &frame_producer.mock)
                    .await?
            } else {
                // Real frame acquisition from hardware
                self.acquire_frame(frame_producer).await?
            };

            // Broadcast frame
            let frame_arc = std::sync::Arc::new(frame);
            let _ = self.frame_broadcaster.send(frame_arc); // Ignore if no subscribers

            frame_count += 1;
            self.frame_counter
                .store(frame_count as u64, std::sync::atomic::Ordering::SeqCst);

            // Small delay to prevent CPU spin
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    /// Generates a mock frame for testing.
    async fn generate_mock_frame(
        &self,
        width: u32,
        height: u32,
        mock_config: &Option<crate::hardware::plugin::schema::MockFrameConfig>,
    ) -> Result<crate::hardware::Frame> {
        let pattern = mock_config
            .as_ref()
            .map(|m| m.pattern.as_str())
            .unwrap_or("checkerboard");

        let intensity = mock_config.as_ref().map(|m| m.intensity).unwrap_or(1000);

        let size = (width * height) as usize;
        let mut buffer = vec![0u16; size];

        match pattern {
            "checkerboard" => {
                for y in 0..height {
                    for x in 0..width {
                        let idx = (y * width + x) as usize;
                        buffer[idx] = if (x + y) % 2 == 0 {
                            intensity
                        } else {
                            intensity / 4
                        };
                    }
                }
            }
            "gradient" => {
                for y in 0..height {
                    for x in 0..width {
                        let idx = (y * width + x) as usize;
                        buffer[idx] = ((x as f32 / width as f32) * intensity as f32) as u16;
                    }
                }
            }
            "noise" => {
                let mut rng = rand::thread_rng();
                for pixel in buffer.iter_mut() {
                    *pixel = rng.gen_range(0..intensity);
                }
            }
            "flat" | _ => {
                buffer.fill(intensity);
            }
        }

        Ok(crate::hardware::Frame::new(width, height, buffer))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::plugin::schema::*;

    /// Creates a minimal valid InstrumentConfig for testing
    fn create_test_config() -> InstrumentConfig {
        InstrumentConfig {
            metadata: InstrumentMetadata {
                id: "test-driver".to_string(),
                name: "Test Driver".to_string(),
                version: "1.0.0".to_string(),
                driver_type: DriverType::SerialScpi,
            },
            protocol: ProtocolConfig::default(),
            on_connect: vec![],
            on_disconnect: vec![],
            error_patterns: vec![],
            capabilities: CapabilitiesConfig::default(),
            ui_layout: vec![],
        }
    }

    /// Creates a config with a readable capability for power measurement
    fn create_readable_config() -> InstrumentConfig {
        let mut config = create_test_config();
        config.capabilities.readable = vec![ReadableCapability {
            name: "power".to_string(),
            command: "READ:POW?".to_string(),
            pattern: "{val:f} W".to_string(),
            unit: Some("W".to_string()),
            mock: Some(MockData {
                default: 0.001,
                jitter: 0.0001,
            }),
        }];
        config
    }

    /// Creates a config with a settable capability
    fn create_settable_config() -> InstrumentConfig {
        let mut config = create_test_config();
        config.capabilities.settable = vec![SettableCapability {
            name: "wavelength".to_string(),
            set_cmd: "WAVE {val}".to_string(),
            get_cmd: Some("WAVE?".to_string()),
            pattern: "{val:f}".to_string(),
            unit: Some("nm".to_string()),
            min: Some(700.0),
            max: Some(1000.0),
            value_type: ValueType::Float,
            options: vec![],
            mock: Some(MockData {
                default: 800.0,
                jitter: 0.0,
            }),
        }];
        config
    }

    /// Creates a config with movable capability
    fn create_movable_config() -> InstrumentConfig {
        let mut config = create_test_config();
        config.capabilities.movable = Some(MovableCapability {
            axes: vec![AxisConfig {
                name: "x".to_string(),
                unit: Some("mm".to_string()),
                min: Some(-100.0),
                max: Some(100.0),
            }],
            set_cmd: "MOVE {axis} {val}".to_string(),
            get_cmd: "POS? {axis}".to_string(),
            get_pattern: "{val:f}".to_string(),
        });
        config
    }

    /// Creates a config with switchable capability
    fn create_switchable_config() -> InstrumentConfig {
        let mut config = create_test_config();
        config.capabilities.switchable = vec![SwitchableCapability {
            name: "shutter".to_string(),
            on_cmd: "SHUTTER ON".to_string(),
            off_cmd: "SHUTTER OFF".to_string(),
            status_cmd: Some("SHUTTER?".to_string()),
            pattern: Some("{val}".to_string()),
            mock: None,
        }];
        config
    }

    // =========================================================================
    // Response Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_response_float() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        // Test float parsing with unit suffix
        let result = driver.parse_response("0.00123 W", "{val:f} W", ValueType::Float);
        assert!(result.is_ok());
        let value = result.unwrap().as_f64().unwrap();
        assert!((value - 0.00123).abs() < 1e-10);
    }

    #[test]
    fn test_parse_response_integer() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.parse_response("COUNT:42", "COUNT:{val:i}", ValueType::Int);
        assert!(result.is_ok());
        let value = result.unwrap().as_i64().unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    fn test_parse_response_string() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.parse_response("STATUS:READY", "STATUS:{val}", ValueType::String);
        assert!(result.is_ok());
        let value = result.unwrap().as_str().unwrap().to_string();
        assert_eq!(value, "READY");
    }

    #[test]
    fn test_parse_response_bool_on() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.parse_response("SHUTTER:ON", "SHUTTER:{val}", ValueType::Bool);
        assert!(result.is_ok());
        assert!(result.unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_parse_response_bool_off() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.parse_response("SHUTTER:OFF", "SHUTTER:{val}", ValueType::Bool);
        assert!(result.is_ok());
        assert!(!result.unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_parse_response_negative_float() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.parse_response("-12.5", "{val:f}", ValueType::Float);
        assert!(result.is_ok());
        let value = result.unwrap().as_f64().unwrap();
        assert!((value - (-12.5)).abs() < 1e-10);
    }

    #[test]
    fn test_parse_response_pattern_mismatch() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.parse_response("INVALID RESPONSE", "{val:f} W", ValueType::Float);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("did not match"));
    }

    // =========================================================================
    // Mock Driver Readable Tests
    // =========================================================================

    #[tokio::test]
    async fn test_mock_readable_returns_value() {
        let config = create_readable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.read_named_f64("power", true).await;
        assert!(result.is_ok());
        let value = result.unwrap();
        // Mock value is 0.001 with 0.0001 jitter, so should be close
        assert!(value > 0.0008 && value < 0.0012);
    }

    #[tokio::test]
    async fn test_mock_readable_nonexistent_capability() {
        let config = create_readable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.read_named_f64("nonexistent", true).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // =========================================================================
    // Mock Driver Settable Tests
    // =========================================================================

    #[tokio::test]
    async fn test_mock_settable_set_and_get() {
        let config = create_settable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        // Set a value
        let set_result = driver
            .set_named_value("wavelength", Value::from(850.0), true)
            .await;
        assert!(set_result.is_ok());

        // Get it back - should return mock default since mock is configured
        let get_result = driver.get_named_value("wavelength", true).await;
        assert!(get_result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_settable_nonexistent() {
        let config = create_settable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver
            .set_named_value("nonexistent", Value::from(100.0), true)
            .await;
        assert!(result.is_err());
    }

    // =========================================================================
    // Mock Driver Movable Tests
    // =========================================================================

    #[tokio::test]
    async fn test_mock_movable_move_and_get() {
        let config = create_movable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        // Initial move
        let move_result = driver.move_axis_abs("x", 25.0, true).await;
        assert!(move_result.is_ok());

        // Get position
        let pos_result = driver.get_axis_position("x", true).await;
        assert!(pos_result.is_ok());
        assert!((pos_result.unwrap() - 25.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_mock_movable_relative_move() {
        let config = create_movable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        // Set initial position
        driver.move_axis_abs("x", 10.0, true).await.unwrap();

        // Move relative
        driver.move_axis_rel("x", 5.0, true).await.unwrap();

        // Should be at 15.0
        let pos = driver.get_axis_position("x", true).await.unwrap();
        assert!((pos - 15.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_mock_movable_nonexistent_axis() {
        let config = create_movable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let result = driver.move_axis_abs("y", 10.0, true).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // =========================================================================
    // Mock Driver Switchable Tests
    // =========================================================================

    #[tokio::test]
    async fn test_mock_switchable_on_off() {
        let config = create_switchable_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        // Turn on
        driver.turn_on_named("shutter", true).await.unwrap();
        let is_on = driver.is_named_on("shutter", true).await.unwrap();
        assert!(is_on);

        // Turn off
        driver.turn_off_named("shutter", true).await.unwrap();
        let is_on = driver.is_named_on("shutter", true).await.unwrap();
        assert!(!is_on);
    }

    // =========================================================================
    // Error Pattern Tests
    // =========================================================================

    #[test]
    fn test_error_pattern_compilation() {
        let mut config = create_test_config();
        config.error_patterns = vec!["ERROR:.*".to_string(), "FAULT:\\d+".to_string()];

        let driver = GenericDriver::new_mock(config);
        assert!(driver.is_ok());
    }

    #[test]
    fn test_invalid_error_pattern() {
        let mut config = create_test_config();
        config.error_patterns = vec![
            "[invalid regex".to_string(), // Unclosed bracket
        ];

        let driver = GenericDriver::new_mock(config);
        assert!(driver.is_err());
        let err_msg = driver.err().unwrap().to_string();
        assert!(err_msg.contains("regex"));
    }

    // =========================================================================
    // Scriptable Capability Tests
    // =========================================================================

    #[test]
    fn test_scriptable_get_and_list() {
        let mut config = create_test_config();
        config.capabilities.scriptable = vec![
            ScriptableCapability {
                name: "init".to_string(),
                description: Some("Initialize device".to_string()),
                script: "print(\"Hello\");".to_string(),
                timeout_ms: 5000,
            },
            ScriptableCapability {
                name: "shutdown".to_string(),
                description: None,
                script: "print(\"Bye\");".to_string(),
                timeout_ms: 10000,
            },
        ];

        let driver = GenericDriver::new_mock(config).unwrap();

        // List scriptables
        let scripts = driver.list_scriptables();
        assert_eq!(scripts.len(), 2);
        assert!(scripts.contains(&"init"));
        assert!(scripts.contains(&"shutdown"));

        // Get specific scriptable
        let init = driver.get_scriptable("init");
        assert!(init.is_ok());
        assert_eq!(init.unwrap().timeout_ms, 5000);

        // Nonexistent
        let missing = driver.get_scriptable("nonexistent");
        assert!(missing.is_err());
    }

    #[test]
    fn test_scriptable_validation() {
        let mut config = create_test_config();
        config.capabilities.scriptable = vec![
            ScriptableCapability {
                name: "valid".to_string(),
                description: None,
                script: "let x = 1 + 2; x".to_string(),
                timeout_ms: 5000,
            },
            ScriptableCapability {
                name: "invalid".to_string(),
                description: None,
                script: "let x = (1 +".to_string(), // Syntax error
                timeout_ms: 5000,
            },
        ];

        let driver = GenericDriver::new_mock(config).unwrap();

        // Valid script should pass validation
        assert!(driver.validate_script("valid").is_ok());

        // Invalid script should fail validation
        let result = driver.validate_script("invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("syntax error"));
    }

    #[test]
    fn test_script_scope_creation() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();

        let scope = driver.create_script_scope(true);

        // Scope should contain expected constants
        assert!(scope.contains("driver_id"));
        assert!(scope.contains("driver_name"));
        assert!(scope.contains("is_mocking"));
    }

    // =========================================================================
    // Frame Producer Tests
    // =========================================================================

    #[test]
    fn test_frame_resolution() {
        let mut config = create_test_config();
        config.capabilities.frame_producer = Some(FrameProducerCapability {
            width: 1024,
            height: 768,
            start_cmd: "ACQ:START".to_string(),
            stop_cmd: "ACQ:STOP".to_string(),
            frame_cmd: "ACQ:FRAME?".to_string(),
            status_cmd: None,
            status_pattern: None,
            mock: None,
        });

        let driver = GenericDriver::new_mock(config).unwrap();
        let (width, height) = driver.frame_resolution();

        assert_eq!(width, 1024);
        assert_eq!(height, 768);
    }

    #[test]
    fn test_frame_resolution_no_capability() {
        let config = create_test_config();
        let driver = GenericDriver::new_mock(config).unwrap();
        let (width, height) = driver.frame_resolution();

        assert_eq!(width, 0);
        assert_eq!(height, 0);
    }
}
