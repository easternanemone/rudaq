//! UniversalDriver: config-driven capability dispatch engine.
//!
//! Holds an `Arc<DeviceManifest>` and a transport, implementing capability traits
//! by dispatching through the validated config. Each trait method maps to a
//! `MethodConfig` which specifies the command, input/output conversions, and
//! response field extraction.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::config::validated::{DeviceManifest, MethodConfig, ValidatedFormula, WaitSettledConfig};
use crate::response;
use crate::template;
use crate::transport::Transport;

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
        Self {
            manifest,
            transport: Arc::new(Mutex::new(transport)),
            address: address.to_string(),
        }
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
        let mut params: HashMap<String, serde_json::Value> = HashMap::new();
        params.insert(
            "address".to_string(),
            serde_json::Value::String(self.address.clone()),
        );

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
                    params.insert(
                        param_name.clone(),
                        serde_json::Value::Number(serde_json::Number::from(
                            converted.round() as i64
                        )),
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
        if let Some(ref resp_ref) = cmd_config.response {
            if let Some(parser) = self.manifest.responses.get(&resp_ref.0) {
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
        }

        // No parser configured -- return raw response as string
        let mut result = HashMap::new();
        result.insert(
            "value".to_string(),
            serde_json::Value::String(raw_response.to_string()),
        );
        Ok(result)
    }

    /// Execute a method and extract a single f64 value (with optional output conversion).
    async fn execute_read(&self, mapping: &MethodConfig) -> Result<f64> {
        let fields = self.execute_method(mapping, None).await?;

        let raw_value = if let Some(output_field) = &mapping.output_field {
            fields
                .get(output_field)
                .ok_or_else(|| anyhow!("Output field '{}' not found in response", output_field))?
                .clone()
        } else if let Some(value) = fields.get("value") {
            value.clone()
        } else if fields.len() == 1 {
            fields
                .values()
                .next()
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        } else {
            anyhow::bail!("output_field must be set when response contains multiple fields");
        };

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

        let value = if let Some(output_field) = &mapping.output_field {
            fields
                .get(output_field)
                .ok_or_else(|| anyhow!("Output field '{}' not found in response", output_field))?
                .clone()
        } else if let Some(value) = fields.get("value") {
            value.clone()
        } else if fields.len() == 1 {
            fields
                .values()
                .next()
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        } else {
            anyhow::bail!("output_field must be set when response contains multiple fields");
        };

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
            if start.elapsed().as_millis() as u64 > u64::from(config.timeout_ms) {
                return Err(anyhow!(
                    "wait_settled timed out after {}ms",
                    config.timeout_ms
                ));
            }

            // Build params for the poll command
            let mut params = HashMap::new();
            params.insert(
                "address".to_string(),
                serde_json::Value::String(self.address.clone()),
            );
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
            if let Ok(expected_num) = value.parse::<i64>() {
                if let Some(actual_num) = actual.as_i64() {
                    return Ok(actual_num == expected_num);
                }
            }
            // String comparison
            Ok(actual.as_str() == Some(value) || actual.to_string().trim_matches('"') == value)
        } else if let Some((field, value)) = condition.split_once("!=") {
            let field = field.trim();
            let value = value.trim();
            let actual = fields
                .get(field)
                .ok_or_else(|| anyhow!("Condition field '{}' not found in response", field))?;
            if let Ok(expected_num) = value.parse::<i64>() {
                if let Some(actual_num) = actual.as_i64() {
                    return Ok(actual_num != expected_num);
                }
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
// Capability Trait Implementations
// =============================================================================

#[async_trait]
impl common::capabilities::Movable for UniversalDriver {
    async fn move_abs(&self, position: f64) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .movable
            .as_ref()
            .ok_or_else(|| anyhow!("Movable not configured"))?;
        let mapping = config
            .move_abs
            .as_ref()
            .ok_or_else(|| anyhow!("move_abs not configured"))?;
        self.execute_method(mapping, Some(position)).await?;
        Ok(())
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .movable
            .as_ref()
            .ok_or_else(|| anyhow!("Movable not configured"))?;
        let mapping = config
            .move_rel
            .as_ref()
            .ok_or_else(|| anyhow!("move_rel not configured in manifest"))?;
        self.execute_method(mapping, Some(distance)).await?;
        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        let config = self
            .manifest
            .capabilities
            .movable
            .as_ref()
            .ok_or_else(|| anyhow!("Movable not configured"))?;
        let mapping = config
            .position
            .as_ref()
            .ok_or_else(|| anyhow!("position not configured"))?;
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
            // No wait_settled config; return immediately
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
        let config = self
            .manifest
            .capabilities
            .readable
            .as_ref()
            .ok_or_else(|| anyhow!("Readable not configured"))?;
        let mapping = config
            .read
            .as_ref()
            .ok_or_else(|| anyhow!("read not configured"))?;
        self.execute_read(mapping).await
    }
}

#[async_trait]
impl common::capabilities::Settable for UniversalDriver {
    async fn set_value(&self, name: &str, value: serde_json::Value) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .settable
            .as_ref()
            .ok_or_else(|| anyhow!("Settable not configured"))?;
        let mapping = config
            .set
            .as_ref()
            .ok_or_else(|| anyhow!("set not configured"))?;

        // Extract f64 from the JSON value for the template
        let f_val = value.as_f64().or_else(|| value.as_i64().map(|i| i as f64));

        // Build params manually since set_value takes a name
        let cmd_name = &mapping.command.0;
        let cmd_config = self
            .manifest
            .commands
            .get(cmd_name)
            .ok_or_else(|| anyhow!("Command '{}' not found", cmd_name))?;

        let mut params: HashMap<String, serde_json::Value> = HashMap::new();
        params.insert(
            "address".to_string(),
            serde_json::Value::String(self.address.clone()),
        );
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

        Ok(())
    }

    async fn get_value(&self, name: &str) -> Result<serde_json::Value> {
        anyhow::bail!("get_value('{}') not configured in manifest", name)
    }
}

#[async_trait]
impl common::capabilities::ShutterControl for UniversalDriver {
    async fn open_shutter(&self) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .shutter_control
            .as_ref()
            .ok_or_else(|| anyhow!("ShutterControl not configured"))?;
        let mapping = config
            .open
            .as_ref()
            .ok_or_else(|| anyhow!("shutter open not configured"))?;
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .shutter_control
            .as_ref()
            .ok_or_else(|| anyhow!("ShutterControl not configured"))?;
        let mapping = config
            .close
            .as_ref()
            .ok_or_else(|| anyhow!("shutter close not configured"))?;
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        let config = self
            .manifest
            .capabilities
            .shutter_control
            .as_ref()
            .ok_or_else(|| anyhow!("ShutterControl not configured"))?;
        let mapping = config
            .is_open
            .as_ref()
            .ok_or_else(|| anyhow!("shutter is_open not configured"))?;
        self.execute_read_bool(mapping).await
    }
}

