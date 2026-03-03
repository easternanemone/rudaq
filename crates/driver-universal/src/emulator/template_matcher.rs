//! Compile MiniJinja command templates into regex matchers for incoming commands.
//!
//! The emulator receives raw command strings and needs to match them against
//! known command templates, extracting parameter values. This module converts
//! MiniJinja templates like `"{{ address }}ma{{ position_pulses | hex(8) }}"`
//! into regex patterns with named capture groups.

use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

/// How to decode a captured regex group back into a value.
#[derive(Debug, Clone)]
pub enum ParamDecoder {
    /// Keep as-is string.
    String,
    /// Parse as f64.
    Float,
    /// Parse as i64 (decimal).
    Int,
    /// Parse as hex, with sign-extension based on width (in hex chars).
    HexInt(usize),
}

/// A compiled template that can match incoming command strings.
#[derive(Debug)]
pub struct TemplateMatcher {
    pub regex: Regex,
    /// Ordered: (param_name, decoder) for each named capture group.
    pub param_decoders: Vec<(String, ParamDecoder)>,
}

impl TemplateMatcher {
    /// Compile a MiniJinja template into a regex matcher.
    ///
    /// Only handles the template patterns used in the codebase:
    /// - `{{ var }}` — basic variable
    /// - `{{ var | hex(N) }}` — hex-formatted with width N
    /// - `{{ var | int | pad(N, '0') }}` — integer zero-padded to N digits
    pub fn compile(template_source: &str, param_types: &HashMap<String, String>) -> Result<Self> {
        let mut regex_str = String::from("^");
        let mut decoders = Vec::new();

        let chars: Vec<char> = template_source.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if i + 1 < chars.len() && chars[i] == '{' && chars[i + 1] == '{' {
                // Start of {{ ... }} expression
                i += 2;

                // Skip whitespace
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }

                // Parse variable name
                let name_start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let var_name: String = chars[name_start..i].iter().collect();

                // Skip whitespace
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }

                // Check for filter pipe
                let (pattern, decoder) = if i < chars.len() && chars[i] == '|' {
                    i += 1; // consume '|'
                            // Collect filter text until }}
                    let filter_start = i;
                    let mut brace_depth = 0;
                    while i < chars.len() {
                        if chars[i] == '(' {
                            brace_depth += 1;
                        }
                        if chars[i] == ')' && brace_depth > 0 {
                            brace_depth -= 1;
                        }
                        if chars[i] == '}'
                            && i + 1 < chars.len()
                            && chars[i + 1] == '}'
                            && brace_depth == 0
                        {
                            break;
                        }
                        i += 1;
                    }
                    let filter_text: String = chars[filter_start..i].iter().collect();
                    parse_filter_chain(filter_text.trim())?
                } else {
                    infer_pattern(&var_name, param_types)
                };

                // Skip whitespace and closing }}
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                if i + 1 < chars.len() && chars[i] == '}' && chars[i + 1] == '}' {
                    i += 2;
                } else {
                    bail!("Expected '}}' in template at position {i}, template: {template_source}");
                }

                use std::fmt::Write;
                write!(regex_str, "(?P<{var_name}>{pattern})").unwrap();
                decoders.push((var_name, decoder));
            } else {
                // Literal character — regex-escape it
                let ch = chars[i];
                regex_str.push_str(&regex::escape(&ch.to_string()));
                i += 1;
            }
        }

        regex_str.push('$');

        let regex = Regex::new(&regex_str).map_err(|e| {
            anyhow::anyhow!(
                "Failed to compile template regex '{regex_str}' from '{template_source}': {e}"
            )
        })?;

        Ok(Self {
            regex,
            param_decoders: decoders,
        })
    }

    /// Match an input string and extract decoded parameter values.
    pub fn match_and_decode(&self, input: &str) -> Option<HashMap<String, Value>> {
        let captures = self.regex.captures(input)?;
        let mut result = HashMap::new();

        for (name, decoder) in &self.param_decoders {
            if let Some(m) = captures.name(name) {
                let value = decode_value(m.as_str(), decoder);
                result.insert(name.clone(), value);
            }
        }

        Some(result)
    }
}

/// Parse a filter chain string and return (regex_pattern, decoder).
fn parse_filter_chain(filter: &str) -> Result<(String, ParamDecoder)> {
    // hex(N)
    if let Some(width) = parse_hex_filter(filter) {
        return Ok((
            format!("[0-9A-Fa-f]{{{width}}}"),
            ParamDecoder::HexInt(width),
        ));
    }

    // int | pad(N, '0')
    if let Some(width) = parse_int_pad_filter(filter) {
        return Ok((format!("\\d{{{width}}}"), ParamDecoder::Int));
    }

    // Unknown filter — fall back to generic
    tracing::debug!("Unknown filter chain '{filter}', using generic pattern");
    Ok((".+?".to_string(), ParamDecoder::String))
}

/// Parse `hex(N)` and return N.
fn parse_hex_filter(filter: &str) -> Option<usize> {
    let filter = filter.trim();
    let rest = filter.strip_prefix("hex(")?;
    let num_str = rest.strip_suffix(')')?;
    num_str.trim().parse().ok()
}

/// Parse `int | pad(N, '0')` or `int | pad(N, "0")` and return N.
fn parse_int_pad_filter(filter: &str) -> Option<usize> {
    let filter = filter.trim();
    if !filter.starts_with("int") {
        return None;
    }
    let pad_part = filter.split('|').nth(1)?.trim();
    let rest = pad_part.strip_prefix("pad(")?;
    let args = rest.strip_suffix(')')?;
    let width_str = args.split(',').next()?;
    width_str.trim().parse().ok()
}

