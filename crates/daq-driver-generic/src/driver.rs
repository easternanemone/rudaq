//! Generic Serial Driver - Config-driven hardware driver implementation.
//!
//! This module provides a [`GenericSerialDriver`] that interprets TOML device
//! configurations at runtime, enabling new devices to be added without code changes.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use common::capabilities::{Movable, Readable, ShutterControl, WavelengthTunable};
use daq_plugin_api::config::{ErrorSeverity, InstrumentConfig, ResponseFieldType};
use evalexpr::{eval_number_with_context, ContextWithMutableVariables, HashMapContext, Value};
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// Cached regex for template interpolation (compiled once).
/// Matches patterns like `${param}` or `${param:format}`.
static INTERPOLATION_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([^}]+)\}").expect("Invalid interpolation regex"));
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

// Rhai scripting support (optional)
#[cfg(feature = "scripting")]
use crate::script_engine::{
    create_sandboxed_engine, execute_script_async, CompiledScripts, ScriptContext,
    ScriptEngineConfig, ScriptResult,
};
#[cfg(feature = "scripting")]
use rhai::Engine;

// Re-use the serial port types
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
#[derive(Clone)]
pub struct GenericSerialDriver {
    /// Device configuration
    config: Arc<InstrumentConfig>,
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
    pub fn new(config: InstrumentConfig, port: SharedPort, address: &str) -> Result<Self> {
        // Pre-compile all response patterns
        let mut response_patterns: HashMap<String, Regex> = HashMap::new();
        for (name, response) in &config.responses {
            let regex = Regex::new(&response.pattern)
                .with_context(|| format!("Failed to compile regex for response '{}'", name))?;
            response_patterns.insert(name.to_string(), regex);
        }

        // Initialize parameters with defaults
        let mut parameters: HashMap<String, f64> = HashMap::new();
        for (name, param) in &config.parameters {
            if let Some(default_val) = param.default.as_f64() {
                parameters.insert(name.to_string(), default_val);
            }
        }

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

    pub fn config(&self) -> &InstrumentConfig {
        &self.config
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub async fn get_parameter(&self, name: &str) -> Option<f64> {
        self.parameters.lock().await.get(name).copied()
    }

    pub async fn set_parameter(&self, name: &str, value: f64) {
        self.parameters.lock().await.insert(name.to_string(), value);
    }

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

    async fn interpolate_template(
        &self,
        template: &str,
        params: &HashMap<String, f64>,
    ) -> Result<String> {
        let mut result = template.to_string();
        let matches: Vec<_> = INTERPOLATION_REGEX
            .captures_iter(template)
            .map(|cap| (cap.get(0).unwrap().as_str().to_string(), cap[1].to_string()))
            .collect();

        for (full_match, inner) in matches {
            let replacement = self.resolve_placeholder(&inner, params).await?;
            result = result.replace(&full_match, &replacement);
        }

        Ok(result)
    }

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
            // Special handling for address (string parameter) - check BEFORE get_param_value
            if placeholder == "address" {
                Ok(self.address.clone())
            } else {
                let value = self.get_param_value(placeholder, params).await?;
                Ok(value.to_string())
            }
        }
    }

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

