//! `migrate-v3` — convert v3 TOML manifests to v4 format.
//!
//! Reads a v3 device manifest, rewrites deprecated patterns, and writes the
//! result to stdout (or overwrites the file with `--in-place`). The tool is
//! conservative: patterns it can rewrite automatically get rewritten; anything
//! ambiguous is preserved with a `# TODO` comment so a human can decide.
//!
//! # Conversion rules
//!
//! 1. **Top-level `regex = "..."` in a response block.**
//!    * Simple prefix form — `^WORD:\s*(?P<field>.*)$` — is synthesized as a
//!      `variants = ["WORD: {field}"]` entry. The deprecated regex is dropped.
//!    * Anything else is moved into `[responses.X.advanced]` and annotated with
//!      `# TODO: consider replacing with variants = [...]`.
//!
//! 2. **`regex_extract('pattern', group)` inside a transform pipeline.**
//!    * If the pattern is a "prefix: value" shape — `^<word>:\s*(.*)` — it is
//!      replaced with `format('{cmd}: {value}', 'value')`. The command prefix
//!      at runtime fills `{cmd}`, so the format op is fully general.
//!    * Other patterns stay verbatim and the enclosing response gets a
//!      `# TODO` comment.
//!
//! 3. Every non-deprecated field passes through untouched.
//!
//! # Output & summary
//!
//! The rewritten TOML goes to stdout (or replaces the file with `--in-place`).
//! A one-line-per-change summary goes to stderr so CI logs can show exactly
//! what changed and what still needs human attention.
//!
//! # Limitations
//!
//! * The underlying `toml` crate rebuilds the file from a value tree, so
//!   original comments and blank lines are lost. Run the migration on a clean
//!   working tree and review the diff before committing.
//! * Only the two v3 patterns listed above are rewritten. Anything else v3 is
//!   parsed into the value tree and re-serialized unchanged.
//!
//! # CLI
//!
//! ```text
//! migrate-v3 <path.toml> [--in-place]
//! ```
//!
//! Exit codes: 0 success, 1 migration failure (parse/I/O error), 2 usage error.

use std::path::PathBuf;
use std::process::ExitCode;

use toml::Value;

/// Comment injected before a regex line moved into `[responses.X.advanced]`.
const ADVANCED_TODO: &str = "# TODO: consider replacing with variants = [...]";

/// Comment injected before `[responses.X]` when a non-trivial regex_extract
/// survived migration.
const TRANSFORM_TODO: &str =
    "# TODO: review regex_extract in transform — could not auto-convert to format(...)";

/// Bookkeeping for one migration run.
#[derive(Debug, Default)]
struct MigrationReport {
    /// Things we converted automatically (human-readable lines).
    auto_migrated: Vec<String>,
    /// Things that need manual review (human-readable lines).
    todos: Vec<String>,
    /// Response names whose regex was pushed into `.advanced` (need a comment
    /// injected before the `regex = ...` line inside that subtable).
    advanced_todo_responses: Vec<String>,
    /// Response names with an unconverted regex_extract (need a comment
    /// injected before the `[responses.X]` header).
    transform_todo_responses: Vec<String>,
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut path: Option<PathBuf> = None;
    let mut in_place = false;

    for arg in args.by_ref() {
        match arg.as_str() {
            "--in-place" => in_place = true,
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => {
                eprintln!("error: unknown flag '{other}'");
                print_usage();
                return ExitCode::from(2);
            }
            _ => {
                if path.is_some() {
                    eprintln!("error: unexpected extra argument '{arg}'");
                    print_usage();
                    return ExitCode::from(2);
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }

    let Some(path) = path else {
        print_usage();
        return ExitCode::from(2);
    };

    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };

    let (output, report) = match migrate_v3_to_v4(&source) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error: migration failed for {}: {e}", path.display());
            return ExitCode::from(1);
        }
    };

    print_summary(&path, &report);

    if in_place {
        if let Err(e) = std::fs::write(&path, &output) {
            eprintln!("error: cannot write {}: {e}", path.display());
            return ExitCode::from(1);
        }
        eprintln!("wrote {}", path.display());
    } else {
        print!("{output}");
    }

