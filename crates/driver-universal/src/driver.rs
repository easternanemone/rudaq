//! UniversalDriver: config-driven capability dispatch engine.
//!
//! Holds an `Arc<DeviceManifest>` and a transport, implementing capability traits
//! by dispatching through the validated config. Each trait method maps to a
//! `MethodConfig` which specifies the command, input/output conversions, and
//! response field extraction.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::config::validated::{DeviceManifest, MethodConfig, ValidatedFormula, WaitSettledConfig};
use crate::response;
use crate::template;
use crate::transport::Transport;
use common::observable::ParameterSet;
use common::parameter::Parameter;

/// A config-driven universal device driver.
///
/// The `UniversalDriver` dispatches capability trait calls through a validated
/// `DeviceManifest`. Each trait method (e.g., `move_abs`, `read`) is mapped to
/// a command template, optional input/output conversions, and response parsing.
pub struct UniversalDriver {
    manifest: Arc<DeviceManifest>,
    transport: Arc<Mutex<Box<dyn Transport>>>,
    /// Device address for bus protocols (e.g., RS-485 hex address).
    address: String,
    /// Runtime parameter registry exposed via `Parameterized`.
    parameters: ParameterSet,
}

impl UniversalDriver {
    /// Create a new `UniversalDriver`.
    ///
    /// # Arguments
    /// * `manifest` - Validated device manifest with commands, responses, conversions.
    /// * `transport` - Communication transport (serial, TCP, mock, etc.).
    /// * `address` - Device address for bus protocols (e.g., "2" for ELL14 RS-485).
    pub fn new(
        manifest: Arc<DeviceManifest>,
        transport: Box<dyn Transport>,
        address: &str,
    ) -> Self {
        let parameters = Self::build_parameter_set(&manifest);
        Self {
            manifest,
            transport: Arc::new(Mutex::new(transport)),
            address: address.to_string(),
            parameters,
        }
    }

    /// Create a new `UniversalDriver` with a pre-shared transport.
    ///
    /// Used when multiple devices share the same physical bus (e.g., RS-485
    /// multidrop). All drivers sharing a transport coordinate through the
    /// same `Mutex`, preventing interleaved I/O.
    pub fn new_shared(
        manifest: Arc<DeviceManifest>,
        transport: Arc<Mutex<Box<dyn Transport>>>,
        address: &str,
    ) -> Self {
        let parameters = Self::build_parameter_set(&manifest);
        Self {
            manifest,
            transport,
            address: address.to_string(),
            parameters,
        }
    }

    /// Build runtime parameters from manifest metadata/defaults.
    fn build_parameter_set(manifest: &DeviceManifest) -> ParameterSet {
        let mut params = ParameterSet::new();
        for meta in &manifest.parameter_metadata {
            let value = manifest
                .parameters
                .get(&meta.name)
                .copied()
                .or(meta.default_value)
                .unwrap_or_default();

            let mut param = Parameter::new(meta.name.clone(), value).with_dtype(meta.dtype.clone());
            if let Some(desc) = &meta.description {
                param = param.with_description(desc.clone());
            }
            if let Some(unit) = &meta.unit {
                param = param.with_unit(unit.clone());
            }
            if let (Some(min), Some(max)) = (meta.min_value, meta.max_value) {
                param = param.with_range(min, max);
            }
            if meta.read_only {
                param = param.read_only();
            }
            params.register(param);
        }
        params
    }

    /// Build a base parameter map containing only the device address.
    fn address_params(&self) -> HashMap<String, serde_json::Value> {
        let mut params = HashMap::with_capacity(4);
        params.insert(
            "address".to_string(),
            serde_json::Value::String(self.address.clone()),
        );
        params
    }

    /// Run the device initialization sequence.
    ///
    /// Sends each command in `manifest.init_sequence` via the transport,
    /// with optional delays between commands. Commands that expect responses
    /// use `query()` to drain the RX buffer and prevent stale data from
    /// corrupting subsequent command-response cycles. Errors are fatal.
    pub async fn run_init_sequence(&self) -> Result<()> {
        let timeout = self.manifest.connection.timeout().as_duration();
        for (i, init_cmd) in self.manifest.init_sequence.iter().enumerate() {
            // Render the command template (substitute address if present)
            let params = self.address_params();
            let command_str = template::render_command(
                &crate::config::validated::ValidatedTemplate {
                    source: init_cmd.command.clone(),
                },
                &params,
            )?;

            let transport = self.transport.lock().await;
            if init_cmd.expects_response {
                // Use query() to drain the response and prevent RX buffer desync
                let _ = transport
                    .query(command_str.as_bytes(), timeout)
                    .await
                    .map_err(|e| {
                        anyhow!(
                            "init_sequence[{i}] failed to query '{}': {e}",
                            init_cmd.command
                        )
                    })?;
            } else {
                transport.send(command_str.as_bytes()).await.map_err(|e| {
                    anyhow!(
                        "init_sequence[{i}] failed to send '{}': {e}",
                        init_cmd.command
                    )
                })?;
            }
            drop(transport);

            if let Some(delay) = init_cmd.delay_ms {
                tokio::time::sleep(std::time::Duration::from_millis(u64::from(delay))).await;
            }
        }
        Ok(())
    }

