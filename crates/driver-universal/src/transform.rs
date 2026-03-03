//! Declarative response transformation pipeline.
//!
//! Adapted from `driver-generic::transform` with the addition of `SplitComma`.
//!
//! # Example
//!
//! ```
//! use driver_universal::transform::{TransformPipeline, TransformOp, TransformValue};
//!
//! let pipeline = TransformPipeline::new(vec![
//!     TransformOp::Trim,
//!     TransformOp::RemoveSuffix { suffix: "C".to_string() },
//!     TransformOp::ToFloat,
//! ]).unwrap();
//!
//! let result = pipeline.execute(" 25.5C ").unwrap();
//! assert_eq!(result.as_f64().unwrap(), 25.5);
//! ```

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// A single transformation operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TransformOp {
    /// Remove leading and trailing whitespace.
    Trim,
    /// Remove a prefix if present.
    RemovePrefix { prefix: String },
    /// Remove a suffix if present.
    RemoveSuffix { suffix: String },
    /// Extract a regex capture group (pre-compiled).
    RegexExtract {
        pattern: String,
        group: usize,
        /// Pre-compiled regex (populated by `compile()` or `from_shorthand()`).
        #[serde(skip)]
        compiled: Option<Regex>,
    },
    /// Parse as floating point number.
    ToFloat,
    /// Parse as integer.
    ToInt,
    /// Multiply by a scaling factor.
    Scale { factor: f64 },
    /// Add an offset value.
    Offset { value: f64 },
    /// Split by comma and take element at index.
    SplitComma { index: usize },
    /// Map a specific string value to a replacement (e.g. "Off" -> "0.0").
    Map { from: String, to: String },
}

impl TransformOp {
    /// Pre-compile any regex patterns in this operation.
    ///
    /// Returns an error if the pattern is invalid, so callers can surface
    /// compilation failures rather than silently falling back to runtime compilation.
    pub fn compile(&mut self) -> Result<()> {
        if let TransformOp::RegexExtract {
            pattern, compiled, ..
        } = self
        {
            if compiled.is_none() {
                *compiled = Some(Regex::new(pattern).context("Failed to compile regex pattern")?);
            }
        }
        Ok(())
    }
}

