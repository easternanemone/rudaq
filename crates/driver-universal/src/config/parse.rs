//! Validation pipeline: `RawManifest` -> `DeviceManifest`.
//!
//! Collects ALL errors rather than bailing on the first, so users see every
//! problem at once.

use crate::config::error::ConfigError;
use crate::config::raw::{
    RawConnectionConfig, RawManifest, RawMethodMapping, RawMovableMapping, RawWaitSettledMapping,
};
use crate::config::validated::{
    BaudRate, CapabilitySet, CommandConfig, CommandRef, CommandUiHint, ConnectionConfig,
    ConversionRef, DeviceInfo, DeviceManifest, EmissionControlConfig, FormatVariant, InitCommand,
    ManifestParameterMeta, MethodConfig, MovableConfig, ParameterUiHint, ReadableConfig,
    ResponseParser, ResponseRef, ScpiResponseType, SerialConfig, SerialFlowControl, SerialParity,
    SettableConfig, ShutterControlConfig, SpectrumReadableConfig, Timeout, ValidatedFormat,
    ValidatedFormula, ValidatedRegex, ValidatedTemplate, WaitSettledConfig,
    WavelengthTunableConfig, WidgetHint,
};
use crate::format_parser;
use crate::template;
use crate::transform::TransformPipeline;
use std::collections::HashMap;

const EXPECTED_SCHEMA_VERSION: u32 = 3;

/// Parse and validate a raw manifest into a fully-validated `DeviceManifest`.
///
/// Collects all validation errors and returns them together, so the user
/// can fix everything in one pass.
///
/// # Errors
///
/// Returns a `Vec<ConfigError>` containing all validation failures.
pub fn parse_manifest(raw: RawManifest) -> Result<DeviceManifest, Vec<ConfigError>> {
    let mut errors: Vec<ConfigError> = Vec::new();

    // 1. Check schema version
    if raw.schema_version != EXPECTED_SCHEMA_VERSION {
        errors.push(ConfigError::UnsupportedSchemaVersion {
            found: raw.schema_version,
            expected: EXPECTED_SCHEMA_VERSION,
        });
        // Schema mismatch is fatal -- rest of config may not make sense
        return Err(errors);
    }

    // 2. Validate connection
    let connection = validate_connection(&raw.connection, &mut errors);

    // 3. Validate all command templates
    let commands = validate_commands(&raw, &mut errors);

    // 4. Validate all response configs
    let responses = validate_responses(&raw, &mut errors);

    // 5. Validate all conversion formulas
    let conversions = validate_conversions(&raw, &mut errors);

    // 6. Cross-validate: command -> response refs
    for (cmd_name, cmd_cfg) in &raw.commands {
        if let Some(ref resp_name) = cmd_cfg.response
            && !raw.responses.contains_key(resp_name)
        {
            errors.push(ConfigError::UnknownResponse {
                command: cmd_name.clone(),
                name: resp_name.clone(),
            });
        }
        // Validate response_type if present
        if let Some(ref rt_str) = cmd_cfg.response_type
            && ScpiResponseType::parse(rt_str).is_none()
        {
            errors.push(ConfigError::Other(format!(
                "command '{cmd_name}' has unknown response_type '{rt_str}' \
                     (expected: float, integer, string, boolean, array_float)"
            )));
        }
    }

    // 7. Cross-validate capabilities
    let capabilities = validate_capabilities(&raw, &mut errors);

    // 8. Extract device parameters (numeric defaults for formula evaluation)
    let mut parameters = HashMap::new();
    for (name, value) in &raw.parameters {
        if let Some(table) = value.as_table() {
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: TOML integers are i64; precision loss acceptable for formula evaluation
            if let Some(default) = table
                .get("default")
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            {
                parameters.insert(name.clone(), default);
            }
        } else if let Some(f) = value.as_float() {
            parameters.insert(name.clone(), f);
        } else if let Some(i) = value.as_integer() {
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: TOML integers are i64; precision loss acceptable for formula evaluation
            let f = i as f64;
            parameters.insert(name.clone(), f);
        }
    }

    // 8b. Extract rich parameter metadata for DB/UI consumption.
    let parameter_metadata = extract_parameter_metadata(&raw.parameters, &mut errors);

    // 9. Validate init_sequence entries
    let init_sequence = validate_init_sequence(&raw, &mut errors);

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(DeviceManifest {
        device: DeviceInfo {
            name: raw.device.name,
            capability_names: raw.device.capabilities,
            manufacturer: raw.device.manufacturer,
            model: raw.device.model,
            category: raw.device.category,
            description: raw.device.description,
        },
        connection,
        commands,
        responses,
        conversions,
        capabilities,
        parameters,
        parameter_metadata,
        init_sequence,
        ui: raw.ui,
    })
}

fn validate_init_sequence(raw: &RawManifest, errors: &mut Vec<ConfigError>) -> Vec<InitCommand> {
    let mut init_sequence = Vec::new();
    for (i, entry) in raw.init_sequence.iter().enumerate() {
        match entry {
            toml::Value::String(cmd_name) => {
                if let Some(cmd) = raw.commands.get(cmd_name) {
                    if !cmd.parameters.is_empty() {
                        errors.push(ConfigError::Other(format!(
                            "init_sequence[{i}]: command '{cmd_name}' requires parameters"
                        )));
                    } else {
                        init_sequence.push(InitCommand {
                            command: cmd.template.clone(),
                            delay_ms: None,
                            expects_response: cmd.expects_response,
                        });
                    }
                } else {
                    errors.push(ConfigError::Other(format!(
                        "init_sequence[{i}]: unknown command '{cmd_name}'"
                    )));
                }
            }
            toml::Value::Table(tbl) => {
                if let Some(init_cmd) = validate_init_table_entry(i, tbl, raw, errors) {
                    init_sequence.push(init_cmd);
                }
            }
            _ => {
                errors.push(ConfigError::Other(format!(
                    "init_sequence[{i}]: must be a string or table"
                )));
            }
        }
    }
    init_sequence
}

fn validate_init_table_entry(
    i: usize,
    tbl: &toml::map::Map<String, toml::Value>,
    raw: &RawManifest,
    errors: &mut Vec<ConfigError>,
) -> Option<InitCommand> {
    let cmd_val = tbl.get("command").or_else(|| {
        errors.push(ConfigError::Other(format!(
            "init_sequence[{i}]: table entry requires 'command' field"
        )));
        None
    })?;
    let cmd_name = cmd_val.as_str().or_else(|| {
        errors.push(ConfigError::Other(format!(
            "init_sequence[{i}]: 'command' must be a string"
        )));
        None
    })?;
    let cmd = raw.commands.get(cmd_name).or_else(|| {
        errors.push(ConfigError::Other(format!(
            "init_sequence[{i}]: unknown command '{cmd_name}'"
        )));
        None
    })?;
    if !cmd.parameters.is_empty() {
        errors.push(ConfigError::Other(format!(
            "init_sequence[{i}]: command '{cmd_name}' requires parameters"
        )));
        return None;
    }
    let delay_ms = match tbl.get("delay_ms") {
        Some(v) => match v.as_integer() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(d) if (0..=60_000).contains(&d) => Some(d as u32),
            Some(d) => {
                errors.push(ConfigError::Other(format!(
                    "init_sequence[{i}]: delay_ms must be 0..=60000 (got {d})"
                )));
                None
            }
            None => {
                errors.push(ConfigError::Other(format!(
                    "init_sequence[{i}]: delay_ms must be an integer (got {v})"
                )));
                None
            }
        },
        None => None,
    };
    Some(InitCommand {
        command: cmd.template.clone(),
        delay_ms,
        expects_response: cmd.expects_response,
    })
}