    /// Execute a method mapping -- the core dispatch engine.
    ///
    /// 1. Looks up the command by reference
    /// 2. Builds template parameters (including address and converted input)
    /// 3. Renders the command template
    /// 4. Sends via transport and optionally parses response
    /// 5. Returns parsed fields as a HashMap
    async fn execute_method(
        &self,
        mapping: &MethodConfig,
        input_value: Option<f64>,
    ) -> Result<HashMap<String, serde_json::Value>> {
        let cmd_name = &mapping.command.0;
        let cmd_config = self
            .manifest
            .commands
            .get(cmd_name)
            .ok_or_else(|| anyhow!("Command '{}' not found", cmd_name))?;

        // Build template parameters
        let mut params = self.address_params();

        // Apply input conversion if configured
        if let Some(input_val) = input_value {
            if let Some(conv_ref) = &mapping.input_conversion {
                let conv = self
                    .manifest
                    .conversions
                    .get(&conv_ref.0)
                    .ok_or_else(|| anyhow!("Conversion '{}' not found", conv_ref.0))?;
                let converted =
                    self.evaluate_formula(conv, mapping.from_param.as_ref(), input_val)?;
                if let Some(param_name) = &mapping.input_param {
                    let rounded = converted.round();
                    #[allow(clippy::cast_precision_loss)]
                    // Imprecise bounds are conservative — subsequent cast is safe
                    let (lo, hi) = (i64::MIN as f64, i64::MAX as f64);
                    if !rounded.is_finite() || rounded < lo || rounded > hi {
                        return Err(anyhow!(
                            "Converted value {} for parameter '{}' overflows i64",
                            converted,
                            param_name
                        ));
                    }
                    #[allow(clippy::cast_possible_truncation)]
                    // SAFETY: bounds checked above — rounded is finite and within i64 range
                    let int_val = rounded as i64;
                    params.insert(
                        param_name.clone(),
                        serde_json::Value::Number(serde_json::Number::from(int_val)),
                    );
                }
            } else if let Some(param_name) = &mapping.from_param {
                params.insert(param_name.clone(), serde_json::json!(input_val));
            }
        }

        // Render command template
        let command_str = template::render_command(&cmd_config.template, &params)?;

        // Send and receive — use manifest connection timeout
        let timeout = self.manifest.connection.timeout().as_duration();
        // NOTE: We intentionally hold the transport lock across send+receive.
        // Serial ports require exclusive access during a command-response cycle
        // to prevent interleaved commands from concurrent tasks.
        let transport = self.transport.lock().await;
        let raw_response = if cmd_config.expects_response {
            transport.query(command_str.as_bytes(), timeout).await?
        } else {
            transport.send(command_str.as_bytes()).await?;
            drop(transport);
            return Ok(HashMap::new());
        };
        drop(transport);

        // Parse response
        self.parse_command_response(cmd_config, &raw_response)
    }

    /// Parse a command response using the appropriate parser tier.
    fn parse_command_response(
        &self,
        cmd_config: &crate::config::validated::CommandConfig,
        raw_response: &str,
    ) -> Result<HashMap<String, serde_json::Value>> {
        // Tier 0: SCPI auto-parse (response_type on the command)
        if let Some(ref scpi_type) = cmd_config.response_type {
            let value = response::parse_scpi(raw_response, scpi_type)?;
            let mut result = HashMap::new();
            result.insert("value".to_string(), value);
            return Ok(result);
        }

        // Tier 1-3: Named response parser
        if let Some(ref resp_ref) = cmd_config.response
            && let Some(parser) = self.manifest.responses.get(&resp_ref.0)
        {
            let parsed = response::parse_with_parser(raw_response, parser)?;
            return match parsed {
                serde_json::Value::Object(map) => Ok(map.into_iter().collect()),
                other => {
                    let mut result = HashMap::new();
                    result.insert("value".to_string(), other);
                    Ok(result)
                }
            };
        }

        // No parser configured -- return raw response as string
        let mut result = HashMap::new();
        result.insert(
            "value".to_string(),
            serde_json::Value::String(raw_response.to_string()),
        );
        Ok(result)
    }

    /// Extract the output value from a parsed response field map.
    ///
    /// Uses `output_field` if specified, otherwise falls back to "value",
    /// then to the single field if the map has exactly one entry.
    fn extract_response_value(
        fields: &HashMap<String, serde_json::Value>,
        output_field: Option<&String>,
    ) -> Result<serde_json::Value> {
        if let Some(output_field) = output_field {
            fields
                .get(output_field)
                .cloned()
                .ok_or_else(|| anyhow!("Output field '{}' not found in response", output_field))
        } else if let Some(value) = fields.get("value") {
            Ok(value.clone())
        } else if fields.len() == 1 {
            Ok(fields
                .values()
                .next()
                .cloned()
                .unwrap_or(serde_json::Value::Null))
        } else {
            anyhow::bail!("output_field must be set when response contains multiple fields")
        }
    }