#[async_trait]
impl common::capabilities::WavelengthTunable for UniversalDriver {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .wavelength_tunable
            .as_ref()
            .ok_or_else(|| anyhow!("WavelengthTunable not configured"))?;
        let mapping = config
            .set_wavelength
            .as_ref()
            .ok_or_else(|| anyhow!("set_wavelength not configured"))?;
        self.execute_method(mapping, Some(wavelength_nm)).await?;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        let config = self
            .manifest
            .capabilities
            .wavelength_tunable
            .as_ref()
            .ok_or_else(|| anyhow!("WavelengthTunable not configured"))?;
        let mapping = config
            .get_wavelength
            .as_ref()
            .ok_or_else(|| anyhow!("get_wavelength not configured"))?;
        self.execute_read(mapping).await
    }
}

#[async_trait]
impl common::capabilities::EmissionControl for UniversalDriver {
    async fn enable_emission(&self) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .emission_control
            .as_ref()
            .ok_or_else(|| anyhow!("EmissionControl not configured"))?;
        let mapping = config
            .enable
            .as_ref()
            .ok_or_else(|| anyhow!("emission enable not configured"))?;
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn disable_emission(&self) -> Result<()> {
        let config = self
            .manifest
            .capabilities
            .emission_control
            .as_ref()
            .ok_or_else(|| anyhow!("EmissionControl not configured"))?;
        let mapping = config
            .disable
            .as_ref()
            .ok_or_else(|| anyhow!("emission disable not configured"))?;
        self.execute_method(mapping, None).await?;
        Ok(())
    }

    async fn is_emission_enabled(&self) -> Result<bool> {
        let config = self
            .manifest
            .capabilities
            .emission_control
            .as_ref()
            .ok_or_else(|| anyhow!("EmissionControl not configured"))?;
        let mapping = config
            .is_enabled
            .as_ref()
            .ok_or_else(|| anyhow!("emission is_enabled not configured"))?;
        self.execute_read_bool(mapping).await
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
    use crate::transport::MockTransport;
    use common::capabilities::Movable;

    fn ell14_toml() -> &'static str {
        r#"
schema_version = 3

[device]
name = "Thorlabs ELL14"
capabilities = ["Movable", "Parameterized"]

[connection]
type = "serial"
baud_rate = 9600
timeout_ms = 1000

[commands.move_absolute]
template = "{{ address }}ma{{ position_pulses | hex(8) }}"
parameters = { position_pulses = "int32" }
response = "position"

[commands.get_position]
template = "{{ address }}gp"
response = "position"

[commands.get_status]
template = "{{ address }}gs"
response = "status"

[commands.stop]
template = "{{ address }}st"
expects_response = false

[responses.position]
format = "{addr:1}PO{pulses:hex8}"

[responses.status]
format = "{addr:1}GS{code:hex2}"

[conversions.degrees_to_pulses]
formula = "round(degrees * 398.2222)"

[conversions.pulses_to_degrees]
formula = "pulses / 398.2222"

[capabilities.movable]
move_abs = { command = "move_absolute", input_conversion = "degrees_to_pulses", input_param = "position_pulses", from_param = "degrees" }
position = { command = "get_position", output_conversion = "pulses_to_degrees", output_field = "pulses" }
stop = { command = "stop" }

[capabilities.movable.wait_settled]
poll_command = "get_status"
success_condition = "code == 0"
poll_interval_ms = 50
timeout_ms = 10000
"#
    }

    fn scpi_tcp_toml() -> &'static str {
        r#"
schema_version = 3

[device]
name = "Keithley 2400"
capabilities = ["Readable", "Settable"]

[connection]
type = "tcp"
host = "192.168.1.50"
port = 5025
timeout_ms = 2000

[commands.measure_voltage]
template = ":MEAS:VOLT?"
response_type = "float"

[commands.set_voltage]
template = ":SOUR:VOLT {{ value }}"
expects_response = false

[capabilities.readable]
read = { command = "measure_voltage" }

[capabilities.settable]
set = { command = "set_voltage", from_param = "value" }
"#
    }

    fn parse_ell14() -> DeviceManifest {
        let raw: RawManifest = toml::from_str(ell14_toml()).unwrap();
        parse_manifest(raw).unwrap()
    }

    fn parse_scpi_tcp() -> DeviceManifest {
        let raw: RawManifest = toml::from_str(scpi_tcp_toml()).unwrap();
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
}