impl TransformOp {
    /// Apply this transformation operation to a value.
    pub fn apply(&self, input: TransformValue) -> Result<TransformValue> {
        match self {
            TransformOp::Trim => match input {
                TransformValue::String(s) => Ok(TransformValue::String(s.trim().to_string())),
                other => Ok(other),
            },
            TransformOp::RemovePrefix { prefix } => match input {
                TransformValue::String(s) => Ok(TransformValue::String(
                    s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string(),
                )),
                other => Ok(other),
            },
            TransformOp::RemoveSuffix { suffix } => match input {
                TransformValue::String(s) => Ok(TransformValue::String(
                    s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_string(),
                )),
                other => Ok(other),
            },
            TransformOp::RegexExtract {
                pattern,
                group,
                compiled,
            } => match input {
                TransformValue::String(s) => {
                    // Use pre-compiled regex if available, otherwise compile on the fly
                    let owned;
                    let regex = match compiled {
                        Some(r) => r,
                        None => {
                            owned =
                                Regex::new(pattern).context("Failed to compile regex pattern")?;
                            &owned
                        }
                    };
                    let captures = regex
                        .captures(&s)
                        .ok_or_else(|| anyhow!("Regex pattern did not match input"))?;
                    let captured = captures
                        .get(*group)
                        .ok_or_else(|| anyhow!("Capture group {} not found", group))?
                        .as_str();
                    Ok(TransformValue::String(captured.to_string()))
                }
                _ => Err(anyhow!("RegexExtract can only be applied to strings")),
            },
            TransformOp::ToFloat => match input {
                TransformValue::String(s) => Ok(TransformValue::Float(
                    s.parse().context("Failed to parse float")?,
                )),
                TransformValue::Float(f) => Ok(TransformValue::Float(f)),
                #[allow(clippy::cast_precision_loss)]
                // SAFETY: i64 to f64 — precision loss acceptable for device transform values
                TransformValue::Int(i) => Ok(TransformValue::Float(i as f64)),
            },
            TransformOp::ToInt => match input {
                TransformValue::String(s) => Ok(TransformValue::Int(
                    s.parse().context("Failed to parse int")?,
                )),
                TransformValue::Int(i) => Ok(TransformValue::Int(i)),
                #[allow(clippy::cast_possible_truncation)]
                // SAFETY: rounding f64 to i64 — truncation acceptable for device values
                TransformValue::Float(f) => Ok(TransformValue::Int(f.round() as i64)),
            },
            TransformOp::Scale { factor } => match input {
                TransformValue::Float(f) => Ok(TransformValue::Float(f * factor)),
                #[allow(clippy::cast_precision_loss)]
                // SAFETY: i64 to f64 — precision loss acceptable for scaling operation
                TransformValue::Int(i) => Ok(TransformValue::Float((i as f64) * factor)),
                _ => Err(anyhow!("Scale can only be applied to numeric values")),
            },
            TransformOp::Offset { value } => match input {
                TransformValue::Float(f) => Ok(TransformValue::Float(f + value)),
                #[allow(clippy::cast_precision_loss)]
                // SAFETY: i64 to f64 — precision loss acceptable for offset operation
                TransformValue::Int(i) => Ok(TransformValue::Float((i as f64) + value)),
                _ => Err(anyhow!("Offset can only be applied to numeric values")),
            },
            TransformOp::SplitComma { index } => match input {
                TransformValue::String(s) => {
                    let parts: Vec<&str> = s.split(',').collect();
                    let part = parts.get(*index).ok_or_else(|| {
                        anyhow!(
                            "SplitComma index {} out of range (found {} parts)",
                            index,
                            parts.len()
                        )
                    })?;
                    Ok(TransformValue::String(part.trim().to_string()))
                }
                _ => Err(anyhow!("SplitComma can only be applied to strings")),
            },
            TransformOp::Map { from, to } => match input {
                TransformValue::String(s) if s == *from => Ok(TransformValue::String(to.clone())),
                other => Ok(other),
            },
        }
    }

    /// Parse a transform operation from shorthand string syntax.
    ///
    /// Supported formats:
    /// - `"trim"` -> `Trim`
    /// - `"remove_prefix('X')"` -> `RemovePrefix { prefix: "X" }`
    /// - `"remove_suffix('C')"` -> `RemoveSuffix { suffix: "C" }`
    /// - `"regex_extract('pattern', 1)"` -> `RegexExtract { pattern, group: 1 }`
    /// - `"to_float"` -> `ToFloat`
    /// - `"to_int"` -> `ToInt`
    /// - `"scale(1000.0)"` -> `Scale { factor: 1000.0 }`
    /// - `"offset(-273.15)"` -> `Offset { value: -273.15 }`
    /// - `"split_comma(0)"` -> `SplitComma { index: 0 }`
    /// - `"map('Off', '0.0')"` -> `Map { from: "Off", to: "0.0" }`
    pub fn from_shorthand(s: &str) -> Result<Self> {
        let s = s.trim();

        match s {
            "trim" => return Ok(TransformOp::Trim),
            "to_float" => return Ok(TransformOp::ToFloat),
            "to_int" => return Ok(TransformOp::ToInt),
            _ => {}
        }

        if let Some(rest) = s.strip_prefix("remove_prefix(") {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("Missing closing parenthesis"))?;
            let prefix = parse_string_arg(arg)?;
            return Ok(TransformOp::RemovePrefix { prefix });
        }