    /// Execute a method and extract a single f64 value (with optional output conversion).
    async fn execute_read(&self, mapping: &MethodConfig) -> Result<f64> {
        let fields = self.execute_method(mapping, None).await?;
        let raw_value = Self::extract_response_value(&fields, mapping.output_field.as_ref())?;

        #[allow(clippy::cast_precision_loss)]
        // SAFETY: i64 to f64 — precision loss acceptable for device readback values
        let raw_f64 = raw_value
            .as_f64()
            .or_else(|| raw_value.as_i64().map(|i| i as f64))
            .ok_or_else(|| anyhow!("Cannot convert response to f64: {:?}", raw_value))?;

        // Apply output conversion
        if let Some(conv_ref) = &mapping.output_conversion {
            let conv = self
                .manifest
                .conversions
                .get(&conv_ref.0)
                .ok_or_else(|| anyhow!("Conversion '{}' not found", conv_ref.0))?;
            self.evaluate_formula(conv, mapping.output_field.as_ref(), raw_f64)
        } else {
            Ok(raw_f64)
        }
    }

    /// Execute a method and extract a boolean value from the response.
    async fn execute_read_bool(&self, mapping: &MethodConfig) -> Result<bool> {
        let fields = self.execute_method(mapping, None).await?;
        let value = Self::extract_response_value(&fields, mapping.output_field.as_ref())?;

        match &value {
            serde_json::Value::Bool(b) => Ok(*b),
            serde_json::Value::Number(n) => Ok(n.as_i64().unwrap_or(0) != 0),
            serde_json::Value::String(s) => {
                Ok(matches!(s.as_str(), "1" | "ON" | "on" | "true" | "TRUE"))
            }
            _ => Err(anyhow!("Cannot convert response to bool: {:?}", value)),
        }
    }

    /// Evaluate an evalexpr formula with a single variable binding.
    fn evaluate_formula(
        &self,
        formula: &ValidatedFormula,
        variable_name: Option<&String>,
        value: f64,
    ) -> Result<f64> {
        use evalexpr::*;
        let mut context = HashMapContext::new();

        // Bind the value to its variable name
        if let Some(name) = variable_name {
            context.set_value(name.clone(), Value::Float(value))?;
        }

        // Bind device parameters from the manifest's [parameters] section
        for (param_name, param_val) in &self.manifest.parameters {
            context.set_value(param_name.clone(), Value::Float(*param_val))?;
        }

        // Add round function
        context.set_function(
            "round".to_string(),
            Function::new(|arg| match arg {
                Value::Float(f) => Ok(Value::Float(f.round())),
                Value::Int(i) => Ok(Value::Int(*i)),
                _ => Err(EvalexprError::expected_number(arg.clone())),
            }),
        )?;

        let result = eval_with_context_mut(&formula.source, &mut context)?;
        match result {
            Value::Float(f) => Ok(f),
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: i64 to f64 — precision loss acceptable for formula result
            Value::Int(i) => Ok(i as f64),
            other => Err(anyhow!(
                "Formula '{}' returned non-numeric: {:?}",
                formula.source,
                other
            )),
        }
    }

    /// Poll a command until a success condition is met (for wait_settled).
    async fn poll_until_settled(&self, config: &WaitSettledConfig) -> Result<()> {
        let cmd_config = self
            .manifest
            .commands
            .get(&config.poll_command.0)
            .ok_or_else(|| anyhow!("Poll command '{}' not found", config.poll_command.0))?;

        let start = std::time::Instant::now();
        loop {
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: elapsed millis for a timeout check will not exceed u64 range
            if start.elapsed().as_millis() as u64 > u64::from(config.timeout_ms) {
                return Err(anyhow!(
                    "wait_settled timed out after {}ms",
                    config.timeout_ms
                ));
            }

            // Build params for the poll command
            let params = self.address_params();
            let command_str = template::render_command(&cmd_config.template, &params)?;

            let timeout = self.manifest.connection.timeout().as_duration();
            let transport = self.transport.lock().await;
            let raw_response = transport.query(command_str.as_bytes(), timeout).await?;
            drop(transport);

            // Parse response
            let fields = self.parse_command_response(cmd_config, &raw_response)?;

            // Check success condition
            if self.check_condition(&config.success_condition, &fields)? {
                return Ok(());
            }

            tokio::time::sleep(std::time::Duration::from_millis(u64::from(
                config.poll_interval_ms,
            )))
            .await;
        }
    }

    /// Simple condition checker for "field == value" or "field != value".
    fn check_condition(
        &self,
        condition: &str,
        fields: &HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        if let Some((field, value)) = condition.split_once("==") {
            let field = field.trim();
            let value = value.trim();
            let actual = fields
                .get(field)
                .ok_or_else(|| anyhow!("Condition field '{}' not found in response", field))?;

            // Try numeric comparison
            if let Ok(expected_num) = value.parse::<i64>()
                && let Some(actual_num) = actual.as_i64()
            {
                return Ok(actual_num == expected_num);
            }
            // String comparison
            Ok(actual.as_str() == Some(value) || actual.to_string().trim_matches('"') == value)
        } else if let Some((field, value)) = condition.split_once("!=") {
            let field = field.trim();
            let value = value.trim();
            let actual = fields
                .get(field)
                .ok_or_else(|| anyhow!("Condition field '{}' not found in response", field))?;
            if let Ok(expected_num) = value.parse::<i64>()
                && let Some(actual_num) = actual.as_i64()
            {
                return Ok(actual_num != expected_num);
            }
            Ok(actual.as_str() != Some(value))
        } else {
            Err(anyhow!(
                "Unsupported condition format: '{}'. Expected 'field == value' or 'field != value'",
                condition
            ))
        }
    }
}

