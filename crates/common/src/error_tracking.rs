//! Error tracking and observability module
//!
//! Provides integration with Sentry for error tracking and performance monitoring.
//! This module is designed to be used by both the daemon (daq-bin) and GUI (daq-egui).
//!
//! # Usage
//!
//! Error tracking is opt-in and requires the `SENTRY_DSN` environment variable to be set.
//! If not set, error tracking is disabled and no data is sent.
//!
//! ## Environment Variables
//!
//! - `SENTRY_DSN`: The Sentry Data Source Name (required for error tracking)
//! - `SENTRY_ENVIRONMENT`: Environment name (e.g., "production", "development")
//! - `SENTRY_RELEASE`: Release version (defaults to package version)
//!
//! ## Example
//!
//! ```rust,ignore
//! // Initialize at application startup
//! let _guard = common::error_tracking::init("daq-daemon", env!("CARGO_PKG_VERSION"));
//!
//! // The guard must be kept alive for the duration of the application
//! // When dropped, it flushes pending events to Sentry
//! ```

use std::env;
use tracing::{info, warn};

/// Configuration for error tracking
#[derive(Debug, Clone)]
pub struct ErrorTrackingConfig {
    /// Application name (e.g., "daq-daemon", "daq-gui")
    pub app_name: String,
    /// Application version
    pub version: String,
    /// Environment (production, development, etc.)
    pub environment: String,
    /// Sample rate for error events (0.0 to 1.0)
    pub sample_rate: f32,
    /// Sample rate for performance transactions (0.0 to 1.0)
    pub traces_sample_rate: f32,
}

impl Default for ErrorTrackingConfig {
    fn default() -> Self {
        Self {
            app_name: "rust-daq".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            environment: env::var("SENTRY_ENVIRONMENT")
                .unwrap_or_else(|_| "development".to_string()),
            sample_rate: 1.0,        // Capture all errors
            traces_sample_rate: 0.1, // Sample 10% of transactions for performance
        }
    }
}

/// Guard that flushes Sentry events when dropped
///
/// Keep this alive for the duration of your application.
/// When dropped, it will flush any pending events to Sentry.
pub struct ErrorTrackingGuard {
    _inner: Option<()>,
}

impl Drop for ErrorTrackingGuard {
    fn drop(&mut self) {
        // Note: In actual Sentry usage, this would call sentry::close()
        // For the stub implementation, we just log
        info!("Error tracking shutdown");
    }
}

/// Initialize error tracking
///
/// Returns a guard that should be kept alive for the application's lifetime.
/// If `SENTRY_DSN` is not set, returns a no-op guard and error tracking is disabled.
///
/// # Arguments
///
/// * `app_name` - Name of the application (e.g., "daq-daemon")
/// * `version` - Version string (e.g., "0.1.0")
///
/// # Example
///
/// ```rust,ignore
/// fn main() {
///     let _sentry_guard = common::error_tracking::init("my-app", "1.0.0");
///     // ... application code ...
/// } // Events flushed when guard is dropped
/// ```
pub fn init(app_name: &str, version: &str) -> ErrorTrackingGuard {
    init_with_config(ErrorTrackingConfig {
        app_name: app_name.to_string(),
        version: version.to_string(),
        ..Default::default()
    })
}

