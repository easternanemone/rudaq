//! Safety heartbeat task for hardware interlock (bd-nfav).
//!
//! Toggles a Comedi digital output bit at a fixed interval to drive an external
//! hardware interlock. If the daemon crashes or hangs, the pulse train stops and
//! the external circuitry cuts laser power.
//!
//! # Design
//!
//! - Runs as a Tokio task (not a bare OS thread) because the toggle interval is
//!   relaxed (100 ms) and a Tokio timer is sufficient.
//! - All Comedi FFI calls go through `spawn_blocking` to avoid blocking the
//!   runtime.
//! - Graceful shutdown via `CancellationToken`: on cancellation the channel is
//!   driven LOW (safe state) before the task exits.
//! - Transient toggle errors are logged and retried on the next interval; after
//!   `MAX_CONSECUTIVE_FAILURES` consecutive failures the task stops (the external
//!   interlock detects the missing pulse).

#[cfg(feature = "comedi_hardware")]
mod inner {
    use anyhow::{Context, Result};
    use hardware::registry::HeartbeatConfig;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    /// Maximum consecutive toggle failures before the task gives up.
    const MAX_CONSECUTIVE_FAILURES: u32 = 10;

    // =========================================================================
    // DIO operations trait (enables mocking in tests)
    // =========================================================================

