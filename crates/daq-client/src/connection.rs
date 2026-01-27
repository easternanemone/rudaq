//! Daemon connection configuration and URL normalization.
//!
//! This module provides types and utilities for managing the gRPC daemon address:
//! - [`DaemonAddress`]: Validated daemon URL with source tracking
//! - [`AddressSource`]: Where the address configuration came from
//! - [`AddressError`]: User-friendly validation errors
//!
//! # Address Resolution Precedence
//!
//! Addresses are resolved in this order (highest priority first):
//! 1. User input (typed in UI)
//! 2. Persisted from previous session (via caller-provided string)
//! 3. `DAQ_DAEMON_URL` environment variable
//! 4. Default: `http://127.0.0.1:50051`
//!
//! # URL Normalization
//!
//! The [`normalize_url`] function handles common input formats:
//! - Bare host:port (e.g., `100.117.5.12:50051` → `http://100.117.5.12:50051`)
//! - Missing port (e.g., `http://localhost` → `http://localhost:50051`)
//! - IPv6 addresses (e.g., `[::1]:50051` → `http://[::1]:50051`)
//!
//! # Example
//!
//! ```
//! use daq_client::connection::{DaemonAddress, AddressSource};
//!
//! // Parse and normalize a user-provided address
//! let addr = DaemonAddress::parse("100.117.5.12:50051", AddressSource::UserInput)?;
//! assert_eq!(addr.as_str(), "http://100.117.5.12:50051/");
//! assert!(!addr.is_tls());
//! # Ok::<(), daq_client::connection::AddressError>(())
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use url::Url;

/// Storage key for persisted daemon address.
pub const STORAGE_KEY_DAEMON_ADDR: &str = "daemon_address";

/// Default gRPC port for the DAQ daemon.
pub const DEFAULT_GRPC_PORT: u16 = 50051;

/// Default daemon address when no configuration is provided.
pub const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:50051";

/// Source of the daemon address configuration.
///
/// Used for display in the UI and for determining precedence when
/// multiple sources provide addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressSource {
    /// Hardcoded default (`http://127.0.0.1:50051`)
    Default,
    /// Loaded from `DAQ_DAEMON_URL` environment variable
    Environment,
    /// Restored from previous session via storage
    Persisted,
    /// User typed in the UI address field
    UserInput,
}

impl AddressSource {
    /// Returns the priority for address resolution (higher = preferred).
    #[must_use]
    #[allow(dead_code)]
    pub fn priority(self) -> u8 {
        match self {
            Self::Default => 0,
            Self::Environment => 1,
            Self::Persisted => 2,
            Self::UserInput => 3,
        }
    }

    /// Returns a short label for display in the UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Environment => "env",
            Self::Persisted => "saved",
            Self::UserInput => "user",
        }
    }
}

impl fmt::Display for AddressSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => write!(f, "Default"),
            Self::Environment => write!(f, "Environment (DAQ_DAEMON_URL)"),
            Self::Persisted => write!(f, "Saved from previous session"),
            Self::UserInput => write!(f, "User input"),
        }
    }
}

/// Validated daemon address with metadata.
///
/// This struct holds a normalized URL that has been validated for use
/// with the gRPC client. It also tracks the source of the address for
/// display purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonAddress {
    /// The normalized URL (always has scheme and port)
    url: String,
    /// Where this address came from
    source: AddressSource,
    /// Original input string (for display/debugging)
    original: String,
}

impl DaemonAddress {
    /// Parse and normalize a daemon URL.
    ///
    /// # Arguments
    ///
    /// * `input` - The URL string (may be bare host:port, missing scheme, etc.)
    /// * `source` - Where this address came from
    ///
    /// # Returns
    ///
    /// A validated `DaemonAddress` or an `AddressError` if validation fails.
    ///
    /// # Example
    ///
    /// ```
    /// use daq_client::connection::{DaemonAddress, AddressSource};
    ///
    /// let addr = DaemonAddress::parse("localhost:50051", AddressSource::UserInput)?;
    /// assert_eq!(addr.as_str(), "http://localhost:50051/");
    /// # Ok::<(), daq_client::connection::AddressError>(())
    /// ```
    pub fn parse(input: &str, source: AddressSource) -> Result<Self, AddressError> {
        let normalized = normalize_url(input)?;
        Ok(Self {
            url: normalized.to_string(),
            source,
            original: input.to_string(),
        })
    }

    /// Returns the normalized URL string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.url
    }

    /// Returns where this address came from.
    #[must_use]
    pub fn source(&self) -> AddressSource {
        self.source
    }

    /// Returns the original input string before normalization.
    #[must_use]
    pub fn original(&self) -> &str {
        &self.original
    }

    /// Returns `true` if this address uses TLS (https scheme).
    #[must_use]
    #[allow(dead_code)]
    pub fn is_tls(&self) -> bool {
        self.url.starts_with("https://")
    }

    /// Creates a new address with a different source.
    ///
    /// Useful when loading a persisted address (changes source from UserInput to Persisted).
    #[must_use]
    #[allow(dead_code)]
    pub fn with_source(mut self, source: AddressSource) -> Self {
        self.source = source;
        self
    }
}

