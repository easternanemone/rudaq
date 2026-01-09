//! Generic Serial Driver - Config-driven hardware driver implementation.
//!
//! This module provides a [`GenericSerialDriver`] that interprets TOML device
//! configurations at runtime, enabling new devices to be added without code changes.
//!
//! # Architecture
//!
//! The driver implements capability traits (like [`Movable`]) by:
//! 1. Looking up trait method mappings from config
//! 2. Formatting commands using template interpolation
//! 3. Parsing responses with regex patterns
//! 4. Applying unit conversions with evalexpr
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_hardware::config::load_device_config;
//! use daq_hardware::drivers::generic_serial::GenericSerialDriver;
//! use daq_hardware::capabilities::Movable;
//! use std::path::Path;
//!
//! // Load ELL14 configuration
//! let config = load_device_config(Path::new("config/devices/ell14.toml"))?;
//!
//! // Create driver with shared port
//! let driver = GenericSerialDriver::new(config, shared_port, "2").await?;
//!
//! // Use via Movable trait
//! driver.move_abs(45.0).await?;
//! let pos = driver.position().await?;
//! ```

use crate::capabilities::{Movable, Readable, ShutterControl, WavelengthTunable};
use crate::config::schema::{DeviceConfig, ErrorSeverity, FieldType, RetryConfig};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use evalexpr::{eval_number_with_context, ContextWithMutableVariables, HashMapContext, Value};
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::{debug, instrument, trace, warn};

// Rhai scripting support (optional)
#[cfg(feature = "scripting")]
use crate::drivers::script_engine::{
    create_sandboxed_engine, execute_script_async, CompiledScripts, ScriptContext,
    ScriptEngineConfig, ScriptResult,
};
#[cfg(feature = "scripting")]
use rhai::Engine;

// Re-use the serial port types from ell14 driver
pub trait SerialPortIO: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SerialPortIO for T {}
pub type DynSerial = Box<dyn SerialPortIO>;
pub type SharedPort = Arc<Mutex<DynSerial>>;

/// Parsed response values from regex matching
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    /// Named fields extracted from the response
    pub fields: HashMap<String, ResponseValue>,
    /// Raw response string
    pub raw: String,
}

/// Value types that can be parsed from responses
#[derive(Debug, Clone)]
pub enum ResponseValue {
    String(String),
    Int(i64),
    Uint(u64),
    Float(f64),
    Bool(bool),
}

impl ResponseValue {
    /// Convert to f64 for numeric operations
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ResponseValue::Float(f) => Some(*f),
            ResponseValue::Int(i) => Some(*i as f64),
            ResponseValue::Uint(u) => Some(*u as f64),
            _ => None,
        }
    }

    /// Convert to i64 for integer operations
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ResponseValue::Int(i) => Some(*i),
            ResponseValue::Uint(u) => Some(*u as i64),
            ResponseValue::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// Convert to string
    pub fn as_string(&self) -> String {
        match self {
            ResponseValue::String(s) => s.clone(),
            ResponseValue::Int(i) => i.to_string(),
            ResponseValue::Uint(u) => u.to_string(),
            ResponseValue::Float(f) => f.to_string(),
            ResponseValue::Bool(b) => b.to_string(),
        }
    }
}

/// Device-specific error with code and recovery information.
#[derive(Debug, Clone)]
pub struct DeviceError {
    /// Error code from the device
    pub code: String,
    /// Error name/identifier
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Severity level
    pub severity: ErrorSeverity,
    /// Whether this error is recoverable
    pub recoverable: bool,
}

impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.name, self.code, self.description)
    }
}

impl std::error::Error for DeviceError {}

/// Result of a command execution with retry tracking.
#[derive(Debug)]
pub struct CommandResult {
    /// The response (if successful)
    pub response: String,
    /// Number of retry attempts made
    pub retries: u8,
    /// Total time taken (including retries)
    pub duration: Duration,
}

/// Generic serial driver that interprets TOML device configurations.
///
/// This driver enables config-driven hardware support by:
/// - Formatting commands from templates with parameter interpolation
/// - Parsing responses using regex patterns
/// - Applying unit conversions using evalexpr formulas
/// - Implementing capability traits based on config mappings
/// - Executing Rhai scripts for complex operations (when `scripting` feature is enabled)
#[derive(Clone)]
pub struct GenericSerialDriver {
    /// Device configuration
    config: Arc<DeviceConfig>,
    /// Shared serial port
    port: SharedPort,
    /// Device address (for RS-485 multidrop)
    address: String,
    /// Cached parameter values (for conversions)
    parameters: Arc<Mutex<HashMap<String, f64>>>,
    /// Compiled regex patterns for responses
    response_patterns: Arc<HashMap<String, Regex>>,
    /// Compiled Rhai scripts (when scripting feature is enabled)
    #[cfg(feature = "scripting")]
    compiled_scripts: Arc<CompiledScripts>,
    /// Rhai scripting engine (when scripting feature is enabled)
    #[cfg(feature = "scripting")]
    script_engine: Arc<Engine>,
}

impl GenericSerialDriver {
    /// Create a new GenericSerialDriver from a device configuration.
    ///
    /// # Arguments
    /// * `config` - Device configuration loaded from TOML
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address on the bus (for RS-485 multidrop)
    ///
    /// # Errors
    /// Returns error if regex patterns fail to compile
    pub fn new(config: DeviceConfig, port: SharedPort, address: &str) -> Result<Self> {
        // Pre-compile all response patterns
        let mut response_patterns = HashMap::new();
        for (name, response) in &config.responses {
            if let Some(ref pattern) = response.pattern {
                let regex = Regex::new(pattern)
                    .with_context(|| format!("Failed to compile regex for response '{}'", name))?;
                response_patterns.insert(name.clone(), regex);
            }
        }

        // Initialize parameters with defaults
        let mut parameters = HashMap::new();
        for (name, param) in &config.parameters {
            if let Some(default_val) = param.default.as_f64() {
                parameters.insert(name.clone(), default_val);
            }
        }
        // Set the address parameter
        parameters.insert("address".to_string(), 0.0); // Address is typically string, handle separately

        // Compile Rhai scripts if scripting feature is enabled
        #[cfg(feature = "scripting")]
        let (script_engine, compiled_scripts) = {
            let engine_config = ScriptEngineConfig::default();
            let engine = create_sandboxed_engine(&engine_config);
            let scripts = CompiledScripts::compile_from_config(&config, &engine)
                .context("Failed to compile device scripts")?;
            (Arc::new(engine), Arc::new(scripts))
        };

        Ok(Self {
            config: Arc::new(config),
            port,
            address: address.to_string(),
            parameters: Arc::new(Mutex::new(parameters)),
            response_patterns: Arc::new(response_patterns),
            #[cfg(feature = "scripting")]
            compiled_scripts,
            #[cfg(feature = "scripting")]
            script_engine,
        })
    }

