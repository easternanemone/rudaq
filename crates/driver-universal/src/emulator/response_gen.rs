//! Generate response strings from emulated device state.
//!
//! Each response type (SCPI, format string, transform pipeline, regex) has a
//! corresponding generation strategy. Transform pipelines are inverted: the
//! emulator applies each operation in reverse to reconstruct a valid raw
//! response string from the stored numeric/string value.

use crate::config::validated::ScpiResponseType;
use crate::format_parser::{FieldKind, FormatSegment};
use crate::transform::TransformOp;
use serde_json::Value;
use std::collections::HashMap;

/// Strategy for generating a response string from emulated state.
#[derive(Debug)]
pub enum ResponseGen {
    /// Tier 0: SCPI auto-parse.
    Scpi(ScpiResponseType),
    /// Tier 1: Format string segments (from `format_parser`).
    FormatString(Vec<FormatSegment>),
    /// Tier 2: Transform pipeline inversion — process ops in reverse.
    TransformInverse {
        /// Forward-order transform ops (inverted during generation).
        ops: Vec<TransformOp>,
        /// Literal prefix from the command template (for regex_extract inversion).
        cmd_prefix: String,
    },
    /// Tier 3: Regex-derived template with `{field_name}` placeholders.
    RegexTemplate(String),
}

