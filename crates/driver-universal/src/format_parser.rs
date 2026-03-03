//! Format string parser for response parsing.
//!
//! Parses format strings like `"{addr:1}PO{pulses:hex8}"` into a sequence of
//! segments that can match and extract named fields from device responses.
//!
//! # Specifiers
//!
//! - `{name}` - greedy string capture (everything until next literal)
//! - `{name:N}` (numeric N) - exactly N characters
//! - `{name:hex2}`, `{name:hex4}`, `{name:hex8}` - fixed-width hex -> integer
//! - `{name:int}` - decimal integer
//! - `{name:float}` - floating point number
//! - `{_:N}` - skip N characters (discard)

use crate::config::ConfigError;
use serde_json::Value;
use std::collections::HashMap;

/// A parsed segment from a format string.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatSegment {
    /// A literal string that must match exactly.
    Literal(String),
    /// A named field to extract.
    Field(FieldSpec),
}

/// Specification for a single field to extract.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSpec {
    /// Field name (or "_" for discard).
    pub name: String,
    /// How to match and parse this field.
    pub kind: FieldKind,
}

/// The kind of field capture/parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// Greedy string capture (until next literal or end).
    Greedy,
    /// Exactly N characters, returned as string.
    FixedWidth(usize),
    /// Fixed-width hex, converted to integer.
    Hex(usize),
    /// Decimal integer.
    Int,
    /// Floating point number.
    Float,
}

/// Parse a format string into segments.
///
/// # Errors
///
/// Returns `ConfigError::InvalidFormat` if the format string is malformed.
pub fn parse_format(format: &str) -> Result<Vec<FormatSegment>, ConfigError> {
    let mut segments = Vec::new();
    let mut chars = format.chars().peekable();
    let mut literal = String::new();

    while let Some(&ch) = chars.peek() {
        if ch == '{' {
            // Flush any accumulated literal
            if !literal.is_empty() {
                segments.push(FormatSegment::Literal(std::mem::take(&mut literal)));
            }

            chars.next(); // consume '{'

            // Parse field name
            let mut name = String::new();
            while let Some(&c) = chars.peek() {
                if c == ':' || c == '}' {
                    break;
                }
                name.push(c);
                chars.next();
            }

            if name.is_empty() {
                return Err(ConfigError::InvalidFormat {
                    format: format.to_string(),
                    reason: "empty field name".to_string(),
                });
            }

            // Parse optional specifier
            let kind = if chars.peek() == Some(&':') {
                chars.next(); // consume ':'
                let mut spec = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '}' {
                        break;
                    }
                    spec.push(c);
                    chars.next();
                }
                parse_field_kind(&spec, format)?
            } else {
                FieldKind::Greedy
            };

            // Expect closing '}'
            if chars.next() != Some('}') {
                return Err(ConfigError::InvalidFormat {
                    format: format.to_string(),
                    reason: "unclosed field (missing '}')".to_string(),
                });
            }

            segments.push(FormatSegment::Field(FieldSpec { name, kind }));
        } else {
            literal.push(ch);
            chars.next();
        }
    }

    // Flush trailing literal
    if !literal.is_empty() {
        segments.push(FormatSegment::Literal(literal));
    }

    // Validate: reject adjacent greedy fields with no literal separator
    for window in segments.windows(2) {
        if let [FormatSegment::Field(a), FormatSegment::Field(b)] = window {
            if matches!(a.kind, FieldKind::Greedy) && matches!(b.kind, FieldKind::Greedy) {
                return Err(ConfigError::InvalidFormat {
                    format: format.to_string(),
                    reason: format!(
                        "adjacent greedy fields '{}' and '{}' with no literal separator",
                        a.name, b.name
                    ),
                });
            }
        }
    }

    Ok(segments)
}

fn parse_field_kind(spec: &str, format: &str) -> Result<FieldKind, ConfigError> {
    match spec {
        "int" => Ok(FieldKind::Int),
        "float" => Ok(FieldKind::Float),
        s if s.starts_with("hex") => {
            let width_str = &s[3..];
            let width: usize = width_str.parse().map_err(|_| ConfigError::InvalidFormat {
                format: format.to_string(),
                reason: format!("invalid hex width: '{width_str}'"),
            })?;
            if width == 0 {
                return Err(ConfigError::InvalidFormat {
                    format: format.to_string(),
                    reason: "hex width must be > 0".to_string(),
                });
            }
            Ok(FieldKind::Hex(width))
        }
        s => {
            // Try to parse as a plain number (fixed width)
            let width: usize = s.parse().map_err(|_| ConfigError::InvalidFormat {
                format: format.to_string(),
                reason: format!("unknown specifier: '{s}'"),
            })?;
            if width == 0 {
                return Err(ConfigError::InvalidFormat {
                    format: format.to_string(),
                    reason: "field width must be > 0".to_string(),
                });
            }
            Ok(FieldKind::FixedWidth(width))
        }
    }
}

