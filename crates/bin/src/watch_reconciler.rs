//! Watch-based reconciler using SQLite broadcast channel (bd-muyth).
//!
//! Subscribes to `DbChangeEvent::InstrumentsUpdated` via `subscribe_changes()`
//! and triggers `reconcile_once()` with debouncing. Falls back to immediate
//! resync when the broadcast receiver lags.
//!
//! Follows the k8s informer pattern: the broadcast channel is a trigger, not the
//! sole source of truth.  Full resync on every reconnect, plus periodic
//! resync as a safety net for eventual consistency.

use std::time::Duration;

use db::DaqDb;
use hardware::registry::DeviceRegistry;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::reconciler;

/// Default debounce window for batching rapid changes.
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Default max debounce wait: caps how long rapid-fire notifications can
/// defer a reconcile (prevents starvation under sustained load).
const DEFAULT_MAX_DEBOUNCE_WAIT: Duration = Duration::from_secs(2);

/// Default periodic resync interval (k8s resync period pattern).
const DEFAULT_RESYNC_INTERVAL: Duration = Duration::from_secs(300);

/// Configuration for the watch reconciler.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Debounce window: coalesce rapid notifications into one reconcile.
    pub debounce: Duration,
    /// Maximum time a pending reconcile can be deferred by sustained rapid
    /// notifications. Prevents starvation under continuous change load.
    pub max_debounce_wait: Duration,
    /// Polling interval (retained for interface compatibility with callers).
    #[expect(dead_code, reason = "broadcast channel doesn't need polling fallback")]
    pub fallback_poll_interval: Duration,
    /// Periodic full resync interval (safety net for missed events).
    /// Set to `Duration::ZERO` to disable periodic resync.
    pub resync_interval: Duration,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
            max_debounce_wait: DEFAULT_MAX_DEBOUNCE_WAIT,
            fallback_poll_interval: Duration::from_secs(30),
            resync_interval: DEFAULT_RESYNC_INTERVAL,
        }
    }
}

/// Start a watch-based reconciler using the SQLite broadcast channel.
///
/// On each `DbChangeEvent::InstrumentsUpdated`, debounces and triggers
/// `reconcile_once()`. The broadcast channel is reliable within the process
/// (no network disconnects), so the retry/backoff logic from the SurrealDB
/// LIVE SELECT version is simplified to just lagged-receiver recovery.
///
/// Runs until the `shutdown` token is cancelled.
#[tracing::instrument(skip_all, name = "watch_reconciler")]
pub async fn start_watch_reconciler(
    db: DaqDb,
    registry: DeviceRegistry,
    config: WatchConfig,
    shutdown: CancellationToken,
) {
    info!(
        debounce_ms = config.debounce.as_millis(),
        resync_s = config.resync_interval.as_secs(),
        "starting watch reconciler (broadcast channel)"
    );

    // Initial full resync (k8s informer pattern).
    do_reconcile(&db, &registry, "watch: initial resync").await;

    let mut change_rx = db.subscribe_changes();

    process_broadcast_stream(
        &mut change_rx,
        &db,
        &registry,
        config.debounce,
        config.max_debounce_wait,
        config.resync_interval,
        &shutdown,
    )
    .await;
}

/// Process broadcast channel events with debouncing and periodic resync.
async fn process_broadcast_stream(
    change_rx: &mut broadcast::Receiver<db::DbChangeEvent>,
    db: &DaqDb,
    registry: &DeviceRegistry,
    debounce: Duration,
    max_debounce_wait: Duration,
    resync_interval: Duration,
    shutdown: &CancellationToken,
) {
    let mut pending = false;
    let mut deadline = tokio::time::Instant::now(); // Past — won't fire.

    // Max deadline caps how far debounce can be pushed by sustained
    // rapid notifications — prevents reconcile starvation.
    let mut max_deadline: Option<tokio::time::Instant> = None;

    // Periodic resync timer (k8s resync period pattern).
    let resync_enabled = resync_interval > Duration::ZERO;
    let mut resync_timer = tokio::time::interval(if resync_enabled {
        resync_interval
    } else {
        Duration::from_secs(86400) // Effectively disabled — won't fire.
    });
    resync_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Skip the first immediate tick (initial resync already done by caller).
    resync_timer.tick().await;

    loop {
        tokio::select! {
            () = shutdown.cancelled() => {
                info!("watch reconciler shutting down");
                return;
            }
            result = change_rx.recv() => {
                match result {
                    Ok(db::DbChangeEvent::InstrumentsUpdated) => {
                        info!("watch: instrument change detected");
                        let now = tokio::time::Instant::now();
                        if !pending {
                            max_deadline = Some(now + max_debounce_wait);
                        }
                        pending = true;
                        let desired = now + debounce;
                        deadline = match max_deadline {
                            Some(cap) => desired.min(cap),
                            None => desired,
                        };
                    }
                    Ok(_) => {
                        // Other change types — not relevant for reconciliation
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(missed = n, "watch: broadcast receiver lagged, triggering resync");
                        pending = false;
                        max_deadline = None;
                        do_reconcile(db, registry, "watch: lagged resync").await;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        error!("watch: broadcast channel closed, reconciler stopping");
                        return;
                    }
                }
            }
            () = tokio::time::sleep_until(deadline), if pending => {
                pending = false;
                max_deadline = None;
                do_reconcile(db, registry, "watch triggered").await;
            }
            _ = resync_timer.tick(), if resync_enabled => {
                do_reconcile(db, registry, "watch periodic resync").await;
            }
        }
    }
}