    ExitCode::SUCCESS
}

fn print_usage() {
    eprintln!("usage: migrate-v3 <path.toml> [--in-place]");
    eprintln!();
    eprintln!("Converts deprecated v3 manifest patterns (top-level response");
    eprintln!("`regex` and `regex_extract(...)` transforms) to v4 format.");
    eprintln!("Prints to stdout unless --in-place is given.");
}

fn print_summary(path: &std::path::Path, report: &MigrationReport) {
    eprintln!("migrate-v3: {}", path.display());
    if report.auto_migrated.is_empty() && report.todos.is_empty() {
        eprintln!("  (no v3 deprecated patterns found)");
        return;
    }
    if !report.auto_migrated.is_empty() {
        eprintln!("  auto-migrated ({}):", report.auto_migrated.len());
        for line in &report.auto_migrated {
            eprintln!("    + {line}");
        }
    }
    if !report.todos.is_empty() {
        eprintln!("  TODOs — manual review needed ({}):", report.todos.len());
        for line in &report.todos {
            eprintln!("    ! {line}");
        }
    }
}

// ---------------------------------------------------------------------------
// Migration core
// ---------------------------------------------------------------------------

/// Run the full migration on a TOML source string.
///
/// Returns the rewritten text (with `# TODO` comments injected where
/// applicable) and a summary of what was changed.
///
/// # Errors
///
/// Returns a stringified error if the input isn't valid TOML or if
/// re-serialization fails. Schema validation is deliberately skipped — the
/// whole point is to accept v3 manifests that the typed parser would reject.
fn migrate_v3_to_v4(source: &str) -> Result<(String, MigrationReport), String> {
    let mut value: Value =
        toml::from_str(source).map_err(|e: toml::de::Error| format!("parse: {e}"))?;
    let mut report = MigrationReport::default();

    // Mutate `[responses.*]` subtables in place. We cannot hold a mutable
    // borrow of `value` across the iteration AND across the serialization, but
    // that's fine — we finish mutating before serializing.
    if let Some(responses) = value.get_mut("responses").and_then(Value::as_table_mut) {
        // Collect response names upfront so we can iterate by key without
        // conflicting borrows when we need to mutate individual subtables.
        let names: Vec<String> = responses.keys().cloned().collect();
        for name in &names {
            let Some(resp) = responses.get_mut(name).and_then(Value::as_table_mut) else {
                continue;
            };
            migrate_response_regex(name, resp, &mut report);
            migrate_response_transform(name, resp, &mut report);
        }
    }

    let serialized = toml::to_string_pretty(&value).map_err(|e| format!("serialize: {e}"))?;
    let with_comments = inject_comments(&serialized, &report);
    Ok((with_comments, report))
}

/// Handle the top-level `regex = "..."` field inside one response table.
fn migrate_response_regex(resp_name: &str, resp: &mut toml::Table, report: &mut MigrationReport) {
    let Some(regex_val) = resp.remove("regex") else {
        return;
    };
    let Some(regex) = regex_val.as_str().map(str::to_string) else {
        // Shouldn't happen for valid TOML, but keep the value if it's not a
        // string — restore it and flag.
        resp.insert("regex".into(), regex_val);
        report.todos.push(format!(
            "responses.{resp_name}: non-string `regex` — left as-is"
        ));
        return;
    };

    match try_regex_to_variant(&regex) {
        Some(variant) => {
            // Replace with `variants = [variant]`.
            resp.insert(
                "variants".into(),
                Value::Array(vec![Value::String(variant.clone())]),
            );
            report.auto_migrated.push(format!(
                "responses.{resp_name}: regex -> variants = [\"{variant}\"]"
            ));
        }
        None => {
            // Move into [responses.X.advanced]. If an advanced block already
            // exists and has its own regex, we preserve the existing one and
            // report a TODO (unlikely in v3, but safe).
            let advanced = resp
                .entry("advanced".to_string())
                .or_insert_with(|| Value::Table(toml::Table::new()));
            let adv_tbl = advanced
                .as_table_mut()
                .expect("`advanced` slot should be a table");
            if adv_tbl.contains_key("regex") {
                report.todos.push(format!(
                    "responses.{resp_name}: both top-level and [advanced] regex present — kept [advanced]"
                ));
            } else {
                adv_tbl.insert("regex".into(), Value::String(regex));
            }
            report
                .todos
                .push(format!("responses.{resp_name}: regex moved to [advanced]"));
            report.advanced_todo_responses.push(resp_name.to_string());
        }
    }
}