fn validate_connection(
    raw: &RawConnectionConfig,
    errors: &mut Vec<ConfigError>,
) -> ConnectionConfig {
    match raw {
        RawConnectionConfig::Serial {
            baud_rate,
            timeout_ms,
            terminator,
            terminator_tx,
            data_bits,
            parity,
            stop_bits,
            flow_control,
            ..
        } => {
            let br = match BaudRate::new(*baud_rate) {
                Ok(br) => br,
                Err(_) => {
                    errors.push(ConfigError::InvalidBaudRate(*baud_rate));
                    BaudRate::new(9600).unwrap()
                }
            };
            let timeout = match Timeout::new(*timeout_ms) {
                Ok(t) => t,
                Err(_) => {
                    errors.push(ConfigError::InvalidTimeout(*timeout_ms));
                    Timeout::new(1000).unwrap()
                }
            };

            // Validate serial config fields
            if let Some(db) = data_bits
                && !(5..=8).contains(db)
            {
                errors.push(ConfigError::Other(format!(
                    "data_bits must be 5, 6, 7, or 8 (got {db})"
                )));
            }
            let validated_parity = parity
                .as_ref()
                .map(|p| match p.as_str() {
                    "odd" => Ok(SerialParity::Odd),
                    "even" => Ok(SerialParity::Even),
                    "none" => Ok(SerialParity::None),
                    _ => {
                        errors.push(ConfigError::Other(format!(
                            "parity must be 'none', 'odd', or 'even' (got '{p}')"
                        )));
                        Err(())
                    }
                })
                .and_then(|r| r.ok());
            if let Some(sb) = stop_bits
                && !matches!(sb, 1 | 2)
            {
                errors.push(ConfigError::Other(format!(
                    "stop_bits must be 1 or 2 (got {sb})"
                )));
            }
            let validated_flow_control = flow_control
                .as_ref()
                .map(|fc| match fc.to_lowercase().as_str() {
                    "none" => Ok(SerialFlowControl::None),
                    "software" | "xon/xoff" | "xonxoff" => Ok(SerialFlowControl::Software),
                    "hardware" | "rts/cts" | "rtscts" => Ok(SerialFlowControl::Hardware),
                    _ => {
                        errors.push(ConfigError::Other(format!(
                            "flow_control must be 'none', 'software'/'xon/xoff', or 'hardware'/'rts/cts' (got '{fc}')"
                        )));
                        Err(())
                    }
                })
                .and_then(|r| r.ok());

            // Resolve terminator: terminator_tx takes precedence over terminator
            let resolved_terminator = terminator_tx.clone().or_else(|| terminator.clone());

            ConnectionConfig::Serial {
                baud_rate: br,
                timeout,
                terminator: resolved_terminator,
                serial_config: SerialConfig {
                    data_bits: *data_bits,
                    parity: validated_parity,
                    stop_bits: *stop_bits,
                    flow_control: validated_flow_control,
                },
            }
        }
        RawConnectionConfig::Tcp {
            host,
            port,
            timeout_ms,
            terminator,
        } => {
            let timeout = match Timeout::new(*timeout_ms) {
                Ok(t) => t,
                Err(_) => {
                    errors.push(ConfigError::InvalidTimeout(*timeout_ms));
                    Timeout::new(1000).unwrap()
                }
            };
            ConnectionConfig::Tcp {
                host: host.clone(),
                port: *port,
                timeout,
                terminator: terminator.clone(),
            }
        }
        RawConnectionConfig::Udp {
            host,
            port,
            timeout_ms,
        } => {
            let timeout = match Timeout::new(*timeout_ms) {
                Ok(t) => t,
                Err(_) => {
                    errors.push(ConfigError::InvalidTimeout(*timeout_ms));
                    Timeout::new(1000).unwrap()
                }
            };
            ConnectionConfig::Udp {
                host: host.clone(),
                port: *port,
                timeout,
            }
        }
        RawConnectionConfig::Usbtmc {
            timeout_ms,
            terminator_tx,
        } => {
            let timeout = match Timeout::new(*timeout_ms) {
                Ok(t) => t,
                Err(_) => {
                    errors.push(ConfigError::InvalidTimeout(*timeout_ms));
                    Timeout::new(1000).unwrap()
                }
            };
            ConnectionConfig::Usbtmc {
                timeout,
                terminator: terminator_tx.clone(),
            }
        }
    }
}

/// Extract rich parameter metadata from the raw `[parameters]` table.
///
/// Each parameter can be either a scalar default (e.g., `position = 0.0`)
/// or a table with metadata fields:
///
/// ```toml
/// [parameters.position_deg]
/// type = "float"
/// default = 0.0
/// range = [0.0, 360.0]
/// unit = "degrees"
/// description = "Current rotation position"
/// read_only = false
/// ```
fn extract_parameter_metadata(
    raw_params: &HashMap<String, toml::Value>,
    errors: &mut Vec<ConfigError>,
) -> Vec<ManifestParameterMeta> {
    let mut metas = Vec::new();

    for (name, value) in raw_params {
        if let Some(table) = value.as_table() {
            let dtype = table
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("float")
                .to_string();
            // Only extract numeric defaults (f64) — used for formula evaluation context.
            // String/bool defaults are preserved in the raw TOML for driver runtime use
            // but are not stored in ManifestParameterMeta.default_value.
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: TOML integers are i64; precision loss acceptable for parameter metadata
            let default_value = table
                .get("default")
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)));
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: TOML integers are i64; precision loss acceptable for parameter range metadata
            let (min_value, max_value) = table
                .get("range")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    let min = arr
                        .first()
                        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)));
                    let max = arr
                        .get(1)
                        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)));
                    (min, max)
                })
                .unwrap_or((None, None));
            let unit = table.get("unit").and_then(|v| v.as_str()).map(String::from);
            let description = table
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            let read_only = table
                .get("read_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // `[parameters.X.ui]` (bd-jcb4x G1) — inline UI declaration.
            let ui_hint = table
                .get("ui")
                .and_then(|v| v.as_table())
                .map(|ui_tbl| parse_parameter_ui(name, ui_tbl, errors));

            metas.push(ManifestParameterMeta {
                name: name.clone(),
                dtype,
                default_value,
                min_value,
                max_value,
                unit,
                description,
                read_only,
                ui_hint,
            });
        } else {
            // Bare scalar — infer type from TOML value
            let (dtype, default_value) = if let Some(f) = value.as_float() {
                ("float".to_string(), Some(f))
            } else if let Some(i) = value.as_integer() {
                #[allow(clippy::cast_precision_loss)]
                // SAFETY: TOML integers are i64; precision loss acceptable for parameter metadata
                ("int".to_string(), Some(i as f64))
            } else {
                continue; // Non-numeric bare values are skipped
            };
            metas.push(ManifestParameterMeta {
                name: name.clone(),
                dtype,
                default_value,
                min_value: None,
                max_value: None,
                unit: None,
                description: None,
                read_only: false,
                ui_hint: None,
            });
        }
    }

    metas.sort_by(|a, b| a.name.cmp(&b.name));
    metas
}