// =============================================================================
// Capability Method Lookup Helpers
// =============================================================================

/// Helper macro to look up a capability's method config from the manifest.
///
/// Reduces the repetitive pattern of:
///   manifest.capabilities.$cap.as_ref().ok_or(...)? .$method.as_ref().ok_or(...)?
macro_rules! capability_method {
    ($self:expr, $cap:ident, $method:ident, $cap_name:expr, $method_name:expr) => {{
        let config = $self
            .manifest
            .capabilities
            .$cap
            .as_ref()
            .ok_or_else(|| anyhow!(concat!("{}", " not configured"), $cap_name))?;
        config
            .$method
            .as_ref()
            .ok_or_else(|| anyhow!(concat!("{}", " not configured"), $method_name))?
    }};
}

// =============================================================================
// Capability Trait Implementations
// =============================================================================

#[async_trait]
impl common::capabilities::Commandable for UniversalDriver {
    async fn execute_command(
        &self,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let cmd_config = self
            .manifest
            .commands
            .get(command)
            .ok_or_else(|| anyhow!("Unknown command: '{}'", command))?;

        // Build template parameters from args + address
        let mut params = self.address_params();
        if let serde_json::Value::Object(map) = &args {
            for (k, v) in map {
                params.insert(k.clone(), v.clone());
            }
        }

        // Render and send command
        let command_str = template::render_command(&cmd_config.template, &params)?;
        let timeout = self.manifest.connection.timeout().as_duration();
        let transport = self.transport.lock().await;
        let raw_response = if cmd_config.expects_response {
            transport.query(command_str.as_bytes(), timeout).await?
        } else {
            transport.send(command_str.as_bytes()).await?;
            drop(transport);
            return Ok(serde_json::json!({"status": "ok"}));
        };
        drop(transport);

        // Parse response and return as JSON
        let fields = self.parse_command_response(cmd_config, &raw_response)?;
        Ok(serde_json::Value::Object(
            fields
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        ))
    }
}

impl common::capabilities::Parameterized for UniversalDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.parameters
    }
}

#[async_trait]
impl common::capabilities::Movable for UniversalDriver {
    async fn move_abs(&self, position: f64) -> Result<()> {
        let mapping = capability_method!(self, movable, move_abs, "Movable", "move_abs");
        self.execute_method(mapping, Some(position)).await?;
        Ok(())
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let mapping = capability_method!(self, movable, move_rel, "Movable", "move_rel");
        self.execute_method(mapping, Some(distance)).await?;
        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        let mapping = capability_method!(self, movable, position, "Movable", "position");
        self.execute_read(mapping).await
    }

    async fn wait_settled(&self) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .movable
            .as_ref()
            .ok_or_else(|| anyhow!("Movable not configured"))?;
        if let Some(ws_config) = &config.wait_settled {
            self.poll_until_settled(ws_config).await
        } else {
            Ok(())
        }
    }

    async fn stop(&self) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .movable
            .as_ref()
            .ok_or_else(|| anyhow!("Movable not configured"))?;
        if let Some(mapping) = &config.stop {
            self.execute_method(mapping, None).await?;
            Ok(())
        } else {
            anyhow::bail!("Stop not configured for this device")
        }
    }
}

#[async_trait]
impl common::capabilities::Readable for UniversalDriver {
    async fn read(&self) -> Result<f64> {
        let mapping = capability_method!(self, readable, read, "Readable", "read");
        self.execute_read(mapping).await
    }
}

#[async_trait]
impl common::capabilities::SpectrumReadable for UniversalDriver {
    async fn read_spectrum(&self) -> Result<common::capabilities::SpectrumData> {
        let mapping = capability_method!(
            self,
            spectrum_readable,
            read_spectrum,
            "SpectrumReadable",
            "read_spectrum"
        );
        let fields = self.execute_method(mapping, None).await?;
        let raw_value = Self::extract_response_value(&fields, mapping.output_field.as_ref())?;

        let values: Vec<f64> = match raw_value {
            serde_json::Value::Array(arr) => {
                let mut vals = Vec::with_capacity(arr.len());
                for (idx, v) in arr.iter().enumerate() {
                    let num = v.as_f64().ok_or_else(|| {
                        anyhow!(
                            "expected numeric spectrum array element at index {}, got: {}",
                            idx,
                            v
                        )
                    })?;
                    vals.push(num);
                }
                vals
            }
            serde_json::Value::String(s) => {
                let mut vals = Vec::new();
                for (idx, token) in s
                    .split([',', ' ', '\t'])
                    .filter(|s| !s.is_empty())
                    .enumerate()
                {
                    let num = token.parse::<f64>().map_err(|e| {
                        anyhow!(
                            "failed to parse spectrum token '{}' at position {}: {}",
                            token,
                            idx,
                            e
                        )
                    })?;
                    vals.push(num);
                }
                vals
            }
            other => {
                let num = other.as_f64().ok_or_else(|| {
                    anyhow!(
                        "expected numeric spectrum value or numeric vector/string, got: {}",
                        other
                    )
                })?;
                vec![num]
            }
        };

        let sr_config = self
            .manifest
            .capabilities
            .spectrum_readable
            .as_ref()
            .expect("SpectrumReadable config must exist");

        Ok(common::capabilities::SpectrumData {
            values,
            axis: None,
            value_units: sr_config.value_units.clone(),
            axis_units: sr_config.axis_units.clone(),
        })
    }

