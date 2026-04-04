//! Tiered response parsing.
//!
//! Supports four tiers of response parsing, in priority order:
//!
//! - **Tier 0: SCPI auto-parse** - When a command has `response_type`, trim
//!   whitespace and parse as the specified type.
//! - **Tier 1: Format strings** - Structured field extraction using format
//!   patterns like `"{addr:1}PO{pulses:hex8}"`.
//! - **Tier 2: Transform pipeline** - Sequential string transformations
//!   (trim, regex extract, to_float, scale, etc.).
//! - **Tier 3: Regex** - Named capture groups from a regex pattern.

use crate::config::validated::{ResponseParser, ScpiResponseType, ValidatedFormat, ValidatedRegex};
use crate::format_parser;
use crate::transform::TransformPipeline;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::collections::HashMap;

/// Parse a response using SCPI auto-parse (Tier 0).
///
/// Trims whitespace and parses based on the declared response type.
pub fn parse_scpi(input: &str, response_type: &ScpiResponseType) -> Result<Value> {
    let trimmed = input.trim();

    match response_type {
        ScpiResponseType::Float => {
            let v: f64 = trimmed
                .parse()
                .context(format!("SCPI: failed to parse float from '{trimmed}'"))?;
            let num = serde_json::Number::from_f64(v)
                .ok_or_else(|| anyhow!("SCPI: float value is not finite: {v}"))?;
            Ok(Value::Number(num))
        }
        ScpiResponseType::Integer => {
            let v: i64 = trimmed
                .parse()
                .context(format!("SCPI: failed to parse integer from '{trimmed}'"))?;
            Ok(Value::Number(v.into()))
        }
        ScpiResponseType::String => Ok(Value::String(trimmed.to_string())),
        ScpiResponseType::Boolean => {
            let b = match trimmed {
                "1" | "ON" | "on" | "true" | "TRUE" => true,
                "0" | "OFF" | "off" | "false" | "FALSE" => false,
                _ => {
                    return Err(anyhow!(
                        "SCPI: unrecognized boolean value '{trimmed}' (expected 1/0/ON/OFF/true/false)"
                    ));
                }
            };
            Ok(Value::Bool(b))
        }
        ScpiResponseType::ArrayFloat => {
            let values: Result<Vec<Value>, _> = trimmed
                .split(',')
                .map(|s| {
                    let v: f64 = s.trim().parse().context(format!(
                        "SCPI: failed to parse float element '{}'",
                        s.trim()
                    ))?;
                    serde_json::Number::from_f64(v)
                        .map(Value::Number)
                        .ok_or_else(|| anyhow!("SCPI: float element is not finite: {v}"))
                })
                .collect();
            Ok(Value::Array(values?))
        }
    }
}

/// Parse a response using a validated format string (Tier 1).
pub fn parse_with_format(input: &str, format: &ValidatedFormat) -> Result<HashMap<String, Value>> {
    format_parser::parse_response(&format.segments, input)
}

/// Parse a response using a transform pipeline (Tier 2).
///
/// Returns the final value as a JSON value.
pub fn parse_with_transform(input: &str, pipeline: &TransformPipeline) -> Result<Value> {
    let result = pipeline.execute(input)?;
    transform_value_to_json(&result)
}

/// Parse a response using a regex with named capture groups (Tier 3).
pub fn parse_with_regex(input: &str, validated: &ValidatedRegex) -> Result<HashMap<String, Value>> {
    let captures = validated
        .regex
        .captures(input)
        .ok_or_else(|| anyhow!("regex did not match input: '{input}'"))?;

    let mut result = HashMap::new();
    for name in validated.regex.capture_names().flatten() {
        if let Some(m) = captures.name(name) {
            result.insert(name.to_string(), Value::String(m.as_str().to_string()));
        }
    }

    Ok(result)
}

/// Parse a response using the appropriate parser tier.
///
/// Dispatches to the correct parser based on the `ResponseParser` variant.
pub fn parse_with_parser(input: &str, parser: &ResponseParser) -> Result<Value> {
    match parser {
        ResponseParser::Format(fmt) => {
            let map = parse_with_format(input, fmt)?;
            Ok(Value::Object(
                map.into_iter().collect::<serde_json::Map<String, Value>>(),
            ))
        }
        ResponseParser::Transform(pipeline) => parse_with_transform(input, pipeline),
        ResponseParser::Regex(regex) => {
            let map = parse_with_regex(input, regex)?;
            Ok(Value::Object(
                map.into_iter().collect::<serde_json::Map<String, Value>>(),
            ))
        }
    }
}

