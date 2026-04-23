//! `export-manifest-schema` — print the JSON Schema for device-manifest TOML.
//!
//! The schema is the source of truth for editor tooling (VS Code
//! `even-better-toml`, JetBrains, etc.). Run this binary to emit the
//! schema to stdout, then point your editor at the generated file.
//!
//! # Example
//!
//! ```text
//! $ cargo run -q --bin export-manifest-schema > .vscode/manifest.schema.json
//! $ # Then in .vscode/settings.json:
//! $ #   "evenBetterToml.schema.associations": {
//! $ #     "config/devices/*.toml": ".vscode/manifest.schema.json"
//! $ #   }
//! ```
//!
//! The schema itself lives at `crates/driver-universal/schemas/manifest.schema.json`
//! and is embedded via `include_str!` so the binary always matches the
//! crate version.

use std::io::Write;
use std::process::ExitCode;

/// Embedded JSON Schema (Draft-07) describing the manifest TOML shape.
pub const MANIFEST_SCHEMA_JSON: &str = include_str!("../../schemas/manifest.schema.json");

fn main() -> ExitCode {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if let Err(e) = handle.write_all(MANIFEST_SCHEMA_JSON.as_bytes()) {
        eprintln!("error: failed to write schema to stdout: {e}");
        return ExitCode::from(2);
    }
    // Guarantee trailing newline so shell redirection produces a clean file.
    if !MANIFEST_SCHEMA_JSON.ends_with('\n') {
        let _ = handle.write_all(b"\n");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::MANIFEST_SCHEMA_JSON;
    use serde_json::Value;

    #[test]
    fn embedded_schema_is_valid_json() {
        let parsed: Value = serde_json::from_str(MANIFEST_SCHEMA_JSON)
            .expect("manifest.schema.json must be valid JSON");
        assert!(parsed.is_object(), "top-level schema must be a JSON object");
    }

    #[test]
    fn embedded_schema_declares_draft_07() {
        let parsed: Value = serde_json::from_str(MANIFEST_SCHEMA_JSON).unwrap();
        let schema_url = parsed
            .get("$schema")
            .and_then(Value::as_str)
            .expect("$schema keyword required");
        assert!(
            schema_url.contains("draft-07"),
            "expected draft-07 schema URL, got {schema_url}"
        );
    }

    #[test]
    fn embedded_schema_covers_required_top_level_fields() {
        let parsed: Value = serde_json::from_str(MANIFEST_SCHEMA_JSON).unwrap();
        let required = parsed
            .get("required")
            .and_then(Value::as_array)
            .expect("required array must be present");
        let required_names: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
        for field in &["schema_version", "device", "connection"] {
            assert!(
                required_names.contains(field),
                "top-level `required` array is missing '{field}'"
            );
        }
    }

    #[test]
    fn embedded_schema_defines_all_connection_variants() {
        let parsed: Value = serde_json::from_str(MANIFEST_SCHEMA_JSON).unwrap();
        let defs = parsed.get("$defs").expect("$defs required");
        for variant in &[
            "connection_serial",
            "connection_tcp",
            "connection_udp",
            "connection_usbtmc",
        ] {
            assert!(defs.get(variant).is_some(), "$defs is missing '{variant}'");
        }
    }

    #[test]
    fn embedded_schema_defines_inline_ui_blocks() {
        // bd-jcb4x G1 added [commands.X.ui] and [parameters.X.ui] inline
        // declarations. The schema must document both so editors autocomplete
        // the widget_hint / label / group / step / enum_values fields.
        let parsed: Value = serde_json::from_str(MANIFEST_SCHEMA_JSON).unwrap();
        let defs = parsed.get("$defs").expect("$defs required");

        // $defs.command_ui / $defs.parameter_ui must exist
        for variant in &["command_ui", "parameter_ui"] {
            let def = defs
                .get(variant)
                .unwrap_or_else(|| panic!("$defs is missing '{variant}'"));
            let widget_enum = def
                .get("properties")
                .and_then(|p| p.get("widget_hint"))
                .and_then(|w| w.get("enum"))
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{variant}.properties.widget_hint.enum missing"));
            let labels: Vec<&str> = widget_enum.iter().filter_map(Value::as_str).collect();
            for expected in &[
                "slider",
                "toggle",
                "enum_select",
                "numeric_input",
                "text_input",
                "button",
            ] {
                assert!(
                    labels.contains(expected),
                    "{variant}.widget_hint enum missing '{expected}'"
                );
            }
        }

        // The command schema must reference command_ui via a .ui property.
        let cmd_ui = parsed
            .get("$defs")
            .and_then(|v| v.get("command"))
            .and_then(|v| v.get("properties"))
            .and_then(|v| v.get("ui"))
            .expect("$defs.command.properties.ui missing");
        let reffed = cmd_ui.get("$ref").and_then(Value::as_str).unwrap_or("");
        assert!(
            reffed.ends_with("/command_ui"),
            "command.ui must $ref command_ui, got {reffed}"
        );
    }

    #[test]
    fn embedded_schema_describes_response_variants_field() {
        // F1 added `variants` as the preferred multi-format shape; make sure
        // the schema documents it so VS Code autocompletes it.
        let parsed: Value = serde_json::from_str(MANIFEST_SCHEMA_JSON).unwrap();
        let resp_props = parsed
            .get("$defs")
            .and_then(|v| v.get("response"))
            .and_then(|v| v.get("properties"))
            .expect("$defs.response.properties missing");
        assert!(
            resp_props.get("variants").is_some(),
            "response schema must describe the `variants` field"
        );
        assert!(
            resp_props.get("advanced").is_some(),
            "response schema must describe the `advanced` escape-hatch block"
        );
    }
}