    fn spectrum_length(&self) -> usize {
        self.manifest
            .capabilities
            .spectrum_readable
            .as_ref()
            .map_or(0, |sr| sr.spectrum_length)
    }
}

#[async_trait]
impl common::capabilities::Settable for UniversalDriver {
    async fn set_value(&self, name: &str, value: serde_json::Value) -> Result<()> {
        let mapping = capability_method!(self, settable, set, "Settable", "set");

        // Extract f64 from the JSON value for the template
        #[allow(clippy::cast_precision_loss)]
        // SAFETY: i64 to f64 — precision loss acceptable for device parameter values
        let f_val = value.as_f64().or_else(|| value.as_i64().map(|i| i as f64));

        // Build params manually since set_value takes a name
        let cmd_name = &mapping.command.0;
        let cmd_config = self
            .manifest
            .commands
            .get(cmd_name)
            .ok_or_else(|| anyhow!("Command '{}' not found", cmd_name))?;

        let mut params = self.address_params();
        params.insert(
            "name".to_string(),
            serde_json::Value::String(name.to_string()),
        );

        // Apply optional input conversion (mirror execute_method logic)
        if let Some(conv_ref) = &mapping.input_conversion {
            let input_f =
                f_val.ok_or_else(|| anyhow!("set_value requires numeric input for conversion"))?;
            let conv = self
                .manifest
                .conversions
                .get(&conv_ref.0)
                .ok_or_else(|| anyhow!("Conversion '{}' not found", conv_ref.0))?;
            let converted = self.evaluate_formula(conv, mapping.from_param.as_ref(), input_f)?;
            let target = mapping
                .input_param
                .as_deref()
                .unwrap_or("value")
                .to_string();
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: rounding f64 to i64 — truncation acceptable for device register values
            params.insert(target, serde_json::json!(converted.round() as i64));
        } else if let Some(param_name) = &mapping.from_param {
            params.insert(param_name.clone(), value);
        } else {
            params.insert("value".to_string(), value);
        }

        let command_str = template::render_command(&cmd_config.template, &params)?;
        let timeout = self.manifest.connection.timeout().as_duration();
        let transport = self.transport.lock().await;
        if cmd_config.expects_response {
            transport.query(command_str.as_bytes(), timeout).await?;
        } else {
            transport.send(command_str.as_bytes()).await?;
        }
        drop(transport);

        // Keep runtime parameter values in sync for manifest-defined params.
        if let Some(v) = f_val
            && let Some(param) = self.parameters.get_typed::<Parameter<f64>>(name)
        {
            let _ = param.set_from_hardware(v).await;
        }

        Ok(())
    }

    async fn get_value(&self, name: &str) -> Result<serde_json::Value> {
        anyhow::bail!("get_value('{}') not configured in manifest", name)
    }
}

#[async_trait]
impl common::capabilities::ShutterControl for UniversalDriver {
    async fn open_shutter(&self) -> Result<()> {
        let mapping = capability_method!(
            self,
            shutter_control,
            open,
            "ShutterControl",
            "shutter open"
        );
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        let mapping = capability_method!(
            self,
            shutter_control,
            close,
            "ShutterControl",
            "shutter close"
        );
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        let mapping = capability_method!(
            self,
            shutter_control,
            is_open,
            "ShutterControl",
            "shutter is_open"
        );
        self.execute_read_bool(mapping).await
    }
}

#[async_trait]
impl common::capabilities::WavelengthTunable for UniversalDriver {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        let mapping = capability_method!(
            self,
            wavelength_tunable,
            set_wavelength,
            "WavelengthTunable",
            "set_wavelength"
        );
        self.execute_method(mapping, Some(wavelength_nm)).await?;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        let mapping = capability_method!(
            self,
            wavelength_tunable,
            get_wavelength,
            "WavelengthTunable",
            "get_wavelength"
        );
        self.execute_read(mapping).await
    }
}

