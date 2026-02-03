//! Declarative response transformation pipeline.
//!
//! This module provides a pipeline for transforming raw device responses into
//! typed values without requiring Rhai scripts for common cases.
//!
//! # Example
//!
//! ```rust
//! use driver_generic::transform::{TransformPipeline, TransformOp, TransformValue};
//!
//! let pipeline = TransformPipeline::new(vec![
//!     TransformOp::Trim,
//!     TransformOp::RemoveSuffix { suffix: "C".to_string() },
//!     TransformOp::ToFloat,
//! ]);
//!
//! let result = pipeline.execute(" 25.5C ").unwrap();
//! assert_eq!(result.as_f64().unwrap(), 25.5);
//! ```

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// A single transformation operation that can be applied to a value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TransformOp {
    /// Remove leading and trailing whitespace
    Trim,
    /// Remove a prefix if present
    RemovePrefix { prefix: String },
    /// Remove a suffix if present
    RemoveSuffix { suffix: String },
    /// Extract a regex capture group
    RegexExtract { pattern: String, group: usize },
    /// Parse as floating point number
    ToFloat,
    /// Parse as integer
    ToInt,
    /// Multiply by a scaling factor
    Scale { factor: f64 },
    /// Add an offset value
    Offset { value: f64 },
}

impl TransformOp {
    /// Apply this transformation operation to a value.
    pub fn apply(&self, input: TransformValue) -> Result<TransformValue> {
        match self {
            TransformOp::Trim => match input {
                TransformValue::String(s) => Ok(TransformValue::String(s.trim().to_string())),
                other => Ok(other), // No-op for non-strings
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
            TransformOp::RegexExtract { pattern, group } => match input {
                TransformValue::String(s) => {
                    let regex = Regex::new(pattern).context("Failed to compile regex pattern")?;
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
                TransformValue::Int(i) => Ok(TransformValue::Float(i as f64)),
            },
            TransformOp::ToInt => match input {
                TransformValue::String(s) => Ok(TransformValue::Int(
                    s.parse().context("Failed to parse int")?,
                )),
                TransformValue::Int(i) => Ok(TransformValue::Int(i)),
                TransformValue::Float(f) => Ok(TransformValue::Int(f.round() as i64)),
            },
            TransformOp::Scale { factor } => match input {
                TransformValue::Float(f) => Ok(TransformValue::Float(f * factor)),
                TransformValue::Int(i) => Ok(TransformValue::Float((i as f64) * factor)),
                _ => Err(anyhow!("Scale can only be applied to numeric values")),
            },
            TransformOp::Offset { value } => match input {
                TransformValue::Float(f) => Ok(TransformValue::Float(f + value)),
                TransformValue::Int(i) => Ok(TransformValue::Float((i as f64) + value)),
                _ => Err(anyhow!("Offset can only be applied to numeric values")),
            },
        }
    }

    /// Parse a transform operation from shorthand string syntax.
    ///
    /// Supported formats:
    /// - `"trim"` → `TransformOp::Trim`
    /// - `"remove_prefix('X')"` → `TransformOp::RemovePrefix { prefix: "X" }`
    /// - `"remove_suffix('C')"` → `TransformOp::RemoveSuffix { suffix: "C" }`
    /// - `"regex_extract('pattern', 1)"` → `TransformOp::RegexExtract { pattern, group }`
    /// - `"to_float"` → `TransformOp::ToFloat`
    /// - `"to_int"` → `TransformOp::ToInt`
    /// - `"scale(1000.0)"` → `TransformOp::Scale { factor: 1000.0 }`
    /// - `"offset(-273.15)"` → `TransformOp::Offset { value: -273.15 }`
    pub fn from_shorthand(s: &str) -> Result<Self> {
        let s = s.trim();

        // Simple operations without arguments
        match s {
            "trim" => return Ok(TransformOp::Trim),
            "to_float" => return Ok(TransformOp::ToFloat),
            "to_int" => return Ok(TransformOp::ToInt),
            _ => {}
        }

        // Operations with arguments
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
            return Ok(TransformOp::RegexExtract { pattern, group });
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

        Err(anyhow!("Unknown transform operation: {}", s))
    }
}

/// Parse a string argument from shorthand syntax (handles quoted strings).
/// Split comma-separated arguments while respecting quoted strings.
/// Handles both single and double quotes.
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

    // Add final argument
    if !current.trim().is_empty() {
        args.push(current.trim().to_string());
    }

    if in_quotes {
        return Err(anyhow!("Unterminated quote in argument list"));
    }

    Ok(args)
}

fn parse_string_arg(s: &str) -> Result<String> {
    let s = s.trim();
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        // Check length to prevent panic on single-char inputs like just a quote
        if s.len() >= 2 {
            Ok(s[1..s.len() - 1].to_string())
        } else {
            // Single quote or double quote alone - treat as literal
            Ok(s.to_string())
        }
    } else {
        Ok(s.to_string())
    }
}

