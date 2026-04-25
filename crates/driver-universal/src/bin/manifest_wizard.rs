//! `manifest-wizard` — interactive scaffolder for v4 device manifests.
//!
//! Prompts the author for device name, transport, and a handful of commands
//! with sample responses, then emits a v4 TOML skeleton on stdout. Sample
//! responses are run through a small inference pass that picks one of:
//!
//!   * `response_type = "float"` — sample parses as `f64`
//!   * `response_type = "integer"` — sample parses as `i64`
//!   * `response = "<cmd>_response"` plus a `[responses.<cmd>_response]`
//!     entry with a `format = "WORD: {value}"` variant — sample looks like
//!     the common `PREFIX: value` instrument reply shape.
//!   * Nothing — the skeleton leaves the response unspecified with a
//!     `# TODO` comment so the author can fill it in.
//!
//! The wizard deliberately stays minimal: it only uses `std::io` (no clap,
//! no dialoguer) and produces a *starting point* — authors are expected to
//! run `manifest-check` and iterate on the generated file.
//!
//! # CLI
//!
//! Just `manifest-wizard`. No flags. All input is read from stdin, and the
//! finished TOML is printed to stdout; prompts go to stderr so you can
//! redirect output cleanly:
//!
//! ```text
//! manifest-wizard > config/devices/my_new_device.toml
//! ```
//!
//! Exit codes: 0 on success, 1 on I/O error, 2 on invalid input (e.g. a
//! non-numeric answer to the "how many commands?" prompt).

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use toml::Value;

/// What the wizard inferred about a command's reply.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InferredResponse {
    /// No response expected (blank sample).
    None,
    /// SCPI auto-parse — sample is a clean float.
    ScpiFloat,
    /// SCPI auto-parse — sample is a clean integer.
    ScpiInteger,
    /// Format variant with a single `{value}` placeholder.
    ///
    /// The stored string is the raw format template, e.g. `"TEMP: {value}"`.
    Format(String),
    /// Nothing useful inferred — emit a TODO.
    Raw,
}