#[async_trait]
impl common::capabilities::EmissionControl for UniversalDriver {
    async fn enable_emission(&self) -> Result<()> {
        let mapping = capability_method!(
            self,
            emission_control,
            enable,
            "EmissionControl",
            "emission enable"
        );
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn disable_emission(&self) -> Result<()> {
        let mapping = capability_method!(
            self,
            emission_control,
            disable,
            "EmissionControl",
            "emission disable"
        );
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn is_emission_enabled(&self) -> Result<bool> {
        let mapping = capability_method!(
            self,
            emission_control,
            is_enabled,
            "EmissionControl",
            "emission is_enabled"
        );
        self.execute_read_bool(mapping).await
    }
}

// =============================================================================
// Post-Reconnection State Refresh (bd-47p2)
// =============================================================================

#[async_trait]
impl common::capabilities::StateRefreshable for UniversalDriver {
    async fn refresh_state(&self) -> Result<HashMap<String, serde_json::Value>> {
        let mut state = HashMap::new();

        // Refresh Movable.position if configured
        if let Some(movable_cfg) = &self.manifest.capabilities.movable
            && let Some(position_mapping) = &movable_cfg.position
        {
            match self.execute_read(position_mapping).await {
                Ok(pos) => {
                    state.insert("position".to_string(), serde_json::json!(pos));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "State refresh: failed to read position"
                    );
                }
            }
        }

        // Refresh WavelengthTunable.get_wavelength if configured
        if let Some(wl_cfg) = &self.manifest.capabilities.wavelength_tunable
            && let Some(get_wl_mapping) = &wl_cfg.get_wavelength
        {
            match self.execute_read(get_wl_mapping).await {
                Ok(wl) => {
                    state.insert("wavelength_nm".to_string(), serde_json::json!(wl));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "State refresh: failed to read wavelength"
                    );
                }
            }
        }

        // Refresh ShutterControl.is_open if configured
        if let Some(shutter_cfg) = &self.manifest.capabilities.shutter_control
            && let Some(is_open_mapping) = &shutter_cfg.is_open
        {
            match self.execute_read_bool(is_open_mapping).await {
                Ok(open) => {
                    state.insert("shutter_open".to_string(), serde_json::json!(open));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "State refresh: failed to read shutter state"
                    );
                }
            }
        }