    /// Get the device configuration
    pub fn config(&self) -> &DeviceConfig {
        &self.config
    }

    /// Get the device address
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Get a parameter value
    pub async fn get_parameter(&self, name: &str) -> Option<f64> {
        self.parameters.lock().await.get(name).copied()
    }

    /// Set a parameter value
    pub async fn set_parameter(&self, name: &str, value: f64) {
        self.parameters.lock().await.insert(name.to_string(), value);
    }

    /// Get pulses_per_degree (common ELL14 parameter)
    pub async fn get_pulses_per_degree(&self) -> f64 {
        self.get_parameter("pulses_per_degree")
            .await
            .unwrap_or(398.2222)
    }

    // =========================================================================
    // Command Formatting
    // =========================================================================

    /// Format a command using its template and provided parameters.
    ///
    /// Template interpolation supports:
    /// - `${param}` - Direct substitution
    /// - `${param:08X}` - Formatted as 8-char uppercase hex
    /// - `${param:format}` - Other format specifiers
    ///
    /// # Example
    /// ```rust,ignore
    /// // Template: "${address}ma${position_pulses:08X}"
    /// // With address="2", position_pulses=17920
    /// // Result: "2ma00004600"
    /// ```
    pub async fn format_command(
        &self,
        command_name: &str,
        params: &HashMap<String, f64>,
    ) -> Result<String> {
        let cmd_config = self
            .config
            .commands
            .get(command_name)
            .ok_or_else(|| anyhow!("Unknown command: {}", command_name))?;

        self.interpolate_template(&cmd_config.template, params)
            .await
    }

    /// Interpolate a template string with parameters.
    async fn interpolate_template(
        &self,
        template: &str,
        params: &HashMap<String, f64>,
    ) -> Result<String> {
        let mut result = template.to_string();

        // Find all ${...} patterns
        let pattern = Regex::new(r"\$\{([^}]+)\}").expect("Invalid interpolation regex");

        // Collect all matches first (to avoid borrowing issues)
        let matches: Vec<_> = pattern
            .captures_iter(template)
            .map(|cap| (cap.get(0).unwrap().as_str().to_string(), cap[1].to_string()))
            .collect();

        for (full_match, inner) in matches {
            let replacement = self.resolve_placeholder(&inner, params).await?;
            result = result.replace(&full_match, &replacement);
        }

        Ok(result)
    }

    /// Resolve a single placeholder like "param" or "param:08X"
    async fn resolve_placeholder(
        &self,
        placeholder: &str,
        params: &HashMap<String, f64>,
    ) -> Result<String> {
        // Check for format specifier
        if let Some((name, format)) = placeholder.split_once(':') {
            let value = self.get_param_value(name, params).await?;
            self.format_value(value, format)
        } else {
            let value = self.get_param_value(placeholder, params).await?;
            // Special handling for address (string parameter)
            if placeholder == "address" {
                Ok(self.address.clone())
            } else {
                Ok(value.to_string())
            }
        }
    }

    /// Get parameter value from provided params or stored parameters
    async fn get_param_value(&self, name: &str, params: &HashMap<String, f64>) -> Result<f64> {
        // Check provided params first
        if let Some(&value) = params.get(name) {
            return Ok(value);
        }

        // Fall back to stored parameters
        if let Some(value) = self.get_parameter(name).await {
            return Ok(value);
        }

        Err(anyhow!("Parameter not found: {}", name))
    }

    /// Format a value with a format specifier
    fn format_value(&self, value: f64, format: &str) -> Result<String> {
        // Parse format specifier
        match format {
            // Uppercase hex with width
            f if f.ends_with('X') => {
                let width = f
                    .trim_start_matches('0')
                    .trim_end_matches('X')
                    .parse::<usize>()
                    .unwrap_or(0);
                let int_val = value.round() as i32;
                // For hex, we need to handle signed values as unsigned representation
                let uint_val = int_val as u32;
                if width > 0 {
                    Ok(format!("{:0width$X}", uint_val, width = width))
                } else {
                    Ok(format!("{:X}", uint_val))
                }
            }
            // Lowercase hex with width
            f if f.ends_with('x') => {
                let width = f
                    .trim_start_matches('0')
                    .trim_end_matches('x')
                    .parse::<usize>()
                    .unwrap_or(0);
                let int_val = value.round() as i32;
                let uint_val = int_val as u32;
                if width > 0 {
                    Ok(format!("{:0width$x}", uint_val, width = width))
                } else {
                    Ok(format!("{:x}", uint_val))
                }
            }
            // Decimal with width
            f if f.ends_with('d') => {
                let width = f
                    .trim_start_matches('0')
                    .trim_end_matches('d')
                    .parse::<usize>()
                    .unwrap_or(0);
                let int_val = value.round() as i64;
                if width > 0 {
                    Ok(format!("{:0width$}", int_val, width = width))
                } else {
                    Ok(format!("{}", int_val))
                }
            }
            _ => Err(anyhow!("Unknown format specifier: {}", format)),
        }
    }

    // =========================================================================
    // Response Parsing
    // =========================================================================