/// Convert a `[parameters.<name>.ui]` TOML sub-table into a validated
/// [`ParameterUiHint`] (bd-jcb4x G1).
///
/// Unknown `widget_hint` strings produce a `ConfigError` rather than silently
/// falling back, so typos surface immediately at manifest load time.
fn parse_parameter_ui(
    param_name: &str,
    ui_tbl: &toml::value::Table,
    errors: &mut Vec<ConfigError>,
) -> ParameterUiHint {
    let label = ui_tbl
        .get("label")
        .and_then(|v| v.as_str())
        .map(String::from);

    let widget_hint = ui_tbl
        .get("widget_hint")
        .and_then(|v| v.as_str())
        .and_then(|s| match WidgetHint::parse(s) {
            Some(h) => Some(h),
            None => {
                errors.push(ConfigError::Other(format!(
                    "parameter '{param_name}': unknown widget_hint '{s}'. \
                     Valid values: {}",
                    WidgetHint::ALL_LABELS.join(", ")
                )));
                None
            }
        });

    let group = ui_tbl
        .get("group")
        .and_then(|v| v.as_str())
        .map(String::from);

    #[allow(clippy::cast_precision_loss)]
    // SAFETY: TOML integers are i64; precision loss acceptable for UI step metadata
    let step = ui_tbl
        .get("step")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)));

    let enum_values = ui_tbl
        .get("enum_values")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    ParameterUiHint {
        label,
        widget_hint,
        group,
        step,
        enum_values,
    }
}

fn validate_commands(
    raw: &RawManifest,
    errors: &mut Vec<ConfigError>,
) -> HashMap<String, CommandConfig> {
    let mut commands = HashMap::new();

    for (name, cmd) in &raw.commands {
        let tmpl = match template::validate_template(name, &cmd.template) {
            Ok(t) => t,
            Err(e) => {
                errors.push(e);
                // Use a placeholder to continue validation
                ValidatedTemplate {
                    source: cmd.template.clone(),
                }
            }
        };

        let response_ref = cmd.response.as_ref().map(|r| ResponseRef(r.clone()));
        let response_type = cmd
            .response_type
            .as_ref()
            .and_then(|rt| ScpiResponseType::parse(rt));

        // Inline UI declaration (bd-jcb4x G1). Convert raw → validated and
        // report a ConfigError for unknown widget_hint strings rather than
        // silently dropping them.
        let ui_hint = cmd.ui.as_ref().map(|raw_ui| {
            let widget_hint =
                raw_ui
                    .widget_hint
                    .as_deref()
                    .and_then(|s| match WidgetHint::parse(s) {
                        Some(h) => Some(h),
                        None => {
                            errors.push(ConfigError::Other(format!(
                                "command '{name}': unknown widget_hint '{s}'. \
                             Valid values: {}",
                                WidgetHint::ALL_LABELS.join(", ")
                            )));
                            None
                        }
                    });
            CommandUiHint {
                label: raw_ui.label.clone(),
                widget_hint,
                group: raw_ui.group.clone(),
                description: raw_ui.description.clone(),
            }
        });

        commands.insert(
            name.clone(),
            CommandConfig {
                template: tmpl,
                parameters: cmd.parameters.clone(),
                response: response_ref,
                response_type,
                expects_response: cmd.expects_response,
                description: cmd.description.clone(),
                ui_hint,
            },
        );
    }

    commands
}

fn validate_responses(
    raw: &RawManifest,
    errors: &mut Vec<ConfigError>,
) -> HashMap<String, ResponseParser> {
    let mut responses = HashMap::new();

    for (name, resp) in &raw.responses {
        let has_format = resp.format.is_some();
        let has_variants = resp.variants.is_some();
        let has_transform = resp.transform.is_some();
        let has_top_regex = resp.regex.is_some() || resp.pattern.is_some();
        let has_advanced_regex = resp
            .advanced
            .as_ref()
            .and_then(|a| a.regex.as_ref())
            .is_some();
        let has_regex = has_top_regex || has_advanced_regex;

        // `format` and `variants` are mutually exclusive (pick one)
        if has_format && has_variants {
            errors.push(ConfigError::Other(format!(
                "response '{name}' sets both `format` and `variants` — pick one. \
                 `variants = [single_format]` is equivalent to `format = single_format`."
            )));
            continue;
        }

        // At most one parsing tier overall (format/variants vs transform vs regex)
        let tier_count = [has_format || has_variants, has_transform, has_regex]
            .iter()
            .filter(|b| **b)
            .count();
        if tier_count > 1 {
            errors.push(ConfigError::Other(format!(
                "response '{name}' must define exactly one of: \
                 format/variants, transform, or regex"
            )));
            continue;
        }

        // Deprecation warning: top-level `regex`/`pattern` without `[advanced]`
        if has_top_regex && !has_advanced_regex {
            tracing::warn!(
                response = %name,
                "response '{name}': top-level `regex`/`pattern` is deprecated; \
                 prefer `variants = [...]` for multi-shape responses, or move to \
                 `[responses.{name}.advanced]` with `regex = \"...\"` to acknowledge \
                 the escape hatch and silence this warning"
            );
        }

        // Branch on which tier
        if has_variants || has_format {
            let sources: Vec<String> = if let Some(v) = &resp.variants {
                v.clone()
            } else {
                vec![
                    resp.format
                        .clone()
                        .expect("has_format implies format is Some"),
                ]
            };

            if sources.is_empty() {
                errors.push(ConfigError::Other(format!(
                    "response '{name}': `variants` must list at least one format string"
                )));
                continue;
            }

            let mut variants: Vec<FormatVariant> = Vec::with_capacity(sources.len());
            let mut any_err = false;
            for src in &sources {
                match format_parser::parse_format(src) {
                    Ok(segments) => variants.push(FormatVariant {
                        source: src.clone(),
                        segments,
                    }),
                    Err(e) => {
                        errors.push(e);
                        any_err = true;
                    }
                }
            }
            if !any_err {
                responses.insert(
                    name.clone(),
                    ResponseParser::Format(ValidatedFormat { variants }),
                );
            }
        } else if let Some(ref transform_list) = resp.transform {
            match TransformPipeline::from_shorthand(transform_list) {
                Ok(pipeline) => {
                    responses.insert(name.clone(), ResponseParser::Transform(pipeline));
                }
                Err(e) => {
                    errors.push(ConfigError::Other(format!(
                        "response '{name}': invalid transform pipeline: {e}"
                    )));
                }
            }
        } else if has_regex {
            // Priority: advanced.regex > top-level regex > v1 pattern
            let regex_str: String = resp
                .advanced
                .as_ref()
                .and_then(|a| a.regex.clone())
                .or_else(|| resp.regex.clone())
                .or_else(|| resp.pattern.clone())
                .expect("has_regex implies at least one regex source is present");

            const MAX_REGEX_LENGTH: usize = 1024;
            if regex_str.len() > MAX_REGEX_LENGTH {
                errors.push(ConfigError::Other(format!(
                    "response '{name}': regex pattern too long ({} > {MAX_REGEX_LENGTH} chars)",
                    regex_str.len()
                )));
                continue;
            }
            match regex::RegexBuilder::new(&regex_str)
                .size_limit(1 << 20)
                .build()
            {
                Ok(regex) => {
                    responses.insert(
                        name.clone(),
                        ResponseParser::Regex(ValidatedRegex {
                            source: regex_str,
                            regex,
                        }),
                    );
                }
                Err(e) => {
                    errors.push(ConfigError::InvalidRegex {
                        pattern: regex_str,
                        source: e,
                    });
                }
            }
        } else {
            errors.push(ConfigError::MissingField(format!(
                "response '{name}' must have one of: format, variants, transform, or regex"
            )));
        }
    }

    responses
}

