//! Device supervisor for automatic fault recovery (bd-qa36.4.2).
//!
//! Periodically scans the device registry for faulted devices and attempts
//! restart with exponential backoff. Integrates with the daemon lifecycle
//! to provide automatic hardware recovery.
//!
//! # Design
//!
//! - Runs as a tokio task alongside the daemon
//! - Checks for faulted devices every `check_interval`
//! - Respects per-device backoff (exponential, capped at `max_backoff`)
//! - Stops when the CancellationToken is cancelled

use crate::registry::DeviceRegistry;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Configuration for the device supervisor.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// How often to check for faulted devices.
    pub check_interval: Duration,
    /// Base delay for restart backoff.
    pub base_backoff: Duration,
    /// Maximum delay between restart attempts.
    pub max_backoff: Duration,
    /// Maximum restart attempts before giving up on a device.
    /// Set to 0 for unlimited attempts.
    pub max_restart_attempts: u32,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(10),
            base_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(120),
            max_restart_attempts: 5,
        }
    }
}

/// Runs the device supervisor loop.
///
/// Scans the registry for faulted devices, respects backoff delays, and
/// attempts restart. Exits when `cancel` is triggered.
///
/// This is designed to be spawned as a tokio task:
///
/// ```rust,ignore
/// let supervisor_task = tokio::spawn(run_device_supervisor(
///     registry.clone(),
///     SupervisorConfig::default(),
///     cancel_token.clone(),
/// ));
/// ```
pub async fn run_device_supervisor(
    registry: Arc<DeviceRegistry>,
    config: SupervisorConfig,
    cancel: CancellationToken,
) {
    tracing::info!(
        check_interval_secs = config.check_interval.as_secs(),
        max_restart_attempts = config.max_restart_attempts,
        "Device supervisor started"
    );

    let mut interval = tokio::time::interval(config.check_interval);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                tracing::info!("Device supervisor shutting down");
                return;
            }
            _ = interval.tick() => {
                check_and_restart(&registry, &config).await;
            }
        }
    }
}

async fn check_and_restart(registry: &DeviceRegistry, config: &SupervisorConfig) {
    let faulted = registry.faulted_devices();
    if faulted.is_empty() {
        return;
    }

    for device_id in faulted {
        let Some(health) = registry.get_device_health(&device_id) else {
            continue;
        };

        // Skip if max restart attempts exceeded
        if config.max_restart_attempts > 0 && health.restart_attempts >= config.max_restart_attempts
        {
            // Only log once when we first hit the limit
            if health.restart_attempts == config.max_restart_attempts {
                tracing::error!(
                    device_id = %device_id,
                    restart_attempts = health.restart_attempts,
                    "Device restart limit reached — giving up"
                );
            }
            continue;
        }

        // Check if enough time has passed since last failure (backoff)
        let required_delay = health.backoff_delay(config.base_backoff, config.max_backoff);
        if let Some(last_failure) = health.last_failure {
            if last_failure.elapsed() < required_delay {
                tracing::debug!(
                    device_id = %device_id,
                    delay_remaining_secs = required_delay.saturating_sub(last_failure.elapsed()).as_secs(),
                    "Device restart backoff — waiting"
                );
                continue;
            }
        }

        tracing::info!(
            device_id = %device_id,
            attempt = health.restart_attempts + 1,
            "Attempting device restart"
        );

        match registry.restart_device(&device_id).await {
            Ok(true) => {
                tracing::info!(device_id = %device_id, "Device restarted successfully");
            }
            Ok(false) => {
                tracing::debug!(
                    device_id = %device_id,
                    "Device restart skipped (not faulted or not found)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    device_id = %device_id,
                    error = %e,
                    "Device restart failed — will retry with backoff"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supervisor_config_defaults() {
        let config = SupervisorConfig::default();
        assert_eq!(config.check_interval, Duration::from_secs(10));
        assert_eq!(config.base_backoff, Duration::from_secs(2));
        assert_eq!(config.max_backoff, Duration::from_secs(120));
        assert_eq!(config.max_restart_attempts, 5);
    }

    #[tokio::test]
    async fn test_supervisor_exits_on_cancel() {
        let registry = Arc::new(DeviceRegistry::new());
        let cancel = CancellationToken::new();

        let cancel_clone = cancel.clone();
        let task = tokio::spawn(async move {
            run_device_supervisor(registry, SupervisorConfig::default(), cancel_clone).await;
        });

        // Cancel immediately
        cancel.cancel();
        // Task should exit promptly
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("supervisor should exit within 2s")
            .expect("supervisor task should not panic");
    }

    #[tokio::test]
    async fn test_supervisor_no_faulted_devices_is_noop() {
        let registry = Arc::new(DeviceRegistry::new());
        let config = SupervisorConfig {
            check_interval: Duration::from_millis(50),
            ..Default::default()
        };

        // Run for a short time — should not panic or error
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let task = tokio::spawn(async move {
            run_device_supervisor(registry, config, cancel_clone).await;
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
        task.await.expect("supervisor should not panic");
    }
}
