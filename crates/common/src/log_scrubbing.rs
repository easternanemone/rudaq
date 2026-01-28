//! Log scrubbing utilities for sensitive data redaction.
//!
//! This module provides patterns and utilities for preventing sensitive data
//! from appearing in logs. Use these when logging data that might contain
//! PII, credentials, or other sensitive information.
//!
//! # Example
//!
//! ```rust
//! use common::log_scrubbing::{Redacted, scrub_email, scrub_ip};
//!
//! // Wrap sensitive fields in Redacted
//! let api_key = Redacted::new("sk-1234567890");
//! tracing::info!(?api_key, "Connecting to service"); // Logs: api_key=[REDACTED]
//!
//! // Scrub known patterns from strings
//! let message = "User email@example.com connected from 192.168.1.1";
//! let safe = scrub_email(&scrub_ip(message));
//! // Result: "User [EMAIL] connected from [IP]"
//! ```

use std::fmt;

/// Wrapper type that redacts its contents when displayed or debugged.
///
/// Use this to wrap sensitive values (API keys, passwords, tokens) to prevent
/// accidental logging of secrets.
#[derive(Clone)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Create a new redacted value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Access the inner value (use with caution).
    pub fn inner(&self) -> &T {
        &self.0
    }

    /// Consume and return the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

/// Scrub email addresses from a string.
///
/// Replaces patterns matching `word@word.word` with `[EMAIL]`.
pub fn scrub_email(input: &str) -> String {
    let email_pattern = regex_lite::Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
        .expect("valid regex");
    email_pattern.replace_all(input, "[EMAIL]").to_string()
}

/// Scrub IPv4 addresses from a string.
///
/// Replaces patterns matching `X.X.X.X` with `[IP]`.
pub fn scrub_ip(input: &str) -> String {
    let ip_pattern =
        regex_lite::Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").expect("valid regex");
    ip_pattern.replace_all(input, "[IP]").to_string()
}

/// Scrub potential API keys and tokens from a string.
///
/// Replaces long alphanumeric strings (32+ chars) that look like tokens.
pub fn scrub_tokens(input: &str) -> String {
    let token_pattern = regex_lite::Regex::new(r"\b[a-zA-Z0-9_-]{32,}\b").expect("valid regex");
    token_pattern.replace_all(input, "[TOKEN]").to_string()
}

/// Scrub serial port paths (may contain device identifiers).
pub fn scrub_serial_port(input: &str) -> String {
    let serial_pattern =
        regex_lite::Regex::new(r"/dev/(tty[A-Za-z0-9]+|serial/by-id/[^\s]+)").expect("valid regex");
    serial_pattern
        .replace_all(input, "/dev/[DEVICE]")
        .to_string()
}

/// Combined scrubbing for common sensitive patterns.
///
/// Applies all scrubbing functions in sequence.
pub fn scrub_all(input: &str) -> String {
    let result = scrub_email(input);
    let result = scrub_ip(&result);
    let result = scrub_tokens(&result);
    scrub_serial_port(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redacted_debug() {
        let secret = Redacted::new("my-secret-key");
        assert_eq!(format!("{:?}", secret), "[REDACTED]");
    }

    #[test]
    fn test_redacted_display() {
        let secret = Redacted::new("password123");
        assert_eq!(format!("{}", secret), "[REDACTED]");
    }

    #[test]
    fn test_redacted_inner() {
        let secret = Redacted::new("value");
        assert_eq!(secret.inner(), &"value");
        assert_eq!(secret.into_inner(), "value");
    }

    #[test]
    fn test_scrub_email() {
        let input = "Contact user@example.com for support";
        assert_eq!(scrub_email(input), "Contact [EMAIL] for support");
    }

    #[test]
    fn test_scrub_ip() {
        let input = "Connected from 192.168.1.100";
        assert_eq!(scrub_ip(input), "Connected from [IP]");
    }

    #[test]
    fn test_scrub_tokens() {
        let input = "API key: sk_live_TESTKEY_not_real_00000000000";
        assert!(scrub_tokens(input).contains("[TOKEN]"));
    }

    #[test]
    fn test_scrub_serial_port() {
        let input = "Opened /dev/ttyUSB0 and /dev/serial/by-id/usb-FTDI_device";
        let result = scrub_serial_port(input);
        assert!(result.contains("/dev/[DEVICE]"));
    }

    #[test]
    fn test_scrub_all() {
        let input =
            "User admin@test.com from 10.0.0.1 using token abc123def456ghi789jkl012mno345pqr";
        let result = scrub_all(input);
        assert!(result.contains("[EMAIL]"));
        assert!(result.contains("[IP]"));
        assert!(result.contains("[TOKEN]"));
    }
}