fn validate_conversions(
    raw: &RawManifest,
    errors: &mut Vec<ConfigError>,
) -> HashMap<String, ValidatedFormula> {
    let mut conversions = HashMap::new();

    for (name, conv) in &raw.conversions {
        const MAX_FORMULA_LENGTH: usize = 512;
        if conv.formula.len() > MAX_FORMULA_LENGTH {
            errors.push(ConfigError::Other(format!(
                "conversion '{name}': formula too long ({} > {MAX_FORMULA_LENGTH} chars)",
                conv.formula.len()
            )));
            continue;
        }
        // Validate the evalexpr formula by attempting to build its AST.
        // We use `build_operator_tree` which parses but doesn't evaluate.
        match evalexpr::build_operator_tree(&conv.formula) {
            Ok(_) => {
                conversions.insert(
                    name.clone(),
                    ValidatedFormula {
                        source: conv.formula.clone(),
                    },
                );
            }
            Err(e) => {
                errors.push(ConfigError::InvalidFormula {
                    formula: conv.formula.clone(),
                    reason: e.to_string(),
                });
            }
        }
    }

    conversions
}

fn validate_capabilities(raw: &RawManifest, errors: &mut Vec<ConfigError>) -> CapabilitySet {
    let mut command_names: std::collections::HashSet<&str> =
        raw.commands.keys().map(String::as_str).collect();
    // Capability mappings may reference entries in `[binary_commands.*]` too
    // (e.g. Modbus devices where reads/writes are binary frames, not text).
    // Merge those keys into the lookup set so those references validate.
    if let Some(toml::Value::Table(bin_cmds)) = raw.binary_commands.as_ref() {
        command_names.extend(bin_cmds.keys().map(String::as_str));
    }
    let conversion_names: std::collections::HashSet<&str> =
        raw.conversions.keys().map(String::as_str).collect();

    let movable = raw
        .capabilities
        .movable
        .as_ref()
        .map(|m| validate_movable(m, &command_names, &conversion_names, errors));

    let readable = raw.capabilities.readable.as_ref().map(|r| {
        let read = r.read.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "readable.read",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        ReadableConfig { read }
    });

    let settable = raw.capabilities.settable.as_ref().map(|s| {
        let set = s.set.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "settable.set",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        SettableConfig { set }
    });

    let shutter_control = raw.capabilities.shutter_control.as_ref().map(|sc| {
        let open = sc.open.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "shutter_control.open",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        let close = sc.close.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "shutter_control.close",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        let is_open = sc.is_open.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "shutter_control.is_open",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        ShutterControlConfig {
            open,
            close,
            is_open,
        }
    });

    let wavelength_tunable = raw.capabilities.wavelength_tunable.as_ref().map(|wt| {
        let set_wavelength = wt.set_wavelength.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "wavelength_tunable.set_wavelength",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        let get_wavelength = wt.get_wavelength.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "wavelength_tunable.get_wavelength",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        WavelengthTunableConfig {
            set_wavelength,
            get_wavelength,
        }
    });

    let emission_control = raw.capabilities.emission_control.as_ref().map(|ec| {
        let enable = ec.enable.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "emission_control.enable",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        let disable = ec.disable.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "emission_control.disable",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        let is_enabled = ec.is_enabled.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "emission_control.is_enabled",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        EmissionControlConfig {
            enable,
            disable,
            is_enabled,
        }
    });

    let spectrum_readable = raw.capabilities.spectrum_readable.as_ref().map(|sr| {
        let read_spectrum = sr.read_spectrum.as_ref().map(|mm| {
            validate_method_mapping(
                mm,
                "spectrum_readable.read_spectrum",
                &command_names,
                &conversion_names,
                errors,
            )
        });
        SpectrumReadableConfig {
            read_spectrum,
            spectrum_length: sr.spectrum_length.unwrap_or(1024),
            value_units: sr
                .value_units
                .clone()
                .unwrap_or_else(|| "counts".to_string()),
            axis_units: sr.axis_units.clone(),
        }
    });

    CapabilitySet {
        movable,
        readable,
        settable,
        shutter_control,
        wavelength_tunable,
        emission_control,
        spectrum_readable,
    }
}

fn validate_movable(
    m: &RawMovableMapping,
    command_names: &std::collections::HashSet<&str>,
    conversion_names: &std::collections::HashSet<&str>,
    errors: &mut Vec<ConfigError>,
) -> MovableConfig {
    let move_abs = m.move_abs.as_ref().map(|mm| {
        validate_method_mapping(
            mm,
            "movable.move_abs",
            command_names,
            conversion_names,
            errors,
        )
    });
    let move_rel = m.move_rel.as_ref().map(|mm| {
        validate_method_mapping(
            mm,
            "movable.move_rel",
            command_names,
            conversion_names,
            errors,
        )
    });
    let position = m.position.as_ref().map(|mm| {
        validate_method_mapping(
            mm,
            "movable.position",
            command_names,
            conversion_names,
            errors,
        )
    });
    let stop = m.stop.as_ref().map(|mm| {
        validate_method_mapping(mm, "movable.stop", command_names, conversion_names, errors)
    });
    let wait_settled = m
        .wait_settled
        .as_ref()
        .map(|ws| validate_wait_settled(ws, command_names, errors));

    MovableConfig {
        move_abs,
        move_rel,
        position,
        stop,
        wait_settled,
    }
}