/// Parse a response string using a pre-parsed format, extracting named fields.
///
/// Returns a map of field names to their parsed JSON values.
/// Fields named `_` are discarded.
///
/// # Errors
///
/// Returns an error if the input doesn't match the format.
pub fn parse_response(
    segments: &[FormatSegment],
    input: &str,
) -> anyhow::Result<HashMap<String, Value>> {
    let mut result = HashMap::new();
    let mut pos = 0;
    let chars: Vec<char> = input.chars().collect();

    for (i, segment) in segments.iter().enumerate() {
        match segment {
            FormatSegment::Literal(lit) => {
                let lit_chars: Vec<char> = lit.chars().collect();
                if pos + lit_chars.len() > chars.len() {
                    anyhow::bail!(
                        "expected literal '{}' at position {} but input is too short",
                        lit,
                        pos
                    );
                }
                let actual: String = chars[pos..pos + lit_chars.len()].iter().collect();
                if actual != *lit {
                    anyhow::bail!(
                        "expected literal '{}' at position {} but found '{}'",
                        lit,
                        pos,
                        actual
                    );
                }
                pos += lit_chars.len();
            }
            FormatSegment::Field(field) => {
                let (value, consumed) = extract_field(field, &chars, pos, i, segments)?;
                if field.name != "_" {
                    result.insert(field.name.clone(), value);
                }
                pos += consumed;
            }
        }
    }

    // Reject unexpected trailing non-whitespace
    if pos < chars.len() && !chars[pos..].iter().all(|c| c.is_whitespace()) {
        let trailing: String = chars[pos..].iter().collect();
        anyhow::bail!(
            "unexpected trailing input at position {}: '{}'",
            pos,
            trailing
        );
    }

    Ok(result)
}

fn extract_field(
    field: &FieldSpec,
    chars: &[char],
    pos: usize,
    segment_idx: usize,
    segments: &[FormatSegment],
) -> anyhow::Result<(Value, usize)> {
    match &field.kind {
        FieldKind::FixedWidth(width) => {
            if pos + width > chars.len() {
                anyhow::bail!(
                    "field '{}' expects {} chars at position {} but only {} remain",
                    field.name,
                    width,
                    pos,
                    chars.len() - pos
                );
            }
            let s: String = chars[pos..pos + width].iter().collect();
            Ok((Value::String(s), *width))
        }
        FieldKind::Hex(width) => {
            if pos + width > chars.len() {
                anyhow::bail!(
                    "field '{}' expects {} hex chars at position {} but only {} remain",
                    field.name,
                    width,
                    pos,
                    chars.len() - pos
                );
            }
            let hex_str: String = chars[pos..pos + width].iter().collect();
            let raw = u64::from_str_radix(&hex_str, 16).map_err(|e| {
                anyhow::anyhow!("field '{}': invalid hex '{}': {}", field.name, hex_str, e)
            })?;
            // SAFETY: Intentional truncation for two's complement sign extension.
            // The value was parsed from exactly `width` hex chars and is in range.
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let value: i64 = match width {
                2 => i64::from((raw as u8) as i8),
                4 => i64::from((raw as u16) as i16),
                8 => i64::from((raw as u32) as i32),
                _ => raw as i64, // Non-standard widths: unsigned
            };
            Ok((Value::Number(serde_json::Number::from(value)), *width))
        }
        FieldKind::Int => {
            let remaining: String = chars[pos..].iter().collect();
            // Find the end of the integer (digits, optional leading sign)
            let end = find_number_end(&remaining, false);
            if end == 0 {
                anyhow::bail!(
                    "field '{}': expected integer at position {}",
                    field.name,
                    pos
                );
            }
            let num_str = &remaining[..end];
            let value: i64 = num_str.parse().map_err(|e| {
                anyhow::anyhow!(
                    "field '{}': invalid integer '{}': {}",
                    field.name,
                    num_str,
                    e
                )
            })?;
            Ok((Value::Number(serde_json::Number::from(value)), end))
        }
        FieldKind::Float => {
            let remaining: String = chars[pos..].iter().collect();
            let end = find_number_end(&remaining, true);
            if end == 0 {
                anyhow::bail!("field '{}': expected float at position {}", field.name, pos);
            }
            let num_str = &remaining[..end];
            let value: f64 = num_str.parse().map_err(|e| {
                anyhow::anyhow!("field '{}': invalid float '{}': {}", field.name, num_str, e)
            })?;
            let json_num = serde_json::Number::from_f64(value).ok_or_else(|| {
                anyhow::anyhow!("field '{}': float {} is not finite", field.name, value)
            })?;
            Ok((Value::Number(json_num), end))
        }
        FieldKind::Greedy => {
            // Find the next literal to know where to stop
            let stop_at = find_next_literal(segment_idx, segments);
            let remaining: String = chars[pos..].iter().collect();
            let captured = match stop_at {
                Some(lit) => {
                    let idx = remaining.find(&lit).ok_or_else(|| {
                        anyhow::anyhow!(
                            "field '{}': greedy capture couldn't find delimiter '{}'",
                            field.name,
                            lit
                        )
                    })?;
                    &remaining[..idx]
                }
                None => &remaining,
            };
            let len = captured.chars().count();
            Ok((Value::String(captured.to_string()), len))
        }
    }
}

