//! Connection state machine and auto-reconnect logic.
//!
//! This module provides a robust connection lifecycle for the GUI:
//! - Explicit state machine with clear transitions
//! - Auto-reconnect with exponential backoff and jitter
//! - Cancellation support for pending connections
//! - Periodic health checks to detect stale connections
//!
//! # State Machine
//!
//! ```text
//! Disconnected ──connect()──> Connecting
//!      ▲                          │
//!      │                    success/failure
//!      │                          ▼
//!      │                    Connected / Error
//!      │                          │
//!      │                  transport error / health fail
//!      │                          ▼
//!      └──cancel()────────── Reconnecting
//! ```

use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::connection::DaemonAddress;

/// Connection state to the DAQ daemon.
///
/// This enum represents all possible states of the connection lifecycle,
/// including reconnection attempts.
#[derive(Debug, Clone)]
pub enum ConnectionState {
    /// Not connected, no reconnection pending.
    Disconnected,

    /// Initial connection attempt in progress.
    Connecting,

    /// Successfully connected to the daemon.
    Connected {
        /// Time when connection was established
        #[allow(dead_code)]
        connected_at: Instant,
    },

    /// Auto-reconnecting after a failure.
    Reconnecting {
        /// Current reconnection attempt number (1-based)
        attempt: u32,
        /// When the next retry will occur
        next_retry_at: Instant,
        /// The error that triggered reconnection
        last_error: String,
    },

    /// Connection failed with an error.
    Error {
        /// Human-readable error message
        message: String,
        /// Whether auto-reconnect is appropriate for this error
        retriable: bool,
    },
}

impl ConnectionState {
    /// Returns true if a connection attempt is in progress.
    #[must_use]
    pub fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting | Self::Reconnecting { .. })
    }

    /// Returns true if connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    /// Returns true if in an error state.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// Returns the error message if in error or reconnecting state.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Error { message, .. } => Some(message),
            Self::Reconnecting { last_error, .. } => Some(last_error),
            _ => None,
        }
    }

    /// Returns a short status label for UI display.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting...",
            Self::Connected { .. } => "Connected",
            Self::Reconnecting { .. } => "Reconnecting...",
            Self::Error { .. } => "Error",
        }
    }
}

impl PartialEq for ConnectionState {
    fn eq(&self, other: &Self) -> bool {
        // Compare variants without comparing Instant fields
        match (self, other) {
            (Self::Disconnected, Self::Disconnected) => true,
            (Self::Connecting, Self::Connecting) => true,
            (Self::Connected { .. }, Self::Connected { .. }) => true,
            (Self::Reconnecting { attempt: a1, .. }, Self::Reconnecting { attempt: a2, .. }) => {
                a1 == a2
            }
            (
                Self::Error {
                    message: m1,
                    retriable: r1,
                },
                Self::Error {
                    message: m2,
                    retriable: r2,
                },
            ) => m1 == m2 && r1 == r2,
            _ => false,
        }
    }
}

/// Configuration for auto-reconnect behavior.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Initial delay before first reconnect attempt.
    pub initial_delay: Duration,
    /// Maximum delay between reconnect attempts.
    pub max_delay: Duration,
    /// Backoff multiplier (e.g., 2.0 for doubling).
    pub backoff_multiplier: f64,
    /// Maximum number of reconnect attempts (0 = unlimited).
    pub max_attempts: u32,
    /// Whether to add jitter to delays.
    pub jitter: bool,
    /// Whether auto-reconnect is enabled.
    pub enabled: bool,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            max_attempts: 0, // Unlimited
            jitter: true,
            enabled: true,
        }
    }
}

/// Configuration for periodic health checks.
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// How often to check connection health.
    pub interval: Duration,
    /// How long to wait for health check response.
    #[allow(dead_code)]
    pub timeout: Duration,
    /// Whether health checks are enabled.
    pub enabled: bool,
    /// Number of consecutive failures before triggering reconnect.
    pub failure_threshold: u32,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(5),
            enabled: true,
            failure_threshold: 2,
        }
    }
}