fn validate_method_mapping(
    mm: &RawMethodMapping,
    context: &str,
    command_names: &std::collections::HashSet<&str>,
    conversion_names: &std::collections::HashSet<&str>,
    errors: &mut Vec<ConfigError>,
) -> MethodConfig {
    // Validate command reference
    if !command_names.contains(mm.command.as_str()) {
        errors.push(ConfigError::MissingCapabilityMethod {
            capability: context.to_string(),
            ref_type: "command".to_string(),
            ref_name: mm.command.clone(),
        });
    }

    // Validate input conversion reference
    let input_conversion = mm.input_conversion.as_ref().map(|ic| {
        if !conversion_names.contains(ic.as_str()) {
            errors.push(ConfigError::MissingCapabilityMethod {
                capability: context.to_string(),
                ref_type: "conversion".to_string(),
                ref_name: ic.clone(),
            });
        }
        ConversionRef(ic.clone())
    });

    // Validate output conversion reference
    let output_conversion = mm.output_conversion.as_ref().map(|oc| {
        if !conversion_names.contains(oc.as_str()) {
            errors.push(ConfigError::MissingCapabilityMethod {
                capability: context.to_string(),
                ref_type: "conversion".to_string(),
                ref_name: oc.clone(),
            });
        }
        ConversionRef(oc.clone())
    });

    MethodConfig {
        command: CommandRef(mm.command.clone()),
        input_conversion,
        input_param: mm.input_param.clone(),
        from_param: mm.from_param.clone(),
        output_conversion,
        output_field: mm.output_field.clone(),
    }
}

fn validate_wait_settled(
    ws: &RawWaitSettledMapping,
    command_names: &std::collections::HashSet<&str>,
    errors: &mut Vec<ConfigError>,
) -> WaitSettledConfig {
    if !command_names.contains(ws.poll_command.as_str()) {
        errors.push(ConfigError::MissingCapabilityMethod {
            capability: "movable.wait_settled".to_string(),
            ref_type: "command".to_string(),
            ref_name: ws.poll_command.clone(),
        });
    }

    WaitSettledConfig {
        poll_command: CommandRef(ws.poll_command.clone()),
        success_condition: ws.success_condition.clone(),
        poll_interval_ms: ws.poll_interval_ms,
        timeout_ms: ws.timeout_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"

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

[capabilities.readable]
read = { command = "measure_voltage" }

[capabilities.settable]
set = { command = "set_voltage", from_param = "value" }
"#
    }

    #[test]
    fn parse_valid_ell14_config() {
        let raw: RawManifest = toml::from_str(ell14_toml()).unwrap();
        let manifest = parse_manifest(raw).expect("should parse valid ELL14 config");

        assert_eq!(manifest.device.name, "Thorlabs ELL14");
        assert_eq!(manifest.commands.len(), 4);
        assert_eq!(manifest.responses.len(), 2);
        assert_eq!(manifest.conversions.len(), 2);
        assert!(manifest.capabilities.movable.is_some());

        let movable = manifest.capabilities.movable.as_ref().unwrap();
        assert!(movable.move_abs.is_some());
        assert!(movable.position.is_some());
        assert!(movable.stop.is_some());
        assert!(movable.wait_settled.is_some());
    }

    #[test]
    fn parse_valid_scpi_tcp_config() {
        let raw: RawManifest = toml::from_str(scpi_tcp_toml()).unwrap();
        let manifest = parse_manifest(raw).expect("should parse valid SCPI TCP config");

        assert_eq!(manifest.device.name, "Keithley 2400");
        assert!(manifest.capabilities.readable.is_some());
        assert!(manifest.capabilities.settable.is_some());

        // Check that the measure_voltage command has a response_type
        let measure = manifest.commands.get("measure_voltage").unwrap();
        assert_eq!(measure.response_type, Some(ScpiResponseType::Float));
    }

    #[test]
    fn parse_preserves_ui_section() {
        let toml_str = r##"
schema_version = 3

[device]
name = "UI Device"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }

[ui]
icon = "meter"
color = "#00AA88"

[ui.control_panel]
layout = "vertical"
"##;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("manifest with [ui] should parse");

        let ui = manifest.ui.as_ref().expect("[ui] should be preserved");
        let icon = ui
            .get("icon")
            .and_then(|value| value.as_str())
            .expect("ui.icon should exist");
        let layout = ui
            .get("control_panel")
            .and_then(|value| value.get("layout"))
            .and_then(|value| value.as_str())
            .expect("ui.control_panel.layout should exist");

        assert_eq!(icon, "meter");
        assert_eq!(layout, "vertical");
    }

    // ---- bd-jcb4x G1: inline [commands.X.ui] / [parameters.X.ui] ----

    /// Helper: build a minimal valid manifest that exercises both inline
    /// UI tables so tests can drop in additional fields easily.
    fn g1_manifest_with_ui() -> &'static str {
        r#"
schema_version = 3

[device]
name = "G1 Test Device"
capabilities = ["Readable", "Settable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read_power]
template = "READ?"
response_type = "float"
description = "Read the current power."

[commands.read_power.ui]
label = "Read Power"
widget_hint = "button"
group = "status"
description = "Query current power from device"

[commands.set_power]
template = "POWER {{ value }}"
parameters = { value = "float" }

[commands.set_power.ui]
widget_hint = "slider"
group = "control"

[capabilities.readable]
read = { command = "read_power" }

[capabilities.settable]
set = { command = "set_power", from_param = "value" }

[parameters.power_mw]
type = "float"
default = 50.0
range = [0.0, 200.0]
unit = "mW"

[parameters.power_mw.ui]
label = "Output Power"
widget_hint = "slider"
group = "control"
step = 0.5

[parameters.mode]
type = "string"
description = "Operating mode"

[parameters.mode.ui]
widget_hint = "enum_select"
enum_values = ["idle", "pulsed", "cw"]
group = "control"
"#
    }

    #[test]
    fn g1_parse_command_ui_hint_basic() {
        let raw: RawManifest = toml::from_str(g1_manifest_with_ui()).unwrap();
        let manifest = parse_manifest(raw).expect("manifest with inline UI should parse");

        let cmd = manifest
            .commands
            .get("read_power")
            .expect("read_power command");
        let ui_hint = cmd
            .ui_hint
            .as_ref()
            .expect("read_power should have ui_hint");
        assert_eq!(ui_hint.label.as_deref(), Some("Read Power"));
        assert_eq!(ui_hint.widget_hint, Some(WidgetHint::Button));
        assert_eq!(ui_hint.group.as_deref(), Some("status"));
        assert_eq!(
            ui_hint.description.as_deref(),
            Some("Query current power from device")
        );
    }

    #[test]
    fn g1_parse_command_ui_hint_partial() {
        // set_power has only widget_hint + group, no label/description — must still parse.
        let raw: RawManifest = toml::from_str(g1_manifest_with_ui()).unwrap();
        let manifest = parse_manifest(raw).expect("partial ui_hint should parse");

        let cmd = manifest
            .commands
            .get("set_power")
            .expect("set_power command");
        let ui_hint = cmd.ui_hint.as_ref().expect("set_power should have ui_hint");
        assert!(ui_hint.label.is_none());
        assert_eq!(ui_hint.widget_hint, Some(WidgetHint::Slider));
        assert_eq!(ui_hint.group.as_deref(), Some("control"));
        assert!(ui_hint.description.is_none());
    }

    #[test]
    fn g1_parse_parameter_ui_hint_numeric() {
        let raw: RawManifest = toml::from_str(g1_manifest_with_ui()).unwrap();
        let manifest = parse_manifest(raw).expect("manifest with parameter.ui should parse");

        let meta = manifest
            .parameter_metadata
            .iter()
            .find(|m| m.name == "power_mw")
            .expect("power_mw metadata");
        let ui_hint = meta.ui_hint.as_ref().expect("power_mw should have ui_hint");
        assert_eq!(ui_hint.label.as_deref(), Some("Output Power"));
        assert_eq!(ui_hint.widget_hint, Some(WidgetHint::Slider));
        assert_eq!(ui_hint.group.as_deref(), Some("control"));
        assert_eq!(ui_hint.step, Some(0.5));
        assert!(ui_hint.enum_values.is_empty());
    }

    #[test]
    fn g1_parse_parameter_ui_hint_enum() {
        let raw: RawManifest = toml::from_str(g1_manifest_with_ui()).unwrap();
        let manifest = parse_manifest(raw).expect("manifest with enum ui should parse");

        let meta = manifest
            .parameter_metadata
            .iter()
            .find(|m| m.name == "mode")
            .expect("mode metadata");
        let ui_hint = meta.ui_hint.as_ref().expect("mode should have ui_hint");
        assert_eq!(ui_hint.widget_hint, Some(WidgetHint::EnumSelect));
        assert_eq!(
            ui_hint.enum_values,
            vec!["idle".to_string(), "pulsed".to_string(), "cw".to_string()]
        );
    }

    #[test]
    fn g1_missing_ui_sections_produce_none() {
        // Baseline ell14 manifest has no [commands.*.ui] / [parameters.*.ui]
        // tables; ui_hint fields must be None across the board.
        let raw: RawManifest = toml::from_str(ell14_toml()).unwrap();
        let manifest = parse_manifest(raw).expect("ell14 should parse");

        for (name, cmd) in &manifest.commands {
            assert!(
                cmd.ui_hint.is_none(),
                "command {name} should have no ui_hint in legacy manifest"
            );
        }
        for meta in &manifest.parameter_metadata {
            assert!(
                meta.ui_hint.is_none(),
                "parameter {} should have no ui_hint in legacy manifest",
                meta.name
            );
        }
    }

    #[test]
    fn g1_unknown_command_widget_hint_is_error() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Bad Hint"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[commands.read.ui]
