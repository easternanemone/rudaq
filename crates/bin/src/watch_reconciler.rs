//! Watch-based reconciler using SurrealDB LIVE SELECT.
//!
//! Subscribes to changes on the `instrument` table and triggers
//! `reconcile_once()` with debouncing.  Falls back to polling on errors
//! with automatic retry and exponential backoff (with jitter).
//!
//! Follows the k8s informer pattern: LIVE SELECT is a trigger, not the
//! sole source of truth.  Full resync on every reconnect, plus periodic
//! resync as a safety net for eventual consistency.

use std::sync::Arc;
use std::time::Duration;

use db::DaqDb;
use futures::StreamExt;
use hardware::registry::DeviceRegistry;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::reconciler;

/// Default debounce window for batching rapid changes.
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);

/// Default max debounce wait: caps how long rapid-fire notifications can
/// defer a reconcile (prevents starvation under sustained load).
const DEFAULT_MAX_DEBOUNCE_WAIT: Duration = Duration::from_secs(2);

/// Initial backoff before retrying LIVE SELECT after failure.
const INITIAL_RETRY_BACKOFF: Duration = Duration::from_secs(5);

/// Maximum backoff (caps exponential growth).
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(60);

/// Default periodic resync interval (k8s resync period pattern).
const DEFAULT_RESYNC_INTERVAL: Duration = Duration::from_secs(300);

/// Consecutive LIVE SELECT failures before escalating to error-level logging.
const CIRCUIT_BREAKER_THRESHOLD: u32 = 5;

/// Configuration for the watch reconciler.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Debounce window: coalesce rapid notifications into one reconcile.
    pub debounce: Duration,
    /// Maximum time a pending reconcile can be deferred by sustained rapid
    /// notifications. Prevents starvation under continuous change load.
    pub max_debounce_wait: Duration,
    /// Polling interval used as fallback when LIVE SELECT is unavailable.
    pub fallback_poll_interval: Duration,
    /// Periodic full resync interval (safety net for missed LIVE SELECT events).
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

/// Start a watch-based reconciler using LIVE SELECT.
///
/// On each change notification, debounces and triggers `reconcile_once()`.
/// On LIVE SELECT failure, does a polling fallback reconcile and retries
/// LIVE SELECT with exponential backoff (plus jitter to avoid thundering herd).
///
/// Runs until the `shutdown` token is cancelled.
#[tracing::instrument(skip_all, name = "watch_reconciler")]
pub async fn start_watch_reconciler(
    db: DaqDb,
    registry: Arc<DeviceRegistry>,
    config: WatchConfig,
    shutdown: CancellationToken,
) {
    info!(
        debounce_ms = config.debounce.as_millis(),
        fallback_poll_s = config.fallback_poll_interval.as_secs(),
        resync_s = config.resync_interval.as_secs(),
        "starting watch reconciler (LIVE SELECT)"
    );

    let mut backoff = INITIAL_RETRY_BACKOFF;
    let mut consecutive_failures: u32 = 0;

    loop {
        if shutdown.is_cancelled() {
            break;
        }

        // Try to establish LIVE SELECT stream.
        match db.live_instruments().await {
            Ok(mut stream) => {
                info!("LIVE SELECT stream established for instrument table");
                backoff = INITIAL_RETRY_BACKOFF; // Reset backoff on success.
                consecutive_failures = 0; // Reset circuit breaker.

                // Initial full resync on connect (k8s informer pattern).
                do_reconcile(&db, &registry, "watch: initial resync").await;

                // Process live notifications with debouncing + periodic resync.
                let disconnected = process_live_stream(
                    &mut stream,
                    &db,
                    &registry,
                    config.debounce,
                    config.max_debounce_wait,
                    config.resync_interval,
                    &shutdown,
                )
                .await;

                if !disconnected {
                    // Shutdown requested — exit.
                    return;
                }

                // Stream ended or errored — loop back to retry.
                consecutive_failures += 1;
                #[cfg(feature = "metrics")]
                if let Some(m) = crate::reconciler_metrics::get() {
                    m.watch_reconnections.inc();
                }
                log_live_select_failure(
                    consecutive_failures,
                    "LIVE SELECT stream disconnected, will reconnect",
                );
            }
            Err(e) => {
                consecutive_failures += 1;
                #[cfg(feature = "metrics")]
                if let Some(m) = crate::reconciler_metrics::get() {
                    m.watch_reconnections.inc();
                }
                log_live_select_failure(
                    consecutive_failures,
                    &format!(
                        "failed to start LIVE SELECT (backoff {}s): {e}",
                        backoff.as_secs()
                    ),
                );
            }
        }

        // Fallback: one polling reconcile before backoff.
        do_reconcile(&db, &registry, "watch fallback").await;

        // Exponential backoff with jitter before retrying LIVE SELECT.
        let jitter = Duration::from_millis(jitter_ms(1000));
        tokio::select! {
            () = shutdown.cancelled() => {
                info!("watch reconciler shutting down");
                return;
            }
            () = tokio::time::sleep(backoff + jitter) => {}
        }

        backoff = (backoff * 2).min(MAX_RETRY_BACKOFF);
    }
}