impl HealthConfig {
    /// Faster health checks for local/Tailscale connections.
    #[must_use]
    #[allow(dead_code)]
    pub fn fast() -> Self {
        Self {
            interval: Duration::from_secs(15),
            timeout: Duration::from_secs(3),
            enabled: true,
            failure_threshold: 2,
        }
    }
}

/// Health check status for monitoring connection state.
#[derive(Debug, Clone)]
pub struct HealthStatus {
    /// When the last health check was performed.
    pub last_check: Option<Instant>,
    /// When the last successful health check completed.
    pub last_success: Option<Instant>,
    /// Number of consecutive health check failures.
    pub consecutive_failures: u32,
    /// Whether a health check is currently in progress.
    pub check_in_progress: bool,
    /// RTT of the last successful health check in milliseconds (bd-j3xz.3.3).
    pub last_rtt_ms: Option<f64>,
    /// Total number of errors since connection established (bd-j3xz.3.3).
    pub total_errors: u32,
    /// When the last error occurred (bd-j3xz.3.3).
    pub last_error_at: Option<Instant>,
    /// The last error message for diagnostics (bd-j3xz.3.3).
    pub last_error_message: Option<String>,
}

impl ReconnectConfig {
    /// Calculate the delay for a given attempt number (1-based).
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base_delay = self.initial_delay.as_secs_f64()
            * self
                .backoff_multiplier
                .powi((attempt.saturating_sub(1)) as i32);
        let capped_delay = base_delay.min(self.max_delay.as_secs_f64());

        let final_delay = if self.jitter {
            // Add up to 25% jitter
            let jitter_factor = 1.0 + (rand_jitter() * 0.25);
            capped_delay * jitter_factor
        } else {
            capped_delay
        };

        Duration::from_secs_f64(final_delay)
    }

    /// Check if another reconnect attempt should be made.
    #[must_use]
    pub fn should_retry(&self, attempt: u32) -> bool {
        self.enabled && (self.max_attempts == 0 || attempt < self.max_attempts)
    }
}

/// Simple pseudo-random jitter using time-based seed.
fn rand_jitter() -> f64 {
    // Use nanoseconds as a simple source of randomness
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 1000) as f64 / 1000.0
}

/// Result of a connection attempt sent through the channel.
pub enum ConnectResult {
    /// Connection succeeded.
    /// The client is boxed to reduce enum size variance.
    Connected {
        client: Box<DaqClient>,
        daemon_version: Option<String>,
        address: DaemonAddress,
    },
    /// Connection failed.
    Failed {
        error: String,
        address: DaemonAddress,
        /// Whether this error is retriable (e.g., network timeout vs invalid address)
        retriable: bool,
    },
    /// Connection attempt was cancelled.
    Cancelled,
}

/// Manages connection state, reconnection logic, and health monitoring.
pub struct ConnectionManager {
    /// Current connection state
    state: ConnectionState,
    /// Reconnection configuration
    config: ReconnectConfig,
    /// Health check configuration
    health_config: HealthConfig,
    /// Health status tracking
    health_status: HealthStatus,
    /// Channel sender for connect results
    tx: mpsc::Sender<ConnectResult>,
    /// Channel receiver for connect results
    rx: mpsc::Receiver<ConnectResult>,
    /// Handle to cancel pending connection
    cancel_handle: Option<tokio::sync::oneshot::Sender<()>>,
    /// Current reconnect attempt (0 if not reconnecting)
    reconnect_attempt: u32,
}