fn main() -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();

    match run_wizard(&mut stdin.lock(), &mut stdout.lock(), &mut stderr.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(WizardError::Io(e)) => {
            eprintln!("error: I/O: {e}");
            ExitCode::from(1)
        }
        Err(WizardError::BadInput(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
    }
}

#[derive(Debug)]
enum WizardError {
    Io(io::Error),
    BadInput(String),
}

impl From<io::Error> for WizardError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Interactive flow
// ---------------------------------------------------------------------------

fn run_wizard<R: BufRead, W: Write, E: Write>(
    input: &mut R,
    output: &mut W,
    prompts: &mut E,
) -> Result<(), WizardError> {
    writeln!(prompts, "manifest-wizard — v4 TOML skeleton generator")?;
    writeln!(
        prompts,
        "(answers go to stdin; the finished TOML prints to stdout)"
    )?;
    writeln!(prompts)?;

    let device_name = ask(
        prompts,
        input,
        "Device name (e.g. my_laser)",
        Some("my_device"),
    )?;
    let transport = ask(
        prompts,
        input,
        "Transport [serial/tcp/scpi]",
        Some("serial"),
    )?
    .to_ascii_lowercase();

    let mut top = toml::Table::new();
    top.insert("schema_version".into(), Value::Integer(3));

    // [device]
    let mut device = toml::Table::new();
    device.insert("name".into(), Value::String(device_name.clone()));
    device.insert("capabilities".into(), Value::Array(vec![]));
    top.insert("device".into(), Value::Table(device));

    // [connection]
    top.insert(
        "connection".into(),
        build_connection(&transport, input, prompts)?,
    );

    let count: usize = {
        let raw = ask(prompts, input, "How many commands?", Some("1"))?;
        raw.parse()
            .map_err(|_| WizardError::BadInput(format!("not a number: {raw}")))?
    };

    let mut commands = toml::Table::new();
    let mut responses = toml::Table::new();

    for i in 0..count {
        writeln!(prompts, "\n--- command {} of {count} ---", i + 1)?;
        let name = ask(
            prompts,
            input,
            "Command name (e.g. read_power)",
            Some(&format!("cmd_{}", i + 1)),
        )?;
        let template = ask(
            prompts,
            input,
            "Send string / template (e.g. ':MEAS:POW?')",
            None,
        )?;
        let sample = ask(prompts, input, "Sample response (blank if none)", Some(""))?;

        let mut cmd = toml::Table::new();
        cmd.insert("template".into(), Value::String(template));

        match infer_response(&sample) {
            InferredResponse::None => {
                cmd.insert("expects_response".into(), Value::Boolean(false));
            }
            InferredResponse::ScpiFloat => {
                cmd.insert("response_type".into(), Value::String("float".into()));
            }
            InferredResponse::ScpiInteger => {
                cmd.insert("response_type".into(), Value::String("integer".into()));
            }
            InferredResponse::Format(fmt) => {
                let resp_name = format!("{name}_response");
                cmd.insert("response".into(), Value::String(resp_name.clone()));
                let mut resp = toml::Table::new();
                resp.insert("format".into(), Value::String(fmt));
                responses.insert(resp_name, Value::Table(resp));
            }
            InferredResponse::Raw => {
                cmd.insert(
                    "_todo".into(),
                    Value::String(
                        "sample response could not be inferred — set \
                         `response_type` or `response` manually"
                            .into(),
                    ),
                );
            }
        }

        commands.insert(name, Value::Table(cmd));
    }

    top.insert("commands".into(), Value::Table(commands));
    if !responses.is_empty() {
        top.insert("responses".into(), Value::Table(responses));
    }

    let rendered = toml::to_string_pretty(&Value::Table(top))
        .map_err(|e| WizardError::BadInput(format!("serialize: {e}")))?;

    // Strip the synthetic `_todo` keys and replace with proper TOML comments
    // above the command header.
    let rendered = hoist_todos(&rendered);

    writeln!(output, "# Generated by manifest-wizard — review before use")?;
    writeln!(
        output,
        "# Next steps: run `manifest-check` and fill in `[capabilities.*]` mappings."
    )?;
    write!(output, "{rendered}")?;
    Ok(())
}

fn build_connection<R: BufRead, E: Write>(
    transport: &str,
    input: &mut R,
    prompts: &mut E,
) -> Result<Value, WizardError> {
    let mut tbl = toml::Table::new();
    match transport {
        "serial" => {
            tbl.insert("type".into(), Value::String("serial".into()));
            let baud = ask(prompts, input, "Baud rate", Some("9600"))?
                .parse::<i64>()
                .map_err(|_| WizardError::BadInput("baud must be a number".into()))?;
            tbl.insert("baud_rate".into(), Value::Integer(baud));
            tbl.insert("timeout_ms".into(), Value::Integer(1000));
        }
        "tcp" | "scpi" => {
            // SCPI is TCP with a sensible SCPI default port.
            tbl.insert("type".into(), Value::String("tcp".into()));
            let host = ask(prompts, input, "Host", Some("127.0.0.1"))?;
            let port_default = if transport == "scpi" { "5025" } else { "5000" };
            let port = ask(prompts, input, "Port", Some(port_default))?
                .parse::<i64>()
                .map_err(|_| WizardError::BadInput("port must be a number".into()))?;
            tbl.insert("host".into(), Value::String(host));
            tbl.insert("port".into(), Value::Integer(port));
            tbl.insert("timeout_ms".into(), Value::Integer(2000));
            if transport == "scpi" {
                tbl.insert("terminator".into(), Value::String("\n".into()));
            }
        }
        other => {
            return Err(WizardError::BadInput(format!(
                "unknown transport '{other}' — expected serial/tcp/scpi"
            )));
        }
    }
    Ok(Value::Table(tbl))
}

/// Prompt with an optional default; a blank response picks the default.
fn ask<R: BufRead, E: Write>(
    prompts: &mut E,
    input: &mut R,
    question: &str,
    default: Option<&str>,
) -> Result<String, WizardError> {
    match default {
        Some(d) if !d.is_empty() => write!(prompts, "{question} [{d}]: ")?,
        _ => write!(prompts, "{question}: ")?,
    }
    prompts.flush()?;

    let mut line = String::new();
    let n = input.read_line(&mut line)?;
    if n == 0 {
        // EOF — treat like blank: use default if present, else error.
        return match default {
            Some(d) => Ok(d.to_string()),
            None => Err(WizardError::BadInput(format!(
                "unexpected EOF at '{question}'"
            ))),
        };
    }
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        return match default {
            Some(d) => Ok(d.to_string()),
            None => Err(WizardError::BadInput(format!(
                "'{question}' requires a value"
            ))),
        };
    }
    Ok(trimmed)
}