/// Find the end position of a number in a string.
fn find_number_end(s: &str, allow_float: bool) -> usize {
    let mut end = 0;
    let bytes = s.as_bytes();

    // Optional sign
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }

    // Digits before decimal point
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }

    if allow_float {
        // Optional decimal point and fractional digits
        if end < bytes.len() && bytes[end] == b'.' {
            end += 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }

        // Optional exponent
        if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
            end += 1;
            if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
                end += 1;
            }
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
    }

    end
}

/// Find the next literal string after the given segment index.
fn find_next_literal(current_idx: usize, segments: &[FormatSegment]) -> Option<String> {
    for seg in &segments[current_idx + 1..] {
        if let FormatSegment::Literal(lit) = seg {
            return Some(lit.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ell14_position_format() {
        let segments = parse_format("{addr:1}PO{pulses:hex8}").unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(
            segments[0],
            FormatSegment::Field(FieldSpec {
                name: "addr".to_string(),
                kind: FieldKind::FixedWidth(1),
            })
        );
        assert_eq!(segments[1], FormatSegment::Literal("PO".to_string()));
        assert_eq!(
            segments[2],
            FormatSegment::Field(FieldSpec {
                name: "pulses".to_string(),
                kind: FieldKind::Hex(8),
            })
        );
    }

    #[test]
    fn parse_response_ell14_position() {
        let segments = parse_format("{addr:1}PO{pulses:hex8}").unwrap();
        let result = parse_response(&segments, "2PO0000A1B3").unwrap();
        assert_eq!(result["addr"], Value::String("2".to_string()));
        assert_eq!(result["pulses"], Value::Number(0x0000_A1B3.into()));
    }

    #[test]
    fn parse_response_ell14_status() {
        let segments = parse_format("{addr:1}GS{code:hex2}").unwrap();
        let result = parse_response(&segments, "2GS00").unwrap();
        assert_eq!(result["addr"], Value::String("2".to_string()));
        assert_eq!(result["code"], Value::Number(0.into()));
    }

    #[test]
    fn parse_format_greedy() {
        let segments = parse_format("{name}={value}").unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(
            segments[0],
            FormatSegment::Field(FieldSpec {
                name: "name".to_string(),
                kind: FieldKind::Greedy,
            })
        );
        assert_eq!(segments[1], FormatSegment::Literal("=".to_string()));
    }

    #[test]
    fn parse_response_greedy() {
        let segments = parse_format("{name}={value}").unwrap();
        let result = parse_response(&segments, "TEMP=25.5").unwrap();
        assert_eq!(result["name"], Value::String("TEMP".to_string()));
        assert_eq!(result["value"], Value::String("25.5".to_string()));
    }

    #[test]
    fn parse_format_int_specifier() {
        let segments = parse_format("VAL:{count:int}").unwrap();
        let result = parse_response(&segments, "VAL:42").unwrap();
        assert_eq!(result["count"], Value::Number(42.into()));
    }

    #[test]
    fn parse_format_float_specifier() {
        let segments = parse_format("T={temp:float}C").unwrap();
        let result = parse_response(&segments, "T=25.5C").unwrap();
        let temp = result["temp"].as_f64().unwrap();
        assert!((temp - 25.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_format_discard_field() {
        let segments = parse_format("{_:2}DATA{value:hex4}").unwrap();
        let result = parse_response(&segments, "XXDATA00FF").unwrap();
        assert!(!result.contains_key("_"));
        assert_eq!(result["value"], Value::Number(0xFF.into()));
    }

    #[test]
    fn parse_format_all_specifiers() {
        // Test hex2 — two's complement: 0xFF = -1 as i8
        let segments = parse_format("{v:hex2}").unwrap();
        let result = parse_response(&segments, "FF").unwrap();
        assert_eq!(result["v"], Value::Number((-1).into()));

        // Test hex4 — 0x00FF = 255, positive in i16
        let segments = parse_format("{v:hex4}").unwrap();
        let result = parse_response(&segments, "00FF").unwrap();
        assert_eq!(result["v"], Value::Number(255.into()));
    }

    #[test]
    fn parse_format_error_empty_name() {
        let result = parse_format("{:3}");
        assert!(result.is_err());
    }

    #[test]
    fn parse_format_error_unclosed() {
        let result = parse_format("{name");
        assert!(result.is_err());
    }

    #[test]
    fn parse_format_error_unknown_specifier() {
        let result = parse_format("{name:foobar}");
        assert!(result.is_err());
    }

    #[test]
    fn parse_response_literal_mismatch() {
        let segments = parse_format("OK{val:3}").unwrap();
        let result = parse_response(&segments, "NO123");
        assert!(result.is_err());
    }

    #[test]
    fn parse_response_too_short() {
        let segments = parse_format("{val:hex8}").unwrap();
        let result = parse_response(&segments, "00FF");
        assert!(result.is_err());
    }

    #[test]
    fn parse_format_negative_int() {
        let segments = parse_format("V={v:int}").unwrap();
        let result = parse_response(&segments, "V=-42").unwrap();
        assert_eq!(result["v"], Value::Number((-42).into()));
    }

    #[test]
    fn parse_format_scientific_float() {
        let segments = parse_format("{v:float}").unwrap();
        let result = parse_response(&segments, "1.5e3").unwrap();
        let v = result["v"].as_f64().unwrap();
        assert!((v - 1500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_format_adjacent_greedy_error() {
        let result = parse_format("{a}{b}");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("adjacent greedy"));
    }

    #[test]
    fn parse_format_adjacent_greedy_with_separator_ok() {
        let result = parse_format("{a}={b}");
        assert!(result.is_ok());
    }

    #[test]
    fn hex_twos_complement_2() {
        let segments = parse_format("{v:hex2}").unwrap();
        // FF -> -1 (signed byte)
        let result = parse_response(&segments, "FF").unwrap();
        assert_eq!(result["v"], Value::Number((-1).into()));
        // 80 -> -128
        let result = parse_response(&segments, "80").unwrap();
        assert_eq!(result["v"], Value::Number((-128).into()));
        // 7F -> 127
        let result = parse_response(&segments, "7F").unwrap();
        assert_eq!(result["v"], Value::Number(127.into()));
    }

    #[test]
    fn hex_twos_complement_4() {
        let segments = parse_format("{v:hex4}").unwrap();
        // FFFF -> -1
        let result = parse_response(&segments, "FFFF").unwrap();
        assert_eq!(result["v"], Value::Number((-1).into()));
        // 7FFF -> 32767
        let result = parse_response(&segments, "7FFF").unwrap();
        assert_eq!(result["v"], Value::Number(32767.into()));
    }

    #[test]
    fn hex_twos_complement_8() {
        let segments = parse_format("{v:hex8}").unwrap();
        // FFFFFFFF -> -1
        let result = parse_response(&segments, "FFFFFFFF").unwrap();
        assert_eq!(result["v"], Value::Number((-1).into()));
        // 0000A1B3 -> 41395 (positive, same as before)
        let result = parse_response(&segments, "0000A1B3").unwrap();
        assert_eq!(result["v"], Value::Number(41395.into()));
    }
}