impl ConnectionManager {
    /// Create a new connection manager.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(4);
        Self {
            state: ConnectionState::Disconnected,
            config: ReconnectConfig::default(),
            health_config: HealthConfig::default(),
            health_status: HealthStatus {
                last_check: None,
                last_success: None,
                consecutive_failures: 0,
                check_in_progress: false,
                last_rtt_ms: None,
                total_errors: 0,
                last_error_at: None,
                last_error_message: None,
            },
            tx,
            rx,
            cancel_handle: None,
            reconnect_attempt: 0,
        }
    }

    /// Create with custom reconnect configuration.
    #[allow(dead_code)]
    pub fn with_config(config: ReconnectConfig) -> Self {
        let mut manager = Self::new();
        manager.config = config;
        manager
    }

    /// Get the current connection state.
    #[must_use]
    pub fn state(&self) -> &ConnectionState {
        &self.state
    }

    /// Get the reconnect configuration.
    #[must_use]
    #[allow(dead_code)]
    pub fn config(&self) -> &ReconnectConfig {
        &self.config
    }

    /// Set the reconnect configuration.
    #[allow(dead_code)]
    pub fn set_config(&mut self, config: ReconnectConfig) {
        self.config = config;
    }

    /// Get the health check configuration.
    #[must_use]
    #[allow(dead_code)]
    pub fn health_config(&self) -> &HealthConfig {
        &self.health_config
    }

    /// Set the health check configuration.
    #[allow(dead_code)]
    pub fn set_health_config(&mut self, config: HealthConfig) {
        self.health_config = config;
    }

    /// Get the current health status.
    #[must_use]
    #[allow(dead_code)]
    pub fn health_status(&self) -> &HealthStatus {
        &self.health_status
    }

    /// Check if a connection attempt is in progress.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.cancel_handle.is_some()
    }

    /// Check if a health check should be performed.
    ///
    /// Returns `true` if connected, health checks are enabled, and enough time
    /// has passed since the last check.
    #[must_use]
    pub fn should_health_check(&self) -> bool {
        if !self.health_config.enabled || !self.state.is_connected() {
            return false;
        }
        if self.health_status.check_in_progress {
            return false;
        }
        match self.health_status.last_check {
            Some(last) => last.elapsed() >= self.health_config.interval,
            None => true, // No previous check, should check now
        }
    }

    /// Record a successful health check with RTT measurement (bd-j3xz.3.3).
    ///
    /// The `rtt_ms` parameter is the round-trip time of the health check in milliseconds.
    pub fn record_health_success(&mut self, rtt_ms: f64) {
        let now = Instant::now();
        self.health_status.last_check = Some(now);
        self.health_status.last_success = Some(now);
        self.health_status.consecutive_failures = 0;
        self.health_status.check_in_progress = false;
        self.health_status.last_rtt_ms = Some(rtt_ms);
        tracing::trace!("Health check passed (RTT: {:.1}ms)", rtt_ms);
    }

    /// Record a failed health check (bd-j3xz.3.3: enhanced diagnostics).
    ///
    /// Returns `true` if the failure threshold was reached and reconnection should start.
    pub fn record_health_failure(&mut self, error: &str) -> bool {
        let now = Instant::now();
        self.health_status.last_check = Some(now);
        self.health_status.consecutive_failures += 1;
        self.health_status.check_in_progress = false;
        // Track total errors and last error details (bd-j3xz.3.3)
        self.health_status.total_errors += 1;
        self.health_status.last_error_at = Some(now);
        self.health_status.last_error_message = Some(error.to_string());

        tracing::warn!(
            "Health check failed ({}/{}, total errors: {}): {}",
            self.health_status.consecutive_failures,
            self.health_config.failure_threshold,
            self.health_status.total_errors,
            error
        );

        self.health_status.consecutive_failures >= self.health_config.failure_threshold
    }

    /// Mark that a health check is in progress.
    pub fn mark_health_check_started(&mut self) {
        self.health_status.check_in_progress = true;
    }

    /// Trigger reconnection due to health check failure.
    pub fn trigger_health_reconnect(
        &mut self,
        address: DaemonAddress,
        runtime: &tokio::runtime::Runtime,
    ) {
        if !self.config.enabled {
            self.state = ConnectionState::Error {
                message: "Connection lost (health check failed)".into(),
                retriable: true,
            };
            return;
        }

        tracing::warn!("Health check threshold exceeded, triggering reconnect");
        self.start_reconnect(
            address,
            runtime,
            "Connection lost (health check failed)".into(),
        );
    }

    /// Reset health status (called on new connection).
    fn reset_health_status(&mut self) {
        self.health_status = HealthStatus {
            last_check: None,
            last_success: None,
            consecutive_failures: 0,
            check_in_progress: false,
            last_rtt_ms: None,
            total_errors: 0,
            last_error_at: None,
            last_error_message: None,
        };
    }

    /// Start a connection attempt.
    ///
    /// Returns `false` if a connection is already in progress.
    pub fn connect(&mut self, address: DaemonAddress, runtime: &tokio::runtime::Runtime) -> bool {
        if self.is_busy() {
            tracing::warn!("Connection attempt already in progress");
            return false;
        }

        self.state = ConnectionState::Connecting;
        self.reconnect_attempt = 0;
        tracing::info!(
            "Connecting to {} ({})",
            address.as_str(),
            address.source().label()
        );

        self.spawn_connect_task(address, runtime, None);
        true
    }

    /// Start a reconnection attempt after a delay.
    fn start_reconnect(
        &mut self,
        address: DaemonAddress,
        runtime: &tokio::runtime::Runtime,
        error: String,
    ) {
        self.reconnect_attempt += 1;

        if !self.config.should_retry(self.reconnect_attempt) {
            tracing::warn!(
                "Max reconnect attempts ({}) reached",
                self.config.max_attempts
            );
            self.state = ConnectionState::Error {
                message: format!("{} (max retries exceeded)", error),
                retriable: false,
            };
            return;
        }

        let delay = self.config.delay_for_attempt(self.reconnect_attempt);
        let next_retry_at = Instant::now() + delay;

        self.state = ConnectionState::Reconnecting {
            attempt: self.reconnect_attempt,
            next_retry_at,
            last_error: error.clone(),
        };

        tracing::info!(
            "Reconnect attempt {} in {:.1}s: {}",
            self.reconnect_attempt,
            delay.as_secs_f64(),
            error
        );

        self.spawn_connect_task(address, runtime, Some(delay));
    }

    /// Spawn the async connection task.
    fn spawn_connect_task(
        &mut self,
        address: DaemonAddress,
        runtime: &tokio::runtime::Runtime,
        delay: Option<Duration>,
    ) {
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        self.cancel_handle = Some(cancel_tx);

        let tx = self.tx.clone();

        runtime.spawn(async move {
            // Wait for delay if reconnecting
            if let Some(delay) = delay {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = cancel_rx => {
                        let _ = tx.send(ConnectResult::Cancelled).await;
                        return;
                    }
                }
            }

            // Attempt connection
            match DaqClient::connect(&address).await {
                Ok(mut client) => {
                    let daemon_version = match client.get_daemon_info().await {
                        Ok(info) => Some(info.version),
                        Err(e) => {
                            tracing::warn!("Connected but failed to get daemon info: {}", e);
                            None
                        }
                    };
                    let _ = tx
                        .send(ConnectResult::Connected {
                            client: Box::new(client),
                            daemon_version,
                            address,
                        })
                        .await;
                }
                Err(e) => {
                    let error_str = e.to_string();
                    // Determine if error is retriable
                    let retriable = is_retriable_error(&error_str);
                    let _ = tx
                        .send(ConnectResult::Failed {
                            error: error_str,
                            address,
                            retriable,
                        })
                        .await;
                }
            }
        });
    }

    /// Cancel any pending connection or reconnection.
    pub fn cancel(&mut self) {
        if let Some(cancel) = self.cancel_handle.take() {
            let _ = cancel.send(());
            tracing::info!("Connection attempt cancelled");
        }
        self.reconnect_attempt = 0;
        self.state = ConnectionState::Disconnected;
    }

    /// Disconnect from the daemon.
    pub fn disconnect(&mut self) {
        self.cancel();
        self.state = ConnectionState::Disconnected;
        tracing::info!("Disconnected from daemon");
    }

    /// Poll for connection results. Call this in the UI update loop.
    ///
    /// Returns the new client if connection succeeded.
    pub fn poll(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        address: &DaemonAddress,
    ) -> Option<(DaqClient, Option<String>)> {
        let result = match self.rx.try_recv() {
            Ok(r) => r,
            Err(_) => return None,
        };

        self.cancel_handle = None;

        match result {
            ConnectResult::Connected {
                client,
                daemon_version,
                address: connected_addr,
            } => {
                // Guard against zombie connections: only accept if we're expecting a result
                // (bd-d8mi: fixes race condition when user cancels while connect is pending)
                if !self.state.is_connecting() {
                    tracing::debug!("Ignored stale Connected result (state={:?})", self.state);
                    return None;
                }

                self.state = ConnectionState::Connected {
                    connected_at: Instant::now(),
                };
                self.reconnect_attempt = 0;
                self.reset_health_status(); // Reset health tracking for new connection
                tracing::info!("Connected to {}", connected_addr.as_str());
                Some((*client, daemon_version))
            }
            ConnectResult::Failed {
                error,
                address: failed_addr,
                retriable,
            } => {
                // Guard against stale failures: only process if we're expecting a result
                // (bd-d8mi: fixes race condition when user cancels while connect is pending)
                if !self.state.is_connecting() {
                    tracing::debug!(
                        "Ignored stale Failed result (state={:?}): {}",
                        self.state,
                        error
                    );
                    return None;
                }

                tracing::error!("Connection to {} failed: {}", failed_addr.as_str(), error);

                if retriable && self.config.enabled {
                    // Start reconnection
                    self.start_reconnect(address.clone(), runtime, error);
                } else {
                    self.state = ConnectionState::Error {
                        message: error,
                        retriable,
                    };
                    self.reconnect_attempt = 0;
                }
                None
            }
            ConnectResult::Cancelled => {
                self.state = ConnectionState::Disconnected;
                self.reconnect_attempt = 0;
                None
            }
        }
    }

    /// Get seconds until next reconnect attempt, if reconnecting.
    #[must_use]
    pub fn seconds_until_retry(&self) -> Option<f64> {
        if let ConnectionState::Reconnecting { next_retry_at, .. } = &self.state {
            let now = Instant::now();
            if *next_retry_at > now {
                Some((*next_retry_at - now).as_secs_f64())
            } else {
                Some(0.0)
            }
        } else {
            None
        }
    }

    /// Trigger a manual reconnect (resets attempt counter).
    pub fn retry(&mut self, address: DaemonAddress, runtime: &tokio::runtime::Runtime) {
        self.cancel();
        self.connect(address, runtime);
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Determine if an error is retriable.
fn is_retriable_error(error: &str) -> bool {
    let error_lower = error.to_lowercase();

    // Non-retriable errors
    if error_lower.contains("invalid url")
        || error_lower.contains("invalid uri")
        || error_lower.contains("invalid address")
        || error_lower.contains("unsupported scheme")
    {
        return false;
    }

    // Retriable errors (network issues)
    error_lower.contains("transport")
        || error_lower.contains("connection refused")
        || error_lower.contains("connection reset")
        || error_lower.contains("timed out")
        || error_lower.contains("timeout")
        || error_lower.contains("network")
        || error_lower.contains("dns")
        || error_lower.contains("resolve")
        || error_lower.contains("unreachable")
        || error_lower.contains("temporarily unavailable")
}

/// Convert a raw error message to a user-friendly description.
///
/// This maps technical gRPC/tonic errors to actionable messages.
#[must_use]
pub fn friendly_error_message(error: &str) -> String {
    let error_lower = error.to_lowercase();

    // Connection refused - daemon not running
    if error_lower.contains("connection refused") {
        return "Daemon not running. Start the daemon with: cargo run --bin rust-daq-daemon -- daemon".into();
    }

    // DNS/resolution errors
    if error_lower.contains("dns")
        || error_lower.contains("resolve")
        || error_lower.contains("no such host")
    {
        return "Cannot resolve hostname. Check the address or network connection.".into();
    }

    // Timeout errors
    if error_lower.contains("timed out") || error_lower.contains("timeout") {
        return "Connection timed out. The daemon may be overloaded or unreachable.".into();
    }

    // Connection reset
    if error_lower.contains("connection reset") {
        return "Connection was reset. The daemon may have restarted.".into();
    }

    // Network unreachable
    if error_lower.contains("unreachable") || error_lower.contains("network is down") {
        return "Network unreachable. Check your network connection.".into();
    }

    // TLS/certificate errors
    if error_lower.contains("certificate")
        || error_lower.contains("tls")
        || error_lower.contains("ssl")
    {
        return "TLS/certificate error. Check that the daemon supports the configured security."
            .into();
    }

    // Transport errors (generic)
    if error_lower.contains("transport") {
        return "Transport error. Connection to daemon was interrupted.".into();
    }

    // Invalid URL/URI
    if error_lower.contains("invalid url") || error_lower.contains("invalid uri") {
        return "Invalid address format. Use http://host:port format.".into();
    }

    // Fallback: return original error with prefix
    format!("Connection failed: {}", error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnect_delay_calculation() {
        let config = ReconnectConfig {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            max_attempts: 0,
            jitter: false,
            enabled: true,
        };

        assert_eq!(config.delay_for_attempt(1), Duration::from_secs(1));
        assert_eq!(config.delay_for_attempt(2), Duration::from_secs(2));
        assert_eq!(config.delay_for_attempt(3), Duration::from_secs(4));
        assert_eq!(config.delay_for_attempt(4), Duration::from_secs(8));
        assert_eq!(config.delay_for_attempt(5), Duration::from_secs(16));
        assert_eq!(config.delay_for_attempt(6), Duration::from_secs(30)); // Capped
        assert_eq!(config.delay_for_attempt(7), Duration::from_secs(30)); // Still capped
    }

    #[test]
    fn test_should_retry() {
        let config = ReconnectConfig {
            max_attempts: 5,
            enabled: true,
            ..Default::default()
        };

        assert!(config.should_retry(1));
        assert!(config.should_retry(4));
        assert!(!config.should_retry(5));
        assert!(!config.should_retry(10));

        // Unlimited attempts
        let unlimited = ReconnectConfig {
            max_attempts: 0,
            enabled: true,
            ..Default::default()
        };
        assert!(unlimited.should_retry(100));

        // Disabled
        let disabled = ReconnectConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!disabled.should_retry(1));
    }

    #[test]
    fn test_is_retriable_error() {
        // Retriable
        assert!(is_retriable_error("transport error"));
        assert!(is_retriable_error("connection refused"));
        assert!(is_retriable_error("request timed out"));
        assert!(is_retriable_error("DNS resolution failed"));

        // Not retriable
        assert!(!is_retriable_error("invalid URL"));
        assert!(!is_retriable_error("invalid uri scheme"));
        assert!(!is_retriable_error("unsupported scheme 'ftp'"));
    }

    #[test]
    fn test_connection_state_labels() {
        assert_eq!(ConnectionState::Disconnected.label(), "Disconnected");
        assert_eq!(ConnectionState::Connecting.label(), "Connecting...");
        assert_eq!(
            ConnectionState::Connected {
                connected_at: Instant::now()
            }
            .label(),
            "Connected"
        );
        assert_eq!(
            ConnectionState::Reconnecting {
                attempt: 1,
                next_retry_at: Instant::now(),
                last_error: "test".into()
            }
            .label(),
            "Reconnecting..."
        );
        assert_eq!(
            ConnectionState::Error {
                message: "test".into(),
                retriable: true
            }
            .label(),
            "Error"
        );
    }

    #[test]
    fn test_friendly_error_message() {
        // Connection refused
        assert!(friendly_error_message("connection refused").contains("Daemon not running"));

        // DNS errors
        assert!(friendly_error_message("dns resolution failed").contains("Cannot resolve hostname"));

        // Timeout
        assert!(friendly_error_message("request timed out").contains("timed out"));

        // Transport error
        assert!(friendly_error_message("transport error").contains("Transport error"));

        // Unknown error - should include original message
        let unknown = friendly_error_message("some random error");
        assert!(unknown.contains("Connection failed"));
        assert!(unknown.contains("some random error"));
    }
}