/// After serialization, replace synthetic `_todo = "..."` keys with comment
/// lines. The toml crate can't emit comments, so we stash the TODO text in a
/// field and post-process the text to lift it out.
fn hoist_todos(rendered: &str) -> String {
    let mut out = String::with_capacity(rendered.len());
    for line in rendered.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("_todo = ") {
            // Strip surrounding quotes and emit as a comment with the same
            // indentation as the original key.
            let indent_len = line.len() - trimmed.len();
            let msg = rest.trim().trim_matches('"');
            out.push_str(&line[..indent_len]);
            out.push_str("# TODO: ");
            out.push_str(msg);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Inference
// ---------------------------------------------------------------------------

/// Inspect a raw sample response and guess the right v4 response shape.
fn infer_response(sample: &str) -> InferredResponse {
    let s = sample.trim();
    if s.is_empty() {
        return InferredResponse::None;
    }

    // Pure integer (must NOT also match a float with a decimal point — try
    // integer first).
    if s.parse::<i64>().is_ok() {
        return InferredResponse::ScpiInteger;
    }
    if s.parse::<f64>().is_ok() {
        return InferredResponse::ScpiFloat;
    }

    // "PREFIX: value" or "PREFIX value" shape — a colon-or-space separated
    // leading word followed by an arbitrary tail. We emit a single-placeholder
    // format template so the author gets a starting point; if the tail is
    // actually multi-field (`"STAT 1,2,3"`) they can edit by hand.
    if let Some(fmt) = format_from_prefix(s) {
        return InferredResponse::Format(fmt);
    }

    InferredResponse::Raw
}

/// If `s` begins with a plain word followed by `:` or whitespace and a
/// non-empty tail, return `"<word>: {value}"` / `"<word> {value}"`.
fn format_from_prefix(s: &str) -> Option<String> {
    // Find the first separator after the leading word. We call the leading
    // run "the prefix" and require it to be non-empty and made of identifier
    // characters only (so we don't pick up a minus sign or leading comma).
    let mut chars = s.char_indices();
    let mut word_end = 0;
    for (idx, c) in chars.by_ref() {
        if is_word_char(c) {
            word_end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if word_end == 0 {
        return None;
    }
    let word = &s[..word_end];
    let tail = &s[word_end..];

    // Require a recognised separator.
    let sep = if let Some(rest) = tail.strip_prefix(':') {
        if rest.trim_start().is_empty() {
            return None;
        }
        ": "
    } else if tail.starts_with(char::is_whitespace) {
        if tail.trim().is_empty() {
            return None;
        }
        " "
    } else {
        return None;
    };

    Some(format!("{word}{sep}{{value}}"))
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '*'
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn infer_response_handles_blank_and_numeric() {
        assert_eq!(infer_response(""), InferredResponse::None);
        assert_eq!(infer_response("   \n"), InferredResponse::None);
        assert_eq!(infer_response("42"), InferredResponse::ScpiInteger);
        assert_eq!(infer_response("-7"), InferredResponse::ScpiInteger);
        assert_eq!(infer_response("1.234"), InferredResponse::ScpiFloat);
        assert_eq!(infer_response("-3.14e-2"), InferredResponse::ScpiFloat);
    }

    #[test]
    fn infer_response_detects_prefix_format() {
        assert_eq!(
            infer_response("TEMP: 25.5"),
            InferredResponse::Format("TEMP: {value}".into()),
        );
        assert_eq!(
            infer_response("ROP: 12.5W"),
            InferredResponse::Format("ROP: {value}".into()),
        );
        // Space-separated prefix (no colon)
        assert_eq!(
            infer_response("STAT 1"),
            InferredResponse::Format("STAT {value}".into()),
        );
        // SCPI-style *IDN? reply
        assert_eq!(
            infer_response("*IDN Foo,Bar,1,1.0"),
            InferredResponse::Format("*IDN {value}".into()),
        );
    }

    #[test]
    fn infer_response_falls_back_to_raw() {
        // No identifier prefix — starts with punctuation
        assert_eq!(infer_response(",12.5"), InferredResponse::Raw);
        // Just a word, no tail
        assert_eq!(infer_response("FOO"), InferredResponse::Raw);
        assert_eq!(infer_response("FOO: "), InferredResponse::Raw);
    }

    /// End-to-end smoke: drive the wizard with scripted stdin and check we
    /// get valid-looking v4 TOML out, with the inferred bits in the right
    /// places.
    #[test]
    fn end_to_end_produces_valid_toml_skeleton() {
        let input = concat!(
            "my_laser\n",              // device name
            "serial\n",                // transport
            "115200\n",                // baud rate
            "2\n",                     // command count
            "read_power\n",            // cmd 1 name
            ":MEAS:POW?\n",            // cmd 1 template
            "ROP: 12.5\n",             // cmd 1 sample → format inference
            "set_power\n",             // cmd 2 name
            ":SOUR:POW {{ value }}\n", // cmd 2 template
            "\n",                      // cmd 2 blank sample → no response
        );
        let mut input = Cursor::new(input.as_bytes());
        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();

        run_wizard(&mut input, &mut stdout, &mut stderr).expect("wizard should succeed");

        let out = String::from_utf8(stdout).unwrap();
        assert!(out.contains("schema_version = 3"), "{out}");
        assert!(out.contains("name = \"my_laser\""), "{out}");
        assert!(out.contains("baud_rate = 115200"), "{out}");
        assert!(
            out.contains("response = \"read_power_response\""),
            "inferred response link missing:\n{out}"
        );
        assert!(
            out.contains("format = \"ROP: {value}\""),
            "inferred format missing:\n{out}"
        );
        assert!(
            out.contains("expects_response = false"),
            "blank-sample command should disable expects_response:\n{out}"
        );

        // Most importantly: the generated TOML must parse.
        let parsed: toml::Value = toml::from_str(&out).expect("wizard output must be valid TOML");
        assert!(parsed.get("commands").is_some());
        assert!(parsed.get("responses").is_some());
    }
}
