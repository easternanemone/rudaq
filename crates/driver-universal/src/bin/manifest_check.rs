//! `manifest-check` — validate a device manifest TOML file.
//!
//! Reads a manifest, runs the full validation pipeline, and pretty-prints
//! every error (collecting all of them, not just the first) with "did you
//! mean?" suggestions where the error context makes that cheap.
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
//! ERR bogus.toml — 2 errors:
//!   • unknown response reference: 'positon'
//!     hint: did you mean 'position'?
//!   • command 'get_pos' references unknown response 'positon'
//!     hint: did you mean response 'position'?
//! ```

use driver_universal::config::{ConfigError, RawManifest, parse_manifest};
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
                eprintln!("  • {}", display_error(err));
                if let Some(hint) = suggest(err, &known_responses) {
                    eprintln!("    hint: {hint}");
                }
            }
            ExitCode::from(1)
        }
    }
}

/// Format a `ConfigError` for CLI output, unwrapping the double-quoted
/// `unknown response reference: '<sentence>'` shape that the cross-validator
/// currently produces — otherwise the full sentence appears nested inside
/// the variant's `{0}` display, which is hard to read.
fn display_error(err: &ConfigError) -> String {
    if let ConfigError::UnknownResponse(inner) = err
        && inner.contains("references unknown response")
    {
        return inner.clone();
    }
    err.to_string()
}

/// Attempt to produce a "did you mean?" hint for an error.
///
/// The `UnknownResponse` variant from `parse_manifest` currently carries
/// either a bare name or a full `"command 'X' references unknown response 'Y'"`
/// sentence; handle both shapes. `Other` can also carry the sentence.
fn suggest(err: &ConfigError, known_responses: &[String]) -> Option<String> {
    let payload = match err {
        ConfigError::UnknownResponse(s) | ConfigError::Other(s) => s.as_str(),
        _ => return None,
    };
    let name = extract_unknown_response_name(payload).unwrap_or(payload);
    closest(name, known_responses).map(|s| format!("did you mean '{s}'?"))
}

/// Pull the `NAME` out of `"... references unknown response 'NAME'"` if present.
fn extract_unknown_response_name(msg: &str) -> Option<&str> {
    let tail = msg.split("references unknown response '").nth(1)?;
    // strip the single trailing quote
    tail.strip_suffix('\'').or(Some(tail))
}

/// Return the candidate within edit distance `max(3, target_len / 3)` of `target`.
/// Ties broken by shortest distance; returns `None` if no candidate qualifies.
fn closest(target: &str, candidates: &[String]) -> Option<String> {
    let threshold = 3usize.max(target.chars().count() / 3);
    candidates
        .iter()
        .map(|c| (c, edit_distance(target, c)))
        .filter(|(_, d)| *d <= threshold)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c.clone())
}

/// Classic Levenshtein edit distance (insertions, deletions, substitutions all cost 1).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_identical() {
        assert_eq!(edit_distance("position", "position"), 0);
    }

    #[test]
    fn edit_distance_single_substitution() {
        assert_eq!(edit_distance("positon", "position"), 1);
    }

    #[test]
    fn edit_distance_empty() {
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("", ""), 0);
    }

    #[test]
    fn closest_picks_nearest_within_threshold() {
        let cands = vec![
            "position".to_string(),
            "status".to_string(),
            "velocity".to_string(),
        ];
        assert_eq!(closest("positon", &cands).as_deref(), Some("position"));
        assert_eq!(closest("stauts", &cands).as_deref(), Some("status"));
    }

    #[test]
    fn closest_returns_none_when_far() {
        let cands = vec!["position".to_string(), "status".to_string()];
        // 'xyzzy' is too far from either candidate.
        assert_eq!(closest("xyzzy", &cands), None);
    }

    #[test]
    fn suggest_reads_embedded_sentence_form() {
        let cands = vec!["position".to_string(), "status".to_string()];
        let err = ConfigError::UnknownResponse(
            "command 'get_pos' references unknown response 'positon'".into(),
        );
        assert_eq!(
            suggest(&err, &cands),
            Some("did you mean 'position'?".to_string())
        );
    }

    #[test]
    fn suggest_reads_bare_name_form() {
        let cands = vec!["position".to_string()];
        let err = ConfigError::UnknownResponse("positon".into());
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
    fn display_error_unwraps_embedded_sentence() {
        let err = ConfigError::UnknownResponse(
            "command 'get_pos' references unknown response 'positon'".into(),
        );
        assert_eq!(
            display_error(&err),
            "command 'get_pos' references unknown response 'positon'"
        );
    }

    #[test]
    fn display_error_bare_name_uses_variant_display() {
        let err = ConfigError::UnknownResponse("positon".into());
        assert_eq!(display_error(&err), "unknown response reference: 'positon'");
    }
}