    /// Abstraction over digital I/O toggle operations.
    ///
    /// The real implementation wraps Comedi FFI via `spawn_blocking`; tests
    /// supply a mock that records calls.
    pub trait DioOps: Send + Sync + 'static {
        /// Write a boolean value to the configured channel.
        ///
        /// Implementations MUST be safe to call from async context (e.g. via
        /// `spawn_blocking` for FFI).
        fn write(&self, value: bool) -> Result<()>;
    }

    // =========================================================================
    // Real Comedi DIO implementation
    // =========================================================================

    /// Real Comedi DIO operations.
    pub struct ComediDioOps {
        dio: driver_comedi::subsystem::digital_io::DigitalIO,
        channel: u32,
    }

    impl ComediDioOps {
        /// Open the Comedi device and configure the heartbeat channel as output.
        ///
        /// Must be called from a blocking context (or via `spawn_blocking`).
        pub fn init(config: &HeartbeatConfig) -> Result<Self> {
            let device =
                driver_comedi::device::ComediDevice::open(&config.device).with_context(|| {
                    format!(
                        "Safety heartbeat: failed to open Comedi device '{}'",
                        config.device
                    )
                })?;

            let dio = if let Some(idx) = config.subdevice {
                device.digital_io_subdevice(idx)
            } else {
                device.digital_io()
            }
            .context("Safety heartbeat: failed to get DIO subsystem")?;

            // Configure channel as output
            dio.configure(
                config.channel,
                driver_comedi::subsystem::digital_io::DioDirection::Output,
            )
            .with_context(|| {
                format!(
                    "Safety heartbeat: failed to configure channel {} as output",
                    config.channel
                )
            })?;

            Ok(Self {
                dio,
                channel: config.channel,
            })
        }
    }

    impl DioOps for ComediDioOps {
        fn write(&self, value: bool) -> Result<()> {
            self.dio
                .write(self.channel, value)
                .with_context(|| format!("Safety heartbeat: DIO write failed (value={value})"))
        }
    }

    // =========================================================================
    // Heartbeat loop (generic over DioOps for testability)
    // =========================================================================

    /// Run the heartbeat toggle loop.
    ///
    /// Toggles the DIO channel on each interval tick. On cancellation, sets the
    /// channel LOW before returning.
    pub async fn run_heartbeat_loop(
        ops: impl DioOps,
        interval: Duration,
        cancel: CancellationToken,
    ) {
        use std::sync::Arc;

        let ops = Arc::new(ops);
        let mut ticker = tokio::time::interval(interval);
        let mut state = false;
        let mut consecutive_failures: u32 = 0;

        tracing::info!(
            interval_ms = interval.as_millis() as u64,
            "Safety heartbeat task started"
        );

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("Safety heartbeat: cancellation received, setting channel LOW");
                    let ops_clone = ops.clone();
                    let _ = tokio::task::spawn_blocking(move || ops_clone.write(false)).await;
                    return;
                }
                _ = ticker.tick() => {
                    state = !state;
                    let ops_clone = ops.clone();
                    let val = state;
                    let result = tokio::task::spawn_blocking(move || ops_clone.write(val)).await;

                    match result {
                        Ok(Ok(())) => {
                            consecutive_failures = 0;
                        }
                        Ok(Err(e)) => {
                            consecutive_failures += 1;
                            tracing::warn!(
                                error = %e,
                                consecutive = consecutive_failures,
                                "Safety heartbeat: toggle failed"
                            );
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            tracing::warn!(
                                error = %e,
                                consecutive = consecutive_failures,
                                "Safety heartbeat: spawn_blocking join error"
                            );
                        }
                    }

                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        tracing::error!(
                            "Safety heartbeat: {} consecutive failures — stopping \
                             (external interlock will detect missing pulse)",
                            MAX_CONSECUTIVE_FAILURES
                        );
                        return;
                    }
                }
            }
        }
    }

    /// Spawn the safety heartbeat task.
    ///
    /// Returns `Ok(JoinHandle)` on success, or `Err` if initialization fails
    /// (the daemon continues without the heartbeat in that case).
    pub async fn spawn_heartbeat(
        config: HeartbeatConfig,
        cancel: CancellationToken,
    ) -> Result<tokio::task::JoinHandle<()>> {
        if !config.enabled {
            anyhow::bail!("Safety heartbeat is disabled in config");
        }

        let interval = Duration::from_millis(config.interval_ms);

        // Open Comedi device on a blocking thread (FFI)
        let ops = tokio::task::spawn_blocking(move || ComediDioOps::init(&config))
            .await
            .context("Safety heartbeat: spawn_blocking join error during init")??;

        let handle = tokio::spawn(run_heartbeat_loop(ops, interval, cancel));
        Ok(handle)
    }

    // =========================================================================
    // Tests
    // =========================================================================

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        use std::sync::Arc;

        /// Simple mock DIO that records writes.
        struct TestDio {
            write_count: Arc<AtomicU32>,
            last_value: Arc<AtomicBool>,
            fail_after: Option<u32>,
        }

        impl TestDio {
            fn new() -> (Self, Arc<AtomicU32>, Arc<AtomicBool>) {
                let count = Arc::new(AtomicU32::new(0));
                let value = Arc::new(AtomicBool::new(false));
                (
                    Self {
                        write_count: count.clone(),
                        last_value: value.clone(),
                        fail_after: None,
                    },
                    count,
                    value,
                )
            }

            fn failing_after(n: u32) -> (Self, Arc<AtomicU32>, Arc<AtomicBool>) {
                let count = Arc::new(AtomicU32::new(0));
                let value = Arc::new(AtomicBool::new(false));
                (
                    Self {
                        write_count: count.clone(),
                        last_value: value.clone(),
                        fail_after: Some(n),
                    },
                    count,
                    value,
                )
            }
        }

        impl DioOps for TestDio {
            fn write(&self, value: bool) -> Result<()> {
                let n = self.write_count.fetch_add(1, Ordering::SeqCst);
                if let Some(limit) = self.fail_after {
                    if n >= limit {
                        anyhow::bail!("simulated DIO failure");
                    }
                }
                self.last_value.store(value, Ordering::SeqCst);
                Ok(())
            }
        }

        #[tokio::test(start_paused = true)]
        async fn test_heartbeat_toggles() {
            let (dio, count, _value) = TestDio::new();
            let cancel = CancellationToken::new();
            let cancel_clone = cancel.clone();

            let handle = tokio::spawn(run_heartbeat_loop(
                dio,
                Duration::from_millis(100),
                cancel_clone,
            ));

            // Advance 1 second = 10 intervals = ~10 toggles
            // (first tick fires immediately, so we get 11 ticks in 1s)
            tokio::time::advance(Duration::from_secs(1)).await;
            tokio::task::yield_now().await;

            cancel.cancel();
            let _ = handle.await;

            // We should have roughly 11 ticks + 1 final LOW write on cancel = 12
            // Allow some tolerance due to async scheduling
            let writes = count.load(Ordering::SeqCst);
            assert!(
                writes >= 10,
                "Expected at least 10 writes in 1s at 100ms interval, got {writes}"
            );
        }

        #[tokio::test(start_paused = true)]
        async fn test_heartbeat_cancel_sets_low() {
            let (dio, _count, last_value) = TestDio::new();
            let cancel = CancellationToken::new();
            let cancel_clone = cancel.clone();

            let handle = tokio::spawn(run_heartbeat_loop(
                dio,
                Duration::from_millis(100),
                cancel_clone,
            ));

            // Let it toggle a few times
            tokio::time::advance(Duration::from_millis(350)).await;
            tokio::task::yield_now().await;

            cancel.cancel();
            let _ = handle.await;

            // The final write on cancellation should be LOW (false)
            assert!(
                !last_value.load(Ordering::SeqCst),
                "Channel should be LOW after cancellation"
            );
        }

        #[tokio::test(start_paused = true)]
        async fn test_heartbeat_stops_after_consecutive_failures() {
            // Fail after the 2nd write (so 3rd write onwards fails)
            let (dio, count, _value) = TestDio::failing_after(2);
            let cancel = CancellationToken::new();

            let handle = tokio::spawn(run_heartbeat_loop(
                dio,
                Duration::from_millis(100),
                cancel.clone(),
            ));

            // Advance enough time for 2 successes + 10 failures
            tokio::time::advance(Duration::from_secs(2)).await;
            tokio::task::yield_now().await;

            // Task should have exited on its own
            let _ = handle.await;

            // 2 successes + 10 failures = 12 total writes attempted
            let writes = count.load(Ordering::SeqCst);
            assert_eq!(
                writes, 12,
                "Expected 2 + 10 = 12 write attempts, got {writes}"
            );
        }

        #[tokio::test(start_paused = true)]
        async fn test_heartbeat_recovers_from_transient_error() {
            // A DIO that fails on exactly the 3rd call but succeeds otherwise
            struct TransientFailDio {
                count: Arc<AtomicU32>,
            }

            impl DioOps for TransientFailDio {
                fn write(&self, _value: bool) -> Result<()> {
                    let n = self.count.fetch_add(1, Ordering::SeqCst);
                    if n == 2 {
                        anyhow::bail!("transient failure");
                    }
                    Ok(())
                }
            }

            let count = Arc::new(AtomicU32::new(0));
            let dio = TransientFailDio {
                count: count.clone(),
            };
            let cancel = CancellationToken::new();
            let cancel_clone = cancel.clone();

            let handle = tokio::spawn(run_heartbeat_loop(
                dio,
                Duration::from_millis(100),
                cancel_clone,
            ));

            // Run for 1 second — task should still be alive despite one failure
            tokio::time::advance(Duration::from_secs(1)).await;
            tokio::task::yield_now().await;

            cancel.cancel();
            let _ = handle.await;

            let writes = count.load(Ordering::SeqCst);
            assert!(
                writes >= 10,
                "Task should continue running after transient error, got {writes} writes"
            );
        }
    }
}

#[cfg(feature = "comedi_hardware")]
pub use inner::spawn_heartbeat;