    fn format_value(&self, value: f64, format: &str) -> Result<String> {
        match format {
            f if f.ends_with('X') => {
                let width = f
                    .trim_start_matches('0')
                    .trim_end_matches('X')
                    .parse::<usize>()
                    .unwrap_or(0);
                let uint_val = value.round() as i32 as u32;
                if width > 0 {
                    Ok(format!("{:0width$X}", uint_val, width = width))
                } else {
                    Ok(format!("{:X}", uint_val))
                }
            }
            f if f.ends_with('x') => {
                let width = f
                    .trim_start_matches('0')
                    .trim_end_matches('x')
                    .parse::<usize>()
                    .unwrap_or(0);
                let uint_val = value.round() as i32 as u32;
                if width > 0 {
                    Ok(format!("{:0width$x}", uint_val, width = width))
                } else {
                    Ok(format!("{:x}", uint_val))
                }
            }
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
        let cleaned = raw_response.trim();
        if let Some(regex) = self.response_patterns.get(response_name) {
            if let Some(captures) = regex.captures(cleaned) {
                let mut fields: HashMap<String, ResponseValue> = HashMap::new();
                for (field_name, field_config) in &response_config.fields {
                    if let Some(captured) = captures.name(field_name.as_str()) {
                        let value = self.parse_field_value(
                            captured.as_str(),
                            &field_config.r#type,
                            field_config.signed,
                        )?;
                        fields.insert(field_name.to_string(), value);
                    }
                }
                return Ok(ParsedResponse {
                    fields,
                    raw: cleaned.to_string(),
                });
            }
        }
        Ok(ParsedResponse {
            fields: HashMap::new(),
            raw: cleaned.to_string(),
        })
    }

    fn parse_field_value(
        &self,
        raw: &str,
        field_type: &ResponseFieldType,
        signed: bool,
    ) -> Result<ResponseValue> {
        match field_type {
            ResponseFieldType::String => Ok(ResponseValue::String(raw.to_string())),
            ResponseFieldType::Int => Ok(ResponseValue::Int(
                raw.parse().context("Failed to parse int")?,
            )),
            ResponseFieldType::Float => Ok(ResponseValue::Float(
                raw.parse().context("Failed to parse float")?,
            )),
            ResponseFieldType::HexU32 => Ok(ResponseValue::Uint(
                u32::from_str_radix(raw, 16).context("Failed to parse hex_u32")? as u64,
            )),
            ResponseFieldType::HexI32 => {
                let unsigned = u32::from_str_radix(raw, 16).context("Failed to parse hex_i32")?;
                if signed {
                    Ok(ResponseValue::Int(unsigned as i32 as i64))
                } else {
                    Ok(ResponseValue::Uint(unsigned as u64))
                }
            }
        }
    }

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
        let params = self.parameters.lock().await;
        let mut context = HashMapContext::new();
        for (name, value) in params.iter() {
            context
                .set_value(name.clone(), Value::Float(*value))
                .map_err(|e| anyhow!("{}", e))?;
        }
        context
            .set_value(input_var.to_string(), Value::Float(input_value))
            .map_err(|e| anyhow!("{}", e))?;
        let result = eval_number_with_context(&conversion.formula, &context)
            .map_err(|e| anyhow!("{}", e))?;
        Ok(result)
    }