/// Try to reduce a v3 regex into a v4 single format variant.
///
/// Returns `Some(variant)` if the regex matches a simple "prefix: value"
/// shape with one or more named capture groups and only "simple" literal
/// characters between them (letters, digits, `:`, `-`, `_`, space, etc.).
/// Returns `None` for anything we can't confidently invert — the caller will
/// push that to `[advanced]`.
fn try_regex_to_variant(regex: &str) -> Option<String> {
    // Only migrate anchored patterns. Unanchored regex like
    // `OFST,(?P<offset>...)V` would match a substring of the full response —
    // but a format variant must match the entire string. Moving those to
    // [advanced] keeps semantics intact.
    let inner = regex.strip_prefix('^')?;
    let inner = inner.strip_suffix('$').unwrap_or(inner);

    // We require at least one named capture group.
    if !inner.contains("(?P<") {
        return None;
    }

    // Walk the pattern and translate:
    //   (?P<name>...) -> {name}
    //   \s+ / \s*     -> single space
    //   regex escapes -> unescaped char
    // Reject any regex metachar we don't explicitly handle (to avoid
    // producing a format string that silently diverges from the regex).
    let mut out = String::new();
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '(' {
            // Named group?
            if chars.get(i + 1) == Some(&'?')
                && chars.get(i + 2) == Some(&'P')
                && chars.get(i + 3) == Some(&'<')
            {
                i += 4;
                let name_start = i;
                while i < chars.len() && chars[i] != '>' {
                    i += 1;
                }
                if i >= chars.len() {
                    return None;
                }
                let name: String = chars[name_start..i].iter().collect();
                i += 1; // consume '>'
                // Skip balanced parens (the group body — we don't care what
                // the capture matches, only what name to emit).
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    match chars[i] {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                out.push('{');
                out.push_str(&name);
                out.push('}');
                continue;
            }
            // Unnamed group — can't round-trip into format string.
            return None;
        }
        if c == '\\' {
            let next = *chars.get(i + 1)?;
            match next {
                's' => {
                    // \s, \s+, \s*  →  single space
                    i += 2;
                    if matches!(chars.get(i), Some('+') | Some('*')) {
                        i += 1;
                    }
                    out.push(' ');
                }
                'd' | 'w' | 'S' | 'W' | 'D' | 'b' | 'B' | '.' | '*' | '+' | '?' | '|' | '['
                | ']' | '(' | ')' | '{' | '}' | '^' | '$' => {
                    // Either a meta-class (which means "any", not a literal
                    // we can emit) or an escaped literal. For class shortcuts
                    // return None; for escaped literals, emit the char.
                    if matches!(next, 'd' | 'w' | 'S' | 'W' | 'D' | 'b' | 'B') {
                        return None;
                    }
                    out.push(next);
                    i += 2;
                }
                other => {
                    out.push(other);
                    i += 2;
                }
            }
            continue;
        }
        // Plain literal character. Reject unescaped regex metacharacters —
        // they imply we are NOT looking at a plain "prefix: value" shape.
        if matches!(c, '*' | '+' | '?' | '|' | '[' | ']' | '{' | '}' | '.') {
            return None;
        }
        out.push(c);
        i += 1;
    }

    // Sanity: must contain at least one `{...}` placeholder, and must not
    // start with the placeholder (a pure-capture regex means no literal
    // prefix — there's nothing descriptive to put in the variant).
    if !out.contains('{') || out.starts_with('{') {
        return None;
    }

    Some(out)
}

