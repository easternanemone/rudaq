//! Periodic health monitoring for PVCAM cameras (bd-oqo7.9).
//!
//! Polls `get_controller_alive()` and `get_ccs_status()` via `spawn_blocking`
//! (FFI calls must not run on the async runtime) and derives the current
//! [`PvcamDeviceState`]. On state transitions the monitor:
//!
//! 1. Logs the transition at `info` level.
//! 2. Persists a lifecycle event to SurrealDB (via the `_lifecycle_state`
//!    pseudo-parameter in `device_runtime_state`).
//! 3. Reports success/failure to the [`DeviceRegistry`] health tracker,
//!    which in turn fires webhook alerts through the existing alerting
//!    infrastructure.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::components::connection::PvcamConnection;
use crate::PvcamDeviceState;
use crate::components::features::PvcamFeatures;

/// Configuration for the health monitor polling interval.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Callback trait for health monitor state transitions.
///
/// Decouples the monitor from `DeviceRegistry` and `DaqDb` so the PVCAM
/// driver crate does not depend on `hardware` or `db` crates directly.
pub trait HealthMonitorCallback: Send + Sync + 'static {
    /// Called when the device state transitions.
    fn on_state_transition(
        &self,
        device_id: &str,
        old_state: PvcamDeviceState,
        new_state: PvcamDeviceState,
    );
}

/// A no-op callback used when no registry/db integration is available.
pub struct NoOpHealthCallback;

impl HealthMonitorCallback for NoOpHealthCallback {
    fn on_state_transition(
        &self,
        device_id: &str,
        old_state: PvcamDeviceState,
        new_state: PvcamDeviceState,
    ) {
        info!(
            device_id,
            %old_state,
            %new_state,
            "PVCAM state transition (no callback wired)"
        );
    }
}

/// Spawn the health monitor background task.
///
/// Returns the `CancellationToken` that stops the monitor. The caller should
/// drop or cancel this token during driver shutdown.
///
/// # Arguments
///
/// * `device_id` - Unique device identifier for logging and DB persistence.
/// * `connection` - Shared connection to the PVCAM SDK (behind `Mutex` for FFI safety).
/// * `callback` - Receives state transition notifications.
/// * `cancel` - Cancellation token; the monitor exits when this is cancelled.
pub fn spawn_health_monitor(
    device_id: String,
    connection: Arc<Mutex<PvcamConnection>>,
    callback: Arc<dyn HealthMonitorCallback>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_health_monitor(device_id, connection, callback, cancel).await;
    })
}

async fn run_health_monitor(
    device_id: String,
    connection: Arc<Mutex<PvcamConnection>>,
    callback: Arc<dyn HealthMonitorCallback>,
    cancel: CancellationToken,
) {
    let mut current_state = PvcamDeviceState::Initializing;
    let mut interval = tokio::time::interval(HEALTH_POLL_INTERVAL);

    // The first tick completes immediately; let the camera finish init.
    interval.tick().await;

    info!(device_id, "PVCAM health monitor started");

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(device_id, "PVCAM health monitor shutting down");
                return;
            }
            _ = interval.tick() => {
                let new_state = poll_device_state(&device_id, &connection).await;
                if new_state != current_state {
                    info!(
                        device_id,
                        old_state = %current_state,
                        new_state = %new_state,
                        "PVCAM lifecycle state transition"
                    );
                    callback.on_state_transition(&device_id, current_state, new_state);
                    current_state = new_state;
                }
            }
        }
    }
}