        if let Some(rest) = s.strip_prefix("remove_suffix(") {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("Missing closing parenthesis"))?;
            let suffix = parse_string_arg(arg)?;
            return Ok(TransformOp::RemoveSuffix { suffix });
        }

        if let Some(rest) = s.strip_prefix("regex_extract(") {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("Missing closing parenthesis"))?;
            let (pattern, group) = parse_regex_extract_args(arg)?;
            let compiled = Regex::new(&pattern).context("Failed to compile regex pattern")?;
            return Ok(TransformOp::RegexExtract {
                pattern,
                group,
                compiled: Some(compiled),
            });
        }

        if let Some(rest) = s.strip_prefix("scale(") {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("Missing closing parenthesis"))?;
            let factor = arg
                .trim()
                .parse::<f64>()
                .context("Failed to parse scale factor")?;
            return Ok(TransformOp::Scale { factor });
        }

        if let Some(rest) = s.strip_prefix("offset(") {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("Missing closing parenthesis"))?;
            let value = arg
                .trim()
                .parse::<f64>()
                .context("Failed to parse offset value")?;
            return Ok(TransformOp::Offset { value });
        }

        if let Some(rest) = s.strip_prefix("split_comma(") {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("Missing closing parenthesis"))?;
            let index = arg
                .trim()
                .parse::<usize>()
                .context("Failed to parse split_comma index")?;
            return Ok(TransformOp::SplitComma { index });
        }

        if let Some(rest) = s.strip_prefix("map(") {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("Missing closing parenthesis"))?;
            // Parse two comma-separated string arguments: map('from', 'to')
            let parts: Vec<&str> = arg.splitn(2, ',').collect();
            if parts.len() != 2 {
                return Err(anyhow!(
                    "map() requires exactly 2 arguments: map('from', 'to')"
                ));
            }
            let from = parse_string_arg(parts[0].trim())?;
            let to = parse_string_arg(parts[1].trim())?;
            return Ok(TransformOp::Map { from, to });
        }

        Err(anyhow!("Unknown transform operation: {}", s))
    }
}

fn parse_string_arg(s: &str) -> Result<String> {
    let s = s.trim();
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        if s.len() >= 2 {
            Ok(s[1..s.len() - 1].to_string())
        } else {
            Ok(s.to_string())
        }
    } else {
        Ok(s.to_string())
    }
}

/// Split comma-separated arguments while respecting quoted strings.
fn split_args_quote_aware(s: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for ch in s.chars() {
        match ch {
            '\'' | '"' if !in_quotes => {
                in_quotes = true;
                quote_char = ch;
                current.push(ch);
            }
            c if in_quotes && c == quote_char => {
                in_quotes = false;
                current.push(c);
            }
            ',' if !in_quotes => {
                if !current.trim().is_empty() {
                    args.push(current.trim().to_string());
                }
                current.clear();
            }
            c => current.push(c),
        }
    }

    if !current.trim().is_empty() {
        args.push(current.trim().to_string());
    }

    if in_quotes {
        return Err(anyhow!("Unterminated quote in argument list"));
    }

    Ok(args)
}

fn parse_regex_extract_args(s: &str) -> Result<(String, usize)> {
    let parts = split_args_quote_aware(s)?;
    if parts.len() != 2 {
        return Err(anyhow!(
            "regex_extract requires 2 arguments: pattern and group"
        ));
    }

    let pattern = parse_string_arg(parts[0].trim())?;
    let group = parts[1]
        .trim()
        .parse::<usize>()
        .context("Failed to parse group number")?;

    Ok((pattern, group))
}

/// A value flowing through the transform pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum TransformValue {
    String(String),
    Float(f64),
    Int(i64),
}

impl TransformValue {
    /// Convert to f64 if possible.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            TransformValue::Float(f) => Some(*f),
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: i64 to f64 — precision loss acceptable for device values
            TransformValue::Int(i) => Some(*i as f64),
            TransformValue::String(_) => None,
        }
    }

    /// Convert to i64 if possible.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            TransformValue::Int(i) => Some(*i),
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: rounding f64 to i64 — truncation acceptable for device values
            TransformValue::Float(f) => Some(f.round() as i64),
            TransformValue::String(_) => None,
        }
    }

    /// Convert to string representation.
    pub fn as_string(&self) -> String {
        match self {
            TransformValue::String(s) => s.clone(),
            TransformValue::Float(f) => f.to_string(),
            TransformValue::Int(i) => i.to_string(),
        }
    }
}

/// A pipeline of transformation operations applied in sequence.
#[derive(Debug, Clone)]
pub struct TransformPipeline {
    ops: Vec<TransformOp>,
}