/// Handle `transform = [..., "regex_extract('pat', N)", ...]` lists.
fn migrate_response_transform(
    resp_name: &str,
    resp: &mut toml::Table,
    report: &mut MigrationReport,
) {
    let Some(Value::Array(arr)) = resp.get_mut("transform") else {
        return;
    };
    let mut response_has_todo = false;

    for op in arr.iter_mut() {
        let Some(op_str) = op.as_str() else { continue };
        let trimmed = op_str.trim();
        if !trimmed.starts_with("regex_extract") {
            continue;
        }
        match try_regex_extract_to_format(trimmed) {
            Some(replacement) => {
                report.auto_migrated.push(format!(
                    "responses.{resp_name}.transform: {trimmed} -> {replacement}"
                ));
                *op = Value::String(replacement);
            }
            None => {
                report.todos.push(format!(
                    "responses.{resp_name}.transform: kept {trimmed} — manual review"
                ));
                response_has_todo = true;
            }
        }
    }

    if response_has_todo {
        report.transform_todo_responses.push(resp_name.to_string());
    }
}

/// Parse `regex_extract('<pat>', <group>)`. Return `Some(replacement)` only
/// when the pattern is a plain "prefix: value" shape we can safely replace
/// with `format('{cmd}: {value}', 'value')`.
fn try_regex_extract_to_format(op: &str) -> Option<String> {
    // Extract the pattern argument (first single-quoted string).
    let bytes = op.as_bytes();
    let first_quote = bytes.iter().position(|&b| b == b'\'')?;
    let after = &op[first_quote + 1..];
    let close = after.find('\'')?;
    let pat = &after[..close];

    // Normalize: strip anchors, drop escape-slash runs that the TOML layer
    // already de-escaped for us. We're matching the *authored* pattern, so
    // `\s` is two characters: backslash + s.
    let inner = pat.trim_start_matches('^').trim_end_matches('$');

    // Must start with at least one word character, then `:`, then optional
    // whitespace, then a capture group covering "the rest".
    // Recognised shapes:
    //   <word>:\s*(.*)
    //   <word>:\s+(.*)
    //   <word>: (.*)   (literal space — common in SCPI-like replies)
    let word_end = inner.find(':')?;
    let word = &inner[..word_end];
    if word.is_empty() || !word.chars().all(is_plain_word_char) {
        return None;
    }
    let after_colon = &inner[word_end + 1..];

    // Accept an optional whitespace-run before the capture.
    let after_ws = after_colon
        .strip_prefix("\\s*")
        .or_else(|| after_colon.strip_prefix("\\s+"))
        .or_else(|| after_colon.strip_prefix(' '))
        .unwrap_or(after_colon);

    // The remainder must be a single unnamed capture covering the tail.
    if !(after_ws == "(.*)" || after_ws == "(.+)") {
        return None;
    }

    Some("format('{cmd}: {value}', 'value')".to_string())
}

/// A "plain" literal word character we're willing to leave in a migrated
/// format string without escaping gymnastics.
fn is_plain_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

// ---------------------------------------------------------------------------
// Comment injection
// ---------------------------------------------------------------------------

/// Inject `# TODO: ...` lines into the serialized TOML text.
///
/// We can't persist comments through `toml::Value`, so instead we rewrite the
/// output text by inserting comment lines at well-known anchors:
///   * before `regex = "..."` inside `[responses.<name>.advanced]`, and
///   * before the `[responses.<name>]` header itself (for transform TODOs).
fn inject_comments(serialized: &str, report: &MigrationReport) -> String {
    if report.advanced_todo_responses.is_empty() && report.transform_todo_responses.is_empty() {
        return serialized.to_string();
    }

    // Process line-by-line. For each line we emit, optionally prepend a
    // TODO comment with the same leading indentation.
    let mut out = String::with_capacity(serialized.len() + 256);
    let mut current_advanced: Option<String> = None;

    for raw_line in serialized.lines() {
        let trimmed = raw_line.trim_start();

        if let Some(rest) = trimmed.strip_prefix('[')
            && let Some(header) = rest.strip_suffix(']')
        {
            // Header. Track whether we're inside `responses.X.advanced`.
            current_advanced = advanced_response_name(header);

            // If this is a response-level TODO anchor, inject before header.
            if let Some(name) = response_header_name(header)
                && report.transform_todo_responses.iter().any(|n| n == name)
            {
                push_comment(&mut out, raw_line, TRANSFORM_TODO);
            }
        } else if trimmed.starts_with("regex")
            && let Some(current) = current_advanced.as_deref()
            && report.advanced_todo_responses.iter().any(|n| n == current)
        {
            push_comment(&mut out, raw_line, ADVANCED_TODO);
        }

        out.push_str(raw_line);
        out.push('\n');
    }

    out
}