impl fmt::Display for DaemonAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.url)
    }
}

impl Default for DaemonAddress {
    fn default() -> Self {
        Self::parse(DEFAULT_DAEMON_URL, AddressSource::Default)
            .expect("Default URL should always parse")
    }
}

/// URL validation error with user-friendly messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressError {
    /// Input was empty or whitespace-only
    EmptyInput,
    /// URL parsing failed
    InvalidUrl(String),
    /// No host was found in the URL
    MissingHost,
    /// Port could not be set (should not happen with valid hosts)
    InvalidPort(String),
    /// Unsupported URL scheme (only http/https allowed)
    UnsupportedScheme(String),
}

impl std::error::Error for AddressError {}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "Address cannot be empty"),
            Self::InvalidUrl(e) => write!(f, "Invalid URL: {e}"),
            Self::MissingHost => write!(f, "URL must include a host"),
            Self::InvalidPort(e) => write!(f, "Invalid port: {e}"),
            Self::UnsupportedScheme(s) => write!(f, "Unsupported scheme '{s}' (use http or https)"),
        }
    }
}

/// Normalize a daemon URL string.
///
/// This function handles common input formats and ensures the result
/// is a valid URL for the gRPC client:
///
/// - Adds `http://` scheme if missing
/// - Adds default port (50051) if missing
/// - Trims whitespace
/// - Normalizes scheme to lowercase
///
/// # Arguments
///
/// * `input` - The URL string to normalize
///
/// # Returns
///
/// A normalized [`Url`] or an [`AddressError`] if validation fails.
///
/// # Examples
///
/// ```
/// use daq_client::connection::normalize_url;
///
/// // Bare host:port
/// let url = normalize_url("192.168.1.100:50051")?;
/// assert_eq!(url.as_str(), "http://192.168.1.100:50051/");
///
/// // With scheme
/// let url = normalize_url("https://secure.example.com")?;
/// assert_eq!(url.as_str(), "https://secure.example.com:50051/");
///
/// // IPv6
/// let url = normalize_url("[::1]:8080")?;
/// assert_eq!(url.as_str(), "http://[::1]:8080/");
/// # Ok::<(), daq_client::connection::AddressError>(())
/// ```
pub fn normalize_url(input: &str) -> Result<Url, AddressError> {
    let input = input.trim();

    if input.is_empty() {
        return Err(AddressError::EmptyInput);
    }

    // Add scheme if missing
    let with_scheme = if input.contains("://") {
        input.to_string()
    } else {
        format!("http://{input}")
    };

    // Parse URL
    let mut url = Url::parse(&with_scheme).map_err(|e| AddressError::InvalidUrl(e.to_string()))?;

    // Validate scheme
    let scheme = url.scheme().to_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(AddressError::UnsupportedScheme(scheme));
    }

    // Validate host
    if url.host().is_none() {
        return Err(AddressError::MissingHost);
    }

    // Add default port if missing
    if url.port().is_none() {
        url.set_port(Some(DEFAULT_GRPC_PORT))
            .map_err(|()| AddressError::InvalidPort("Cannot set port on this URL".to_string()))?;
    }

    Ok(url)
}

