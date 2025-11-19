//! V4 Tracing Infrastructure
//!
//! This module provides structured, async-aware logging infrastructure for the V4 architecture.
//! It uses the `tracing` and `tracing-subscriber` crates to provide:
//! - Structured logging with spans and events
//! - Async-aware context propagation
//! - Multiple output formats (pretty, compact, JSON)
//! - Environment-based filtering
//! - Integration with V4 configuration system
//!
//! # Example
//! ```no_run
//! use rust_daq::{config_v4::V4Config, tracing_v4};
//! use tracing::{info, warn, error, debug};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load configuration
//! let config = V4Config::load()?;
//!
//! // Initialize tracing
//! tracing_v4::init_from_config(&config)?;
//!
//! // Use tracing macros
//! info!("Application started");
//! warn!(component = "instrument", "Connection timeout");
//! error!(error = ?std::io::Error::from(std::io::ErrorKind::NotFound), "File not found");
//! # Ok(())
//! # }
//! ```

use crate::config_v4::V4Config;
use tracing::Level;
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

/// Output format for tracing
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    /// Pretty-printed format with colors (for development)
    Pretty,
    /// Compact format without colors (for production)
    Compact,
    /// JSON format for structured logging (for log aggregation)
    Json,
}

/// Tracing configuration options
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// Log level (trace, debug, info, warn, error)
    pub level: Level,
    /// Output format
    pub format: OutputFormat,
    /// Whether to include span events (ENTER, EXIT, CLOSE)
    pub with_span_events: bool,
    /// Whether to include file and line numbers
    pub with_file_and_line: bool,
    /// Whether to include thread IDs
    pub with_thread_ids: bool,
    /// Whether to include thread names
    pub with_thread_names: bool,
    /// Whether to enable ANSI colors (only for Pretty format)
    pub with_ansi: bool,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            level: Level::INFO,
            format: OutputFormat::Pretty,
            with_span_events: true,
            with_file_and_line: true,
            with_thread_ids: false,
            with_thread_names: true,
            with_ansi: true,
        }
    }
}

impl TracingConfig {
    /// Create tracing config from V4 configuration
    pub fn from_v4_config(config: &V4Config) -> Result<Self, String> {
        let level = parse_log_level(&config.application.log_level)?;

        Ok(Self {
            level,
            ..Default::default()
        })
    }

    /// Create tracing config with custom settings
    pub fn new(level: Level) -> Self {
        Self {
            level,
            ..Default::default()
        }
    }

    /// Set output format
    pub fn with_format(mut self, format: OutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Enable or disable span events
    pub fn with_span_events(mut self, enabled: bool) -> Self {
        self.with_span_events = enabled;
        self
    }

    /// Enable or disable ANSI colors
    pub fn with_ansi(mut self, enabled: bool) -> Self {
        self.with_ansi = enabled;
        self
    }
}

/// Initialize tracing from V4 configuration
///
/// This is the recommended way to initialize tracing for V4 applications.
/// It reads the log level from the V4 configuration and sets up appropriate
/// subscribers.
///
/// # Example
/// ```no_run
/// use rust_daq::{config_v4::V4Config, tracing_v4};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let config = V4Config::load()?;
/// tracing_v4::init_from_config(&config)?;
/// # Ok(())
/// # }
/// ```
pub fn init_from_config(config: &V4Config) -> Result<(), String> {
    let tracing_config = TracingConfig::from_v4_config(config)?;
    init(tracing_config)
}

/// Initialize tracing with custom configuration
///
/// This allows more fine-grained control over tracing initialization.
///
/// This function is idempotent - if tracing is already initialized, it will
/// return Ok(()) without error. This makes it safe to call in tests and libraries.
///
/// # Example
/// ```no_run
/// use rust_daq::tracing_v4::{self, TracingConfig, OutputFormat};
/// use tracing::Level;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let config = TracingConfig::new(Level::DEBUG)
///     .with_format(OutputFormat::Json)
///     .with_span_events(false);
///
/// tracing_v4::init(config)?;
/// # Ok(())
/// # }
/// ```
pub fn init(config: TracingConfig) -> Result<(), String> {
    // Create env filter with default level
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level_to_filter_string(config.level)));

    // Determine span events
    let span_events = if config.with_span_events {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    // Build subscriber based on format
    match config.format {
        OutputFormat::Pretty => {
            let fmt_layer = fmt::layer()
                .pretty()
                .with_span_events(span_events)
                .with_file(config.with_file_and_line)
                .with_line_number(config.with_file_and_line)
                .with_thread_ids(config.with_thread_ids)
                .with_thread_names(config.with_thread_names)
                .with_ansi(config.with_ansi)
                .with_filter(env_filter);

            tracing_subscriber::registry()
                .with(fmt_layer)
                .try_init()
                .or_else(|e| {
                    // Handle "already initialized" gracefully - this is expected in tests
                    // and when multiple components try to init tracing
                    if e.to_string()
                        .contains("a global default trace dispatcher has already been set")
                    {
                        Ok(())
                    } else {
                        Err(format!("Failed to initialize tracing: {}", e))
                    }
                })?;
        }
        OutputFormat::Compact => {
            let fmt_layer = fmt::layer()
                .compact()
                .with_span_events(span_events)
                .with_file(config.with_file_and_line)
                .with_line_number(config.with_file_and_line)
                .with_thread_ids(config.with_thread_ids)
                .with_thread_names(config.with_thread_names)
                .with_ansi(false)
                .with_filter(env_filter);

            tracing_subscriber::registry()
                .with(fmt_layer)
                .try_init()
                .or_else(|e| {
                    if e.to_string()
                        .contains("a global default trace dispatcher has already been set")
                    {
                        Ok(())
                    } else {
                        Err(format!("Failed to initialize tracing: {}", e))
                    }
                })?;
        }
        OutputFormat::Json => {
            let fmt_layer = fmt::layer()
                .json()
                .with_span_events(span_events)
                .with_file(config.with_file_and_line)
                .with_line_number(config.with_file_and_line)
                .with_thread_ids(config.with_thread_ids)
                .with_thread_names(config.with_thread_names)
                .with_filter(env_filter);

            tracing_subscriber::registry()
                .with(fmt_layer)
                .try_init()
                .or_else(|e| {
                    if e.to_string()
                        .contains("a global default trace dispatcher has already been set")
                    {
                        Ok(())
                    } else {
                        Err(format!("Failed to initialize tracing: {}", e))
                    }
                })?;
        }
    }

    Ok(())
}

