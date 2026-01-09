//! Custom validation functions for device configuration.
//!
//! This module provides validators for:
//! - Regex patterns (valid regex syntax)
//! - Evalexpr formulas (valid expression syntax)
//! - Additional custom validation rules

use serde_valid::validation::Error as ValidationError;

/// Validate that a string is a valid regex pattern.
///
/// This validator is used by `serde_valid` on fields marked with
/// `#[validate(custom(validate_regex_pattern))]`.
///
/// # Arguments
///
/// * `pattern` - Optional regex pattern string to validate
///
/// # Returns
///
/// * `Ok(())` if pattern is None or a valid regex
/// * `Err(ValidationError)` if pattern is invalid
pub fn validate_regex_pattern(pattern: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref p) = pattern {
        validate_regex_string(p)?;
    }
    Ok(())
}

/// Validate that a string is a valid regex pattern (non-optional version).
///
/// # Arguments
///
/// * `pattern` - Regex pattern string to validate
///
/// # Returns
///
/// * `Ok(())` if pattern is a valid regex
/// * `Err(ValidationError)` if pattern is invalid
pub fn validate_regex_string(pattern: &str) -> Result<(), ValidationError> {
    match regex::Regex::new(pattern) {
        Ok(_) => Ok(()),
        Err(e) => Err(ValidationError::Custom(format!(
            "Invalid regex pattern '{}': {}",
            pattern, e
        ))),
    }
}

/// Validate that a string is a valid evalexpr formula.
///
/// This validator is used by `serde_valid` on fields marked with
/// `#[validate(custom(validate_evalexpr_formula))]`.
///
/// The validator checks basic syntax by attempting to build an expression tree.
/// Note that variable references are not validated here - they are resolved
/// at runtime when the formula is evaluated.
///
/// # Arguments
///
/// * `formula` - The formula string to validate
///
/// # Returns
///
/// * `Ok(())` if formula has valid syntax
/// * `Err(ValidationError)` if formula has syntax errors
pub fn validate_evalexpr_formula(formula: &String) -> Result<(), ValidationError> {
    // evalexpr::build_operator_tree validates syntax without needing values
    match evalexpr::build_operator_tree(formula) {
        Ok(_) => Ok(()),
        Err(e) => Err(ValidationError::Custom(format!(
            "Invalid formula '{}': {}",
            formula, e
        ))),
    }
}

/// Validate baud rate is within acceptable range.
///
/// Standard baud rates: 300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600
pub fn validate_baud_rate(baud_rate: u32) -> Result<(), ValidationError> {
    const MIN_BAUD: u32 = 300;
    const MAX_BAUD: u32 = 921600;

    if !(MIN_BAUD..=MAX_BAUD).contains(&baud_rate) {
        return Err(ValidationError::Custom(format!(
            "Baud rate {} is out of range ({}-{})",
            baud_rate, MIN_BAUD, MAX_BAUD
        )));
    }
    Ok(())
}

/// Validate timeout is within acceptable range.
pub fn validate_timeout_ms(timeout_ms: u32) -> Result<(), ValidationError> {
    const MIN_TIMEOUT: u32 = 1;
    const MAX_TIMEOUT: u32 = 60000;

    if !(MIN_TIMEOUT..=MAX_TIMEOUT).contains(&timeout_ms) {
        return Err(ValidationError::Custom(format!(
            "Timeout {} ms is out of range ({}-{} ms)",
            timeout_ms, MIN_TIMEOUT, MAX_TIMEOUT
        )));
    }
    Ok(())
}

/// Validate that a numeric range is valid (min <= max).
pub fn validate_range(range: &Option<(f64, f64)>) -> Result<(), ValidationError> {
    if let Some((min, max)) = range {
        if min > max {
            return Err(ValidationError::Custom(format!(
                "Invalid range: min ({}) is greater than max ({})",
                min, max
            )));
        }
    }
    Ok(())
}