/// Resolve daemon address from multiple sources with precedence.
///
/// Tries sources in order of priority (highest first):
/// 1. User input (if provided and valid)
/// 2. Persisted address (from previous session)
/// 3. `DAQ_DAEMON_URL` environment variable
/// 4. Default: `http://127.0.0.1:50051`
///
/// # Arguments
///
/// * `user_input` - Optional user-typed address
/// * `persisted_addr` - Optional persisted address string from storage
///
/// # Returns
///
/// The resolved [`DaemonAddress`] (never fails, falls back to default).
pub fn resolve_address(user_input: Option<&str>, persisted_addr: Option<&str>) -> DaemonAddress {
    // 1. User input (highest priority)
    if let Some(input) = user_input {
        if !input.trim().is_empty() {
            if let Ok(addr) = DaemonAddress::parse(input, AddressSource::UserInput) {
                return addr;
            }
        }
    }

    // 2. Persisted from previous session
    if let Some(persisted) = persisted_addr {
        if let Ok(addr) = DaemonAddress::parse(persisted, AddressSource::Persisted) {
            return addr;
        }
    }

    // 3. Environment variable
    if let Ok(env_url) = std::env::var("DAQ_DAEMON_URL") {
        if let Ok(addr) = DaemonAddress::parse(&env_url, AddressSource::Environment) {
            return addr;
        }
    }

    // 4. Default fallback
    DaemonAddress::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_bare_host_port() {
        let url = normalize_url("127.0.0.1:50051").unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:50051/");
    }

    #[test]
    fn test_normalize_with_http_scheme() {
        let url = normalize_url("http://localhost:8080").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/");
    }

    #[test]
    fn test_normalize_with_https_scheme() {
        // With explicit non-default port
        let url = normalize_url("https://secure.example.com:8443").unwrap();
        assert_eq!(url.as_str(), "https://secure.example.com:8443/");

        // Without port - defaults to 50051 (gRPC default, not HTTPS default)
        let url = normalize_url("https://secure.example.com").unwrap();
        assert_eq!(url.as_str(), "https://secure.example.com:50051/");
    }

    #[test]
    fn test_normalize_adds_default_port() {
        let url = normalize_url("http://localhost").unwrap();
        assert_eq!(url.as_str(), "http://localhost:50051/");
    }

    #[test]
    fn test_normalize_ipv6() {
        let url = normalize_url("[::1]:8080").unwrap();
        assert_eq!(url.as_str(), "http://[::1]:8080/");
    }

    #[test]
    fn test_normalize_ipv6_with_default_port() {
        let url = normalize_url("http://[::1]").unwrap();
        assert_eq!(url.as_str(), "http://[::1]:50051/");
    }

    #[test]
    fn test_normalize_trims_whitespace() {
        let url = normalize_url("  localhost:5000  ").unwrap();
        assert_eq!(url.as_str(), "http://localhost:5000/");
    }

    #[test]
    fn test_normalize_empty_input() {
        let err = normalize_url("").unwrap_err();
        assert_eq!(err, AddressError::EmptyInput);
    }

    #[test]
    fn test_normalize_whitespace_only() {
        let err = normalize_url("   ").unwrap_err();
        assert_eq!(err, AddressError::EmptyInput);
    }

    #[test]
    fn test_normalize_unsupported_scheme() {
        let err = normalize_url("ftp://example.com").unwrap_err();
        assert!(matches!(err, AddressError::UnsupportedScheme(_)));
    }

    #[test]
    fn test_daemon_address_parse() {
        let addr = DaemonAddress::parse("100.117.5.12:50051", AddressSource::UserInput).unwrap();
        assert_eq!(addr.as_str(), "http://100.117.5.12:50051/");
        assert_eq!(addr.source(), AddressSource::UserInput);
        assert_eq!(addr.original(), "100.117.5.12:50051");
        assert!(!addr.is_tls());
    }

    #[test]
    fn test_daemon_address_tls() {
        let addr =
            DaemonAddress::parse("https://secure.example.com", AddressSource::Environment).unwrap();
        assert!(addr.is_tls());
    }

    #[test]
    fn test_daemon_address_default() {
        let addr = DaemonAddress::default();
        assert_eq!(addr.as_str(), "http://127.0.0.1:50051/");
        assert_eq!(addr.source(), AddressSource::Default);
    }

    #[test]
    fn test_address_source_priority() {
        assert!(AddressSource::UserInput.priority() > AddressSource::Persisted.priority());
        assert!(AddressSource::Persisted.priority() > AddressSource::Environment.priority());
        assert!(AddressSource::Environment.priority() > AddressSource::Default.priority());
    }

    #[test]
    fn test_address_source_labels() {
        assert_eq!(AddressSource::Default.label(), "default");
        assert_eq!(AddressSource::Environment.label(), "env");
        assert_eq!(AddressSource::Persisted.label(), "saved");
        assert_eq!(AddressSource::UserInput.label(), "user");
    }

    #[test]
    fn test_resolve_address_default() {
        // Clear env var for test isolation
        std::env::remove_var("DAQ_DAEMON_URL");

        let addr = resolve_address(None, None);
        assert_eq!(addr.source(), AddressSource::Default);
    }

    #[test]
    fn test_resolve_address_env() {
        std::env::set_var("DAQ_DAEMON_URL", "http://test.local:9999");
        let addr = resolve_address(None, None);
        assert_eq!(addr.as_str(), "http://test.local:9999/");
        assert_eq!(addr.source(), AddressSource::Environment);
        std::env::remove_var("DAQ_DAEMON_URL");
    }

    #[test]
    fn test_resolve_address_user_input_priority() {
        std::env::set_var("DAQ_DAEMON_URL", "http://env.local:8888");
        let addr = resolve_address(Some("user.local:7777"), None);
        assert_eq!(addr.as_str(), "http://user.local:7777/");
        assert_eq!(addr.source(), AddressSource::UserInput);
        std::env::remove_var("DAQ_DAEMON_URL");
    }

    #[test]
    fn test_resolve_address_persisted_priority() {
        std::env::set_var("DAQ_DAEMON_URL", "http://env.local:8888");
        let addr = resolve_address(None, Some("http://persisted.local:6666"));
        assert_eq!(addr.as_str(), "http://persisted.local:6666/");
        assert_eq!(addr.source(), AddressSource::Persisted);
        std::env::remove_var("DAQ_DAEMON_URL");
    }

    #[test]
    fn test_address_error_display() {
        assert_eq!(
            AddressError::EmptyInput.to_string(),
            "Address cannot be empty"
        );
        assert!(AddressError::UnsupportedScheme("ftp".to_string())
            .to_string()
            .contains("ftp"));
    }
}