        // Refresh EmissionControl.is_enabled if configured
        if let Some(emission_cfg) = &self.manifest.capabilities.emission_control
            && let Some(is_enabled_mapping) = &emission_cfg.is_enabled
        {
            match self.execute_read_bool(is_enabled_mapping).await {
                Ok(enabled) => {
                    state.insert("emission_enabled".to_string(), serde_json::json!(enabled));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "State refresh: failed to read emission state"
                    );
                }
            }
        }

        // Refresh Readable.read if configured
        if let Some(readable_cfg) = &self.manifest.capabilities.readable
            && let Some(read_mapping) = &readable_cfg.read
        {
            match self.execute_read(read_mapping).await {
                Ok(val) => {
                    state.insert("value".to_string(), serde_json::json!(val));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "State refresh: failed to read value"
                    );
                }
            }
        }

        if let Some(sr_cfg) = &self.manifest.capabilities.spectrum_readable {
            state.insert(
                "spectrum_length".to_string(),
                serde_json::json!(sr_cfg.spectrum_length),
            );
        }

        tracing::debug!(
            device = %self.manifest.device.name,
            address = %self.address,
            params_refreshed = state.len(),
            "State refresh complete"
        );

        Ok(state)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::config::parse::parse_manifest;
    use crate::config::raw::RawManifest;
    use crate::test_fixtures;
    use crate::transport::MockTransport;
    use common::capabilities::Movable;
    use common::parameter::Parameter;

    fn parse_ell14() -> DeviceManifest {
        let raw: RawManifest = toml::from_str(test_fixtures::ELL14_TOML).unwrap();
        parse_manifest(raw).unwrap()
    }

    fn parse_scpi_tcp() -> DeviceManifest {
        let raw: RawManifest = toml::from_str(test_fixtures::SCPI_TCP_TOML).unwrap();
        parse_manifest(raw).unwrap()
    }

    fn parse_parameterized_settable() -> DeviceManifest {
        let raw: RawManifest = toml::from_str(
            r#"
schema_version = 3

[device]
name = "Parameterized Settable Device"
capabilities = ["Settable", "Parameterized"]

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025
timeout_ms = 1000

[commands.set_gain]
template = ":SOUR:GAIN {{ value }}"
expects_response = false

[capabilities.settable]
set = { command = "set_gain", from_param = "value" }

[parameters.gain]
default = 1.0
type = "float"
description = "Gain factor"
unit = "x"
min = 0.0
max = 10.0
"#,
        )
        .unwrap();
        parse_manifest(raw).unwrap()
    }

    #[tokio::test]
    async fn test_movable_position() {
        let manifest = Arc::new(parse_ell14());
        // 0x0000A1B3 = 41395 pulses
        // 41395 / 398.2222 ≈ 103.95°
        let mock = MockTransport::new(vec!["2PO0000A1B3".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");
        let position = common::capabilities::Movable::position(&driver)
            .await
            .unwrap();
        assert!(
            (position - 103.95).abs() < 0.1,
            "position was {position}, expected ~103.95"
        );
    }

    #[tokio::test]
    async fn test_movable_move_abs() {
        let manifest = Arc::new(parse_ell14());
        // After move_abs, the device returns a position echo
        let mock = MockTransport::new(vec!["2PO00009C40".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");
        // move_abs(100.0) should send: 2ma + hex(round(100 * 398.2222)) = round(39822.22) = 39822
        // 39822 in hex = 0x9B8E
        driver
            .move_abs(100.0)
            .await
            .expect("move_abs should succeed");
    }

    #[tokio::test]
    async fn test_movable_move_abs_sends_correct_command() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec!["2PO00009B8E".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        driver.move_abs(100.0).await.unwrap();

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        // round(100.0 * 398.2222) = 39822 = 0x9B8E
        assert_eq!(sent[0], "2ma00009B8E");
    }

    #[tokio::test]
    async fn test_movable_stop() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec![]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        driver.stop().await.expect("stop should succeed");

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "2st");
    }

    #[tokio::test]
    async fn test_movable_wait_settled() {
        let manifest = Arc::new(parse_ell14());
        // First poll returns non-zero (moving), second returns zero (settled)
        let mock = MockTransport::new(vec![
            "2GS09".to_string(), // status code 9 (moving)
            "2GS00".to_string(), // status code 0 (settled)
        ]);

        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");
        driver
            .wait_settled()
            .await
            .expect("wait_settled should succeed after two polls");
    }

    #[tokio::test]
    async fn test_scpi_readable() {
        let manifest = Arc::new(parse_scpi_tcp());
        let mock = MockTransport::new(vec!["  1.23456E+01\r\n".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock), "");
        let value = common::capabilities::Readable::read(&driver).await.unwrap();
        assert!(
            (value - 12.3456).abs() < 0.001,
            "value was {value}, expected ~12.3456"
        );
    }

    #[tokio::test]
    async fn test_scpi_readable_sends_correct_command() {
        let manifest = Arc::new(parse_scpi_tcp());
        let mock = MockTransport::new(vec!["42.0".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "");
        let _ = common::capabilities::Readable::read(&driver).await.unwrap();

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], ":MEAS:VOLT?");
    }

    #[tokio::test]
    async fn test_settable_set_value() {
        let manifest = Arc::new(parse_scpi_tcp());
        let mock = MockTransport::new(vec![]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "");
        common::capabilities::Settable::set_value(&driver, "voltage", serde_json::json!(2.5))
            .await
            .unwrap();

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], ":SOUR:VOLT 2.5");
    }

    #[tokio::test]
    async fn test_parameterized_exposes_manifest_parameters() {
        let manifest = Arc::new(parse_parameterized_settable());
        let driver = UniversalDriver::new(manifest, Box::new(MockTransport::new(vec![])), "0");

        let params = common::capabilities::Parameterized::parameters(&driver);
        assert!(params.get("gain").is_some(), "gain parameter should exist");

        let gain = params
            .get_typed::<Parameter<f64>>("gain")
            .expect("gain should be typed as Parameter<f64>");
        assert_eq!(gain.get(), 1.0);

        let meta = params.get("gain").unwrap().metadata();
        assert_eq!(meta.dtype, "float");
        assert_eq!(meta.units.as_deref(), Some("x"));
        assert_eq!(meta.description.as_deref(), Some("Gain factor"));
    }

    #[tokio::test]
    async fn test_settable_updates_parameterized_runtime_value() {
        let manifest = Arc::new(parse_parameterized_settable());
        let mock = MockTransport::new(vec![]);
        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "0");

        common::capabilities::Settable::set_value(&driver, "gain", serde_json::json!(2.5))
            .await
            .expect("set_value should succeed");

        let gain = common::capabilities::Parameterized::parameters(&driver)
            .get_typed::<Parameter<f64>>("gain")
            .expect("gain should be typed as Parameter<f64>");
        assert_eq!(gain.get(), 2.5);

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], ":SOUR:GAIN 2.5");
    }

    #[tokio::test]
    async fn test_condition_equality() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec![]);
        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");

        let mut fields = HashMap::new();
        fields.insert("code".to_string(), serde_json::json!(0));
        assert!(driver.check_condition("code == 0", &fields).unwrap());
        assert!(!driver.check_condition("code == 1", &fields).unwrap());
        assert!(!driver.check_condition("code != 0", &fields).unwrap());
        assert!(driver.check_condition("code != 1", &fields).unwrap());
    }

    #[tokio::test]
    async fn test_formula_evaluation_round() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec![]);
        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");

        let formula = ValidatedFormula {
            source: "round(degrees * 398.2222)".to_string(),
        };
        let result = driver
            .evaluate_formula(&formula, Some(&"degrees".to_string()), 100.0)
            .unwrap();
        assert_eq!(result, 39822.0);
    }

    #[tokio::test]
    async fn test_formula_evaluation_division() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec![]);
        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");

        let formula = ValidatedFormula {
            source: "pulses / 398.2222".to_string(),
        };
        let result = driver
            .evaluate_formula(&formula, Some(&"pulses".to_string()), 41395.0)
            .unwrap();
        assert!(
            (result - 103.95).abs() < 0.01,
            "result was {result}, expected ~103.95"
        );
    }

    #[tokio::test]
    async fn test_run_init_sequence() {
        use crate::config::validated::InitCommand;

        let mut manifest = parse_ell14();
        manifest.init_sequence = vec![
            InitCommand {
                command: "*RST".to_string(),
                delay_ms: None,
                expects_response: false,
            },
            InitCommand {
                command: "*CLS".to_string(),
                delay_ms: Some(10),
                expects_response: false,
            },
        ];
        let manifest = Arc::new(manifest);
        let mock = MockTransport::new(vec![]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        driver
            .run_init_sequence()
            .await
            .expect("init sequence should succeed");

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], "*RST");
        assert_eq!(sent[1], "*CLS");
    }

    #[tokio::test]
    async fn test_init_sequence_drains_response() {
        use crate::config::validated::InitCommand;

        let mut manifest = parse_ell14();
        manifest.init_sequence = vec![InitCommand {
            command: "*IDN?".to_string(),
            delay_ms: None,
            expects_response: true,
        }];
        let manifest = Arc::new(manifest);
        // Queue a response that the init command will drain
        let mock = MockTransport::new(vec!["Device XYZ".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        driver
            .run_init_sequence()
            .await
            .expect("init sequence with response should succeed");

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "*IDN?");
        // The response queue should be drained (empty)
        assert!(mock.sent_data().len() == 1); // only the query was sent
    }

    #[tokio::test]
    async fn test_empty_init_sequence() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec![]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        driver
            .run_init_sequence()
            .await
            .expect("empty init sequence should succeed");

        assert!(mock.sent_strings().is_empty());
    }

    #[tokio::test]
    async fn test_commandable_execute_known_command() {
        let manifest = Arc::new(parse_ell14());
        // ELL14 get_position command returns hex position
        let mock = MockTransport::new(vec!["2PO0000A1B3".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        let result = common::capabilities::Commandable::execute_command(
            &driver,
            "get_position",
            serde_json::json!({}),
        )
        .await
        .unwrap();

        // Should return parsed response fields
        assert!(result.is_object());
        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "2gp");
    }

    #[tokio::test]
    async fn test_commandable_unknown_command_fails() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec![]);

        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");
        let result = common::capabilities::Commandable::execute_command(
            &driver,
            "nonexistent_command",
            serde_json::json!({}),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_commandable_fire_and_forget() {
        let manifest = Arc::new(parse_ell14());
        let mock = MockTransport::new(vec![]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        // stop command doesn't expect a response
        let result = common::capabilities::Commandable::execute_command(
            &driver,
            "stop",
            serde_json::json!({}),
        )
        .await
        .unwrap();

        assert_eq!(result, serde_json::json!({"status": "ok"}));
        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "2st");
    }

    // =========================================================================
    // StateRefreshable tests (bd-47p2)
    // =========================================================================

    #[tokio::test]
    async fn test_refresh_state_movable_position() {
        let manifest = Arc::new(parse_ell14());
        // ELL14 position response: 41395 pulses / 398.2222 ~ 103.95 degrees
        let mock = MockTransport::new(vec!["2PO0000A1B3".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "2");
        let state = common::capabilities::StateRefreshable::refresh_state(&driver)
            .await
            .expect("refresh_state should succeed");

        // ELL14 has Movable.position configured, so we should get position back
        assert!(
            state.contains_key("position"),
            "should contain position key"
        );
        let pos = state["position"].as_f64().expect("position should be f64");
        assert!(
            (pos - 103.95).abs() < 0.1,
            "position was {pos}, expected ~103.95"
        );

        // Verify the query command was sent
        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "2gp");
    }

    #[tokio::test]
    async fn test_refresh_state_readable() {
        let manifest = Arc::new(parse_scpi_tcp());
        // SCPI readable device returns voltage
        let mock = MockTransport::new(vec!["42.5".to_string()]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "");
        let state = common::capabilities::StateRefreshable::refresh_state(&driver)
            .await
            .expect("refresh_state should succeed");

        // SCPI device has Readable configured
        assert!(state.contains_key("value"), "should contain value key");
        let val = state["value"].as_f64().expect("value should be f64");
        assert!((val - 42.5).abs() < 0.001, "value was {val}, expected 42.5");

        // Verify the query command was sent
        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], ":MEAS:VOLT?");
    }

    #[tokio::test]
    async fn test_refresh_state_partial_failure_is_non_fatal() {
        let manifest = Arc::new(parse_ell14());
        // No responses queued -- the position query will fail
        let mock = MockTransport::new(vec![]);

        let driver = UniversalDriver::new(manifest, Box::new(mock), "2");
        let state = common::capabilities::StateRefreshable::refresh_state(&driver)
            .await
            .expect("refresh_state should succeed even if individual reads fail");

        // Position read failed, so it should be absent from the map
        assert!(
            !state.contains_key("position"),
            "failed reads should be absent"
        );
    }

    #[tokio::test]
    async fn test_refresh_state_multiple_capabilities() {
        // Build a manifest with both Movable and Readable capabilities
        let toml_str = r#"
schema_version = 3

[device]
name = "Multi-Cap Device"
capabilities = ["Movable", "Readable"]

[connection]
type = "tcp"
host = "192.168.1.50"
port = 5025
timeout_ms = 2000

[commands.get_position]
template = "POS?"
response_type = "float"

[commands.read_value]
template = "MEAS?"
response_type = "float"

[capabilities.movable]
position = { command = "get_position" }

[capabilities.readable]
read = { command = "read_value" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = Arc::new(parse_manifest(raw).unwrap());

        // Queue responses for both position and read queries
        let mock = MockTransport::new(vec![
            "45.0".to_string(),  // position response
            "1.234".to_string(), // read value response
        ]);

        let driver = UniversalDriver::new(manifest, Box::new(mock.clone()), "");
        let state = common::capabilities::StateRefreshable::refresh_state(&driver)
            .await
            .expect("refresh_state should succeed");

        assert_eq!(state.len(), 2, "should have 2 refreshed parameters");
        assert!(state.contains_key("position"));
        assert!(state.contains_key("value"));
        assert!((state["position"].as_f64().unwrap() - 45.0).abs() < 0.001);
        assert!((state["value"].as_f64().unwrap() - 1.234).abs() < 0.001);
    }
}