/// Infer regex pattern and decoder from parameter type declarations.
fn infer_pattern(var_name: &str, param_types: &HashMap<String, String>) -> (String, ParamDecoder) {
    if let Some(ptype) = param_types.get(var_name) {
        match ptype.as_str() {
            "float" => (
                r"[+-]?\d+\.?\d*(?:[eE][+-]?\d+)?".to_string(),
                ParamDecoder::Float,
            ),
            "int" | "int32" | "uint32" => (r"[+-]?\d+".to_string(), ParamDecoder::Int),
            _ => (".+?".to_string(), ParamDecoder::String),
        }
    } else if var_name == "address" {
        // Address is typically a short string (1-2 chars)
        (".+?".to_string(), ParamDecoder::String)
    } else {
        // Unknown param without type — assume numeric (common for SCPI)
        (
            r"[+-]?\d+\.?\d*(?:[eE][+-]?\d+)?".to_string(),
            ParamDecoder::Float,
        )
    }
}

/// Decode a captured regex group into a serde_json::Value.
fn decode_value(raw: &str, decoder: &ParamDecoder) -> Value {
    match decoder {
        ParamDecoder::String => Value::String(raw.to_string()),
        ParamDecoder::Float => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(raw.to_string())),
        ParamDecoder::Int => raw
            .parse::<i64>()
            .ok()
            .map(|i| Value::Number(i.into()))
            .unwrap_or_else(|| Value::String(raw.to_string())),
        ParamDecoder::HexInt(width) => u64::from_str_radix(raw, 16)
            .ok()
            .map(|raw_val| {
                // SAFETY: Intentional truncation + sign extension for two's complement protocol decoding.
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let signed: i64 = match width {
                    2 => i64::from((raw_val as u8) as i8),
                    4 => i64::from((raw_val as u16) as i16),
                    8 => i64::from((raw_val as u32) as i32),
                    _ => raw_val as i64,
                };
                Value::Number(signed.into())
            })
            .unwrap_or_else(|| Value::String(raw.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn ell14_hex_template() {
        let tm = TemplateMatcher::compile(
            "{{ address }}ma{{ position_pulses | hex(8) }}",
            &types(&[("position_pulses", "int32")]),
        )
        .unwrap();

        let params = tm.match_and_decode("2ma0000A1B3").unwrap();
        assert_eq!(params["address"], Value::String("2".into()));
        // 0x0000A1B3 = 41395 (positive, fits in i32)
        assert_eq!(params["position_pulses"], json!(41395_i64));
    }

    #[test]
    fn ell14_hex_negative() {
        let tm = TemplateMatcher::compile(
            "{{ address }}ma{{ position_pulses | hex(8) }}",
            &types(&[("position_pulses", "int32")]),
        )
        .unwrap();

        let params = tm.match_and_decode("0maFFFFFFFF").unwrap();
        // 0xFFFFFFFF as i32 = -1
        assert_eq!(params["position_pulses"], json!(-1_i64));
    }

    #[test]
    fn scpi_query_literal() {
        let tm = TemplateMatcher::compile("WAVELENGTH?", &HashMap::new()).unwrap();
        let params = tm.match_and_decode("WAVELENGTH?").unwrap();
        assert!(params.is_empty());
        assert!(tm.match_and_decode("WAVELENGTH").is_none());
    }

    #[test]
    fn scpi_setter_with_float() {
        let tm = TemplateMatcher::compile(
            "WAVELENGTH:{{ wavelength }}",
            &types(&[("wavelength", "float")]),
        )
        .unwrap();

        let params = tm.match_and_decode("WAVELENGTH:820.5").unwrap();
        assert_eq!(params["wavelength"], json!(820.5));
    }

    #[test]
    fn plain_getter_with_address() {
        let tm = TemplateMatcher::compile("{{ address }}gp", &HashMap::new()).unwrap();
        let params = tm.match_and_decode("0gp").unwrap();
        assert_eq!(params["address"], Value::String("0".into()));
    }

    #[test]
    fn esp300_float_position() {
        let tm = TemplateMatcher::compile(
            "{{ address }}PA{{ position }}",
            &types(&[("position", "float")]),
        )
        .unwrap();

        let params = tm.match_and_decode("1PA25.5").unwrap();
        assert_eq!(params["address"], Value::String("1".into()));
        assert_eq!(params["position"], json!(25.5));
    }

    #[test]
    fn ipg_literal_command() {
        let tm = TemplateMatcher::compile("ROP", &HashMap::new()).unwrap();
        let params = tm.match_and_decode("ROP").unwrap();
        assert!(params.is_empty());
        assert!(tm.match_and_decode("ROPX").is_none());
    }

    #[test]
    fn scpi_with_space() {
        let tm =
            TemplateMatcher::compile("SCS {{ value }}", &types(&[("value", "float")])).unwrap();

        let params = tm.match_and_decode("SCS 100.5").unwrap();
        assert_eq!(params["value"], json!(100.5));
    }

    #[test]
    fn hex2_filter() {
        let tm = TemplateMatcher::compile(
            "{{ address }}sv{{ percent | hex(2) }}",
            &types(&[("percent", "uint32")]),
        )
        .unwrap();

        let params = tm.match_and_decode("0sv64").unwrap();
        assert_eq!(params["address"], Value::String("0".into()));
        // 0x64 = 100, as u8 cast to i8 = 100 (positive, fits)
        assert_eq!(params["percent"], json!(100_i64));
    }

    use serde_json::json;
}
