//! System health monitoring for headless operation (bd-pauy)
//!
//! This module provides the SystemHealthMonitor service which tracks:
//! - Module heartbeats to detect unresponsive components
//! - Error collection from background tasks
//! - Overall system health status for remote monitoring

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Severity level for health errors
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorSeverity {
    /// Informational message
    Info = 0,
    /// Warning - degraded performance but still functional
    Warning = 1,
    /// Error - component malfunction, may affect data quality
    Error = 2,
    /// Critical - system-level failure requiring immediate attention
    Critical = 3,
}

impl std::fmt::Display for ErrorSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorSeverity::Info => write!(f, "INFO"),
            ErrorSeverity::Warning => write!(f, "WARNING"),
            ErrorSeverity::Error => write!(f, "ERROR"),
            ErrorSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// A health error event
#[derive(Debug, Clone)]
pub struct HealthError {
    /// Module/component that reported the error
    pub module_name: String,
    /// Error severity level
    pub severity: ErrorSeverity,
    /// Human-readable error message
    pub message: String,
    /// When the error occurred
    pub timestamp: Instant,
    /// Optional context (e.g., device ID, operation name)
    pub context: HashMap<String, String>,
}

/// Module health status
#[derive(Debug, Clone)]
pub struct ModuleHealth {
    /// Module identifier
    pub name: String,
    /// Last heartbeat timestamp
    pub last_heartbeat: Instant,
    /// Whether module is currently healthy
    pub is_healthy: bool,
    /// Optional status message from module
    pub status_message: Option<String>,
}

/// Overall system health status
#[derive(Debug, Clone)]
pub enum SystemHealth {
    /// All modules healthy, no errors
    Healthy,
    /// Some warnings or non-critical errors
    Degraded,
    /// Critical errors or unresponsive modules
    Critical,
}

/// Configuration for the health monitor
#[derive(Debug, Clone)]
pub struct HealthMonitorConfig {
    /// Maximum time since heartbeat before marking module as unhealthy
    pub heartbeat_timeout: Duration,
    /// Maximum number of errors to keep in history
    pub max_error_history: usize,
}

impl Default for HealthMonitorConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout: Duration::from_secs(30),
            max_error_history: 1000,
        }
    }
}

/// Inner state for SystemHealthMonitor
struct HealthMonitorState {
    /// Module heartbeat tracking
    module_heartbeats: HashMap<String, ModuleHealth>,
    /// Recent error history (circular buffer)
    error_history: VecDeque<HealthError>,
    /// Configuration
    config: HealthMonitorConfig,
}

/// System health monitor service
///
/// Tracks health of all running modules to prevent silent failures
/// in headless operation.
///
/// # Example
/// ```no_run
/// use rust_daq::health::SystemHealthMonitor;
///
/// #[tokio::main]
/// async fn main() {
///     let monitor = SystemHealthMonitor::new(Default::default());
///
///     // Module registers heartbeat
///     monitor.heartbeat("data_acquisition").await;
///
///     // Module reports error
///     monitor.report_error(
///         "camera",
///         rust_daq::health::ErrorSeverity::Error,
///         "Frame timeout",
///         vec![("device_id", "cam0")],
///     ).await;
///
///     // Check system health
///     let health = monitor.get_system_health().await;
///     println!("System health: {:?}", health);
/// }
/// ```
pub struct SystemHealthMonitor {
    state: Arc<RwLock<HealthMonitorState>>,
}

impl SystemHealthMonitor {
    /// Create a new health monitor with the given configuration
    pub fn new(config: HealthMonitorConfig) -> Self {
        let state = HealthMonitorState {
            module_heartbeats: HashMap::new(),
            error_history: VecDeque::new(),
            config,
        };

        Self {
            state: Arc::new(RwLock::new(state)),
        }
    }

    /// Record a heartbeat from a module
    ///
    /// Modules should call this periodically (e.g., every 5-10 seconds)
    /// to indicate they are still alive and functioning.
    pub async fn heartbeat(&self, module_name: impl Into<String>) {
        self.heartbeat_with_message(module_name, None).await;
    }

    /// Record a heartbeat with an optional status message
    pub async fn heartbeat_with_message(
        &self,
        module_name: impl Into<String>,
        status_message: Option<String>,
    ) {
        let module_name = module_name.into();
        let mut state = self.state.write().await;

        let health = ModuleHealth {
            name: module_name.clone(),
            last_heartbeat: Instant::now(),
            is_healthy: true,
            status_message,
        };

        state.module_heartbeats.insert(module_name, health);
    }

    /// Report an error from a module
    ///
    /// Errors are stored in a circular buffer and contribute to overall
    /// system health assessment.
    pub async fn report_error(
        &self,
        module_name: impl Into<String>,
        severity: ErrorSeverity,
        message: impl Into<String>,
        context: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) {
        let module_name = module_name.into();
        let message = message.into();
        let context: HashMap<String, String> = context
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();

        let error = HealthError {
            module_name,
            severity,
            message,
            timestamp: Instant::now(),
            context,
        };

        let mut state = self.state.write().await;

        // Add to circular buffer
        state.error_history.push_back(error);
        if state.error_history.len() > state.config.max_error_history {
            state.error_history.pop_front();
        }
    }

