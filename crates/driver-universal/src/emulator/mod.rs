//! Manifest-driven device emulator for mock transport.
//!
//! The `ManifestEmulator` is to mock transport as the `UniversalDriver` is to real
//! transport: **both are driven entirely by the same `DeviceManifest`, with zero
//! per-device code.** Adding a new device TOML automatically gives you both a real
//! driver AND a working mock.
//!
//! # Architecture
//!
//! The emulator mirrors the `UniversalDriver` pipeline in reverse:
//! - `UniversalDriver`: manifest → capability call → template render → send → parse response
//! - `ManifestEmulator`: manifest → receive command → match template → update state → generate response

mod response_gen;
mod template_matcher;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::config::validated::{
    CapabilitySet, CommandConfig, DeviceManifest, MethodConfig, ResponseParser,
};
use crate::format_parser::FormatSegment;
use crate::transport::Transport;
use response_gen::{regex_capture_names, regex_to_template, ResponseGen};
use template_matcher::TemplateMatcher;

/// A compiled command ready for matching and response generation.
struct CompiledCommand {
    name: String,
    matcher: TemplateMatcher,
    expects_response: bool,
    response_gen: Option<ResponseGen>,
    /// State key for this command's response (response ref name or command name).
    response_key: String,
}

/// Route setter command params to a getter command's state.
struct SetterRoute {
    /// Target response key where state should be updated.
    target_key: String,
    /// Map setter param names to getter state field names.
    param_to_field: Vec<(String, String)>,
    /// Fixed values to set (for boolean setters like open/close).
    fixed_values: Vec<(String, Value)>,
}

/// Device lifecycle state for the emulator (bd-wn20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmulatorDeviceState {
    /// Initializing (firmware load, self-test)
    Initializing,
    /// Warming up (laser warmup, sensor cooling, etc.)
    WarmingUp,
    /// Ready for operation
    Ready,
    /// Actively operating (acquiring, moving, etc.)
    Operating,
    /// Error state
    Error,
}

impl EmulatorDeviceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::WarmingUp => "warming_up",
            Self::Ready => "ready",
            Self::Operating => "operating",
            Self::Error => "error",
        }
    }
}

/// Manifest-driven stateful device emulator.
///
/// ONE implementation for ALL devices — mirrors the universality of `UniversalDriver`.
pub struct ManifestEmulator {
    device_name: String,
    #[allow(dead_code)]
    address: String,
    /// Per-response-key state: `response_key → { field_name → Value }`.
    state: HashMap<String, HashMap<String, Value>>,
    /// Compiled command matchers (tried in order).
    commands: Vec<CompiledCommand>,
    /// Setter routing: `setter_cmd_name → Vec<SetterRoute>`.
    setter_routes: HashMap<String, Vec<SetterRoute>>,
    /// Buffered response from the last command.
    pending_response: Option<String>,
    /// Device lifecycle state (bd-wn20).
    device_state: EmulatorDeviceState,
}

/// Emulator fidelity profile controlling response behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmulatorProfile {
    /// Instant responses, zero noise. Good for fast unit tests.
    #[default]
    Fast,
    /// Simulated transport delays proportional to command type.
    Realistic,
    /// Adds gaussian noise to numeric responses.
    Noisy,
    /// Randomly injects timeout/error responses (~5% of commands).
    Faulty,
}

impl EmulatorProfile {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s {
            Some("realistic") => Self::Realistic,
            Some("noisy") => Self::Noisy,
            Some("faulty") => Self::Faulty,
            _ => Self::Fast,
        }
    }
}

/// Transport wrapper around `ManifestEmulator`.
pub struct EmulatorTransport {
    emulator: Mutex<ManifestEmulator>,
    profile: EmulatorProfile,
}

impl ManifestEmulator {
    /// Construct an emulator from a device manifest and address string.
    pub fn from_manifest(manifest: &DeviceManifest, address: &str) -> Result<Self> {
        let mut compiled_commands = Vec::new();
        let mut state: HashMap<String, HashMap<String, Value>> = HashMap::new();

        // 1. Compile command templates and build response generators
        for (name, cmd) in &manifest.commands {
            let matcher = TemplateMatcher::compile(&cmd.template.source, &cmd.parameters)?;

            let response_key = response_key_for(name, cmd);

            let response_gen = build_response_gen(cmd, &response_key, manifest)?;

            compiled_commands.push(CompiledCommand {
                name: name.clone(),
                matcher,
                expects_response: cmd.expects_response,
                response_gen,
                response_key: response_key.clone(),
            });

            // Initialize default state for commands that expect a response
            if cmd.expects_response {
                let entry = state.entry(response_key.clone()).or_default();
                init_default_state(entry, cmd, &response_key, manifest, address);
            }
        }

        // 2. Build setter→getter routes from capabilities
        let mut setter_routes: HashMap<String, Vec<SetterRoute>> = HashMap::new();
        build_capability_routes(
            &manifest.capabilities,
            &manifest.commands,
            &manifest.responses,
            &mut setter_routes,
        );

        // 3. Add SCPI pairing heuristic for non-capability commands
        add_scpi_pairing_routes(&manifest.commands, &manifest.responses, &mut setter_routes);

        Ok(Self {
            device_name: manifest.device.name.clone(),
            address: address.to_string(),
            state,
            commands: compiled_commands,
            setter_routes,
            pending_response: None,
            device_state: EmulatorDeviceState::Ready,
        })
    }