widget_hint = "holographic_dial"

[capabilities.readable]
read = { command = "read" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let err = parse_manifest(raw).expect_err("unknown widget_hint must error");
        let combined = err
            .iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("holographic_dial"),
            "error message should mention the offending value: {combined}"
        );
        assert!(
            combined.contains("widget_hint"),
            "error message should name the field: {combined}"
        );
    }

    #[test]
    fn g1_unknown_parameter_widget_hint_is_error() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Bad Param Hint"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }

[parameters.spin]
type = "float"
default = 0.0

[parameters.spin.ui]
widget_hint = "antigravity"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let err = parse_manifest(raw).expect_err("unknown widget_hint must error");
        let combined = err
            .iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("antigravity"),
            "error message should mention the offending value: {combined}"
        );
        assert!(
            combined.contains("spin"),
            "error message should name the parameter: {combined}"
        );
    }

    #[test]
    fn g1_widget_hint_aliases_accepted() {
        // "enum", "select", "dropdown" should all map to EnumSelect;
        // "checkbox" → Toggle; "numeric", "number" → NumericInput.
        let toml_str = r#"
schema_version = 3

[device]
name = "Aliases"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[commands.read.ui]
widget_hint = "CHECKBOX"

[capabilities.readable]
read = { command = "read" }

[parameters.pick]
type = "string"

[parameters.pick.ui]
widget_hint = "dropdown"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("aliases should resolve");
        assert_eq!(
            manifest.commands["read"]
                .ui_hint
                .as_ref()
                .unwrap()
                .widget_hint,
            Some(WidgetHint::Toggle),
            "CHECKBOX (case-insensitive) must map to Toggle"
        );
        let pick = manifest
            .parameter_metadata
            .iter()
            .find(|m| m.name == "pick")
            .unwrap();
        assert_eq!(
            pick.ui_hint.as_ref().unwrap().widget_hint,
            Some(WidgetHint::EnumSelect),
            "'dropdown' must map to EnumSelect"
        );
    }

    #[test]
    fn reject_wrong_schema_version() {
        let toml_str = r#"
schema_version = 1

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ConfigError::UnsupportedSchemaVersion {
                found: 1,
                expected: 3
            }
        )));
    }

    #[test]
    fn reject_unknown_command_ref() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.real_command]
template = "RC"

[capabilities.readable]
read = { command = "nonexistent_command" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ConfigError::MissingCapabilityMethod { ref_name, .. }
            if ref_name == "nonexistent_command"
        )));
    }

    #[test]
    fn reject_unknown_conversion_ref() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = ["Movable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.move_cmd]
template = "M{{ pos }}"

[capabilities.movable]
move_abs = { command = "move_cmd", input_conversion = "nonexistent_conv", input_param = "pos" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ConfigError::MissingCapabilityMethod { ref_name, .. }
            if ref_name == "nonexistent_conv"
        )));
    }

    #[test]
    fn reject_invalid_regex() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.cmd]
template = "CMD"
response = "resp"

[responses.resp]
regex = "[invalid("
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ConfigError::InvalidRegex { .. }))
        );
    }

    #[test]
    fn reject_invalid_template() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.cmd]
template = "{{ unclosed"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ConfigError::InvalidTemplate { .. }))
        );
    }

    #[test]
    fn reject_invalid_formula() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[conversions.bad]
formula = "((( unclosed"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ConfigError::InvalidFormula { .. }))
        );
    }

    #[test]
    fn reject_invalid_baud_rate() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 0
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ConfigError::InvalidBaudRate(0)))
        );
    }

    #[test]
    fn reject_invalid_timeout() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
timeout_ms = 0
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, ConfigError::InvalidTimeout(0)))
        );
    }

    #[test]
    fn reject_unknown_response_ref_in_command() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.cmd]
template = "CMD"
response = "nonexistent_response"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ConfigError::UnknownResponse { name, .. }
            if name == "nonexistent_response"
        )));
    }

    #[test]
    fn collects_multiple_errors() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 0
timeout_ms = 0

[commands.cmd]
template = "{{ unclosed"

[conversions.bad]
formula = "((("
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        // Should have at least: baud rate, timeout, template, formula
        assert!(
            errs.len() >= 4,
            "expected at least 4 errors, got {}: {:?}",
            errs.len(),
            errs
        );
    }

    #[test]
    fn accept_transform_response() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read_temp]
template = "RT?"
response = "temp"

[responses.temp]
transform = ["trim", "remove_suffix('C')", "to_float"]

[capabilities.readable]
read = { command = "read_temp" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse transform response");
        assert!(matches!(
            manifest.responses.get("temp"),
            Some(ResponseParser::Transform(_))
        ));
    }

    #[test]
    fn accept_regex_response() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = ["Readable"]

[connection]
type = "tcp"
host = "192.168.1.1"
port = 5000

[commands.read_val]
template = "READ?"
response = "val"

[responses.val]
regex = "(?P<value>\\d+\\.\\d+)\\s+(?P<unit>\\w+)"