/// Pull the response name out of `responses.<name>.advanced` headers.
fn advanced_response_name(header: &str) -> Option<String> {
    let rest = header.strip_prefix("responses.")?;
    let name = rest.strip_suffix(".advanced")?;
    Some(unquote(name).to_string())
}

/// Pull the response name out of `responses.<name>` headers.
/// Returns `None` for deeper paths like `responses.X.advanced`.
fn response_header_name(header: &str) -> Option<&str> {
    let rest = header.strip_prefix("responses.")?;
    if rest.contains('.') {
        return None;
    }
    Some(unquote(rest))
}

/// TOML table headers quote names with special chars. Strip outer quotes.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// Append a TODO comment line with indentation matching `model_line`.
fn push_comment(out: &mut String, model_line: &str, comment: &str) {
    let indent_len = model_line.len() - model_line.trim_start().len();
    out.push_str(&model_line[..indent_len]);
    out.push_str(comment);
    out.push('\n');
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper — expects migration to succeed and returns (output, report).
    fn migrate(input: &str) -> (String, MigrationReport) {
        migrate_v3_to_v4(input).expect("migration should parse")
    }

    #[test]
    fn simple_prefix_regex_becomes_variant() {
        let input = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.temp]
regex = "^TEMP:\\s*(?P<value>.*)$"
"#;

        let (output, report) = migrate(input);

        assert!(
            output.contains("variants = [\"TEMP: {value}\"]"),
            "expected synthesized variant in output:\n{output}"
        );
        assert!(
            !output.contains("regex ="),
            "deprecated regex field should be gone:\n{output}"
        );
        assert!(
            report
                .auto_migrated
                .iter()
                .any(|s| s.contains("responses.temp: regex -> variants")),
            "expected auto-migrated entry, got: {:?}",
            report.auto_migrated
        );
        assert!(
            report.todos.is_empty(),
            "simple prefix should not produce TODOs, got: {:?}",
            report.todos
        );
    }

    #[test]
    fn complex_regex_moves_to_advanced_with_todo() {
        let input = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.bswv]
regex = "OFST,(?P<offset>[\\-\\.0-9eE]+)V"
"#;

        let (output, report) = migrate(input);

        assert!(
            output.contains("[responses.bswv.advanced]"),
            "expected advanced subtable:\n{output}"
        );
        assert!(
            output.contains(ADVANCED_TODO),
            "expected TODO comment in output:\n{output}"
        );
        assert!(
            report
                .todos
                .iter()
                .any(|t| t.contains("responses.bswv: regex moved to [advanced]")),
            "expected 'moved to advanced' TODO, got: {:?}",
            report.todos
        );
    }

    #[test]
    fn regex_extract_prefix_becomes_format() {
        let input = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.power]
transform = [
  "trim",
  "regex_extract('^ROP:\\s*(.*)', 1)",
  "to_float",
]
"#;

        let (output, report) = migrate(input);

        assert!(
            output.contains("format('{cmd}: {value}', 'value')"),
            "expected format(...) replacement in output:\n{output}"
        );
        assert!(
            !output.contains("regex_extract"),
            "regex_extract should be gone:\n{output}"
        );
        assert!(
            report
                .auto_migrated
                .iter()
                .any(|m| m.contains("transform") && m.contains("format")),
            "expected auto-migrated transform entry, got: {:?}",
            report.auto_migrated
        );
    }

    #[test]
    fn complex_regex_extract_keeps_todo_comment() {
        let input = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.percent]