    /// Get the current device lifecycle state.
    pub fn device_state(&self) -> EmulatorDeviceState {
        self.device_state
    }

    /// Transition the device state.
    pub fn set_device_state(&mut self, state: EmulatorDeviceState) {
        if self.device_state != state {
            tracing::info!(
                device = %self.device_name,
                from = %self.device_state.as_str(),
                to = %state.as_str(),
                "Emulator state transition"
            );
            self.device_state = state;
        }
    }

    /// Process an incoming command string.
    pub fn handle_command(&mut self, cmd: &str) {
        let cmd = cmd.trim_end_matches(['\r', '\n']);

        for idx in 0..self.commands.len() {
            if let Some(params) = self.commands[idx].matcher.match_and_decode(cmd) {
                let compiled = &self.commands[idx];
                tracing::trace!(
                    device = %self.device_name,
                    cmd = %cmd,
                    matched = %compiled.name,
                    "Emulator matched command"
                );

                let cmd_name = compiled.name.clone();
                let response_key = compiled.response_key.clone();
                let expects_response = compiled.expects_response;

                // (a) Update this command's own state with extracted params
                if let Some(cmd_state) = self.state.get_mut(&response_key) {
                    for (k, v) in &params {
                        cmd_state.insert(k.clone(), v.clone());
                    }
                    // Copy "address" param as "addr" (format responses use abbreviated name)
                    if let Some(addr) = params.get("address") {
                        cmd_state.insert("addr".to_string(), addr.clone());
                    }
                }

                // (b) Apply setter routes
                if let Some(routes) = self.setter_routes.get(&cmd_name) {
                    for route in routes {
                        let target_state = self.state.entry(route.target_key.clone()).or_default();
                        for (src, dst) in &route.param_to_field {
                            if let Some(val) = params.get(src) {
                                target_state.insert(dst.clone(), val.clone());
                            }
                        }
                        for (field, val) in &route.fixed_values {
                            target_state.insert(field.clone(), val.clone());
                        }
                        // Also propagate address
                        if let Some(addr) = params.get("address") {
                            target_state.insert("addr".to_string(), addr.clone());
                        }
                    }
                }

                // (c) Generate response if expected
                if expects_response {
                    let compiled = &self.commands[idx];
                    if let Some(gen) = &compiled.response_gen {
                        let current_state = self
                            .state
                            .get(&compiled.response_key)
                            .cloned()
                            .unwrap_or_default();
                        match gen.generate(&current_state) {
                            Ok(resp) => self.pending_response = Some(resp),
                            Err(e) => {
                                tracing::error!(
                                    device = %self.device_name,
                                    cmd = %cmd,
                                    err = %e,
                                    "Failed to generate emulator response"
                                );
                                self.pending_response = None;
                            }
                        }
                    } else {
                        // No response generator but expects a response — return empty
                        // string so query() doesn't error. The driver will handle it.
                        self.pending_response = Some(String::new());
                    }
                } else {
                    self.pending_response = None;
                }

                return;
            }
        }

        tracing::warn!(
            device = %self.device_name,
            cmd = %cmd,
            "Emulator received unmatched command"
        );
        self.pending_response = None;
    }
}