/// Validate a device configuration completely.
///
/// This performs additional cross-field validation beyond what
/// serde_valid provides per-field.
pub fn validate_device_config(
    config: &super::schema::DeviceConfig,
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    // Validate all regex patterns in responses
    for (name, response) in &config.responses {
        if let Some(ref pattern) = response.pattern {
            if let Err(e) = validate_regex_string(pattern) {
                errors.push(ValidationError::Custom(format!(
                    "Response '{}': {}",
                    name, e
                )));
            }
        }
    }

    // Validate all conversion formulas
    for (name, conversion) in &config.conversions {
        if let Err(e) = validate_evalexpr_formula(&conversion.formula) {
            errors.push(ValidationError::Custom(format!(
                "Conversion '{}': {}",
                name, e
            )));
        }
    }

    // Validate parameter ranges
    for (name, param) in &config.parameters {
        if let Err(e) = validate_range(&param.range) {
            errors.push(ValidationError::Custom(format!(
                "Parameter '{}': {}",
                name, e
            )));
        }
    }

    // Validate command references
    for (name, mapping) in &config.trait_mapping {
        for (method_name, method) in &mapping.methods {
            // Check command exists if specified (polling-only methods may not have a command)
            if let Some(ref command) = method.command {
                if !config.commands.contains_key(command) {
                    errors.push(ValidationError::Custom(format!(
                        "Trait mapping '{}.{}': references non-existent command '{}'",
                        name, method_name, command
                    )));
                }
            }

            // Check conversion exists if referenced
            if let Some(ref conv) = method.input_conversion {
                if !config.conversions.contains_key(conv) {
                    errors.push(ValidationError::Custom(format!(
                        "Trait mapping '{}.{}': references non-existent conversion '{}'",
                        name, method_name, conv
                    )));
                }
            }

            if let Some(ref conv) = method.output_conversion {
                if !config.conversions.contains_key(conv) {
                    errors.push(ValidationError::Custom(format!(
                        "Trait mapping '{}.{}': references non-existent conversion '{}'",
                        name, method_name, conv
                    )));
                }
            }
        }
    }

    // Validate command response references
    for (name, cmd) in &config.commands {
        if let Some(ref response_name) = cmd.response {
            if !config.responses.contains_key(response_name) {
                errors.push(ValidationError::Custom(format!(
                    "Command '{}': references non-existent response '{}'",
                    name, response_name
                )));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_regex() {
        let valid_patterns = vec![
            r"^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8})$",
            r".*",
            r"\d+",
            r"[a-z]+",
            r"^hello$",
        ];

        for pattern in valid_patterns {
            assert!(
                validate_regex_string(pattern).is_ok(),
                "Pattern should be valid: {}",
                pattern
            );
        }
    }

    #[test]
    fn test_invalid_regex() {
        let invalid_patterns = vec![
            r"[",        // Unclosed bracket
            r"(?P<>x)",  // Empty group name
            r"(abc",     // Unclosed paren
            r"*abc",     // Nothing to repeat
            r"[z-a]",    // Invalid range
        ];

        for pattern in invalid_patterns {
            assert!(
                validate_regex_string(pattern).is_err(),
                "Pattern should be invalid: {}",
                pattern
            );
        }
    }

    #[test]
    fn test_valid_formula() {
        let valid_formulas = vec![
            "x * 2".to_string(),
            "round(degrees * pulses_per_degree)".to_string(),
            "pulses / pulses_per_degree".to_string(),
            "1 + 2 * 3".to_string(),
            "abs(-5)".to_string(),
            "floor(3.7)".to_string(),
            "ceil(2.1)".to_string(),
            "a + b - c".to_string(),
        ];

        for formula in valid_formulas {
            assert!(
                validate_evalexpr_formula(&formula).is_ok(),
                "Formula should be valid: {}",
                formula
            );
        }
    }

    #[test]
    fn test_invalid_formula() {
        // Note: evalexpr is VERY permissive - it accepts many unusual expressions
        // "1 +" is 1 (unary +), "1 + +" is also valid
        // Use formulas that actually fail the parser
        let invalid_formulas = vec![
            "round(".to_string(),    // Unclosed parenthesis
            "((1 + 2)".to_string(),  // Mismatched parentheses
            ")1 + 2(".to_string(),   // Reversed parentheses
        ];

        for formula in invalid_formulas {
            assert!(
                validate_evalexpr_formula(&formula).is_err(),
                "Formula should be invalid: {}",
                formula
            );
        }
    }

    #[test]
    fn test_baud_rate_validation() {
        // Valid baud rates
        assert!(validate_baud_rate(9600).is_ok());
        assert!(validate_baud_rate(115200).is_ok());
        assert!(validate_baud_rate(300).is_ok());
        assert!(validate_baud_rate(921600).is_ok());

        // Invalid baud rates
        assert!(validate_baud_rate(0).is_err());
        assert!(validate_baud_rate(299).is_err());
        assert!(validate_baud_rate(1000000).is_err());
    }

    #[test]
    fn test_timeout_validation() {
        // Valid timeouts
        assert!(validate_timeout_ms(1).is_ok());
        assert!(validate_timeout_ms(1000).is_ok());
        assert!(validate_timeout_ms(60000).is_ok());

        // Invalid timeouts
        assert!(validate_timeout_ms(0).is_err());
        assert!(validate_timeout_ms(60001).is_err());
    }

    #[test]
    fn test_range_validation() {
        // Valid ranges
        assert!(validate_range(&Some((0.0, 100.0))).is_ok());
        assert!(validate_range(&Some((0.0, 0.0))).is_ok()); // min == max is valid
        assert!(validate_range(&None).is_ok());

        // Invalid ranges
        assert!(validate_range(&Some((100.0, 0.0))).is_err()); // min > max
    }

    #[test]
    fn test_optional_regex_validation() {
        assert!(validate_regex_pattern(&None).is_ok());
        assert!(validate_regex_pattern(&Some(r"\d+".to_string())).is_ok());
        assert!(validate_regex_pattern(&Some(r"[".to_string())).is_err());
    }
}
