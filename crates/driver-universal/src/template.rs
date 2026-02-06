//! MiniJinja template engine with custom filters for device commands.
//!
//! Provides a template engine with filters useful for instrument protocols:
//! - `hex(width)` - format integer as uppercase hex with zero-padding
//! - `pad(width, char)` - pad a string to a given width

use crate::config::error::ConfigError;
use crate::config::validated::ValidatedTemplate;
use minijinja::Environment;
use minijinja::Value;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Get or create the shared MiniJinja environment with custom filters.
fn get_env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Environment::new();

        // hex(width) filter: format integer as uppercase hex with zero-padding
        env.add_filter("hex", hex_filter);

        // pad(width, char) filter: pad string to given width
        env.add_filter("pad", pad_filter);

        env
    })
}

/// MiniJinja filter: format value as uppercase hex with zero-padding.
///
/// Usage: `{{ value | hex(8) }}` -> "0000A1B3"
///
/// MiniJinja automatically converts the Value to i64 for us.
fn hex_filter(value: i64, width: usize) -> String {
    format!("{:0>width$X}", value, width = width)
}

/// MiniJinja filter: pad string to given width with a fill character.
///
/// Usage: `{{ value | pad(5, "0") }}` -> "00042"
fn pad_filter(value: String, width: usize, fill: Option<String>) -> String {
    let fill_char = fill.and_then(|s| s.chars().next()).unwrap_or(' ');
    if value.len() >= width {
        value
    } else {
        let padding = width - value.len();
        let pad_str: String = std::iter::repeat_n(fill_char, padding).collect();
        format!("{pad_str}{value}")
    }
}

/// Validate that a template string parses correctly under MiniJinja.
///
/// # Errors
///
/// Returns `ConfigError::InvalidTemplate` if the template has syntax errors.
pub fn validate_template(name: &str, source: &str) -> Result<ValidatedTemplate, ConfigError> {
    let env = get_env();
    // Try to compile the template to check for syntax errors
    env.template_from_str(source)
        .map_err(|e| ConfigError::InvalidTemplate {
            name: name.to_string(),
            reason: e.to_string(),
        })?;

    Ok(ValidatedTemplate {
        source: source.to_string(),
    })
}

/// Render a validated command template with the given parameters.
///
/// # Errors
///
/// Returns an error if rendering fails (e.g., missing variables).
pub fn render_command(
    template: &ValidatedTemplate,
    params: &HashMap<String, JsonValue>,
) -> anyhow::Result<String> {
    let env = get_env();
    let tmpl = env.template_from_str(&template.source)?;

    // Convert serde_json::Value params to minijinja::Value
    let ctx = json_map_to_minijinja(params);

    let rendered = tmpl.render(ctx)?;
    Ok(rendered)
}

/// Convert a `HashMap<String, serde_json::Value>` to a `minijinja::Value` context.
fn json_map_to_minijinja(map: &HashMap<String, JsonValue>) -> Value {
    let converted: HashMap<String, Value> = map
        .iter()
        .map(|(k, v)| (k.clone(), json_to_minijinja(v)))
        .collect();
    Value::from_serialize(&converted)
}

/// Convert a single `serde_json::Value` to a `minijinja::Value`.
fn json_to_minijinja(value: &JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::from(()),
        JsonValue::Bool(b) => Value::from(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(f) = n.as_f64() {
                Value::from(f)
            } else {
                Value::from(n.to_string())
            }
        }
        JsonValue::String(s) => Value::from(s.as_str()),
        JsonValue::Array(arr) => {
            let items: Vec<Value> = arr.iter().map(json_to_minijinja).collect();
            Value::from(items)
        }
        JsonValue::Object(obj) => {
            let map: HashMap<String, Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_minijinja(v)))
                .collect();
            Value::from_serialize(&map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_good_template() {
        let result = validate_template("test", "{{ address }}gp");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_template_with_filter() {
        let result = validate_template("test", "{{ address }}ma{{ pulses | hex(8) }}");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_bad_template() {
        let result = validate_template("test", "{{ unclosed");
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::InvalidTemplate { name, .. } => assert_eq!(name, "test"),
            other => panic!("expected InvalidTemplate, got: {other}"),
        }
    }

    #[test]
    fn render_simple_command() {
        let tmpl = validate_template("test", "{{ address }}gp").unwrap();
        let mut params = HashMap::new();
        params.insert("address".to_string(), JsonValue::String("2".to_string()));
        let result = render_command(&tmpl, &params).unwrap();
        assert_eq!(result, "2gp");
    }

    #[test]
    fn render_with_hex_filter() {
        let tmpl = validate_template("test", "{{ address }}ma{{ position | hex(8) }}").unwrap();
        let mut params = HashMap::new();
        params.insert("address".to_string(), JsonValue::String("2".to_string()));
        params.insert(
            "position".to_string(),
            JsonValue::Number(serde_json::Number::from(0xA1B3)),
        );
        let result = render_command(&tmpl, &params).unwrap();
        assert_eq!(result, "2ma0000A1B3");
    }

    #[test]
    fn render_with_pad_filter() {
        let tmpl = validate_template("test", "{{ value | pad(5, \"0\") }}").unwrap();
        let mut params = HashMap::new();
        params.insert("value".to_string(), JsonValue::String("42".to_string()));
        let result = render_command(&tmpl, &params).unwrap();
        assert_eq!(result, "00042");
    }

    #[test]
    fn render_scpi_command() {
        let tmpl = validate_template("test", ":SOUR:VOLT {{ value }}").unwrap();
        let mut params = HashMap::new();
        params.insert(
            "value".to_string(),
            JsonValue::Number(serde_json::Number::from_f64(2.5).unwrap()),
        );
        let result = render_command(&tmpl, &params).unwrap();
        assert_eq!(result, ":SOUR:VOLT 2.5");
    }

    #[test]
    fn render_no_params() {
        let tmpl = validate_template("test", ":MEAS:VOLT?").unwrap();
        let params = HashMap::new();
        let result = render_command(&tmpl, &params).unwrap();
        assert_eq!(result, ":MEAS:VOLT?");
    }

    #[test]
    fn hex_filter_zero() {
        let tmpl = validate_template("test", "{{ v | hex(4) }}").unwrap();
        let mut params = HashMap::new();
        params.insert("v".to_string(), JsonValue::Number(0.into()));
        let result = render_command(&tmpl, &params).unwrap();
        assert_eq!(result, "0000");
    }

    #[test]
    fn hex_filter_large_value() {
        let tmpl = validate_template("test", "{{ v | hex(8) }}").unwrap();
        let mut params = HashMap::new();
        params.insert(
            "v".to_string(),
            JsonValue::Number(serde_json::Number::from(0x0FFF_FFFF_i64)),
        );
        let result = render_command(&tmpl, &params).unwrap();
        assert_eq!(result, "0FFFFFFF");
    }
}