#[async_trait::async_trait]
impl Transport for EmulatorTransport {
    async fn send(&self, data: &[u8]) -> Result<()> {
        if self.profile == EmulatorProfile::Faulty && should_inject_fault() {
            return Err(anyhow!("EmulatorTransport(faulty): simulated send timeout"));
        }
        if self.profile == EmulatorProfile::Realistic {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let cmd = String::from_utf8_lossy(data);
        self.emulator
            .lock()
            .map_err(|e| anyhow!("EmulatorTransport lock poisoned: {e}"))?
            .handle_command(&cmd);
        Ok(())
    }

    async fn receive(&self, _timeout: Duration) -> Result<String> {
        if self.profile == EmulatorProfile::Faulty && should_inject_fault() {
            return Err(anyhow!(
                "EmulatorTransport(faulty): simulated receive timeout"
            ));
        }
        if self.profile == EmulatorProfile::Realistic {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let mut response = self
            .emulator
            .lock()
            .map_err(|e| anyhow!("EmulatorTransport lock poisoned: {e}"))?
            .pending_response
            .take()
            .ok_or_else(|| anyhow!("EmulatorTransport: no pending response"))?;

        if self.profile == EmulatorProfile::Noisy {
            response = add_numeric_noise(&response);
        }
        Ok(response)
    }

    async fn query(&self, data: &[u8], _timeout: Duration) -> Result<String> {
        if self.profile == EmulatorProfile::Faulty && should_inject_fault() {
            return Err(anyhow!(
                "EmulatorTransport(faulty): simulated query timeout"
            ));
        }
        if self.profile == EmulatorProfile::Realistic {
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
        let cmd = String::from_utf8_lossy(data);
        let mut emu = self
            .emulator
            .lock()
            .map_err(|e| anyhow!("EmulatorTransport lock poisoned: {e}"))?;
        emu.handle_command(&cmd);
        let mut response = emu
            .pending_response
            .take()
            .ok_or_else(|| anyhow!("EmulatorTransport: command did not generate a response"))?;

        if self.profile == EmulatorProfile::Noisy {
            response = add_numeric_noise(&response);
        }
        Ok(response)
    }
}

/// ~5% fault injection probability.
fn should_inject_fault() -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Simple deterministic pseudo-random: fault on every 20th call
    n % 20 == 7
}

/// Add small gaussian-ish noise to the first float found in a response string.
fn add_numeric_noise(response: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NOISE_COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = NOISE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let noise = ((n as f64 * 0.618_033_988_749_895).fract() - 0.5) * 0.001;

    if let Ok(val) = response.trim().parse::<f64>() {
        format!("{}", val + noise)
    } else {
        response.to_string()
    }
}

impl EmulatorTransport {
    /// Get the current device state.
    pub fn device_state(&self) -> EmulatorDeviceState {
        self.emulator
            .lock()
            .map(|e| e.device_state())
            .unwrap_or(EmulatorDeviceState::Error)
    }
}

/// Create an emulator transport from any `DeviceManifest`.
pub fn create_emulator_transport(
    manifest: &DeviceManifest,
    address: &str,
) -> Result<EmulatorTransport> {
    create_emulator_transport_with_profile(manifest, address, EmulatorProfile::Fast)
}

/// Create an emulator transport with a specific fidelity profile.
pub fn create_emulator_transport_with_profile(
    manifest: &DeviceManifest,
    address: &str,
    profile: EmulatorProfile,
) -> Result<EmulatorTransport> {
    let emulator = ManifestEmulator::from_manifest(manifest, address)?;
    Ok(EmulatorTransport {
        emulator: Mutex::new(emulator),
        profile,
    })
}

// ===========================================================================
// Construction helpers
// ===========================================================================

/// Determine the state key for a command: response ref name if present, else command name.
fn response_key_for(cmd_name: &str, cmd: &CommandConfig) -> String {
    cmd.response
        .as_ref()
        .map(|r| r.0.clone())
        .unwrap_or_else(|| cmd_name.to_string())
}

/// Build a `ResponseGen` for a command.
fn build_response_gen(
    cmd: &CommandConfig,
    _response_key: &str,
    manifest: &DeviceManifest,
) -> Result<Option<ResponseGen>> {
    // Tier 0: SCPI auto-parse
    if let Some(scpi_type) = &cmd.response_type {
        return Ok(Some(ResponseGen::Scpi(scpi_type.clone())));
    }

    // Tier 1-3: named response reference
    if let Some(resp_ref) = &cmd.response {
        if let Some(parser) = manifest.responses.get(&resp_ref.0) {
            return Ok(Some(match parser {
                ResponseParser::Format(fmt) => ResponseGen::FormatString(fmt.segments.clone()),
                ResponseParser::Transform(pipeline) => ResponseGen::TransformInverse {
                    ops: pipeline.ops().to_vec(),
                    cmd_prefix: extract_cmd_prefix(&cmd.template.source),
                },
                ResponseParser::Regex(regex) => {
                    ResponseGen::RegexTemplate(regex_to_template(&regex.source))
                }
            }));
        }
    }

    Ok(None)
}

/// Initialize default state fields for a command's response.
fn init_default_state(
    state: &mut HashMap<String, Value>,
    cmd: &CommandConfig,
    _response_key: &str,
    manifest: &DeviceManifest,
    address: &str,
) {
    if let Some(resp_ref) = &cmd.response {
        if let Some(parser) = manifest.responses.get(&resp_ref.0) {
            match parser {
                ResponseParser::Format(fmt) => {
                    for seg in &fmt.segments {
                        if let FormatSegment::Field(spec) = seg {
                            let default = match spec.name.as_str() {
                                "addr" | "address" => Value::String(address.to_string()),
                                _ => match &spec.kind {
                                    crate::format_parser::FieldKind::Hex(_)
                                    | crate::format_parser::FieldKind::Int => json!(0),
                                    crate::format_parser::FieldKind::Float => json!(0.0),
                                    _ => json!(""),
                                },
                            };
                            state.entry(spec.name.clone()).or_insert(default);
                        }
                    }
                }
                ResponseParser::Transform(_) => {
                    state.entry("value".to_string()).or_insert(json!(0.0));
                }
                ResponseParser::Regex(regex) => {
                    for name in regex_capture_names(&regex.source) {
                        let default = match name.as_str() {
                            "addr" | "address" => Value::String(address.to_string()),
                            "manufacturer" => Value::String(
                                manifest
                                    .device
                                    .name
                                    .split_whitespace()
                                    .next()
                                    .unwrap_or("Manufacturer")
                                    .to_string(),
                            ),
                            "model" => Value::String(
                                manifest
                                    .device
                                    .name
                                    .split_whitespace()
                                    .last()
                                    .unwrap_or("Model")
                                    .to_string(),
                            ),
                            "serial" => Value::String("SN000001".to_string()),
                            "version" | "firmware" => Value::String("01".to_string()),
                            "year" => Value::String("2024".to_string()),
                            "type" => Value::String("0E".to_string()),
                            "rest" => Value::String(String::new()),
                            "flags" => Value::String("00".to_string()),
                            "mode" => Value::String("PID".to_string()),
                            _ => json!("0"),
                        };
                        state.entry(name).or_insert(default);
                    }
                }
            }
        }
    } else if cmd.response_type.is_some() {
        // SCPI auto-parse: single "value" field
        state.entry("value".to_string()).or_insert(json!(0.0));
    }

    // Identity commands get a sensible default
    if cmd.template.source.contains("*IDN?") || cmd.template.source.contains("IDN") {
        let ident = format!("{},Model,SN00001,v1.0", manifest.device.name);
        state.insert("value".to_string(), json!(ident));
    }
}

/// Extract literal text from a command template (skip `{{ ... }}` blocks).
fn extract_cmd_prefix(template_source: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = template_source.chars().collect();
    let mut i = 0;
    let mut in_expr = false;

    while i < chars.len() {
        if !in_expr && i + 1 < chars.len() && chars[i] == '{' && chars[i + 1] == '{' {
            in_expr = true;
            i += 2;
        } else if in_expr && i + 1 < chars.len() && chars[i] == '}' && chars[i + 1] == '}' {
            in_expr = false;
            i += 2;
        } else if !in_expr {
            result.push(chars[i]);
            i += 1;
        } else {
            i += 1;
        }
    }
    result
}

// ===========================================================================
// Capability-based setter→getter routing
// ===========================================================================

/// Build setter→getter routes from the capability configuration.
fn build_capability_routes(
    caps: &CapabilitySet,
    commands: &HashMap<String, CommandConfig>,
    responses: &HashMap<String, ResponseParser>,
    routes: &mut HashMap<String, Vec<SetterRoute>>,
) {
    // Movable
    if let Some(movable) = &caps.movable {
        if let Some(position) = &movable.position {
            if let Some(move_abs) = &movable.move_abs {
                add_param_route(routes, move_abs, position, commands, responses);
            }
            if let Some(move_rel) = &movable.move_rel {
                add_param_route(routes, move_rel, position, commands, responses);
            }
        }
    }

    // WavelengthTunable
    if let Some(wave) = &caps.wavelength_tunable {
        if let (Some(setter), Some(getter)) = (&wave.set_wavelength, &wave.get_wavelength) {
            add_param_route(routes, setter, getter, commands, responses);
        }
    }

    // ShutterControl
    if let Some(shutter) = &caps.shutter_control {
        if let Some(is_open) = &shutter.is_open {
            let target_key = getter_response_key(&is_open.command.0, commands);
            let target_field = determine_target_field(is_open, responses.get(&target_key));

            if let Some(open) = &shutter.open {
                routes
                    .entry(open.command.0.clone())
                    .or_default()
                    .push(SetterRoute {
                        target_key: target_key.clone(),
                        param_to_field: vec![],
                        fixed_values: vec![(target_field.clone(), json!(1))],
                    });
            }
            if let Some(close) = &shutter.close {
                routes
                    .entry(close.command.0.clone())
                    .or_default()
                    .push(SetterRoute {
                        target_key: target_key.clone(),
                        param_to_field: vec![],
                        fixed_values: vec![(target_field, json!(0))],
                    });
            }
        }
    }

    // EmissionControl
    if let Some(emission) = &caps.emission_control {
        if let Some(is_enabled) = &emission.is_enabled {
            let target_key = getter_response_key(&is_enabled.command.0, commands);
            let target_field = determine_target_field(is_enabled, responses.get(&target_key));

            if let Some(enable) = &emission.enable {
                routes
                    .entry(enable.command.0.clone())
                    .or_default()
                    .push(SetterRoute {
                        target_key: target_key.clone(),
                        param_to_field: vec![],
                        fixed_values: vec![(target_field.clone(), json!(1))],
                    });
            }
            if let Some(disable) = &emission.disable {
                routes
                    .entry(disable.command.0.clone())
                    .or_default()
                    .push(SetterRoute {
                        target_key: target_key.clone(),
                        param_to_field: vec![],
                        fixed_values: vec![(target_field, json!(0))],
                    });
            }
        }
    }

    // Settable + Readable
    if let (Some(settable), Some(readable)) = (&caps.settable, &caps.readable) {
        if let (Some(setter), Some(reader)) = (&settable.set, &readable.read) {
            add_param_route(routes, setter, reader, commands, responses);
        }
    }
}

/// Add a parameterized setter→getter route.
fn add_param_route(
    routes: &mut HashMap<String, Vec<SetterRoute>>,
    setter: &MethodConfig,
    getter: &MethodConfig,
    commands: &HashMap<String, CommandConfig>,
    responses: &HashMap<String, ResponseParser>,
) {
    let target_key = getter_response_key(&getter.command.0, commands);
    let target_field = determine_target_field(getter, responses.get(&target_key));

    // Source param: use input_param if specified, else first param from template
    let source_param = setter
        .input_param
        .clone()
        .or_else(|| {
            commands
                .get(&setter.command.0)
                .and_then(|cmd| cmd.parameters.keys().next().cloned())
        })
        .or_else(|| {
            // Extract first {{ var }} from template
            commands
                .get(&setter.command.0)
                .and_then(|cmd| extract_first_template_var(&cmd.template.source))
        })
        .unwrap_or_else(|| "value".to_string());

    routes
        .entry(setter.command.0.clone())
        .or_default()
        .push(SetterRoute {
            target_key,
            param_to_field: vec![(source_param, target_field)],
            fixed_values: vec![],
        });
}

/// Get the response key for a getter command.
fn getter_response_key(cmd_name: &str, commands: &HashMap<String, CommandConfig>) -> String {
    commands
        .get(cmd_name)
        .and_then(|cmd| cmd.response.as_ref().map(|r| r.0.clone()))
        .unwrap_or_else(|| cmd_name.to_string())
}

/// Determine the target field name for a getter method.
fn determine_target_field(
    method: &MethodConfig,
    getter_response: Option<&ResponseParser>,
) -> String {
    // Transform/SCPI responses always use "value" as the field name
    if let Some(parser) = getter_response {
        if matches!(parser, ResponseParser::Transform(_)) {
            return "value".to_string();
        }
    }

    // Use output_field from capability config if specified
    if let Some(field) = &method.output_field {
        return field.clone();
    }

    "value".to_string()
}

/// Extract the first `{{ var_name }}` from a template string.
fn extract_first_template_var(template: &str) -> Option<String> {
    let start = template.find("{{")?;
    let rest = &template[start + 2..];
    let end = rest.find("}}")?;
    let expr = rest[..end].trim();
    // Take just the variable name (before any pipe)
    let var = expr.split('|').next()?.trim();
    if var.is_empty() {
        None
    } else {
        Some(var.to_string())
    }
}

/// Add SCPI pairing heuristic routes (e.g., `"CMD {{ value }}"` ↔ `"CMD?"`)
fn add_scpi_pairing_routes(
    commands: &HashMap<String, CommandConfig>,
    _responses: &HashMap<String, ResponseParser>,
    routes: &mut HashMap<String, Vec<SetterRoute>>,
) {
    // Collect getter commands that end with "?" in their template
    let getters: Vec<(&String, &CommandConfig)> = commands
        .iter()
        .filter(|(_, cmd)| {
            cmd.template.source.ends_with('?')
                && (cmd.response.is_some() || cmd.response_type.is_some())
        })
        .collect();

    for (getter_name, getter_cmd) in &getters {
        let query_template = &getter_cmd.template.source;
        // Strip the trailing "?" to find the prefix
        let prefix = &query_template[..query_template.len() - 1];

        // Look for a setter whose template starts with the same prefix
        for (setter_name, setter_cmd) in commands {
            if setter_name == *getter_name {
                continue;
            }
            // Check if setter template starts with the prefix and has params
            let setter_tmpl = &setter_cmd.template.source;
            let setter_literal = extract_cmd_prefix(setter_tmpl);
            if setter_literal.starts_with(prefix)
                && !setter_cmd.parameters.is_empty()
                && !routes.contains_key(setter_name)
            {
                let target_key = getter_response_key(getter_name, commands);
                let source_param = setter_cmd
                    .parameters
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "value".to_string());

                routes
                    .entry(setter_name.clone())
                    .or_default()
                    .push(SetterRoute {
                        target_key,
                        param_to_field: vec![(source_param, "value".to_string())],
                        fixed_values: vec![],
                    });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse::parse_manifest;
    use crate::config::raw::RawManifest;

    fn load_manifest(toml_str: &str) -> DeviceManifest {
        let raw: RawManifest = toml::from_str(toml_str).expect("valid TOML");
        parse_manifest(raw).expect("valid manifest")
    }

    #[test]
    fn ell14_move_and_position() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let mut emu = ManifestEmulator::from_manifest(&manifest, "0").unwrap();

        // Move to position: 0x0000A1B3 = 41395 pulses
        emu.handle_command("0ma0000A1B3");
        // Now query position
        emu.handle_command("0gp");
        let resp = emu.pending_response.take().expect("should have response");
        // Response should be "0PO0000A1B3"
        assert_eq!(resp, "0PO0000A1B3");
    }

    #[test]
    fn ell14_status_default() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let mut emu = ManifestEmulator::from_manifest(&manifest, "0").unwrap();

        emu.handle_command("0gs");
        let resp = emu.pending_response.take().expect("should have response");
        // Default status: code=0 → "0GS00"
        assert_eq!(resp, "0GS00");
    }

    #[test]
    fn scpi_set_and_measure() {
        let manifest = load_manifest(crate::test_fixtures::SCPI_TCP_TOML);
        let mut emu = ManifestEmulator::from_manifest(&manifest, "").unwrap();

        // Set voltage
        emu.handle_command(":SOUR:VOLT 5.0");
        assert!(emu.pending_response.is_none());

        // Measure voltage
        emu.handle_command(":MEAS:VOLT?");
        let resp = emu.pending_response.take().expect("should have response");
        // Should return the set voltage as float
        assert!(resp.contains('5'));
    }

    #[test]
    fn fire_and_forget_no_response() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let mut emu = ManifestEmulator::from_manifest(&manifest, "0").unwrap();

        emu.handle_command("0st"); // stop command
        assert!(emu.pending_response.is_none());
    }

    #[tokio::test]
    async fn noisy_profile_perturbs_numeric_response() {
        let manifest = load_manifest(crate::test_fixtures::SCPI_TCP_TOML);
        let emu = super::create_emulator_transport_with_profile(
            &manifest,
            "",
            super::EmulatorProfile::Noisy,
        )
        .unwrap();

        // Set a known value
        emu.send(b":SOUR:VOLT 5.0").await.unwrap();
        let resp = emu
            .query(b":MEAS:VOLT?", Duration::from_secs(1))
            .await
            .unwrap();
        let val: f64 = resp.trim().parse().expect("should parse as float");
        // With noise, value should be close to 5.0 but not exactly 5.0
        assert!(
            (val - 5.0).abs() < 0.01,
            "noisy value should be within 0.01 of 5.0, got {val}"
        );
    }

    #[tokio::test]
    async fn realistic_profile_adds_delay() {
        let manifest = load_manifest(crate::test_fixtures::SCPI_TCP_TOML);
        let emu = super::create_emulator_transport_with_profile(
            &manifest,
            "",
            super::EmulatorProfile::Realistic,
        )
        .unwrap();

        let start = tokio::time::Instant::now();
        let _resp = emu
            .query(b":MEAS:VOLT?", Duration::from_secs(1))
            .await
            .unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() >= 5,
            "realistic profile should add transport delay, elapsed={:?}",
            elapsed
        );
    }