[capabilities.readable]
read = { command = "read_val" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse regex response");
        assert!(matches!(
            manifest.responses.get("val"),
            Some(ResponseParser::Regex(_))
        ));
    }

    #[test]
    fn accept_variants_response_single() {
        // `variants = [one]` is equivalent to `format = one` — single-variant path.
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read_pos]
template = "{{ addr }}PO?"
response = "pos"

[responses.pos]
variants = ["{addr:1}PO{pulses:hex8}"]

[capabilities.readable]
read = { command = "read_pos" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse single-variant response");
        let Some(ResponseParser::Format(fmt)) = manifest.responses.get("pos") else {
            panic!("expected Format parser");
        };
        assert_eq!(fmt.variants.len(), 1);
        assert_eq!(fmt.first_source(), "{addr:1}PO{pulses:hex8}");
    }

    #[test]
    fn accept_variants_response_multi() {
        // F1 acceptance criterion: ELL14-style 30/33-byte device_info via variants.
        // Variants differ only in the travel-field width (5 vs 8 hex chars).
        let toml_str = r#"
schema_version = 3

[device]
name = "Thorlabs ELL14"
capabilities = ["Movable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.info]
template = "{{ addr }}IN"
response = "device_info"

[responses.device_info]
variants = [
  "{addr:1}IN{dev_type:hex2}{serial:8}{year:4}{firmware:2}{hw:1}{travel:5}{pulses:hex8}",
  "{addr:1}IN{dev_type:hex2}{serial:8}{year:4}{firmware:2}{hw:1}{travel:8}{pulses:hex8}",
]
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse multi-variant response");
        let Some(ResponseParser::Format(fmt)) = manifest.responses.get("device_info") else {
            panic!("expected Format parser");
        };
        assert_eq!(fmt.variants.len(), 2);
        // Variant order is preserved (declaration order = try order)
        assert!(fmt.variants[0].source.contains("{travel:5}"));
        assert!(fmt.variants[1].source.contains("{travel:8}"));

        // End-to-end: 33-char short form parses via variant 0 (travel:5).
        // Widths: 1 + 2(IN) + 2 + 8 + 4 + 2 + 1 + 5 + 8 = 33
        //         addr  IN  dev serial year fw hw travel pulses
        let short = "2IN0E1140051720231701016800023000";
        assert_eq!(short.len(), 33);
        let parsed_short = crate::response::parse_with_format(short, fmt)
            .expect("33-char short form should match first variant");
        assert_eq!(parsed_short["travel"].as_str(), Some("10168"));
        assert_eq!(parsed_short["pulses"].as_i64(), Some(0x0002_3000));

        // 36-char long form parses via variant 1 (travel:8).
        // Widths: 1 + 2 + 2 + 8 + 4 + 2 + 1 + 8 + 8 = 36
        // Variant 0 rejects (trailing input after consuming 33 chars),
        // variant 1 succeeds — demonstrating first-match-wins with fallback.
        let long = "2IN0E1140051720231701234567800023000";
        assert_eq!(long.len(), 36);
        let parsed_long = crate::response::parse_with_format(long, fmt)
            .expect("36-char long form should match second variant");
        assert_eq!(parsed_long["travel"].as_str(), Some("12345678"));
        assert_eq!(parsed_long["pulses"].as_i64(), Some(0x0002_3000));
    }

    #[test]
    fn accept_advanced_regex_response() {
        // Authors who genuinely need regex can put it under [advanced] to
        // acknowledge the escape hatch. Validator accepts it without warning.
        let toml_str = r#"
schema_version = 3

[device]
name = "Regex Device"
capabilities = ["Readable"]

[connection]
type = "tcp"
host = "10.0.0.1"
port = 5000

[commands.read_val]
template = "READ?"
response = "val"

[responses.val.advanced]
regex = "(?P<value>\\d+\\.\\d+)\\s+(?P<unit>\\w+)"

[capabilities.readable]
read = { command = "read_val" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should accept [advanced] regex");
        assert!(matches!(
            manifest.responses.get("val"),
            Some(ResponseParser::Regex(_))
        ));
    }

    #[test]
    fn rejects_format_and_variants_together() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.bad]
format = "{x:1}"
variants = ["{x:1}"]
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.to_string().contains("both `format` and `variants`")),
            "expected mutual-exclusion error for format+variants, got: {errs:?}"
        );
    }

    #[test]
    fn rejects_variants_and_transform_together() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.bad]
variants = ["{v:2}"]
transform = ["trim", "to_float"]
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter().any(|e| e.to_string().contains("exactly one")),
            "expected cross-tier exclusion error, got: {errs:?}"
        );
    }

    #[test]
    fn rejects_empty_variants_list() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.bad]
variants = []
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.iter().any(|e| e.to_string().contains("at least one")),
            "expected empty-variants error, got: {errs:?}"
        );
    }

    #[test]
    fn advanced_regex_wins_over_top_level_regex() {
        // When both are present (transitional manifests), the canonical
        // [advanced] location wins and no warning is emitted.
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.val]
regex = "SHOULD_NOT_WIN"

[responses.val.advanced]
regex = "(?P<v>\\d+)"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("advanced regex should win");
        let Some(ResponseParser::Regex(r)) = manifest.responses.get("val") else {
            panic!("expected Regex parser");
        };
        assert_eq!(r.source, "(?P<v>\\d+)");
    }

    #[test]
    fn accept_udp_connection() {
        let toml_str = r#"
schema_version = 3

[device]
name = "UDP Device"
capabilities = []

[connection]
type = "udp"
host = "10.0.0.1"
port = 8080
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse UDP config");
        assert!(matches!(manifest.connection, ConnectionConfig::Udp { .. }));
    }

    #[test]
    fn accept_usbtmc_connection() {
        let toml_str = r#"
schema_version = 3

[device]
name = "USB TMC Device"
capabilities = ["Readable"]

[connection]
type = "usbtmc"
timeout_ms = 5000
terminator_tx = "\n"

[commands.read]
template = "MEASure?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse USB TMC config");
        match &manifest.connection {
            ConnectionConfig::Usbtmc {
                timeout,
                terminator,
            } => {
                assert_eq!(timeout.value(), 5000);
                assert_eq!(terminator.as_deref(), Some("\n"));
            }
            _ => panic!("expected USB TMC connection"),
        }
    }

    #[test]
    fn rejects_regex_too_long() {
        let long_pattern = "a".repeat(2000);
        let toml_str = format!(
            r#"
schema_version = 3

[device]
name = "Test"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"

[responses.data]
regex = "{long_pattern}"

[capabilities.readable]
read = {{ command = "read" }}
"#
        );
        let raw: RawManifest = toml::from_str(&toml_str).unwrap();
        let errors = parse_manifest(raw).unwrap_err();
        let msg = errors.iter().map(|e| e.to_string()).collect::<String>();
        assert!(msg.contains("too long"), "expected 'too long' in: {msg}");
    }

    #[test]
    fn rejects_formula_too_long() {
        // Build a formula exceeding 512 chars
        let long_formula = format!("x + {}", "1 + ".repeat(200));
        let toml_str = format!(
            r#"
schema_version = 3

[device]
name = "Test"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.read]
template = "READ?"
response_type = "float"

[conversions.too_long]
formula = "{long_formula}"

[capabilities.readable]
read = {{ command = "read" }}
"#
        );
        let raw: RawManifest = toml::from_str(&toml_str).unwrap();
        let errors = parse_manifest(raw).unwrap_err();
        let msg = errors.iter().map(|e| e.to_string()).collect::<String>();
        assert!(msg.contains("too long"), "expected 'too long' in: {msg}");
    }

    #[test]
    fn serial_config_fields_preserved() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