/// Log LIVE SELECT failure, escalating to error level after repeated failures.
fn log_live_select_failure(consecutive_failures: u32, msg: &str) {
    if consecutive_failures >= CIRCUIT_BREAKER_THRESHOLD {
        error!(
            consecutive_failures,
            "watch: {msg} (circuit breaker: {consecutive_failures} consecutive failures)"
        );
    } else {
        warn!(consecutive_failures, "watch: {msg}");
    }
}

/// Process live stream notifications with debouncing and periodic resync.
///
/// Returns `true` if the stream disconnected (caller should retry),
/// `false` if shutdown was requested.
async fn process_live_stream<S>(
    stream: &mut S,
    db: &DaqDb,
    registry: &DeviceRegistry,
    debounce: Duration,
    max_debounce_wait: Duration,
    resync_interval: Duration,
    shutdown: &CancellationToken,
) -> bool
where
    S: futures::Stream<
            Item = std::result::Result<
                db::surrealdb::Notification<db::config_store::DbInstrument>,
                db::surrealdb::Error,
            >,
        > + Unpin,
{
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
                return false;
            }
            item = stream.next() => {
                match item {
                    Some(Ok(notification)) => {
                        info!(
                            action = ?notification.action,
                            device_id = notification.data.device_id,
                            "watch: instrument change detected"
                        );
                        let now = tokio::time::Instant::now();
                        if !pending {
                            // First notification in batch — set the max wait cap.
                            max_deadline = Some(now + max_debounce_wait);
                        }
                        pending = true;
                        // Extend debounce, but clamp to max_deadline to prevent
                        // starvation under sustained rapid-fire changes.
                        let desired = now + debounce;
                        deadline = match max_deadline {
                            Some(cap) => desired.min(cap),
                            None => desired,
                        };
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "watch: LIVE SELECT error");
                        return true; // Disconnected.
                    }
                    None => {
                        warn!("watch: LIVE SELECT stream ended");
                        return true; // Disconnected.
                    }
                }
            }
            () = tokio::time::sleep_until(deadline), if pending => {
                pending = false;
                max_deadline = None; // Reset for next batch.
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
fn jitter_ms(max_ms: u64) -> u64 {
    // Use thread-id + nanosecond timestamp as entropy source.
    let seed = {
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let tid = std::thread::current().id();
        // Mix thread ID bits into the timestamp.
        now ^ (format!("{tid:?}").len() as u64).wrapping_mul(0x517c_c1b7_2722_0a95)
    };
    // xorshift64
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
#[cfg(feature = "db-surreal-mem")]
mod tests {
    use super::*;
    use db::config_store::DbInstrument;
    use db::DbConfig;
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
    async fn test_live_instruments_stream() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let mut stream = db.live_instruments().await.unwrap();

        // Insert an instrument — should produce a notification.
        db.upsert_instruments(&[sample_instrument("pm_live")])
            .await
            .unwrap();

        let notification = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("timeout waiting for LIVE SELECT notification")
            .expect("stream should not end")
            .expect("notification should not be an error");

        assert_eq!(notification.data.device_id, "pm_live");
    }

    #[tokio::test]
    async fn test_watch_reconciler_processes_change() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        let config = WatchConfig {
            debounce: Duration::from_millis(50),
            max_debounce_wait: Duration::from_secs(1),
            fallback_poll_interval: Duration::from_secs(60),
            resync_interval: Duration::ZERO, // Disable for test speed.
        };

        // Start watch reconciler in background.
        let db2 = db.clone();
        let reg2 = Arc::new(registry);
        let reg3 = reg2.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(db2, reg2, config, shutdown2).await;
        });

        // Give the reconciler time to establish LIVE SELECT.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Add an instrument to DB.
        db.upsert_instruments(&[sample_instrument("pm_watch")])
            .await
            .unwrap();

        // Wait for watch reconciler to process the notification + debounce.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Device should now be in the registry.
        let devices = reg3.list_devices();
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
        let reg = Arc::new(registry);
        let reg2 = reg.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(db2, reg, config, shutdown2).await;
        });

        // Give the reconciler time to establish LIVE SELECT.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Rapid-fire 5 inserts — should be debounced into one reconcile.
        for i in 0..5 {
            db.upsert_instruments(&[sample_instrument(&format!("bulk_{i}"))])
                .await
                .unwrap();
        }

        // Wait for debounce + reconcile.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let devices = reg2.list_devices();
        assert_eq!(devices.len(), 5, "all 5 bulk devices should be registered");

        shutdown.cancel();
    }

    #[tokio::test]
    async fn test_watch_reconciler_shutdown() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = Arc::new(test_registry());
        let shutdown = CancellationToken::new();

        let config = WatchConfig::default();
        let db2 = db.clone();
        let reg = registry.clone();
        let shutdown2 = shutdown.clone();

        let handle = tokio::spawn(async move {
            start_watch_reconciler(db2, reg, config, shutdown2).await;
        });

        // Let it start.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Request shutdown.
        shutdown.cancel();

        // Should exit cleanly.
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
        // Edge case: max_ms = 0 should not panic.
        assert_eq!(jitter_ms(0), 0);
    }
}