    #[test]
    fn unmatched_command_no_response() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let mut emu = ManifestEmulator::from_manifest(&manifest, "0").unwrap();

        emu.handle_command("XYZGARBAGE");
        assert!(emu.pending_response.is_none());
    }
}

// =========================================================================
// Phase 5: Integration tests — full round-trip through UniversalDriver + EmulatorTransport
// =========================================================================

#[allow(clippy::float_cmp)]
#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;

    use crate::config::parse::parse_manifest;
    use crate::config::raw::RawManifest;
    use crate::config::validated::DeviceManifest;
    use crate::driver::UniversalDriver;
    use common::capabilities::{
        EmissionControl, Movable, Readable, ShutterControl, WavelengthTunable,
    };

    use super::create_emulator_transport;

    fn load_manifest(toml_str: &str) -> DeviceManifest {
        let raw: RawManifest = toml::from_str(toml_str).expect("valid TOML");
        parse_manifest(raw).expect("valid manifest")
    }

    fn load_manifest_file(relative_path: &str) -> DeviceManifest {
        // Navigate from crate dir to workspace root (../../)
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let path = workspace_root.join(relative_path);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        load_manifest(&content)
    }

    /// Build a UniversalDriver backed by EmulatorTransport.
    fn build_emulated_driver(manifest: DeviceManifest, address: &str) -> UniversalDriver {
        let manifest = Arc::new(manifest);
        let transport =
            Box::new(create_emulator_transport(&manifest, address).expect("create emulator"));
        UniversalDriver::new(manifest, transport, address)
    }

    // =====================================================================
    // ELL14: Movable — hex format strings, degree↔pulse conversions
    // =====================================================================

    #[tokio::test]
    async fn ell14_move_abs_then_position() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let driver = build_emulated_driver(manifest, "2");

        // Move to 45 degrees
        driver
            .move_abs(45.0)
            .await
            .expect("move_abs should succeed");

        // Read position back — should round-trip through:
        // degrees→pulses→hex→emulator state→hex→pulses→degrees
        let pos = driver.position().await.expect("position should succeed");
        assert!((pos - 45.0).abs() < 0.1, "expected ~45.0, got {pos}");
    }

    #[tokio::test]
    async fn ell14_move_abs_100_degrees() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let driver = build_emulated_driver(manifest, "2");

        driver.move_abs(100.0).await.expect("move_abs");
        let pos = driver.position().await.expect("position");
        assert!((pos - 100.0).abs() < 0.1, "expected ~100.0, got {pos}");
    }

    #[tokio::test]
    async fn ell14_stop_succeeds() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let driver = build_emulated_driver(manifest, "0");

        driver.stop().await.expect("stop should succeed");
    }

    #[tokio::test]
    async fn ell14_default_position_is_zero() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let driver = build_emulated_driver(manifest, "0");

        let pos = driver.position().await.expect("position");
        assert!(pos.abs() < 0.1, "expected ~0.0 default position, got {pos}");
    }

    #[tokio::test]
    async fn ell14_wait_settled() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let driver = build_emulated_driver(manifest, "0");

        // Default status is 0 (OK/settled), so wait_settled should pass immediately
        driver
            .wait_settled()
            .await
            .expect("wait_settled should succeed with code==0");
    }

    // =====================================================================
    // SCPI (Keithley 2400): Readable + Settable
    // =====================================================================

    #[tokio::test]
    async fn scpi_read_returns_float() {
        let manifest = load_manifest(crate::test_fixtures::SCPI_TCP_TOML);
        let driver = build_emulated_driver(manifest, "");

        let value = Readable::read(&driver).await.expect("read");
        // Default state is 0.0
        assert!(value.abs() < 1e-6, "expected ~0.0 default, got {value}");
    }

    #[tokio::test]
    async fn scpi_set_then_read() {
        let manifest = load_manifest(crate::test_fixtures::SCPI_TCP_TOML);
        let driver = build_emulated_driver(manifest, "");

        // Set voltage to 2.75
        common::capabilities::Settable::set_value(&driver, "voltage", serde_json::json!(2.75))
            .await
            .expect("set_value");

        // Read it back
        let value = Readable::read(&driver).await.expect("read");
        assert!((value - 2.75).abs() < 0.01, "expected ~2.75, got {value}");
    }

    // =====================================================================
    // MaiTai: WavelengthTunable + ShutterControl + transform responses
    // =====================================================================

    #[tokio::test]
    async fn maitai_set_and_get_wavelength() {
        let manifest = load_manifest_file("config/devices/maitai.toml");
        let driver = build_emulated_driver(manifest, "");

        driver.set_wavelength(820.0).await.expect("set_wavelength");
        let wl = driver.get_wavelength().await.expect("get_wavelength");
        assert!((wl - 820.0).abs() < 0.1, "expected ~820.0, got {wl}");
    }

    #[tokio::test]
    async fn maitai_shutter_open_close() {
        let manifest = load_manifest_file("config/devices/maitai.toml");
        let driver = build_emulated_driver(manifest, "");

        // Default: shutter closed
        let is_open = driver.is_shutter_open().await.expect("is_shutter_open");
        assert!(!is_open, "shutter should default to closed");

        // Open shutter
        driver.open_shutter().await.expect("open_shutter");
        let is_open = driver.is_shutter_open().await.expect("is_shutter_open");
        assert!(is_open, "shutter should be open after open_shutter");

        // Close shutter
        driver.close_shutter().await.expect("close_shutter");
        let is_open = driver.is_shutter_open().await.expect("is_shutter_open");
        assert!(!is_open, "shutter should be closed after close_shutter");
    }

    #[tokio::test]
    async fn maitai_read_power() {
        let manifest = load_manifest_file("config/devices/maitai.toml");
        let driver = build_emulated_driver(manifest, "");

        // Default power should be 0.0
        let power = Readable::read(&driver).await.expect("read power");
        assert!(
            power.abs() < 1e-6,
            "expected ~0.0 default power, got {power}"
        );
    }

    #[tokio::test]
    async fn maitai_init_sequence() {
        let manifest = load_manifest_file("config/devices/maitai.toml");
        let manifest = Arc::new(manifest);
        let transport =
            Box::new(create_emulator_transport(&manifest, "").expect("create emulator"));
        let driver = UniversalDriver::new(manifest.clone(), transport, "");

        // Init sequence queries identity and shutter state — should succeed
        if !manifest.init_sequence.is_empty() {
            driver
                .run_init_sequence()
                .await
                .expect("init sequence should succeed");
        }
    }

    // =====================================================================
    // ESP300: Movable with transform responses (trim + to_float)
    // =====================================================================

    #[tokio::test]
    async fn esp300_move_abs_then_position() {
        let manifest = load_manifest_file("config/devices/esp300.toml");
        let driver = build_emulated_driver(manifest, "1");

        driver.move_abs(25.0).await.expect("move_abs");
        let pos = driver.position().await.expect("position");
        assert!((pos - 25.0).abs() < 0.1, "expected ~25.0, got {pos}");
    }

    #[tokio::test]
    async fn esp300_stop_succeeds() {
        let manifest = load_manifest_file("config/devices/esp300.toml");
        let driver = build_emulated_driver(manifest, "1");

        driver.stop().await.expect("stop should succeed");
    }

    // =====================================================================
    // PM400: Readable (SCPI transform) + WavelengthTunable
    // =====================================================================

    #[tokio::test]
    async fn pm400_read_power() {
        let manifest = load_manifest_file("config/devices/thorlabs_pm400.toml");
        let driver = build_emulated_driver(manifest, "");

        let power = Readable::read(&driver).await.expect("read power");
        // Default should be 0.0
        assert!(power.abs() < 1e-6, "expected ~0.0 default, got {power}");
    }

    #[tokio::test]
    async fn pm400_set_and_get_wavelength() {
        let manifest = load_manifest_file("config/devices/thorlabs_pm400.toml");
        let driver = build_emulated_driver(manifest, "");

        driver.set_wavelength(632.8).await.expect("set_wavelength");
        let wl = driver.get_wavelength().await.expect("get_wavelength");
        assert!((wl - 632.8).abs() < 0.1, "expected ~632.8, got {wl}");
    }

    // =====================================================================
    // IPG Laser: EmissionControl + Readable (regex_extract + map)
    // =====================================================================

    #[tokio::test]
    async fn ipg_emission_on_off() {
        let manifest = load_manifest_file("config/devices/ipg_laser.toml");
        let driver = build_emulated_driver(manifest, "");

        // Default: emission off
        let enabled = driver
            .is_emission_enabled()
            .await
            .expect("is_emission_enabled");
        assert!(!enabled, "emission should default to disabled");

        // Enable emission
        driver.enable_emission().await.expect("enable_emission");
        let enabled = driver
            .is_emission_enabled()
            .await
            .expect("is_emission_enabled");
        assert!(enabled, "emission should be enabled");

        // Disable emission
        driver.disable_emission().await.expect("disable_emission");
        let enabled = driver
            .is_emission_enabled()
            .await
            .expect("is_emission_enabled");
        assert!(!enabled, "emission should be disabled after disable");
    }

    #[tokio::test]
    async fn ipg_read_power() {
        let manifest = load_manifest_file("config/devices/ipg_laser.toml");
        let driver = build_emulated_driver(manifest, "");

        let power = Readable::read(&driver).await.expect("read power");
        // Default power is 0.0 (emission off → map("Off","0.0") → 0.0)
        assert!(
            power.abs() < 1e-6,
            "expected ~0.0 default power, got {power}"
        );
    }

    // =====================================================================
    // ELL14 full round-trip: degrees → pulses → hex → transport → hex → pulses → degrees
    // =====================================================================

    #[tokio::test]
    async fn ell14_full_roundtrip_multiple_positions() {
        let manifest = load_manifest(crate::test_fixtures::ELL14_TOML);
        let driver = build_emulated_driver(manifest, "0");

        for &target in &[0.0, 45.0, 90.0, 135.0, 180.0, 270.0, 359.0] {
            driver.move_abs(target).await.unwrap();
            let pos = driver.position().await.unwrap();
            assert!(
                (pos - target).abs() < 0.5,
                "at target {target}: expected ~{target}, got {pos}"
            );
        }
    }

    // =====================================================================
    // Real ELL14 TOML file: init sequence succeeds
    // =====================================================================

    #[tokio::test]
    async fn ell14_real_toml_init_sequence() {
        let manifest = load_manifest_file("config/devices/ell14.toml");
        let manifest = Arc::new(manifest);
        let transport =
            Box::new(create_emulator_transport(&manifest, "0").expect("create emulator"));
        let driver = UniversalDriver::new(manifest.clone(), transport, "0");

        if !manifest.init_sequence.is_empty() {
            driver
                .run_init_sequence()
                .await
                .expect("init sequence should succeed");
        }
    }

    #[tokio::test]
    async fn ell14_real_toml_move_and_position() {
        let manifest = load_manifest_file("config/devices/ell14.toml");
        let driver = build_emulated_driver(manifest, "2");

        driver.move_abs(45.0).await.expect("move_abs");
        let pos = driver.position().await.expect("position");
        assert!((pos - 45.0).abs() < 0.5, "expected ~45.0, got {pos}");
    }
}