fn transform_value_to_json(value: &crate::transform::TransformValue) -> Result<Value> {
    match value {
        crate::transform::TransformValue::String(s) => Ok(Value::String(s.clone())),
        crate::transform::TransformValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .ok_or_else(|| anyhow!("transform produced non-finite float: {f}")),
        crate::transform::TransformValue::Int(i) => Ok(Value::Number((*i).into())),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::config::validated::ValidatedFormat;
    use crate::format_parser::parse_format;
    use crate::transform::{TransformOp, TransformPipeline};

    // ---- Tier 0: SCPI auto-parse ----

    #[test]
    fn scpi_float() {
        let result = parse_scpi("  1.234  ", &ScpiResponseType::Float).unwrap();
        assert_eq!(result.as_f64().unwrap(), 1.234);
    }

    #[test]
    fn scpi_float_scientific() {
        let result = parse_scpi("1.5E+3", &ScpiResponseType::Float).unwrap();
        assert_eq!(result.as_f64().unwrap(), 1500.0);
    }

    #[test]
    fn scpi_float_negative_exponent() {
        let result = parse_scpi("-2.5e-3", &ScpiResponseType::Float).unwrap();
        let v = result.as_f64().unwrap();
        assert!((v - (-0.0025)).abs() < 1e-10);
    }

    #[test]
    fn scpi_integer() {
        let result = parse_scpi("  42  ", &ScpiResponseType::Integer).unwrap();
        assert_eq!(result.as_i64().unwrap(), 42);
    }

    #[test]
    fn scpi_integer_negative() {
        let result = parse_scpi("-100", &ScpiResponseType::Integer).unwrap();
        assert_eq!(result.as_i64().unwrap(), -100);
    }

    #[test]
    fn scpi_boolean_on() {
        assert_eq!(
            parse_scpi("1", &ScpiResponseType::Boolean).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            parse_scpi("ON", &ScpiResponseType::Boolean).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn scpi_boolean_off() {
        assert_eq!(
            parse_scpi("0", &ScpiResponseType::Boolean).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            parse_scpi("OFF", &ScpiResponseType::Boolean).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn scpi_string() {
        let result = parse_scpi("  hello world  ", &ScpiResponseType::String).unwrap();
        assert_eq!(result.as_str().unwrap(), "hello world");
    }

    #[test]
    fn scpi_array_float() {
        let result = parse_scpi("1.0, 2.5, 3.7", &ScpiResponseType::ArrayFloat).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_f64().unwrap(), 1.0);
        assert_eq!(arr[1].as_f64().unwrap(), 2.5);
        assert_eq!(arr[2].as_f64().unwrap(), 3.7);
    }

    #[test]
    fn scpi_whitespace_trimming() {
        let result = parse_scpi("\r\n  42.0  \r\n", &ScpiResponseType::Float).unwrap();
        assert_eq!(result.as_f64().unwrap(), 42.0);
    }

    #[test]
    fn scpi_invalid_float() {
        assert!(parse_scpi("abc", &ScpiResponseType::Float).is_err());
    }

    #[test]
    fn scpi_invalid_boolean() {
        assert!(parse_scpi("maybe", &ScpiResponseType::Boolean).is_err());
    }

    // ---- Tier 1: Format strings ----

    #[test]
    fn tier1_format_parsing() {
        let segments = parse_format("{addr:1}PO{pulses:hex8}").unwrap();
        let fmt = ValidatedFormat {
            source: "{addr:1}PO{pulses:hex8}".to_string(),
            segments,
        };
        let result = parse_with_format("2PO0000A1B3", &fmt).unwrap();
        assert_eq!(result["addr"], Value::String("2".to_string()));
        assert_eq!(result["pulses"], Value::Number(0xA1B3.into()));
    }

    // ---- Tier 2: Transform pipeline ----

    #[test]
    fn tier2_transform() {
        let pipeline = TransformPipeline::new(vec![
            TransformOp::Trim,
            TransformOp::RemoveSuffix {
                suffix: "C".to_string(),
            },
            TransformOp::ToFloat,
        ])
        .unwrap();
        let result = parse_with_transform(" 25.5C ", &pipeline).unwrap();
        assert_eq!(result.as_f64().unwrap(), 25.5);
    }

    // ---- Tier 3: Regex ----

    #[test]
    fn tier3_regex() {
        let regex = regex::Regex::new(r"(?P<name>\w+)=(?P<value>\d+\.\d+)").unwrap();
        let validated = ValidatedRegex {
            source: r"(?P<name>\w+)=(?P<value>\d+\.\d+)".to_string(),
            regex,
        };
        let result = parse_with_regex("TEMP=25.5", &validated).unwrap();
        assert_eq!(result["name"], Value::String("TEMP".to_string()));
        assert_eq!(result["value"], Value::String("25.5".to_string()));
    }

    #[test]
    fn tier3_regex_no_match() {
        let regex = regex::Regex::new(r"(?P<v>\d+)").unwrap();
        let validated = ValidatedRegex {
            source: r"(?P<v>\d+)".to_string(),
            regex,
        };
        assert!(parse_with_regex("abc", &validated).is_err());
    }

    // ---- parse_with_parser dispatch ----

    #[test]
    fn dispatch_format_parser() {
        let segments = parse_format("OK{code:2}").unwrap();
        let parser = ResponseParser::Format(ValidatedFormat {
            source: "OK{code:2}".to_string(),
            segments,
        });
        let result = parse_with_parser("OK42", &parser).unwrap();
        assert_eq!(result["code"], Value::String("42".to_string()));
    }

    #[test]
    fn dispatch_transform_parser() {
        let pipeline =
            TransformPipeline::new(vec![TransformOp::Trim, TransformOp::ToFloat]).unwrap();
        let parser = ResponseParser::Transform(pipeline);
        let result = parse_with_parser("  7.25  ", &parser).unwrap();
        assert_eq!(result.as_f64().unwrap(), 7.25);
    }
}