    /// Parse a response using the named response definition.
    ///
    /// # Arguments
    /// * `response_name` - Name of the response definition in config
    /// * `raw_response` - Raw response string from device
    ///
    /// # Returns
    /// Parsed fields as a map of name -> value
    pub fn parse_response(
        &self,
        response_name: &str,
        raw_response: &str,
    ) -> Result<ParsedResponse> {
        let response_config = self
            .config
            .responses
            .get(response_name)
            .ok_or_else(|| anyhow!("Unknown response: {}", response_name))?;

        // Clean the response (trim whitespace)
        let cleaned = raw_response.trim();

        // Try regex pattern matching
        if let Some(regex) = self.response_patterns.get(response_name) {
            if let Some(captures) = regex.captures(cleaned) {
                let mut fields = HashMap::new();

                // Extract named capture groups
                for (field_name, field_config) in &response_config.fields {
                    if let Some(captured) = captures.name(field_name) {
                        let raw_value = captured.as_str();
                        let value = self.parse_field_value(
                            raw_value,
                            &field_config.field_type,
                            field_config.signed,
                        )?;
                        fields.insert(field_name.clone(), value);
                    }
                }

                return Ok(ParsedResponse {
                    fields,
                    raw: cleaned.to_string(),
                });
            } else {
                return Err(anyhow!(
                    "Response '{}' didn't match pattern: '{}'",
                    response_name,
                    cleaned
                ));
            }
        }

        // No pattern defined - return raw
        Ok(ParsedResponse {
            fields: HashMap::new(),
            raw: cleaned.to_string(),
        })
    }