    /// Get the overall system health status
    pub async fn get_system_health(&self) -> SystemHealth {
        let state = self.state.read().await;
        let now = Instant::now();

        // Check for unresponsive modules
        let has_unresponsive = state.module_heartbeats.values().any(|health| {
            now.duration_since(health.last_heartbeat) > state.config.heartbeat_timeout
        });

        // Check for critical errors in recent history (last 5 minutes)
        let five_minutes_ago = now - Duration::from_secs(300);
        let has_critical_errors = state
            .error_history
            .iter()
            .filter(|err| err.timestamp > five_minutes_ago)
            .any(|err| err.severity == ErrorSeverity::Critical);

        if has_unresponsive || has_critical_errors {
            SystemHealth::Critical
        } else {
            // Check for recent warnings or errors
            let has_warnings = state
                .error_history
                .iter()
                .filter(|err| err.timestamp > five_minutes_ago)
                .any(|err| err.severity >= ErrorSeverity::Warning);

            if has_warnings {
                SystemHealth::Degraded
            } else {
                SystemHealth::Healthy
            }
        }
    }

    /// Get detailed health status for all modules
    pub async fn get_module_health(&self) -> Vec<ModuleHealth> {
        let mut state = self.state.write().await;
        let now = Instant::now();
        let timeout = state.config.heartbeat_timeout;

        // Update health status based on heartbeat timeout
        for health in state.module_heartbeats.values_mut() {
            let elapsed = now.duration_since(health.last_heartbeat);
            health.is_healthy = elapsed <= timeout;
        }

        state.module_heartbeats.values().cloned().collect()
    }

    /// Get recent error history
    ///
    /// Returns up to `limit` most recent errors, or all errors if limit is None.
    pub async fn get_error_history(&self, limit: Option<usize>) -> Vec<HealthError> {
        let state = self.state.read().await;

        let errors: Vec<_> = state.error_history.iter().cloned().collect();

        if let Some(limit) = limit {
            // Return most recent errors
            errors.into_iter().rev().take(limit).collect()
        } else {
            errors.into_iter().rev().collect()
        }
    }

    /// Get errors for a specific module
    pub async fn get_module_errors(&self, module_name: &str, limit: Option<usize>) -> Vec<HealthError> {
        let state = self.state.read().await;

        let errors: Vec<_> = state
            .error_history
            .iter()
            .filter(|err| err.module_name == module_name)
            .cloned()
            .collect();

        if let Some(limit) = limit {
            errors.into_iter().rev().take(limit).collect()
        } else {
            errors.into_iter().rev().collect()
        }
    }

    /// Clear all error history
    pub async fn clear_error_history(&self) {
        let mut state = self.state.write().await;
        state.error_history.clear();
    }

    /// Remove a module from tracking
    ///
    /// Useful when a module is intentionally shut down.
    pub async fn unregister_module(&self, module_name: &str) {
        let mut state = self.state.write().await;
        state.module_heartbeats.remove(module_name);
    }

    /// Get the number of registered modules
    pub async fn module_count(&self) -> usize {
        let state = self.state.read().await;
        state.module_heartbeats.len()
    }

    /// Get the number of errors in history
    pub async fn error_count(&self) -> usize {
        let state = self.state.read().await;
        state.error_history.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_heartbeat_tracking() {
        let monitor = SystemHealthMonitor::new(Default::default());

        // Register heartbeat
        monitor.heartbeat("test_module").await;

        let modules = monitor.get_module_health().await;
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "test_module");
        assert!(modules[0].is_healthy);
    }

    #[tokio::test]
    async fn test_heartbeat_timeout() {
        let config = HealthMonitorConfig {
            heartbeat_timeout: Duration::from_millis(100),
            max_error_history: 100,
        };
        let monitor = SystemHealthMonitor::new(config);

        monitor.heartbeat("test_module").await;

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        let modules = monitor.get_module_health().await;
        assert_eq!(modules.len(), 1);
        assert!(!modules[0].is_healthy);
    }

    #[tokio::test]
    async fn test_error_reporting() {
        let monitor = SystemHealthMonitor::new(Default::default());

        monitor
            .report_error(
                "test_module",
                ErrorSeverity::Error,
                "Test error",
                vec![("key", "value")],
            )
            .await;

        let errors = monitor.get_error_history(None).await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].module_name, "test_module");
        assert_eq!(errors[0].severity, ErrorSeverity::Error);
        assert_eq!(errors[0].message, "Test error");
        assert_eq!(errors[0].context.get("key"), Some(&"value".to_string()));
    }

    #[tokio::test]
    async fn test_system_health_degraded() {
        let monitor = SystemHealthMonitor::new(Default::default());

        // Report a warning
        monitor
            .report_error("test_module", ErrorSeverity::Warning, "Warning message", Vec::<(&str, &str)>::new())
            .await;

        let health = monitor.get_system_health().await;
        assert!(matches!(health, SystemHealth::Degraded));
    }

    #[tokio::test]
    async fn test_system_health_critical() {
        let monitor = SystemHealthMonitor::new(Default::default());

        // Report a critical error
        monitor
            .report_error("test_module", ErrorSeverity::Critical, "Critical error", Vec::<(&str, &str)>::new())
            .await;

        let health = monitor.get_system_health().await;
        assert!(matches!(health, SystemHealth::Critical));
    }

    #[tokio::test]
    async fn test_error_history_limit() {
        let config = HealthMonitorConfig {
            heartbeat_timeout: Duration::from_secs(30),
            max_error_history: 5,
        };
        let monitor = SystemHealthMonitor::new(config);

        // Report 10 errors
        for i in 0..10 {
            monitor
                .report_error(
                    "test_module",
                    ErrorSeverity::Info,
                    format!("Error {}", i),
                    Vec::<(&str, &str)>::new(),
                )
                .await;
        }

        let errors = monitor.get_error_history(None).await;
        assert_eq!(errors.len(), 5); // Only keeps last 5
    }

    #[tokio::test]
    async fn test_module_unregister() {
        let monitor = SystemHealthMonitor::new(Default::default());

        monitor.heartbeat("test_module").await;
        assert_eq!(monitor.module_count().await, 1);

        monitor.unregister_module("test_module").await;
        assert_eq!(monitor.module_count().await, 0);
    }
}