/// Poll the camera's controller_alive and CCS status via spawn_blocking.
async fn poll_device_state(
    device_id: &str,
    connection: &Arc<Mutex<PvcamConnection>>,
) -> PvcamDeviceState {
    let conn = connection.clone();
    let device_id_owned = device_id.to_owned();

    match tokio::task::spawn_blocking(move || {
        // Block on the async mutex from a blocking context.
        // This is safe because the lock is held only briefly for FFI calls.
        let conn_guard = conn.blocking_lock();
        let controller_alive = PvcamFeatures::get_controller_alive(&conn_guard)?;
        let ccs_status = PvcamFeatures::get_ccs_status(&conn_guard)?;
        Ok::<_, anyhow::Error>((controller_alive, ccs_status))
    })
    .await
    {
        Ok(Ok((alive, ccs))) => PvcamDeviceState::from_health_check(alive, ccs),
        Ok(Err(e)) => {
            warn!(
                device_id = device_id_owned,
                error = %e,
                "PVCAM health check SDK call failed"
            );
            PvcamDeviceState::Error
        }
        Err(e) => {
            warn!(
                device_id = device_id_owned,
                error = %e,
                "PVCAM health check task panicked"
            );
            PvcamDeviceState::Offline
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn test_state_derivation_ready() {
        assert_eq!(
            PvcamDeviceState::from_health_check(true, 0),
            PvcamDeviceState::Ready
        );
    }

    #[test]
    fn test_state_derivation_initializing() {
        assert_eq!(
            PvcamDeviceState::from_health_check(true, 1),
            PvcamDeviceState::Initializing
        );
    }

    #[test]
    fn test_state_derivation_streaming_running() {
        assert_eq!(
            PvcamDeviceState::from_health_check(true, 2),
            PvcamDeviceState::Streaming
        );
    }

    #[test]
    fn test_state_derivation_streaming_clearing() {
        assert_eq!(
            PvcamDeviceState::from_health_check(true, 3),
            PvcamDeviceState::Streaming
        );
    }

    #[test]
    fn test_state_derivation_offline() {
        assert_eq!(
            PvcamDeviceState::from_health_check(false, 0),
            PvcamDeviceState::Offline
        );
    }

    #[test]
    fn test_state_derivation_offline_with_nonzero_ccs() {
        // Even if CCS reports something, dead controller means offline.
        assert_eq!(
            PvcamDeviceState::from_health_check(false, 2),
            PvcamDeviceState::Offline
        );
    }

    #[test]
    fn test_state_derivation_error_unknown_ccs() {
        assert_eq!(
            PvcamDeviceState::from_health_check(true, 99),
            PvcamDeviceState::Error
        );
    }

    #[test]
    fn test_state_derivation_error_negative_ccs() {
        assert_eq!(
            PvcamDeviceState::from_health_check(true, -1),
            PvcamDeviceState::Error
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(PvcamDeviceState::Offline.to_string(), "offline");
        assert_eq!(PvcamDeviceState::Initializing.to_string(), "initializing");
        assert_eq!(PvcamDeviceState::Ready.to_string(), "ready");
        assert_eq!(PvcamDeviceState::Streaming.to_string(), "streaming");
        assert_eq!(PvcamDeviceState::Error.to_string(), "error");
        assert_eq!(PvcamDeviceState::ShuttingDown.to_string(), "shutting_down");
    }

    #[test]
    fn test_try_from_str() {
        assert_eq!(
            PvcamDeviceState::try_from("ready"),
            Ok(PvcamDeviceState::Ready)
        );
        assert_eq!(
            PvcamDeviceState::try_from("shutting_down"),
            Ok(PvcamDeviceState::ShuttingDown)
        );
        assert!(PvcamDeviceState::try_from("bogus").is_err());
    }

    #[test]
    fn test_serde_round_trip() {
        let state = PvcamDeviceState::Streaming;
        let json = serde_json::to_string(&state).expect("serialize");
        let deserialized: PvcamDeviceState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, state);
    }

    /// Callback that counts transitions for testing.
    struct CountingCallback {
        count: AtomicU32,
    }

    impl CountingCallback {
        fn new() -> Self {
            Self {
                count: AtomicU32::new(0),
            }
        }
    }

    impl HealthMonitorCallback for CountingCallback {
        fn on_state_transition(
            &self,
            _device_id: &str,
            _old_state: PvcamDeviceState,
            _new_state: PvcamDeviceState,
        ) {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn test_health_monitor_exits_on_cancel() {
        let conn = Arc::new(Mutex::new(PvcamConnection::new()));
        let callback = Arc::new(CountingCallback::new());
        let cancel = CancellationToken::new();

        let handle = spawn_health_monitor(
            "test_cam".to_string(),
            conn,
            callback.clone(),
            cancel.clone(),
        );

        // Cancel immediately -- monitor should exit promptly.
        cancel.cancel();

        let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(result.is_ok(), "health monitor should exit on cancel");
    }

    #[tokio::test]
    async fn test_health_monitor_detects_initial_transition() {
        // In mock mode, controller_alive=true and ccs_status=0, so state
        // transitions from Initializing -> Ready on the first poll.
        //
        // NOTE: Cannot use start_paused=true here because spawn_blocking +
        // blocking_lock interacts poorly with the paused-time runtime.
        // Instead we directly test poll_device_state which exercises the
        // same derivation logic.
        let conn = Arc::new(Mutex::new(PvcamConnection::new()));

        // Mock mode returns controller_alive=true, ccs_status=0 -> Ready.
        let state = poll_device_state("test_cam", &conn).await;
        assert_eq!(
            state,
            PvcamDeviceState::Ready,
            "mock camera should report Ready state"
        );
    }
}