/// Run `reconcile_once` and log the result.
#[tracing::instrument(skip(db, registry), name = "do_reconcile")]
async fn do_reconcile(db: &DaqDb, registry: &DeviceRegistry, context: &str) {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    match reconciler::reconcile_once(db, registry).await {
        Ok(report) => {
            #[cfg(feature = "metrics")]
            if let Some(m) = crate::reconciler_metrics::get() {
                m.reconcile_duration.observe(start.elapsed().as_secs_f64());
                m.record_report(&report);
            }
            if !report.added.is_empty()
                || !report.removed.is_empty()
                || !report.updated.is_empty()
                || !report.errors.is_empty()
            {
                info!(%report, "{context}");
            }
        }
        Err(e) => {
            #[cfg(feature = "metrics")]
            if let Some(m) = crate::reconciler_metrics::get() {
                m.reconcile_duration.observe(start.elapsed().as_secs_f64());
                m.reconcile_errors.inc();
            }
            warn!(error = %e, "{context}: reconcile failed");
        }
    }
}

/// Generate a random jitter value in milliseconds using a lightweight xorshift.
///
/// Avoids adding `rand` as a dependency for a single use case.
#[cfg(test)] // Only used in tests; retained for future retry backoff
fn jitter_ms(max_ms: u64) -> u64 {
    let seed = {
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let tid = std::thread::current().id();
        now ^ (format!("{tid:?}").len() as u64).wrapping_mul(0x517c_c1b7_2722_0a95)
    };
    let mut x = seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x % max_ms.max(1)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "db")]
mod tests {
    use super::*;
    use db::DbConfig;
    use db::config_store::DbInstrument;
    use driver_mock::MockPowerMeterFactory;

    fn test_registry() -> DeviceRegistry {
        let registry = DeviceRegistry::new();
        registry.register_factory(Box::new(MockPowerMeterFactory));
        registry
    }

    fn sample_instrument(id: &str) -> DbInstrument {
        DbInstrument {
            device_id: id.into(),
            name: format!("Test {id}"),
            driver_type: "mock_power_meter".into(),
            config: serde_json::json!({}),
            enabled: true,
        }
    }

    #[tokio::test]
    async fn test_broadcast_triggers_reconcile() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        let config = WatchConfig {
            debounce: Duration::from_millis(50),
            max_debounce_wait: Duration::from_secs(1),
            fallback_poll_interval: Duration::from_secs(60),
            resync_interval: Duration::ZERO,
        };

        let db2 = db.clone();
        let reg = registry;
        let reg2 = reg.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(db2, reg, config, shutdown2).await;
        });

        // Give the reconciler time to subscribe.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Insert — broadcasts InstrumentsUpdated.
        db.upsert_instruments(&[sample_instrument("pm_watch")])
            .await
            .unwrap();

        // Wait for debounce + reconcile.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let devices = reg2.list_devices();
        assert!(
            devices.iter().any(|d| d.id == "pm_watch"),
            "watch reconciler should have added pm_watch to registry, found: {:?}",
            devices.iter().map(|d| &d.id).collect::<Vec<_>>()
        );

        shutdown.cancel();
    }

    #[tokio::test]
    async fn test_watch_reconciler_debounces_bulk_changes() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        let config = WatchConfig {
            debounce: Duration::from_millis(100),
            max_debounce_wait: Duration::from_secs(1),
            fallback_poll_interval: Duration::from_secs(60),
            resync_interval: Duration::ZERO,
        };

        let db2 = db.clone();
        let reg = registry;
        let reg2 = reg.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(db2, reg, config, shutdown2).await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        for i in 0..5 {
            db.upsert_instruments(&[sample_instrument(&format!("bulk_{i}"))])
                .await
                .unwrap();
        }

        tokio::time::sleep(Duration::from_millis(500)).await;

        let devices = reg2.list_devices();
        assert_eq!(devices.len(), 5, "all 5 bulk devices should be registered");

        shutdown.cancel();
    }

    #[tokio::test]
    async fn test_watch_reconciler_shutdown() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        let config = WatchConfig::default();
        let db2 = db.clone();
        let reg = registry.clone();
        let shutdown2 = shutdown.clone();

        let handle = tokio::spawn(async move {
            start_watch_reconciler(db2, reg, config, shutdown2).await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.cancel();

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("watch reconciler should shut down within 2s")
            .expect("task should not panic");
    }

    #[test]
    fn test_jitter_produces_bounded_values() {
        for _ in 0..100 {
            let j = jitter_ms(1000);
            assert!(j < 1000, "jitter should be < max_ms, got {j}");
        }
        assert_eq!(jitter_ms(0), 0);
    }
}