/// Initialize error tracking with custom configuration
pub fn init_with_config(config: ErrorTrackingConfig) -> ErrorTrackingGuard {
    // Check if Sentry DSN is configured
    let dsn = match env::var("SENTRY_DSN") {
        Ok(dsn) if !dsn.is_empty() => dsn,
        _ => {
            info!(
                app = %config.app_name,
                "SENTRY_DSN not set, error tracking disabled"
            );
            return ErrorTrackingGuard { _inner: None };
        }
    };

    info!(
        app = %config.app_name,
        version = %config.version,
        environment = %config.environment,
        "Initializing error tracking"
    );

    // Log configuration (DSN is sensitive, don't log full value)
    let dsn_preview = if dsn.len() > 20 {
        format!("{}...{}", &dsn[..10], &dsn[dsn.len() - 10..])
    } else {
        "[configured]".to_string()
    };

    info!(
        dsn = %dsn_preview,
        sample_rate = %config.sample_rate,
        traces_sample_rate = %config.traces_sample_rate,
        "Error tracking configuration"
    );

    // Note: Actual Sentry initialization would happen here when the sentry
    // feature is enabled. This is a stub that shows the integration pattern.
    //
    // With sentry feature enabled, this would be:
    // ```
    // let guard = sentry::init((dsn, sentry::ClientOptions {
    //     release: Some(config.version.into()),
    //     environment: Some(config.environment.into()),
    //     sample_rate: config.sample_rate,
    //     traces_sample_rate: config.traces_sample_rate,
    //     ..Default::default()
    // }));
    // ```

    ErrorTrackingGuard { _inner: Some(()) }
}

/// Capture an error and send to Sentry
///
/// This is a convenience function for capturing errors.
/// If error tracking is not initialized, this is a no-op.
pub fn capture_error(error: &anyhow::Error) {
    // In production with sentry feature:
    // sentry::capture_error(error);
    warn!(error = %error, "Error captured for tracking");
}

/// Capture a message and send to Sentry
///
/// Useful for logging important events that aren't errors.
pub fn capture_message(message: &str, level: MessageLevel) {
    // In production with sentry feature:
    // sentry::capture_message(message, level.into());
    match level {
        MessageLevel::Debug => tracing::debug!(message),
        MessageLevel::Info => tracing::info!(message),
        MessageLevel::Warning => tracing::warn!(message),
        MessageLevel::Error => tracing::error!(message),
        MessageLevel::Fatal => tracing::error!(fatal = true, message),
    }
}

/// Message severity level for Sentry
#[derive(Debug, Clone, Copy)]
pub enum MessageLevel {
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

/// Add context to the current Sentry scope
///
/// Context is attached to all subsequent events until the scope is popped.
pub fn set_context(key: &str, value: serde_json::Value) {
    // In production with sentry feature:
    // sentry::configure_scope(|scope| {
    //     scope.set_context(key, sentry::protocol::Context::Other(value.into()));
    // });
    tracing::debug!(context_key = %key, "Setting error tracking context");
    let _ = value; // Suppress unused warning in stub
}

/// Set user information for error tracking
pub fn set_user(user_id: Option<&str>, email: Option<&str>, username: Option<&str>) {
    // In production with sentry feature:
    // sentry::configure_scope(|scope| {
    //     scope.set_user(Some(sentry::User {
    //         id: user_id.map(String::from),
    //         email: email.map(String::from),
    //         username: username.map(String::from),
    //         ..Default::default()
    //     }));
    // });
    tracing::debug!(
        user_id = ?user_id,
        email = ?email,
        username = ?username,
        "Setting error tracking user"
    );
}

/// Add a breadcrumb for debugging
///
/// Breadcrumbs are trail of events that led to an error.
pub fn add_breadcrumb(category: &str, message: &str) {
    // In production with sentry feature:
    // sentry::add_breadcrumb(sentry::Breadcrumb {
    //     category: Some(category.to_string()),
    //     message: Some(message.to_string()),
    //     ..Default::default()
    // });
    tracing::trace!(category = %category, message = %message, "Breadcrumb");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_without_dsn() {
        // Should not panic even without SENTRY_DSN
        let _guard = init("test-app", "0.1.0");
    }

    #[test]
    fn test_default_config() {
        let config = ErrorTrackingConfig::default();
        assert_eq!(config.sample_rate, 1.0);
        assert_eq!(config.traces_sample_rate, 0.1);
    }

    #[test]
    fn test_capture_message_levels() {
        capture_message("test debug", MessageLevel::Debug);
        capture_message("test info", MessageLevel::Info);
        capture_message("test warning", MessageLevel::Warning);
        capture_message("test error", MessageLevel::Error);
    }

    #[test]
    fn test_breadcrumb() {
        add_breadcrumb("test", "test message");
    }
}