/// Parse regex_extract arguments: pattern and group number.
/// Handles quoted strings that may contain commas.
fn parse_regex_extract_args(s: &str) -> Result<(String, usize)> {
    // Expected format: "'pattern', 1" or "\"pattern\", 1"
    // Use quote-aware splitting to handle commas inside patterns
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

/// A value that can be transformed through the pipeline.
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
            TransformValue::Int(i) => Some(*i as f64),
            TransformValue::String(_) => None,
        }
    }

    /// Convert to i64 if possible.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            TransformValue::Int(i) => Some(*i),
            TransformValue::Float(f) => Some(f.round() as i64),
            TransformValue::String(_) => None,
        }
    }

    /// Convert to string.
    pub fn as_string(&self) -> String {
        match self {
            TransformValue::String(s) => s.clone(),
            TransformValue::Float(f) => f.to_string(),
            TransformValue::Int(i) => i.to_string(),
        }
    }
}

/// A pipeline of transformation operations.
#[derive(Debug, Clone)]
pub struct TransformPipeline {
    ops: Vec<TransformOp>,
}

impl TransformPipeline {
    /// Create a new transform pipeline from a sequence of operations.
    pub fn new(ops: Vec<TransformOp>) -> Self {
        Self { ops }
    }

    /// Create a pipeline from shorthand string syntax.
    ///
    /// # Example
    ///
    /// ```
    /// use driver_generic::transform::TransformPipeline;
    ///
    /// let pipeline = TransformPipeline::from_shorthand(&[
    ///     "trim".to_string(),
    ///     "remove_suffix('C')".to_string(),
    ///     "to_float".to_string(),
    /// ]).unwrap();
    /// ```
    pub fn from_shorthand(shorthand: &[String]) -> Result<Self> {
        let ops = shorthand
            .iter()
            .map(|s| TransformOp::from_shorthand(s))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::new(ops))
    }

    /// Execute the pipeline on an input string.
    ///
    /// Each operation is applied in sequence, with the output of one
    /// becoming the input to the next.
    pub fn execute(&self, input: &str) -> Result<TransformValue> {
        let mut value = TransformValue::String(input.to_string());
        for op in &self.ops {
            value = op
                .apply(value)
                .context(format!("Failed to apply {:?}", op))?;
        }
        Ok(value)
    }

    /// Check if this pipeline is empty.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Test assertions use exact float comparisons for simplicity
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
        // Use approximate comparison for floating point
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
        };
        let result = op
            .apply(TransformValue::String("VOLT 12.34".to_string()))
            .unwrap();
        assert_eq!(result, TransformValue::String("12.34".to_string()));
    }

    #[test]
    fn test_pipeline_temperature() {
        let pipeline = TransformPipeline::new(vec![
            TransformOp::Trim,
            TransformOp::RemoveSuffix {
                suffix: "C".to_string(),
            },
            TransformOp::ToFloat,
        ]);

        let result = pipeline.execute(" 25.5C ").unwrap();
        assert_eq!(result.as_f64().unwrap(), 25.5);
    }

    #[test]
    fn test_pipeline_voltage_with_offset() {
        let pipeline = TransformPipeline::new(vec![
            TransformOp::RegexExtract {
                pattern: r"VOLT\s+([+-]?\d+\.\d+)".to_string(),
                group: 1,
            },
            TransformOp::ToFloat,
            TransformOp::Scale { factor: 1000.0 },
            TransformOp::Offset { value: 100.0 },
        ]);

        let result = pipeline.execute("VOLT 2.5").unwrap();
        assert_eq!(result.as_f64().unwrap(), 2600.0); // (2.5 * 1000) + 100
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
    fn test_shorthand_regex_extract() {
        let op = TransformOp::from_shorthand("regex_extract('VOLT\\s+(\\d+)', 1)").unwrap();
        match op {
            TransformOp::RegexExtract { pattern, group } => {
                assert_eq!(pattern, r"VOLT\s+(\d+)");
                assert_eq!(group, 1);
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
        assert_eq!(result, TransformValue::Int(43)); // Rounds
    }

    #[test]
    fn test_scale_with_int() {
        let op = TransformOp::Scale { factor: 2.5 };
        let result = op.apply(TransformValue::Int(10)).unwrap();
        assert_eq!(result, TransformValue::Float(25.0));
    }

    #[test]
    fn test_offset_with_int() {
        let op = TransformOp::Offset { value: 10.0 };
        let result = op.apply(TransformValue::Int(5)).unwrap();
        assert_eq!(result, TransformValue::Float(15.0));
    }

    #[test]
    fn test_empty_pipeline() {
        let pipeline = TransformPipeline::new(vec![]);
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
        };
        let result = op.apply(TransformValue::String("CURRENT 5".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_error_regex_invalid_group() {
        let op = TransformOp::RegexExtract {
            pattern: r"VOLT\s+(\d+)".to_string(),
            group: 5, // Group doesn't exist
        };
        let result = op.apply(TransformValue::String("VOLT 42".to_string()));
        assert!(result.is_err());
    }
}