transform = [
  "trim",
  "regex_extract('([-+]?\\d+(?:\\.\\d+)?)', 1)",
  "to_float",
]
"#;

        let (output, report) = migrate(input);

        // Unchanged op text preserved (the backslashes escape in TOML output).
        assert!(
            output.contains("regex_extract"),
            "non-trivial regex_extract should remain:\n{output}"
        );
        assert!(
            output.contains(TRANSFORM_TODO),
            "expected transform TODO comment in output:\n{output}"
        );
        assert!(
            report.todos.iter().any(|t| t.contains("manual review")),
            "expected a manual-review TODO, got: {:?}",
            report.todos
        );
    }

    #[test]
    fn passthrough_preserves_non_deprecated_fields() {
        let input = r#"
schema_version = 3

[device]
name = "Clean Device"
capabilities = ["Readable"]

[connection]
type = "tcp"
host = "192.168.1.50"
port = 5025
timeout_ms = 2000

[commands.read]
template = ":MEAS?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }
"#;

        let (output, report) = migrate(input);

        assert!(output.contains("schema_version = 3"));
        assert!(output.contains("name = \"Clean Device\""));
        assert!(output.contains("template = \":MEAS?\""));
        assert!(output.contains("host = \"192.168.1.50\""));
        assert!(output.contains("port = 5025"));
        assert!(
            report.auto_migrated.is_empty() && report.todos.is_empty(),
            "clean manifest should produce no changes; got auto={:?} todos={:?}",
            report.auto_migrated,
            report.todos,
        );
    }

    #[test]
    fn regex_without_named_groups_goes_to_advanced() {
        // Positional captures can't round-trip into a format variant.
        let input = r#"
schema_version = 3

[device]
name = "T"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600

[responses.x]
regex = "^STAT:\\s*(.*)$"
"#;

        let (output, _report) = migrate(input);
        assert!(
            output.contains("[responses.x.advanced]"),
            "positional-only regex should move to advanced:\n{output}"
        );
    }

    #[test]
    fn try_regex_to_variant_recognizes_prefix_form() {
        assert_eq!(
            try_regex_to_variant(r"^TEMP:\s*(?P<value>.*)$"),
            Some("TEMP: {value}".to_string()),
        );
        assert_eq!(
            try_regex_to_variant(r"^STAT (?P<code>\d+),(?P<msg>.*)$"),
            Some("STAT {code},{msg}".to_string()),
        );
    }

    #[test]
    fn try_regex_to_variant_rejects_unanchored_regex() {
        // No `^`: pattern matches a substring — can't collapse to a format
        // variant, which must match the full response.
        assert!(try_regex_to_variant(r"OFST,(?P<v>[\-\.0-9]+)V").is_none());
    }

    #[test]
    fn try_regex_to_variant_rejects_when_no_named_groups() {
        assert!(try_regex_to_variant(r"^STAT:\s*(.*)$").is_none());
    }

    #[test]
    fn try_regex_to_variant_rejects_metachars_outside_groups() {
        // `.+` between groups is ambiguous: the format parser has no
        // equivalent for "any run of chars here" as a literal.
        assert!(try_regex_to_variant(r"^AB.+(?P<v>\d+)$").is_none());
    }

    #[test]
    fn try_regex_extract_to_format_recognizes_prefixes() {
        assert_eq!(
            try_regex_extract_to_format("regex_extract('^ROP:\\s*(.*)', 1)"),
            Some("format('{cmd}: {value}', 'value')".to_string()),
        );
        assert_eq!(
            try_regex_extract_to_format("regex_extract('^POW:\\s+(.*)', 1)"),
            Some("format('{cmd}: {value}', 'value')".to_string()),
        );
    }

    #[test]
    fn try_regex_extract_to_format_rejects_non_prefix() {
        assert!(
            try_regex_extract_to_format("regex_extract('([-+]?\\d+)', 1)").is_none(),
            "pure numeric capture should not be auto-migrated",
        );
    }
}
