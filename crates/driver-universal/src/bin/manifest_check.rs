//! `manifest-check` — validate a device manifest TOML file.
//!
//! Reads a manifest, runs the full validation pipeline, and pretty-prints
//! every error (collecting all of them, not just the first) with "did you
//! mean?" suggestions where the error carries enough structural info.
//!
//! Exit codes:
//!   0 — manifest is valid
//!   1 — validation failed (one or more errors reported)
//!   2 — usage error (bad CLI args or I/O failure)
//!
//! # Example
//!
//! ```text
//! $ manifest-check config/devices/ipg_laser.toml
//! OK  config/devices/ipg_laser.toml — 8 commands, 7 responses, 5 parameters
//!
//! $ manifest-check bogus.toml
//! ERR bogus.toml — 1 error:
//!   • command 'get_pos' references unknown response 'positon'
//!     hint: did you mean 'position'?
//! ```

use driver_universal::config::{ConfigError, RawManifest, parse_manifest};
use driver_universal::string_utils::closest_within_threshold;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let path = match args.next() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: manifest-check <path.toml>");
            return ExitCode::from(2);
        }
    };

    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };

    let raw: RawManifest = match toml::from_str(&source) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERR {} — TOML parse failed:", path.display());
            for line in e.to_string().lines() {
                eprintln!("  {line}");
            }
            return ExitCode::from(1);
        }
    };

    // Capture known names for suggestions before consuming `raw`.
    let known_responses: Vec<String> = raw.responses.keys().cloned().collect();

    match parse_manifest(raw) {
        Ok(m) => {
            println!(
                "OK  {} — {} commands, {} responses, {} parameters",
                path.display(),
                m.commands.len(),
                m.responses.len(),
                m.parameters.len(),
            );
            ExitCode::SUCCESS
        }
        Err(errors) => {
            let count = errors.len();
            eprintln!(
                "ERR {} — {count} error{}:",
                path.display(),
                if count == 1 { "" } else { "s" }
            );
            for err in &errors {
                eprintln!("  • {err}");
                if let Some(hint) = suggest(err, &known_responses) {
                    eprintln!("    hint: {hint}");
                }
            }
            ExitCode::from(1)
        }
    }
}

/// Attempt to produce a "did you mean?" hint for an error.
///
/// Currently only fires on `UnknownResponse`. If other "referenced X not
/// found" variants appear, extend this match — the structured error
/// fields give us the typo'd name directly, no sentence parsing needed.
fn suggest(err: &ConfigError, known_responses: &[String]) -> Option<String> {
    match err {
        ConfigError::UnknownResponse { name, .. } => {
            closest_within_threshold(name, known_responses.iter().map(String::as_str))
                .map(|s| format!("did you mean '{s}'?"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_hints_for_typo_in_unknown_response() {
        let cands = vec!["position".to_string(), "status".to_string()];
        let err = ConfigError::UnknownResponse {
            command: "get_pos".into(),
            name: "positon".into(),
        };
        assert_eq!(
            suggest(&err, &cands),
            Some("did you mean 'position'?".to_string())
        );
    }

    #[test]
    fn suggest_returns_none_when_variant_irrelevant() {
        let cands = vec!["position".to_string()];
        let err = ConfigError::InvalidBaudRate(0);
        assert_eq!(suggest(&err, &cands), None);
    }

    #[test]
    fn suggest_returns_none_when_name_is_far_from_all() {
        let cands = vec!["position".to_string(), "status".to_string()];
        let err = ConfigError::UnknownResponse {
            command: "get_pos".into(),
            name: "xyzzy".into(),
        };
        assert_eq!(suggest(&err, &cands), None);
    }
}
