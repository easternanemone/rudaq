//! Command templating using minijinja.
//!
//! This module provides template rendering for instrument commands using the
//! minijinja template engine. It supports both the new `{{ var }}` syntax and
//! legacy `{var}` syntax for backward compatibility with existing configurations.
//!
//! # Example
//!
//! ```rust,ignore
//! use std::collections::HashMap;
//! use hardware::plugin::templating::render_command;
//!
//! let mut context = HashMap::new();
//! context.insert("axis".to_string(), "X".to_string());
//! context.insert("val".to_string(), "10.5".to_string());
//!
//! // Works with legacy {var} syntax
//! let cmd = render_command("MOVE {axis} {val}", &context)?;
//! assert_eq!(cmd, "MOVE X 10.5");
//!
//! // Also works with minijinja {{ var }} syntax
//! let cmd = render_command("MOVE {{ axis }} {{ val }}", &context)?;
//! assert_eq!(cmd, "MOVE X 10.5");
//! ```

use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::OnceLock;

/// LEGACY: Converts strfmt `{var}` syntax to minijinja `{{ var }}` syntax.
///
/// Existing TOML/YAML device manifests may use the `{var}` placeholder format.
/// Remove after all manifests are verified to use `{{ var }}` syntax.
/// See docs/reference/deprecation-plan.md Section 3.1.
///
/// # Arguments
/// * `template` - Template string potentially containing `{var}` placeholders
///
/// # Returns
/// Template string with placeholders converted to minijinja format
///
/// # Example
/// ```rust,ignore
/// let converted = convert_legacy_template("SET {val}");
/// assert_eq!(converted, "SET {{ val }}");
/// ```
pub fn convert_legacy_template(template: &str) -> String {
    // Check if already using native minijinja syntax (with spaces: {{ var }})
    // Note: strfmt uses {{ and }} for escaping literal braces, so we need to
    // distinguish between "{{ var }}" (minijinja) and "{{" alone (strfmt escape)
    if template.contains("{{ ") || template.contains(" }}") {
        return template.to_string();
    }

    let mut result = String::with_capacity(template.len() * 2);
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            // Check for strfmt escape sequence {{ -> literal {
            if chars.peek() == Some(&'{') {
                chars.next(); // consume second {
                result.push('{');
                continue;
            }

            // Check if this is a placeholder
            let mut var_name = String::new();
            let mut found_close = false;

            for inner in chars.by_ref() {
                if inner == '}' {
                    found_close = true;
                    break;
                }
                var_name.push(inner);
            }

            if found_close && !var_name.is_empty() && is_valid_identifier(&var_name) {
                // Convert to minijinja syntax
                result.push_str("{{ ");
                result.push_str(&var_name);
                result.push_str(" }}");
            } else {
                // Not a valid placeholder, keep original
                result.push('{');
                result.push_str(&var_name);
                if found_close {
                    result.push('}');
                }
            }
        } else if c == '}' {
            // Check for strfmt escape sequence }} -> literal }
            if chars.peek() == Some(&'}') {
                chars.next(); // consume second }
                result.push('}');
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Checks if a string is a valid variable identifier.
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let first = s.chars().next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Renders a command template with the given context.
///
/// Supports both legacy `{var}` syntax (automatically converted) and
/// native minijinja `{{ var }}` syntax.
///
/// # Arguments
/// * `template` - Command template string
/// * `context` - HashMap of variable names to values
///
/// # Returns
/// * `Ok(String)` - Rendered command string
/// * `Err` - If template rendering fails
///
/// # Example
/// ```rust,ignore
/// let mut ctx = HashMap::new();
/// ctx.insert("val".to_string(), "42".to_string());
///
/// let result = render_command("SET {val}", &ctx)?;
/// assert_eq!(result, "SET 42");
/// ```
/// Static minijinja environment for template rendering.
/// Reused across calls to avoid repeated environment creation overhead.
static ENV: OnceLock<minijinja::Environment<'static>> = OnceLock::new();

fn get_env() -> &'static minijinja::Environment<'static> {
    ENV.get_or_init(|| {
        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        env
    })
}

pub fn render_command(template: &str, context: &HashMap<String, String>) -> Result<String> {
    // Convert legacy syntax if needed
    let converted = convert_legacy_template(template);

    let rendered = get_env()
        .render_str(&converted, context)
        .map_err(|e| anyhow!("Template rendering failed: {e}"))?;

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_legacy_single_var() {
        assert_eq!(convert_legacy_template("SET {val}"), "SET {{ val }}");
    }

    #[test]
    fn test_convert_legacy_multiple_vars() {
        assert_eq!(
            convert_legacy_template("MOVE {axis} {val}"),
            "MOVE {{ axis }} {{ val }}"
        );
    }

    #[test]
    fn test_convert_legacy_no_vars() {
        assert_eq!(convert_legacy_template("QUERY?"), "QUERY?");
    }

    #[test]
    fn test_convert_legacy_already_minijinja() {
        // Should not double-convert
        assert_eq!(convert_legacy_template("SET {{ val }}"), "SET {{ val }}");
    }

    #[test]
    fn test_convert_legacy_mixed_not_supported() {
        // If already has minijinja syntax, don't convert
        let input = "SET {{ val }} {other}";
        assert_eq!(convert_legacy_template(input), input);
    }

    #[test]
    fn test_render_command_legacy_syntax() {
        let mut ctx = HashMap::new();
        ctx.insert("val".to_string(), "42".to_string());

        let result = render_command("SET {val}", &ctx).unwrap();
        assert_eq!(result, "SET 42");
    }

    #[test]
    fn test_render_command_minijinja_syntax() {
        let mut ctx = HashMap::new();
        ctx.insert("val".to_string(), "42".to_string());

        let result = render_command("SET {{ val }}", &ctx).unwrap();
        assert_eq!(result, "SET 42");
    }

    #[test]
    fn test_render_command_multiple_vars() {
        let mut ctx = HashMap::new();
        ctx.insert("axis".to_string(), "X".to_string());
        ctx.insert("val".to_string(), "10.5".to_string());

        let result = render_command("MOVE {axis} {val}", &ctx).unwrap();
        assert_eq!(result, "MOVE X 10.5");
    }

    #[test]
    fn test_render_command_missing_var() {
        let ctx = HashMap::new();
        let result = render_command("SET {val}", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("val"));
        assert!(is_valid_identifier("axis"));
        assert!(is_valid_identifier("my_var"));
        assert!(is_valid_identifier("_private"));
        assert!(is_valid_identifier("var123"));

        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123"));
        assert!(!is_valid_identifier("my-var"));
        assert!(!is_valid_identifier("my.var"));
    }

    #[test]
    fn test_convert_legacy_strfmt_escaped_braces() {
        // strfmt uses {{ to escape a literal {
        assert_eq!(convert_legacy_template("{{literal}}"), "{literal}");
    }

    #[test]
    fn test_convert_legacy_strfmt_escaped_with_var() {
        // strfmt: {{ -> {, {var} -> {{ var }}, }} -> }
        assert_eq!(
            convert_legacy_template("prefix {{json: {val}}}"),
            "prefix {json: {{ val }}}"
        );
    }

    #[test]
    fn test_render_strfmt_escaped_braces() {
        let mut ctx = HashMap::new();
        ctx.insert("val".to_string(), "42".to_string());

        // Template with escaped braces and a variable
        let result = render_command("{{json: {val}}}", &ctx).unwrap();
        assert_eq!(result, "{json: 42}");
    }
}