data_bits = 7
parity = "even"
stop_bits = 2
flow_control = "hardware"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse serial config");
        match &manifest.connection {
            ConnectionConfig::Serial { serial_config, .. } => {
                assert_eq!(serial_config.data_bits, Some(7));
                assert_eq!(serial_config.parity, Some(SerialParity::Even));
                assert_eq!(serial_config.stop_bits, Some(2));
                assert_eq!(
                    serial_config.flow_control,
                    Some(SerialFlowControl::Hardware)
                );
            }
            _ => panic!("expected serial connection"),
        }
    }

    #[test]
    fn serial_config_defaults_to_none() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse minimal serial config");
        match &manifest.connection {
            ConnectionConfig::Serial { serial_config, .. } => {
                assert!(serial_config.data_bits.is_none());
                assert!(serial_config.parity.is_none());
                assert!(serial_config.stop_bits.is_none());
                assert!(serial_config.flow_control.is_none());
            }
            _ => panic!("expected serial connection"),
        }
    }

    #[test]
    fn reject_invalid_serial_config() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
data_bits = 4
parity = "invalid"
stop_bits = 3
flow_control = "xon"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        assert!(
            errs.len() >= 4,
            "expected at least 4 errors, got {}: {:?}",
            errs.len(),
            errs
        );
    }

    #[test]
    fn init_sequence_string_entries() {
        let toml_str = r#"
schema_version = 3
init_sequence = ["reset", "clear"]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.reset]
template = "*RST"
expects_response = false

[commands.clear]
template = "*CLS"
expects_response = false
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse init sequence");
        assert_eq!(manifest.init_sequence.len(), 2);
        assert_eq!(manifest.init_sequence[0].command, "*RST");
        assert_eq!(manifest.init_sequence[1].command, "*CLS");
        assert!(manifest.init_sequence[0].delay_ms.is_none());
        assert!(!manifest.init_sequence[0].expects_response);
        assert!(!manifest.init_sequence[1].expects_response);
    }

    #[test]
    fn init_sequence_table_entries() {
        let toml_str = r#"
schema_version = 3
init_sequence = [{ command = "reset", delay_ms = 500 }]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.reset]
template = "*RST"
expects_response = false
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse init sequence with delay");
        assert_eq!(manifest.init_sequence.len(), 1);
        assert_eq!(manifest.init_sequence[0].command, "*RST");
        assert_eq!(manifest.init_sequence[0].delay_ms, Some(500));
        assert!(!manifest.init_sequence[0].expects_response);
    }

    #[test]
    fn init_sequence_rejects_negative_delay() {
        let toml_str = r#"
schema_version = 3
init_sequence = [{ command = "reset", delay_ms = -100 }]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.reset]
template = "*RST"
expects_response = false
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        let msg = errs.iter().map(|e| e.to_string()).collect::<String>();
        assert!(
            msg.contains("delay_ms must be 0..=60000"),
            "expected delay_ms range error in: {msg}"
        );
    }

    #[test]
    fn init_sequence_expects_response_from_command() {
        let toml_str = r#"
schema_version = 3
init_sequence = ["identify"]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.identify]
template = "*IDN?"
response_type = "string"
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest = parse_manifest(raw).expect("should parse");
        assert_eq!(manifest.init_sequence.len(), 1);
        assert!(manifest.init_sequence[0].expects_response);
    }

    #[test]
    fn init_sequence_rejects_unknown_command() {
        let toml_str = r#"
schema_version = 3
init_sequence = ["nonexistent"]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        let msg = errs.iter().map(|e| e.to_string()).collect::<String>();
        assert!(
            msg.contains("unknown command"),
            "expected 'unknown command' in: {msg}"
        );
    }

    #[test]
    fn init_sequence_rejects_parameterized_command() {
        let toml_str = r#"
schema_version = 3
init_sequence = ["move"]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.move]
template = "MA{{ position }}"
parameters = { position = "int32" }
expects_response = false
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        let msg = errs.iter().map(|e| e.to_string()).collect::<String>();
        assert!(
            msg.contains("requires parameters"),
            "expected 'requires parameters' in: {msg}"
        );
    }

    #[test]
    fn init_sequence_rejects_parameterized_command_table_entry() {
        let toml_str = r#"
schema_version = 3
init_sequence = [{ command = "move", delay_ms = 100 }]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.move]
template = "MA{{ position }}"
parameters = { position = "int32" }
expects_response = false
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        let msg = errs.iter().map(|e| e.to_string()).collect::<String>();
        assert!(
            msg.contains("requires parameters"),
            "expected 'requires parameters' in: {msg}"
        );
    }

    #[test]
    fn init_sequence_rejects_non_integer_delay() {
        let toml_str = r#"
schema_version = 3
init_sequence = [{ command = "reset", delay_ms = "fast" }]

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[commands.reset]
template = "*RST"
expects_response = false
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        let msg = errs.iter().map(|e| e.to_string()).collect::<String>();
        assert!(
            msg.contains("must be an integer"),
            "expected 'must be an integer' in: {msg}"
        );
    }

    /// Regression: capability mappings that reference a `[binary_commands.*]`
    /// entry (e.g. Modbus devices) must validate successfully. Before the fix,
    /// `validate_capabilities` only consulted `[commands.*]` and emitted a
    /// spurious "missing command" error even when the reference was valid.
    #[test]
    fn capability_references_binary_command_validates_ok() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Modbus Slave"
capabilities = ["Readable", "Settable"]

[connection]
type = "serial"
baud_rate = 9600

[binary_commands.read_holding_registers]
function_code = 3

[binary_commands.write_single_register]
function_code = 6

[capabilities.readable]
read = { command = "read_holding_registers" }

[capabilities.settable]
set = { command = "write_single_register", from_param = "value" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let manifest =
            parse_manifest(raw).expect("binary_command references in capabilities should validate");
        assert!(manifest.capabilities.readable.is_some());
        assert!(manifest.capabilities.settable.is_some());
    }

    /// A capability that references a command NOT present in either
    /// `[commands]` or `[binary_commands]` still fails, so the fix does not
    /// silently accept genuinely-missing references.
    #[test]
    fn capability_references_nonexistent_command_still_fails() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Modbus Slave"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600

[binary_commands.read_holding_registers]
function_code = 3

[capabilities.readable]
read = { command = "does_not_exist" }
"#;
        let raw: RawManifest = toml::from_str(toml_str).unwrap();
        let errs = parse_manifest(raw).unwrap_err();
        let msg = errs.iter().map(|e| e.to_string()).collect::<String>();
        assert!(
            msg.contains("does_not_exist"),
            "expected missing-command error mentioning 'does_not_exist' in: {msg}"
        );
    }
}