    /// Parse a field value according to its type
    fn parse_field_value(
        &self,
        raw: &str,
        field_type: &FieldType,
        signed: bool,
    ) -> Result<ResponseValue> {
        match field_type {
            FieldType::String => Ok(ResponseValue::String(raw.to_string())),
            FieldType::Int => {
                let val: i64 = raw.parse().context("Failed to parse int")?;
                Ok(ResponseValue::Int(val))
            }
            FieldType::Uint => {
                let val: u64 = raw.parse().context("Failed to parse uint")?;
                Ok(ResponseValue::Uint(val))
            }
            FieldType::Float => {
                let val: f64 = raw.parse().context("Failed to parse float")?;
                Ok(ResponseValue::Float(val))
            }
            FieldType::Bool => {
                let val = matches!(raw.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
                Ok(ResponseValue::Bool(val))
            }
            FieldType::HexU8 => {
                let val = u8::from_str_radix(raw, 16).context("Failed to parse hex_u8")?;
                Ok(ResponseValue::Uint(val as u64))
            }
            FieldType::HexU16 => {
                let val = u16::from_str_radix(raw, 16).context("Failed to parse hex_u16")?;
                Ok(ResponseValue::Uint(val as u64))
            }
            FieldType::HexU32 => {
                let val = u32::from_str_radix(raw, 16).context("Failed to parse hex_u32")?;
                Ok(ResponseValue::Uint(val as u64))
            }
            FieldType::HexU64 => {
                let val = u64::from_str_radix(raw, 16).context("Failed to parse hex_u64")?;
                Ok(ResponseValue::Uint(val))
            }
            FieldType::HexI32 => {
                // Parse as unsigned first, then reinterpret as signed if needed
                let unsigned = u32::from_str_radix(raw, 16).context("Failed to parse hex_i32")?;
                if signed {
                    Ok(ResponseValue::Int(unsigned as i32 as i64))
                } else {
                    Ok(ResponseValue::Uint(unsigned as u64))
                }
            }
            FieldType::HexI64 => {
                let unsigned = u64::from_str_radix(raw, 16).context("Failed to parse hex_i64")?;
                if signed {
                    Ok(ResponseValue::Int(unsigned as i64))
                } else {
                    Ok(ResponseValue::Uint(unsigned))
                }
            }
        }
    }

    // =========================================================================
    // Unit Conversion
    // =========================================================================

    /// Apply a conversion formula to transform a value.
    ///
    /// # Arguments
    /// * `conversion_name` - Name of the conversion in config
    /// * `input_var` - Name of the input variable in the formula
    /// * `input_value` - Value to convert
    ///
    /// # Example
    /// ```rust,ignore
    /// // Formula: "round(degrees * pulses_per_degree)"
    /// let pulses = driver.apply_conversion("degrees_to_pulses", "degrees", 45.0).await?;
    /// // pulses = 17920 (with pulses_per_degree = 398.2222)
    /// ```
    pub async fn apply_conversion(
        &self,
        conversion_name: &str,
        input_var: &str,
        input_value: f64,
    ) -> Result<f64> {
        let conversion = self
            .config
            .conversions
            .get(conversion_name)
            .ok_or_else(|| anyhow!("Unknown conversion: {}", conversion_name))?;

        // Build context with stored parameters and input
        let params = self.parameters.lock().await;
        let mut context = HashMapContext::new();

        // Add stored parameters
        for (name, value) in params.iter() {
            context
                .set_value(name.clone(), Value::Float(*value))
                .map_err(|e| anyhow!("Failed to set context value '{}': {}", name, e))?;
        }

        // Add input value
        context
            .set_value(input_var.to_string(), Value::Float(input_value))
            .map_err(|e| anyhow!("Failed to set input value '{}': {}", input_var, e))?;

        // Evaluate formula
        let result = eval_number_with_context(&conversion.formula, &context)
            .map_err(|e| anyhow!("Conversion '{}' failed: {}", conversion_name, e))?;

        Ok(result)
    }

    // =========================================================================
    // Serial Communication
    // =========================================================================

    /// Send a command and read the response.
    ///
    /// This method handles the low-level serial communication:
    /// 1. Writes the command bytes
    /// 2. Waits for response with timeout
    /// 3. Returns raw response string
    #[instrument(skip(self), fields(address = %self.address), err)]
    pub async fn transaction(&self, command: &str) -> Result<String> {
        let timeout = Duration::from_millis(self.config.connection.timeout_ms as u64);
        let mut port = self.port.lock().await;

        trace!(command = %command, "Sending command");

        // Write command
        port.write_all(command.as_bytes())
            .await
            .context("Failed to write command")?;

        // Add TX terminator if configured
        if !self.config.connection.terminator_tx.is_empty() {
            port.write_all(self.config.connection.terminator_tx.as_bytes())
                .await
                .context("Failed to write terminator")?;
        }

        // Small delay for device to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Read response with timeout
        let mut response_buf = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(
                remaining.min(Duration::from_millis(100)),
                port.read(&mut buf),
            )
            .await
            {
                Ok(Ok(n)) if n > 0 => {
                    response_buf.extend_from_slice(&buf[..n]);
                    // Brief delay for remaining bytes
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(Ok(_)) => {
                    if !response_buf.is_empty() {
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    if !response_buf.is_empty() {
                        break;
                    }
                }
            }

            // If we have data, try one more read
            if !response_buf.is_empty() {
                tokio::time::sleep(Duration::from_millis(30)).await;
                if let Ok(Ok(n)) =
                    tokio::time::timeout(Duration::from_millis(50), port.read(&mut buf)).await
                {
                    if n > 0 {
                        response_buf.extend_from_slice(&buf[..n]);
                    }
                }
                break;
            }
        }

        let response = std::str::from_utf8(&response_buf)
            .context("Invalid UTF-8 in response")?
            .trim()
            .to_string();

        debug!(command = %command, response = %response, "Transaction complete");

        Ok(response)
    }

    /// Send a command without waiting for response.
    #[instrument(skip(self), fields(address = %self.address), err)]
    pub async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        trace!(command = %command, "Sending command (no response)");

        port.write_all(command.as_bytes())
            .await
            .context("Failed to write command")?;

        if !self.config.connection.terminator_tx.is_empty() {
            port.write_all(self.config.connection.terminator_tx.as_bytes())
                .await
                .context("Failed to write terminator")?;
        }

        // Brief delay
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(())
    }

    // =========================================================================
    // Error Detection and Retry
    // =========================================================================

    /// Check if a response contains a device error code.
    ///
    /// Returns `Some(DeviceError)` if an error is detected, `None` otherwise.
    pub fn check_for_error(&self, response: &str) -> Option<DeviceError> {
        // Check each configured error code
        for (code, error_config) in &self.config.error_codes {
            // Simple check: see if the response contains this error code
            // More sophisticated matching could be added based on device protocol
            if response.contains(code) {
                return Some(DeviceError {
                    code: code.clone(),
                    name: error_config.name.clone(),
                    description: error_config.description.clone(),
                    severity: error_config.severity,
                    recoverable: error_config.recoverable,
                });
            }
        }
        None
    }

    /// Determine if an error should trigger a retry.
    fn should_retry(&self, error: &DeviceError, retry_config: &RetryConfig) -> bool {
        // Check no_retry_on_errors first (takes precedence)
        if retry_config.no_retry_on_errors.contains(&error.code) {
            return false;
        }

        // If retry_on_errors is empty, retry on all recoverable errors
        if retry_config.retry_on_errors.is_empty() {
            return error.recoverable;
        }

        // Otherwise, only retry on specified errors
        retry_config.retry_on_errors.contains(&error.code)
    }

    /// Execute a command with retry logic.
    ///
    /// Uses per-command timeout if configured, otherwise falls back to connection timeout.
    /// Applies exponential backoff between retries.
    #[instrument(skip(self), fields(address = %self.address, command_name), err)]
    pub async fn execute_with_retry(
        &self,
        command_name: &str,
        params: &HashMap<String, f64>,
    ) -> Result<CommandResult> {
        let start = std::time::Instant::now();

        // Get command config
        let cmd_config = self
            .config
            .commands
            .get(command_name)
            .ok_or_else(|| anyhow!("Unknown command: {}", command_name))?;

        // Format the command
        let cmd = self.format_command(command_name, params).await?;

        // Get retry config (command-specific or default)
        let retry_config = cmd_config
            .retry
            .clone()
            .or_else(|| self.config.default_retry.clone())
            .unwrap_or_default();

        let max_retries = retry_config.max_retries;
        let mut current_delay = retry_config.initial_delay_ms;
        let mut retries = 0u8;

        loop {
            // Execute the transaction
            let result = if cmd_config.expects_response {
                self.transaction_with_timeout(&cmd, cmd_config.timeout_ms)
                    .await
            } else {
                self.send_command(&cmd).await.map(|_| String::new())
            };

            match result {
                Ok(response) => {
                    // Check for device error in response
                    if let Some(device_error) = self.check_for_error(&response) {
                        if retries < max_retries && self.should_retry(&device_error, &retry_config)
                        {
                            warn!(
                                command = %cmd,
                                error = %device_error,
                                retry = retries + 1,
                                "Device error, retrying"
                            );
                            retries += 1;
                            tokio::time::sleep(Duration::from_millis(current_delay as u64)).await;
                            current_delay = (current_delay as f64 * retry_config.backoff_multiplier)
                                .min(retry_config.max_delay_ms as f64)
                                as u32;
                            continue;
                        }
                        return Err(anyhow!("Device error: {}", device_error));
                    }

                    return Ok(CommandResult {
                        response,
                        retries,
                        duration: start.elapsed(),
                    });
                }
                Err(e) => {
                    if retries < max_retries {
                        warn!(
                            command = %cmd,
                            error = %e,
                            retry = retries + 1,
                            "Command failed, retrying"
                        );
                        retries += 1;
                        tokio::time::sleep(Duration::from_millis(current_delay as u64)).await;
                        current_delay = (current_delay as f64 * retry_config.backoff_multiplier)
                            .min(retry_config.max_delay_ms as f64)
                            as u32;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }

    /// Execute transaction with custom timeout (or default if None).
    async fn transaction_with_timeout(
        &self,
        command: &str,
        timeout_ms: Option<u32>,
    ) -> Result<String> {
        let timeout =
            Duration::from_millis(timeout_ms.unwrap_or(self.config.connection.timeout_ms) as u64);
        let mut port = self.port.lock().await;

        trace!(command = %command, timeout_ms = ?timeout.as_millis(), "Sending command with timeout");

        // Write command
        port.write_all(command.as_bytes())
            .await
            .context("Failed to write command")?;

        // Add TX terminator if configured
        if !self.config.connection.terminator_tx.is_empty() {
            port.write_all(self.config.connection.terminator_tx.as_bytes())
                .await
                .context("Failed to write terminator")?;
        }

        // Small delay for device to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Read response with timeout
        let mut response_buf = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(
                remaining.min(Duration::from_millis(100)),
                port.read(&mut buf),
            )
            .await
            {
                Ok(Ok(n)) if n > 0 => {
                    response_buf.extend_from_slice(&buf[..n]);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(Ok(_)) => {
                    if !response_buf.is_empty() {
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    if !response_buf.is_empty() {
                        break;
                    }
                }
            }

            if !response_buf.is_empty() {
                tokio::time::sleep(Duration::from_millis(30)).await;
                if let Ok(Ok(n)) =
                    tokio::time::timeout(Duration::from_millis(50), port.read(&mut buf)).await
                {
                    if n > 0 {
                        response_buf.extend_from_slice(&buf[..n]);
                    }
                }
                break;
            }
        }

        let response = std::str::from_utf8(&response_buf)
            .context("Invalid UTF-8 in response")?
            .trim()
            .to_string();

        debug!(command = %command, response = %response, "Transaction complete");

        Ok(response)
    }

    // =========================================================================
    // Initialization Sequence
    // =========================================================================

    /// Run the device initialization sequence.
    ///
    /// Executes each step in the `init_sequence` configuration, validating
    /// responses against expected patterns if specified.
    #[instrument(skip(self), fields(address = %self.address), err)]
    pub async fn run_init_sequence(&self) -> Result<()> {
        if self.config.init_sequence.is_empty() {
            debug!("No initialization sequence configured");
            return Ok(());
        }

        debug!(
            steps = self.config.init_sequence.len(),
            "Running initialization sequence"
        );

        for (i, step) in self.config.init_sequence.iter().enumerate() {
            debug!(
                step = i + 1,
                command = %step.command,
                description = %step.description,
                "Running init step"
            );

            // Convert params from JSON values to f64
            let mut params = HashMap::new();
            for (name, value) in &step.params {
                if let Some(num) = value.as_f64() {
                    params.insert(name.clone(), num);
                }
            }

            // Execute the command
            let result = self.execute_with_retry(&step.command, &params).await;

            match result {
                Ok(cmd_result) => {
                    // Validate expected response if configured
                    if let Some(ref expect) = step.expect {
                        if !cmd_result.response.contains(expect) {
                            if step.required {
                                return Err(anyhow!(
                                    "Init step {} failed: expected '{}' in response, got '{}'",
                                    i + 1,
                                    expect,
                                    cmd_result.response
                                ));
                            } else {
                                warn!(
                                    step = i + 1,
                                    expected = %expect,
                                    got = %cmd_result.response,
                                    "Init step validation failed (non-required)"
                                );
                            }
                        }
                    }

                    // Apply post-step delay
                    if step.delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(step.delay_ms as u64)).await;
                    }
                }
                Err(e) => {
                    if step.required {
                        return Err(e.context(format!(
                            "Required init step {} ('{}') failed",
                            i + 1,
                            step.command
                        )));
                    } else {
                        warn!(
                            step = i + 1,
                            command = %step.command,
                            error = %e,
                            "Optional init step failed, continuing"
                        );
                    }
                }
            }
        }

        debug!("Initialization sequence complete");
        Ok(())
    }

    // =========================================================================
    // Trait Method Execution
    // =========================================================================

    /// Execute a trait method using the config mapping.
    ///
    /// This is the core method that enables trait implementations:
    /// 1. Looks up method mapping in config
    /// 2. Applies input conversion if specified
    /// 3. Formats and sends command
    /// 4. Parses response if expected
    /// 5. Applies output conversion if specified
    pub async fn execute_trait_method(
        &self,
        trait_name: &str,
        method_name: &str,
        input_value: Option<f64>,
    ) -> Result<Option<f64>> {
        // Look up trait mapping
        let trait_mapping = self
            .config
            .trait_mapping
            .get(trait_name)
            .ok_or_else(|| anyhow!("Trait '{}' not mapped in config", trait_name))?;

        let method = trait_mapping.methods.get(method_name).ok_or_else(|| {
            anyhow!(
                "Method '{}' not mapped for trait '{}'",
                method_name,
                trait_name
            )
        })?;

        // Check if method uses a script (scripting feature must be enabled)
        #[cfg(feature = "scripting")]
        if let Some(ref script_name) = method.script {
            return self.execute_script_method(script_name, input_value).await;
        }

        // Build command parameters
        let mut params = HashMap::new();

        // Apply input conversion if specified
        if let (Some(input), Some(ref conv_name), Some(ref input_param), Some(ref from_param)) = (
            input_value,
            &method.input_conversion,
            &method.input_param,
            &method.from_param,
        ) {
            let converted = self.apply_conversion(conv_name, from_param, input).await?;
            params.insert(input_param.clone(), converted);
        } else if let Some(input) = input_value {
            // No conversion - use input directly
            if let Some(ref input_param) = method.input_param {
                params.insert(input_param.clone(), input);
            }
        }

        // Get command name (required for non-polling methods)
        let command_name = method.command.as_ref().ok_or_else(|| {
            anyhow!(
                "Method '{}' has no command (use execute_poll_method for polling)",
                method_name
            )
        })?;

        // Format command
        let cmd = self.format_command(command_name, &params).await?;

        // Get command config
        let cmd_config = self
            .config
            .commands
            .get(command_name)
            .ok_or_else(|| anyhow!("Command '{}' not found", command_name))?;

        // Send command and get response
        let response = if cmd_config.expects_response {
            self.transaction(&cmd).await?
        } else {
            self.send_command(&cmd).await?;
            String::new()
        };

        // Parse response and apply output conversion
        if let Some(ref response_name) = cmd_config.response {
            let parsed = self.parse_response(response_name, &response)?;

            if let Some(ref output_field) = method.output_field {
                if let Some(value) = parsed.fields.get(output_field) {
                    if let Some(raw_value) = value.as_f64() {
                        // Apply output conversion if specified
                        if let Some(ref conv_name) = method.output_conversion {
                            let converted = self
                                .apply_conversion(conv_name, output_field, raw_value)
                                .await?;
                            return Ok(Some(converted));
                        }
                        return Ok(Some(raw_value));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Execute a polling wait operation (e.g., wait_settled).
    pub async fn execute_poll_method(&self, trait_name: &str, method_name: &str) -> Result<()> {
        let trait_mapping = self
            .config
            .trait_mapping
            .get(trait_name)
            .ok_or_else(|| anyhow!("Trait '{}' not mapped", trait_name))?;

        let method = trait_mapping.methods.get(method_name).ok_or_else(|| {
            anyhow!(
                "Method '{}' not mapped for trait '{}'",
                method_name,
                trait_name
            )
        })?;

        let poll_command = method
            .poll_command
            .as_ref()
            .ok_or_else(|| anyhow!("Method '{}' has no poll_command", method_name))?;

        let success_condition = method
            .success_condition
            .as_ref()
            .ok_or_else(|| anyhow!("Method '{}' has no success_condition", method_name))?;

        let poll_interval = Duration::from_millis(method.poll_interval_ms.unwrap_or(50) as u64);
        let timeout = Duration::from_millis(method.timeout_ms.unwrap_or(10000) as u64);

        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("Polling timeout after {:?}", timeout));
            }

            // Format and send poll command
            let cmd = self.format_command(poll_command, &HashMap::new()).await?;
            let response = self.transaction(&cmd).await?;

            // Get command's response definition
            let cmd_config = self
                .config
                .commands
                .get(poll_command)
                .ok_or_else(|| anyhow!("Poll command '{}' not found", poll_command))?;

            if let Some(ref response_name) = cmd_config.response {
                let parsed = self.parse_response(response_name, &response)?;

                // Evaluate success condition
                if self.evaluate_condition(success_condition, &parsed)? {
                    return Ok(());
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Evaluate a success condition against parsed response.
    fn evaluate_condition(&self, condition: &str, parsed: &ParsedResponse) -> Result<bool> {
        // Simple condition parsing: "field == value" or "field != value"
        if let Some((field, value)) = condition.split_once("==") {
            let field = field.trim();
            let expected: i64 = value.trim().parse().unwrap_or(0);

            if let Some(actual) = parsed.fields.get(field) {
                if let Some(actual_val) = actual.as_i64() {
                    return Ok(actual_val == expected);
                }
            }
            return Ok(false);
        }

        if let Some((field, value)) = condition.split_once("!=") {
            let field = field.trim();
            let expected: i64 = value.trim().parse().unwrap_or(0);

            if let Some(actual) = parsed.fields.get(field) {
                if let Some(actual_val) = actual.as_i64() {
                    return Ok(actual_val != expected);
                }
            }
            return Ok(true); // Field not found, condition passes
        }

        Err(anyhow!("Cannot evaluate condition: {}", condition))
    }

    // =========================================================================
    // Script Execution (requires "scripting" feature)
    // =========================================================================

    /// Execute a Rhai script for a trait method.
    ///
    /// This enables complex device operations that can't be expressed declaratively:
    /// - Multi-step sequences with conditional logic
    /// - Iterative correction loops
    /// - Custom parsing and data transformations
    ///
    /// **Timeout Enforcement:** Scripts are executed in a blocking thread pool with
    /// a timeout wrapper. If the configured timeout is exceeded, the script is
    /// cancelled and an error is returned.
    ///
    /// # Arguments
    /// * `script_name` - Name of the script in the config's `[scripts]` section
    /// * `input_value` - Optional input value passed to the script as `input`
    ///
    /// # Script Context
    /// Scripts receive these variables:
    /// - `input` - The input value (if provided)
    /// - `address` - Device address string
    /// - All parameters from `[parameters]` section
    ///
    /// # Returns
    /// Script's return value converted to f64, or None if script returns nothing.
    #[cfg(feature = "scripting")]
    #[instrument(skip(self), fields(address = %self.address, script = %script_name), err)]
    pub async fn execute_script_method(
        &self,
        script_name: &str,
        input_value: Option<f64>,
    ) -> Result<Option<f64>> {
        // Get compiled script (Arc<AST> for thread-safe sharing)
        let ast = self
            .compiled_scripts
            .get(script_name)
            .ok_or_else(|| anyhow!("Script '{}' not found", script_name))?;

        // Build script context with current parameter snapshot
        // Note: Parameters are cloned to ensure consistent values during script execution
        let params = self.parameters.lock().await.clone();
        let context = ScriptContext::new(&self.address, input_value, params);

        // Get script timeout from config
        let timeout = self
            .config
            .scripts
            .get(script_name)
            .map(|s| Duration::from_millis(s.timeout_ms as u64))
            .unwrap_or(Duration::from_secs(30));

        // Execute script with timeout enforcement
        debug!(script = %script_name, timeout_ms = %timeout.as_millis(), "Executing Rhai script");
        let result =
            execute_script_async(self.script_engine.clone(), ast, &context, timeout).await?;

        // Convert result to f64
        match result {
            ScriptResult::Float(f) => Ok(Some(f)),
            ScriptResult::Int(i) => Ok(Some(i as f64)),
            ScriptResult::None => Ok(None),
            ScriptResult::String(s) => {
                // Try to parse string as number
                Ok(s.parse::<f64>().ok())
            }
            ScriptResult::Bool(b) => Ok(Some(if b { 1.0 } else { 0.0 })),
        }
    }

    /// Check if a script exists in the configuration.
    #[cfg(feature = "scripting")]
    pub fn has_script(&self, name: &str) -> bool {
        self.compiled_scripts.contains(name)
    }
}

// =============================================================================
// Movable Trait Implementation
// =============================================================================

#[async_trait]
impl Movable for GenericSerialDriver {
    #[instrument(skip(self), fields(address = %self.address, position_deg), err)]
    async fn move_abs(&self, position_deg: f64) -> Result<()> {
        self.execute_trait_method("Movable", "move_abs", Some(position_deg))
            .await?;
        Ok(())
    }

    #[instrument(skip(self), fields(address = %self.address, distance_deg), err)]
    async fn move_rel(&self, distance_deg: f64) -> Result<()> {
        self.execute_trait_method("Movable", "move_rel", Some(distance_deg))
            .await?;
        Ok(())
    }

    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn position(&self) -> Result<f64> {
        let result = self
            .execute_trait_method("Movable", "position", None)
            .await?;
        result.ok_or_else(|| anyhow!("Position query returned no value"))
    }

    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn wait_settled(&self) -> Result<()> {
        self.execute_poll_method("Movable", "wait_settled").await
    }

    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn stop(&self) -> Result<()> {
        self.execute_trait_method("Movable", "stop", None).await?;
        Ok(())
    }
}

// =============================================================================
// Readable Trait Implementation
// =============================================================================

#[async_trait]
impl Readable for GenericSerialDriver {
    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn read(&self) -> Result<f64> {
        let result = self.execute_trait_method("Readable", "read", None).await?;
        result.ok_or_else(|| anyhow!("Read returned no value"))
    }
}

// =============================================================================
// WavelengthTunable Trait Implementation
// =============================================================================

#[async_trait]
impl WavelengthTunable for GenericSerialDriver {
    #[instrument(skip(self), fields(address = %self.address, wavelength_nm), err)]
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.execute_trait_method("WavelengthTunable", "set_wavelength", Some(wavelength_nm))
            .await?;
        Ok(())
    }

    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn get_wavelength(&self) -> Result<f64> {
        let result = self
            .execute_trait_method("WavelengthTunable", "get_wavelength", None)
            .await?;
        result.ok_or_else(|| anyhow!("get_wavelength returned no value"))
    }

    fn wavelength_range(&self) -> (f64, f64) {
        // Try to read from config parameters, fall back to defaults
        let min = self
            .config
            .parameters
            .get("wavelength_min")
            .and_then(|p| p.default.as_f64())
            .unwrap_or(700.0);
        let max = self
            .config
            .parameters
            .get("wavelength_max")
            .and_then(|p| p.default.as_f64())
            .unwrap_or(1000.0);
        (min, max)
    }
}

// =============================================================================
// ShutterControl Trait Implementation
// =============================================================================

#[async_trait]
impl ShutterControl for GenericSerialDriver {
    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn open_shutter(&self) -> Result<()> {
        self.execute_trait_method("ShutterControl", "open_shutter", None)
            .await?;
        Ok(())
    }

    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn close_shutter(&self) -> Result<()> {
        self.execute_trait_method("ShutterControl", "close_shutter", None)
            .await?;
        Ok(())
    }

    #[instrument(skip(self), fields(address = %self.address), err)]
    async fn is_shutter_open(&self) -> Result<bool> {
        let result = self
            .execute_trait_method("ShutterControl", "is_shutter_open", None)
            .await?;
        // Convert numeric to bool: 0 = closed (false), non-zero = open (true)
        let value = result.ok_or_else(|| anyhow!("is_shutter_open returned no value"))?;
        Ok(value > 0.5)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_device_config_from_str;
    use std::io::Cursor;

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

        #[allow(dead_code)]
        fn get_written(&self) -> String {
            let buf = self.write_buf.try_lock().unwrap();
            String::from_utf8_lossy(&buf).to_string()
        }
    }

    impl tokio::io::AsyncRead for MockPort {
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

    impl tokio::io::AsyncWrite for MockPort {
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

    const TEST_CONFIG: &str = r#"
[device]
name = "Test ELL14"
protocol = "elliptec"
capabilities = ["Movable"]

[connection]
type = "serial"
timeout_ms = 500

[parameters.address]
type = "string"
default = "0"

[parameters.pulses_per_degree]
type = "float"
default = 398.2222

[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
parameters = { position_pulses = "int32" }

[commands.get_position]
template = "${address}gp"
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
type = "hex_u8"

[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"

[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "position"

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

    #[test]
    fn test_config_loads() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        assert_eq!(config.device.name, "Test ELL14");
        assert_eq!(config.device.protocol, "elliptec");
    }

    #[tokio::test]
    async fn test_format_command_hex() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
        let driver = GenericSerialDriver::new(config, port, "2").unwrap();

        // 45 degrees * 398.2222 = 17920 pulses = 0x4600
        let mut params = HashMap::new();
        params.insert("position_pulses".to_string(), 17920.0);

        let cmd = driver
            .format_command("move_absolute", &params)
            .await
            .unwrap();
        assert_eq!(cmd, "2ma00004600");
    }

    #[tokio::test]
    async fn test_apply_conversion() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
        let driver = GenericSerialDriver::new(config, port, "2").unwrap();

        // degrees_to_pulses: 45 * 398.2222 = 17920
        let pulses = driver
            .apply_conversion("degrees_to_pulses", "degrees", 45.0)
            .await
            .unwrap();
        assert!((pulses - 17920.0).abs() < 1.0);

        // pulses_to_degrees: 17920 / 398.2222 = 45
        let degrees = driver
            .apply_conversion("pulses_to_degrees", "pulses", 17920.0)
            .await
            .unwrap();
        assert!((degrees - 45.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_position_response() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
        let driver = GenericSerialDriver::new(config, port, "2").unwrap();

        // Parse "2PO00004600" -> pulses = 0x4600 = 17920
        let parsed = driver.parse_response("position", "2PO00004600").unwrap();
        assert_eq!(parsed.fields.get("addr").unwrap().as_string(), "2");

        let pulses = parsed.fields.get("pulses").unwrap().as_i64().unwrap();
        assert_eq!(pulses, 17920);
    }

    #[test]
    fn test_parse_status_response() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
        let driver = GenericSerialDriver::new(config, port, "2").unwrap();

        // Parse "2GS00" -> code = 0 (OK)
        let parsed = driver.parse_response("status", "2GS00").unwrap();
        let code = parsed.fields.get("code").unwrap().as_i64().unwrap();
        assert_eq!(code, 0);

        // Parse "2GS02" -> code = 2 (mechanical timeout)
        let parsed = driver.parse_response("status", "2GS02").unwrap();
        let code = parsed.fields.get("code").unwrap().as_i64().unwrap();
        assert_eq!(code, 2);
    }

    #[test]
    fn test_evaluate_condition() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
        let driver = GenericSerialDriver::new(config, port, "2").unwrap();

        let mut fields = HashMap::new();
        fields.insert("code".to_string(), ResponseValue::Uint(0));
        let parsed = ParsedResponse {
            fields,
            raw: "2GS00".to_string(),
        };

        // code == 0 should be true
        assert!(driver.evaluate_condition("code == 0", &parsed).unwrap());

        // code == 1 should be false
        assert!(!driver.evaluate_condition("code == 1", &parsed).unwrap());

        // code != 0 should be false
        assert!(!driver.evaluate_condition("code != 0", &parsed).unwrap());
    }

    // =========================================================================
    // Scripting Tests (requires "scripting" feature)
    // =========================================================================

    #[cfg(feature = "scripting")]
    mod scripting_tests {
        use super::*;

        const SCRIPT_CONFIG: &str = r#"
[device]
name = "Test Device with Scripts"
protocol = "custom"
capabilities = ["Readable"]

[connection]
type = "serial"
timeout_ms = 500

[parameters.address]
type = "string"
default = "0"

[parameters.scale_factor]
type = "float"
default = 2.5

[scripts.calculate_scaled]
script = "input * scale_factor"
description = "Multiply input by scale factor"
timeout_ms = 5000
inputs = ["input"]
returns = "float"

[scripts.hex_conversion]
script = """
let rounded = round(input);
let as_int = rounded.to_int();
let hex_str = to_hex_padded(as_int, 4);
parse_hex(hex_str)
"""
description = "Convert to hex and back"
timeout_ms = 5000
inputs = ["input"]
returns = "int"

[scripts.conditional_logic]
script = """
if input > 100.0 {
    input / 2.0
} else {
    input * 2.0
}
"""
description = "Apply conditional scaling"
timeout_ms = 5000
inputs = ["input"]
returns = "float"

[commands.dummy]
template = "dummy"
expects_response = false

[responses]

[conversions]

[trait_mapping.Readable.read]
script = "calculate_scaled"
"#;

        #[test]
        fn test_script_config_loads() {
            let config = load_device_config_from_str(SCRIPT_CONFIG).unwrap();
            assert!(config.scripts.contains_key("calculate_scaled"));
            assert!(config.scripts.contains_key("hex_conversion"));
            assert!(config.scripts.contains_key("conditional_logic"));
        }

        #[test]
        fn test_driver_compiles_scripts() {
            let config = load_device_config_from_str(SCRIPT_CONFIG).unwrap();
            let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
            let driver = GenericSerialDriver::new(config, port, "0").unwrap();

            assert!(driver.has_script("calculate_scaled"));
            assert!(driver.has_script("hex_conversion"));
            assert!(driver.has_script("conditional_logic"));
            assert!(!driver.has_script("nonexistent"));
        }

        #[tokio::test]
        async fn test_execute_script_with_input() {
            let config = load_device_config_from_str(SCRIPT_CONFIG).unwrap();
            let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
            let driver = GenericSerialDriver::new(config, port, "0").unwrap();

            // scale_factor = 2.5, input = 10.0 -> result = 25.0
            let result = driver
                .execute_script_method("calculate_scaled", Some(10.0))
                .await
                .unwrap();

            assert!(result.is_some());
            let value = result.unwrap();
            assert!((value - 25.0).abs() < 0.001);
        }

        #[tokio::test]
        async fn test_execute_script_hex_conversion() {
            let config = load_device_config_from_str(SCRIPT_CONFIG).unwrap();
            let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
            let driver = GenericSerialDriver::new(config, port, "0").unwrap();

            // 255 -> "00FF" -> 255
            let result = driver
                .execute_script_method("hex_conversion", Some(255.0))
                .await
                .unwrap();

            assert!(result.is_some());
            let value = result.unwrap();
            assert!((value - 255.0).abs() < 0.001);
        }

        #[tokio::test]
        async fn test_execute_script_conditional() {
            let config = load_device_config_from_str(SCRIPT_CONFIG).unwrap();
            let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));
            let driver = GenericSerialDriver::new(config, port, "0").unwrap();

            // input = 50 (<= 100) -> result = 50 * 2 = 100
            let result = driver
                .execute_script_method("conditional_logic", Some(50.0))
                .await
                .unwrap();
            assert!((result.unwrap() - 100.0).abs() < 0.001);

            // input = 200 (> 100) -> result = 200 / 2 = 100
            let result = driver
                .execute_script_method("conditional_logic", Some(200.0))
                .await
                .unwrap();
            assert!((result.unwrap() - 100.0).abs() < 0.001);
        }

        #[test]
        fn test_script_invalid_syntax_fails() {
            let bad_config = r#"
[device]
name = "Bad Script Device"
protocol = "custom"
capabilities = []

[connection]
type = "serial"
timeout_ms = 500

[parameters]

[scripts.bad_script]
script = "let x = 1 +"
description = "Invalid syntax"
timeout_ms = 5000
inputs = []
returns = "int"

[commands]
[responses]
[conversions]
[trait_mapping]
"#;

            let config = load_device_config_from_str(bad_config).unwrap();
            let port: SharedPort = Arc::new(Mutex::new(Box::new(MockPort::new())));

            // Should fail to create driver due to script compilation error
            let result = GenericSerialDriver::new(config, port, "0");
            assert!(result.is_err(), "Expected error for invalid script syntax");
        }
    }
}