/// Parse log level string into tracing Level
fn parse_log_level(level: &str) -> Result<Level, String> {
    match level.to_lowercase().as_str() {
        "trace" => Ok(Level::TRACE),
        "debug" => Ok(Level::DEBUG),
        "info" => Ok(Level::INFO),
        "warn" => Ok(Level::WARN),
        "error" => Ok(Level::ERROR),
        _ => Err(format!(
            "Invalid log level '{}'. Must be one of: trace, debug, info, warn, error",
            level
        )),
    }
}

/// Convert Level to env filter string
fn level_to_filter_string(level: Level) -> String {
    match level {
        Level::TRACE => "trace".to_string(),
        Level::DEBUG => "debug".to_string(),
        Level::INFO => "info".to_string(),
        Level::WARN => "warn".to_string(),
        Level::ERROR => "error".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_level() {
        assert!(matches!(parse_log_level("trace"), Ok(Level::TRACE)));
        assert!(matches!(parse_log_level("debug"), Ok(Level::DEBUG)));
        assert!(matches!(parse_log_level("info"), Ok(Level::INFO)));
        assert!(matches!(parse_log_level("warn"), Ok(Level::WARN)));
        assert!(matches!(parse_log_level("error"), Ok(Level::ERROR)));

        // Case insensitive
        assert!(matches!(parse_log_level("INFO"), Ok(Level::INFO)));
        assert!(matches!(parse_log_level("Debug"), Ok(Level::DEBUG)));

        // Invalid
        assert!(parse_log_level("invalid").is_err());
    }

    #[test]
    fn test_tracing_config_from_v4() {
        use crate::config_v4::{ActorConfig, ApplicationConfig, StorageConfig};
        use std::path::PathBuf;

        let v4_config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "debug".to_string(),
            },
            actors: ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 5000,
                shutdown_timeout_ms: 5000,
            },
            storage: StorageConfig {
                default_backend: "hdf5".to_string(),
                output_dir: PathBuf::from("data"),
                compression_level: 6,
                auto_flush_interval_secs: 30,
            },
            instruments: vec![],
        };

        let tracing_config = TracingConfig::from_v4_config(&v4_config).unwrap();
        assert!(matches!(tracing_config.level, Level::DEBUG));
    }

    #[test]
    fn test_tracing_config_builder() {
        let config = TracingConfig::new(Level::WARN)
            .with_format(OutputFormat::Json)
            .with_span_events(false)
            .with_ansi(false);

        assert!(matches!(config.level, Level::WARN));
        assert!(matches!(config.format, OutputFormat::Json));
        assert!(!config.with_span_events);
        assert!(!config.with_ansi);
    }
}