impl TransformPipeline {
    /// Access the operations in this pipeline (for emulator transform inversion).
    pub fn ops(&self) -> &[TransformOp] {
        &self.ops
    }
}

impl TransformPipeline {
    /// Create a new pipeline from a sequence of operations.
    ///
    /// Pre-compiles any regex patterns in the operations.
    /// Returns an error if any regex pattern is invalid.
    pub fn new(ops: Vec<TransformOp>) -> Result<Self> {
        let mut ops = ops;
        for op in &mut ops {
            op.compile()?;
        }
        Ok(Self { ops })
    }

    /// Create a pipeline from shorthand string syntax.
    pub fn from_shorthand(shorthand: &[String]) -> Result<Self> {
        let ops = shorthand
            .iter()
            .map(|s| TransformOp::from_shorthand(s))
            .collect::<Result<Vec<_>>>()?;
        Self::new(ops)
    }

    /// Execute the pipeline on an input string.
    pub fn execute(&self, input: &str) -> Result<TransformValue> {
        let mut value = TransformValue::String(input.to_string());
        for op in &self.ops {
            value = op
                .apply(value)
                .context(format!("Failed to apply {:?}", op))?;
        }
        Ok(value)
    }

    /// Check if the pipeline is empty.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn test_trim() {
        let op = TransformOp::Trim;
        let result = op
            .apply(TransformValue::String("  hello  ".to_string()))
            .unwrap();
        assert_eq!(result, TransformValue::String("hello".to_string()));
    }

    #[test]
    fn test_remove_suffix() {
        let op = TransformOp::RemoveSuffix {
            suffix: "C".to_string(),
        };
        let result = op
            .apply(TransformValue::String("25.5C".to_string()))
            .unwrap();
        assert_eq!(result, TransformValue::String("25.5".to_string()));
    }

    #[test]
    fn test_remove_prefix() {
        let op = TransformOp::RemovePrefix {
            prefix: "TEMP ".to_string(),
        };
        let result = op
            .apply(TransformValue::String("TEMP 25.5".to_string()))
            .unwrap();
        assert_eq!(result, TransformValue::String("25.5".to_string()));
    }

    #[test]
    fn test_to_float() {
        let op = TransformOp::ToFloat;
        let result = op
            .apply(TransformValue::String("25.5".to_string()))
            .unwrap();
        assert_eq!(result, TransformValue::Float(25.5));
    }

    #[test]
    fn test_to_int() {
        let op = TransformOp::ToInt;
        let result = op.apply(TransformValue::String("42".to_string())).unwrap();
        assert_eq!(result, TransformValue::Int(42));
    }

    #[test]
    fn test_scale() {
        let op = TransformOp::Scale { factor: 1000.0 };
        let result = op.apply(TransformValue::Float(2.5)).unwrap();
        assert_eq!(result, TransformValue::Float(2500.0));
    }

    #[test]
    fn test_offset() {
        let op = TransformOp::Offset { value: -273.15 };
        let result = op.apply(TransformValue::Float(300.0)).unwrap();
        match result {
            TransformValue::Float(f) => assert!((f - 26.85).abs() < 1e-10),
            _ => panic!("Expected Float variant"),
        }
    }

    #[test]
    fn test_regex_extract() {
        let op = TransformOp::RegexExtract {
            pattern: r"VOLT\s+(\d+\.\d+)".to_string(),
            group: 1,
            compiled: None,
        };
        let result = op
            .apply(TransformValue::String("VOLT 12.34".to_string()))
            .unwrap();
        assert_eq!(result, TransformValue::String("12.34".to_string()));
    }

    #[test]
    fn test_split_comma() {
        let op = TransformOp::SplitComma { index: 1 };
        let result = op
            .apply(TransformValue::String("1.0, 2.5, 3.7".to_string()))
            .unwrap();
        assert_eq!(result, TransformValue::String("2.5".to_string()));
    }

    #[test]
    fn test_split_comma_out_of_range() {
        let op = TransformOp::SplitComma { index: 5 };
        let result = op.apply(TransformValue::String("a,b,c".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_temperature() {
        let pipeline = TransformPipeline::new(vec![
            TransformOp::Trim,
            TransformOp::RemoveSuffix {
                suffix: "C".to_string(),
            },
            TransformOp::ToFloat,
        ])
        .unwrap();
        let result = pipeline.execute(" 25.5C ").unwrap();
        assert_eq!(result.as_f64().unwrap(), 25.5);
    }

    #[test]
    fn test_pipeline_voltage_with_offset() {
        let pipeline = TransformPipeline::new(vec![
            TransformOp::RegexExtract {
                pattern: r"VOLT\s+([+-]?\d+\.\d+)".to_string(),
                group: 1,
                compiled: None,
            },
            TransformOp::ToFloat,
            TransformOp::Scale { factor: 1000.0 },
            TransformOp::Offset { value: 100.0 },
        ])
        .unwrap();
        let result = pipeline.execute("VOLT 2.5").unwrap();
        assert_eq!(result.as_f64().unwrap(), 2600.0);
    }

    #[test]
    fn test_shorthand_trim() {
        let op = TransformOp::from_shorthand("trim").unwrap();
        assert!(matches!(op, TransformOp::Trim));
    }

    #[test]
    fn test_shorthand_remove_suffix() {
        let op = TransformOp::from_shorthand("remove_suffix('C')").unwrap();
        match op {
            TransformOp::RemoveSuffix { suffix } => assert_eq!(suffix, "C"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_shorthand_scale() {
        let op = TransformOp::from_shorthand("scale(1000.0)").unwrap();
        match op {
            TransformOp::Scale { factor } => assert_eq!(factor, 1000.0),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_shorthand_split_comma() {
        let op = TransformOp::from_shorthand("split_comma(2)").unwrap();
        match op {
            TransformOp::SplitComma { index } => assert_eq!(index, 2),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_shorthand_regex_extract() {
        let op = TransformOp::from_shorthand("regex_extract('VOLT\\s+(\\d+)', 1)").unwrap();
        match op {
            TransformOp::RegexExtract {
                pattern,
                group,
                compiled,
            } => {
                assert_eq!(pattern, r"VOLT\s+(\d+)");
                assert_eq!(group, 1);
                assert!(compiled.is_some(), "regex should be pre-compiled");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_pipeline_from_shorthand() {
        let pipeline = TransformPipeline::from_shorthand(&[
            "trim".to_string(),
            "remove_suffix('C')".to_string(),
            "to_float".to_string(),
        ])
        .unwrap();
        let result = pipeline.execute(" 25.5C ").unwrap();
        assert_eq!(result.as_f64().unwrap(), 25.5);
    }

    #[test]
    fn test_type_conversion_int_to_float() {
        let op = TransformOp::ToFloat;
        let result = op.apply(TransformValue::Int(42)).unwrap();
        assert_eq!(result, TransformValue::Float(42.0));
    }

    #[test]
    fn test_type_conversion_float_to_int() {
        let op = TransformOp::ToInt;
        let result = op.apply(TransformValue::Float(42.7)).unwrap();
        assert_eq!(result, TransformValue::Int(43));
    }

    #[test]
    fn test_scale_with_int() {
        let op = TransformOp::Scale { factor: 2.5 };
        let result = op.apply(TransformValue::Int(10)).unwrap();
        assert_eq!(result, TransformValue::Float(25.0));
    }

    #[test]
    fn test_empty_pipeline() {
        let pipeline = TransformPipeline::new(vec![]).unwrap();
        let result = pipeline.execute("hello").unwrap();
        assert_eq!(result.as_string(), "hello");
    }

    #[test]
    fn test_error_invalid_float() {
        let op = TransformOp::ToFloat;
        let result = op.apply(TransformValue::String("not a number".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_error_regex_no_match() {
        let op = TransformOp::RegexExtract {
            pattern: r"VOLT\s+(\d+)".to_string(),
            group: 1,
            compiled: None,
        };
        let result = op.apply(TransformValue::String("CURRENT 5".to_string()));
        assert!(result.is_err());
    }
}