impl ResponseGen {
    /// Generate a response string from state values.
    pub fn generate(&self, state: &HashMap<String, Value>) -> anyhow::Result<String> {
        match self {
            ResponseGen::Scpi(scpi_type) => Ok(generate_scpi(scpi_type, state)),
            ResponseGen::FormatString(segments) => Ok(generate_format_string(segments, state)),
            ResponseGen::TransformInverse { ops, cmd_prefix } => {
                Ok(generate_transform_inverse(ops, cmd_prefix, state))
            }
            ResponseGen::RegexTemplate(template) => Ok(generate_regex_template(template, state)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tier 0: SCPI auto-parse
// ---------------------------------------------------------------------------

fn generate_scpi(scpi_type: &ScpiResponseType, state: &HashMap<String, Value>) -> String {
    let val = state.get("value").cloned().unwrap_or(Value::Null);
    match scpi_type {
        ScpiResponseType::Float => format_float_value(&val),
        ScpiResponseType::Integer => {
            let i = value_as_i64(&val).unwrap_or(0);
            i.to_string()
        }
        ScpiResponseType::String => match &val {
            Value::String(s) => s.clone(),
            _ => val.to_string(),
        },
        ScpiResponseType::Boolean => {
            let b = match &val {
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
                _ => false,
            };
            if b { "1" } else { "0" }.to_string()
        }
        ScpiResponseType::ArrayFloat => {
            // Return comma-separated floats
            let f = value_as_f64(&val).unwrap_or(0.0);
            format_float(f)
        }
    }
}

// ---------------------------------------------------------------------------
// Tier 1: Format string
// ---------------------------------------------------------------------------

fn generate_format_string(segments: &[FormatSegment], state: &HashMap<String, Value>) -> String {
    let mut out = String::new();
    for seg in segments {
        match seg {
            FormatSegment::Literal(s) => out.push_str(s),
            FormatSegment::Field(spec) => {
                let val = state.get(&spec.name).cloned().unwrap_or(Value::Null);
                match &spec.kind {
                    FieldKind::Greedy => {
                        out.push_str(&value_as_string(&val));
                    }
                    FieldKind::FixedWidth(n) => {
                        let s = value_as_string(&val);
                        // Left-align, truncate or pad to n chars
                        let formatted = if s.len() >= *n {
                            s[..*n].to_string()
                        } else {
                            format!("{:<width$}", s, width = n)
                        };
                        out.push_str(&formatted);
                    }
                    FieldKind::Hex(n) => {
                        let i = value_as_i64(&val).unwrap_or(0);
                        out.push_str(&format_hex(i, *n));
                    }
                    FieldKind::Int => {
                        let i = value_as_i64(&val).unwrap_or(0);
                        out.push_str(&i.to_string());
                    }
                    FieldKind::Float => {
                        out.push_str(&format_float_value(&val));
                    }
                }
            }
        }
    }
    out
}

/// Format i64 as uppercase hex with two's complement masking and zero-padding.
fn format_hex(value: i64, width: usize) -> String {
    // SAFETY: Intentional truncation + sign reinterpretation for two's complement hex encoding.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    let masked: u64 = match width {
        2 => u64::from(value as u8),
        4 => u64::from(value as u16),
        8 => u64::from(value as u32),
        _ => value as u64,
    };
    format!("{:0>width$X}", masked, width = width)
}

// ---------------------------------------------------------------------------
// Tier 2: Transform pipeline inversion
// ---------------------------------------------------------------------------

fn generate_transform_inverse(
    ops: &[TransformOp],
    cmd_prefix: &str,
    state: &HashMap<String, Value>,
) -> String {
    let val = state.get("value").cloned().unwrap_or(Value::Null);
    let mut current = value_as_string(&val);

    // Process transforms in reverse order
    for op in ops.iter().rev() {
        current = match op {
            TransformOp::Trim => current,
            TransformOp::RemovePrefix { prefix } => format!("{prefix}{current}"),
            TransformOp::RemoveSuffix { suffix } => format!("{current}{suffix}"),
            TransformOp::ToFloat => {
                // Convert stored numeric value to string
                if let Some(f) = value_as_f64(&val) {
                    format_float(f)
                } else {
                    current
                }
            }
            TransformOp::ToInt => {
                if let Some(i) = value_as_i64(&val) {
                    i.to_string()
                } else if let Ok(f) = current.parse::<f64>() {
                    #[allow(clippy::cast_possible_truncation)]
                    // SAFETY: rounding f64 to i64 — truncation acceptable for device values
                    let truncated = f.round() as i64;
                    truncated.to_string()
                } else {
                    current
                }
            }
            TransformOp::Scale { factor } => {
                if let Ok(f) = current.parse::<f64>() {
                    format_float(f / factor)
                } else {
                    current
                }
            }
            TransformOp::Offset { value } => {
                if let Ok(f) = current.parse::<f64>() {
                    format_float(f - value)
                } else {
                    current
                }
            }
            TransformOp::RegexExtract { .. } => {
                // Reconstruct with command prefix: "CMD: value"
                format!("{cmd_prefix}: {current}")
            }
            TransformOp::Map { from, to } => {
                // Inverse: if current matches `to`, replace with `from`
                if current == *to {
                    from.clone()
                } else {
                    current
                }
            }
            TransformOp::SplitComma { .. } => current, // Can't meaningfully invert
        };
    }

    current
}

// ---------------------------------------------------------------------------
// Tier 3: Regex-derived template
// ---------------------------------------------------------------------------

fn generate_regex_template(template: &str, state: &HashMap<String, Value>) -> String {
    let mut result = template.to_string();
    for (key, val) in state {
        let placeholder = format!("{{{key}}}");
        if result.contains(&placeholder) {
            result = result.replace(&placeholder, &value_as_string(val));
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Public helper: regex pattern → response template
// ---------------------------------------------------------------------------

/// Convert a regex pattern with named groups into a simple `{field_name}` template.
///
/// Steps:
/// 1. Strip `^` and `$` anchors
/// 2. Replace `(?P<name>...)` with `{name}` (handles balanced parens)
/// 3. Replace `\\s+` / `\\s*` with a single space
/// 4. Unescape remaining regex escapes
pub fn regex_to_template(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut result = String::new();
    let mut i = 0;

    // Skip leading ^
    if i < chars.len() && chars[i] == '^' {
        i += 1;
    }

    let end = if !chars.is_empty() && chars[chars.len() - 1] == '$' {
        chars.len() - 1
    } else {
        chars.len()
    };

    while i < end {
        // Check for (?P<name>...)
        if i + 4 < end
            && chars[i] == '('
            && chars[i + 1] == '?'
            && chars[i + 2] == 'P'
            && chars[i + 3] == '<'
        {
            i += 4; // skip "(?P<"
            let name_start = i;
            while i < end && chars[i] != '>' {
                i += 1;
            }
            let name: String = chars[name_start..i].iter().collect();
            i += 1; // skip '>'

            // Skip the pattern inside parens (balanced)
            let mut depth = 1;
            while i < end && depth > 0 {
                if chars[i] == '(' {
                    depth += 1;
                }
                if chars[i] == ')' {
                    depth -= 1;
                }
                if depth > 0 {
                    i += 1;
                }
            }
            if i < end {
                i += 1; // skip closing ')'
            }

            result.push('{');
            result.push_str(&name);
            result.push('}');
        }
        // Check for \\s+ or \\s*
        else if i + 1 < end && chars[i] == '\\' && chars[i + 1] == 's' {
            i += 2;
            // Skip quantifier
            if i < end && (chars[i] == '+' || chars[i] == '*') {
                i += 1;
            }
            result.push(' ');
        }
        // Other escape sequences
        else if i + 1 < end && chars[i] == '\\' {
            i += 1;
            result.push(chars[i]);
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Extract named capture groups from a regex pattern.
pub fn regex_capture_names(pattern: &str) -> Vec<String> {
    let mut names = Vec::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i + 4 < chars.len() {
        if chars[i] == '(' && chars[i + 1] == '?' && chars[i + 2] == 'P' && chars[i + 3] == '<' {
            i += 4;
            let start = i;
            while i < chars.len() && chars[i] != '>' {
                i += 1;
            }
            names.push(chars[start..i].iter().collect());
            i += 1;
        } else {
            i += 1;
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

pub fn value_as_f64(val: &Value) -> Option<f64> {
    match val {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn value_as_i64(val: &Value) -> Option<i64> {
    match val {
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: rounding f64 to i64 — truncation acceptable for device protocol values
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn value_as_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            // Prefer integer display when possible
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(f) = n.as_f64() {
                format_float(f)
            } else {
                n.to_string()
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        _ => val.to_string(),
    }
}

/// Format f64 cleanly: avoid trailing ".0" only when value is exactly integral.
fn format_float(f: f64) -> String {
    if f == f.trunc() && f.abs() < 1e15 {
        format!("{:.1}", f) // e.g., "820.0"
    } else {
        format!("{}", f) // e.g., "3.14", "1.23e-6"
    }
}

fn format_float_value(val: &Value) -> String {
    let f = value_as_f64(val).unwrap_or(0.0);
    format_float(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_parser;
    use serde_json::json;

    #[test]
    fn format_string_ell14_position() {
        // "{addr:1}PO{pulses:hex8}"
        let segments = format_parser::parse_format("{addr:1}PO{pulses:hex8}").unwrap();
        let mut state = HashMap::new();
        state.insert("addr".into(), json!("0"));
        state.insert("pulses".into(), json!(41395_i64));

        let result = generate_format_string(&segments, &state);
        assert_eq!(result, "0PO0000A1B3");
    }

    #[test]
    fn format_string_hex_negative() {
        let segments = format_parser::parse_format("{addr:1}PO{pulses:hex8}").unwrap();
        let mut state = HashMap::new();
        state.insert("addr".into(), json!("0"));
        state.insert("pulses".into(), json!(-1_i64));

        let result = generate_format_string(&segments, &state);
        assert_eq!(result, "0POFFFFFFFF");
    }

    #[test]
    fn format_string_status() {
        let segments = format_parser::parse_format("{addr:1}GS{code:hex2}").unwrap();
        let mut state = HashMap::new();
        state.insert("addr".into(), json!("0"));
        state.insert("code".into(), json!(0_i64));

        let result = generate_format_string(&segments, &state);
        assert_eq!(result, "0GS00");
    }

    #[test]
    fn transform_inverse_maitai_wavelength() {
        let ops = vec![
            TransformOp::Trim,
            TransformOp::RemoveSuffix {
                suffix: "nm".into(),
            },
            TransformOp::RemoveSuffix {
                suffix: "NM".into(),
            },
            TransformOp::ToFloat,
        ];
        let mut state = HashMap::new();
        state.insert("value".into(), json!(820.0));

        let result = generate_transform_inverse(&ops, "", &state);
        // The inverse appends both suffixes (ugly but round-trips correctly)
        // Forward: result → trim → remove_suffix(nm) → remove_suffix(NM) → to_float
        assert!(result.contains("820"));
    }

    #[test]
    fn transform_inverse_ipg_power() {
        let ops = vec![
            TransformOp::RegexExtract {
                pattern: r"^\w+:\s+(.*)".into(),
                group: 1,
                compiled: None,
            },
            TransformOp::Trim,
            TransformOp::ToFloat,
        ];
        let mut state = HashMap::new();
        state.insert("value".into(), json!(12.5));

        let result = generate_transform_inverse(&ops, "ROP", &state);
        assert_eq!(result, "ROP: 12.5");
    }

    #[test]
    fn transform_inverse_simple_trim_to_float() {
        let ops = vec![TransformOp::Trim, TransformOp::ToFloat];
        let mut state = HashMap::new();
        state.insert("value".into(), json!(25.5));

        let result = generate_transform_inverse(&ops, "", &state);
        assert_eq!(result, "25.5");
    }

    #[test]
    fn regex_to_template_temp() {
        let template = regex_to_template(r"^TEMP\s+(?P<value>[+-]?\d+\.?\d*)$");
        assert_eq!(template, "TEMP {value}");
    }

    #[test]
    fn regex_to_template_pid() {
        let template =
            regex_to_template(r"^PID\s+(?P<p>\d+\.?\d*),(?P<i>\d+\.?\d*),(?P<d>\d+\.?\d*)$");
        assert_eq!(template, "PID {p},{i},{d}");
    }

    #[test]
    fn regex_to_template_identity() {
        let template = regex_to_template(
            r"^(?P<manufacturer>\w+),(?P<model>[\w-]+),(?P<serial>\w+),v(?P<version>[\d.]+)$",
        );
        assert_eq!(template, "{manufacturer},{model},{serial},v{version}");
    }

    #[test]
    fn regex_template_substitution() {
        let template = "TEMP {value}";
        let mut state = HashMap::new();
        state.insert("value".into(), json!(25.5));

        let result = generate_regex_template(template, &state);
        assert_eq!(result, "TEMP 25.5");
    }

    #[test]
    fn scpi_float_generation() {
        let mut state = HashMap::new();
        state.insert("value".into(), json!(1.23));

        let result = generate_scpi(&ScpiResponseType::Float, &state);
        assert_eq!(result, "1.23");
    }

    #[test]
    fn scpi_integer_generation() {
        let mut state = HashMap::new();
        state.insert("value".into(), json!(42));

        let result = generate_scpi(&ScpiResponseType::Integer, &state);
        assert_eq!(result, "42");
    }
}