    pub async fn transaction(&self, command: &str) -> Result<String> {
        let timeout = Duration::from_millis(self.config.connection.timeout_ms as u64);
        let mut port = self.port.lock().await;
        port.write_all(command.as_bytes())
            .await
            .context("Failed to write command")?;
        if !self.config.connection.terminator_tx.is_empty() {
            port.write_all(self.config.connection.terminator_tx.as_bytes())
                .await
                .context("Failed to write terminator")?;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut response_buf = Vec::new();
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
                _ => {
                    if !response_buf.is_empty() {
                        break;
                    }
                }
            }
        }
        Ok(String::from_utf8(response_buf)
            .context("Invalid UTF-8")?
            .trim()
            .to_string())
    }

    pub async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;
        port.write_all(command.as_bytes()).await?;
        if !self.config.connection.terminator_tx.is_empty() {
            port.write_all(self.config.connection.terminator_tx.as_bytes())
                .await?;
        }
        Ok(())
    }

    pub async fn run_init_sequence(&self) -> Result<()> {
        Ok(())
    }

    /// Check if a response contains a device error code.
    /// Returns `Some(DeviceError)` if an error is detected, `None` otherwise.
    pub fn check_for_error(&self, response: &str) -> Option<DeviceError> {
        for (code, error_config) in &self.config.error_codes {
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

    pub async fn execute_trait_method(
        &self,
        trait_name: &str,
        method_name: &str,
        input_value: Option<f64>,
    ) -> Result<Option<f64>> {
        let trait_mapping = self
            .config
            .trait_mapping
            .traits
            .get(trait_name)
            .or_else(|| {
                if trait_name == "Movable" {
                    self.config.trait_mapping.movable.as_ref()
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("Trait '{}' not mapped", trait_name))?;

        let method = trait_mapping
            .get(method_name)
            .ok_or_else(|| anyhow!("Method '{}' not mapped", method_name))?;
        let mut params: HashMap<String, f64> = HashMap::new();
        if let (Some(input), Some(ref conv_name), Some(ref input_param), Some(ref from_param)) = (
            input_value,
            &method.input_conversion,
            &method.input_param,
            &method.from_param,
        ) {
            params.insert(
                input_param.to_string(),
                self.apply_conversion(conv_name, from_param, input).await?,
            );
        } else if let (Some(input), Some(ref input_param)) = (input_value, &method.input_param) {
            params.insert(input_param.to_string(), input);
        }

        let command_name = method
            .command
            .as_ref()
            .ok_or_else(|| anyhow!("No command for method"))?;
        let cmd = self.format_command(command_name, &params).await?;
        let cmd_config = self
            .config
            .commands
            .get(command_name)
            .ok_or_else(|| anyhow!("Command not found"))?;
        let response = if cmd_config.expects_response {
            self.transaction(&cmd).await?
        } else {
            self.send_command(&cmd).await?;
            String::new()
        };

        if let (Some(ref response_name), Some(ref output_field)) =
            (&cmd_config.response, &method.output_field)
        {
            let parsed = self.parse_response(response_name, &response)?;
            if let Some(val) = parsed
                .fields
                .get(output_field.as_str())
                .and_then(|v| v.as_f64())
            {
                if let Some(ref conv_name) = method.output_conversion {
                    return Ok(Some(
                        self.apply_conversion(conv_name, output_field, val).await?,
                    ));
                }
                return Ok(Some(val));
            }
        }
        Ok(None)
    }

    pub async fn execute_poll_method(&self, _trait_name: &str, _method_name: &str) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl Movable for GenericSerialDriver {
    async fn move_abs(&self, pos: f64) -> Result<()> {
        self.execute_trait_method("Movable", "move_abs", Some(pos))
            .await?;
        Ok(())
    }

    async fn move_rel(&self, dist: f64) -> Result<()> {
        self.execute_trait_method("Movable", "move_rel", Some(dist))
            .await?;
        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        self.execute_trait_method("Movable", "position", None)
            .await?
            .ok_or_else(|| anyhow!("No position returned"))
    }

    async fn wait_settled(&self) -> Result<()> {
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.execute_trait_method("Movable", "stop", None).await?;
        Ok(())
    }
}

#[async_trait]
impl Readable for GenericSerialDriver {
    async fn read(&self) -> Result<f64> {
        self.execute_trait_method("Readable", "read", None)
            .await?
            .ok_or_else(|| anyhow!("No read value returned"))
    }
}

#[async_trait]
impl WavelengthTunable for GenericSerialDriver {
    async fn set_wavelength(&self, wl: f64) -> Result<()> {
        self.execute_trait_method("WavelengthTunable", "set_wavelength", Some(wl))
            .await?;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        self.execute_trait_method("WavelengthTunable", "get_wavelength", None)
            .await?
            .ok_or_else(|| anyhow!("No wavelength returned"))
    }

    fn wavelength_range(&self) -> (f64, f64) {
        (700.0, 1000.0)
    }
}

#[async_trait]
impl ShutterControl for GenericSerialDriver {
    async fn open_shutter(&self) -> Result<()> {
        self.execute_trait_method("ShutterControl", "open_shutter", None)
            .await?;
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        self.execute_trait_method("ShutterControl", "close_shutter", None)
            .await?;
        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        let value = self
            .execute_trait_method("ShutterControl", "is_shutter_open", None)
            .await?
            .unwrap_or(0.0);
        Ok(value > 0.5)
    }
}
