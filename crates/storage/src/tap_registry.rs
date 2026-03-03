use anyhow::{anyhow, Result};
use std::collections::HashMap;
#[cfg(feature = "metrics")]
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
#[cfg(feature = "metrics")]
use std::sync::Mutex;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[cfg(feature = "metrics")]
use prometheus::{register_gauge_vec, register_int_counter_vec, GaugeVec, IntCounterVec};
#[cfg(feature = "metrics")]
use std::sync::LazyLock;

/// Default channel capacity for tap consumers (number of frames buffered)
const DEFAULT_TAP_CHANNEL_SIZE: usize = 32;
const DROP_LOG_INTERVAL_SECS: u64 = 1;
#[cfg(feature = "metrics")]
const MAX_TAP_METRICS_LABELS: usize = 100;
#[cfg(feature = "metrics")]
const TAP_METRICS_OTHER_LABEL: &str = "other";

#[cfg(feature = "metrics")]
static TAP_FRAMES_DROPPED_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "tap_frames_dropped_total",
        "Total frames dropped by tap consumers due to backpressure",
        &["tap_id"]
    )
    .expect("Failed to register tap_frames_dropped_total")
});
#[cfg(feature = "metrics")]
static TAP_CHANNEL_UTILIZATION: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "tap_channel_utilization",
        "Tap channel utilization (0.0-1.0)",
        &["tap_id"]
    )
    .expect("Failed to register tap_channel_utilization")
});
#[cfg(feature = "metrics")]
static TAP_METRICS_LABELS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// A tap consumer that receives every Nth frame from the ring buffer.
#[derive(Debug)]
pub struct TapConsumer {
    /// Unique identifier for this tap
    pub id: String,

    /// Deliver every nth frame (1 = every frame, 10 = every 10th frame)
    pub nth_frame: usize,

    /// Current frame count for this tap (internal counter)
    frame_count: AtomicU64,

    /// Async channel sender for delivering frames
    /// Uses try_send to avoid blocking on backpressure
    sender: mpsc::Sender<Vec<u8>>,

    /// Number of dropped frames due to backpressure
    dropped_frames: AtomicU64,
    /// Number of frames successfully delivered
    delivered_frames: AtomicU64,
    /// Channel capacity for utilization tracking
    channel_capacity: usize,
    /// Last time we logged a drop (epoch seconds)
    last_drop_log_secs: AtomicU64,
    /// Metrics label for this tap (bounded to avoid cardinality blowups)
    #[allow(dead_code)] // read only when `metrics` feature is enabled
    metrics_label: String,
}

impl TapConsumer {
    /// Create a new tap consumer
    pub fn new(
        id: String,
        nth_frame: usize,
        channel_capacity: usize,
        sender: mpsc::Sender<Vec<u8>>,
    ) -> Self {
        let metrics_label = allocate_metrics_label(&id);
        Self {
            id,
            nth_frame: nth_frame.max(1), // Ensure at least 1
            frame_count: AtomicU64::new(0),
            sender,
            dropped_frames: AtomicU64::new(0),
            delivered_frames: AtomicU64::new(0),
            channel_capacity: channel_capacity.max(1),
            last_drop_log_secs: AtomicU64::new(0),
            metrics_label,
        }
    }

    /// Check if this frame should be delivered based on nth_frame setting
    pub fn should_deliver(&self) -> bool {
        let count = self.frame_count.fetch_add(1, Ordering::Relaxed);
        count.is_multiple_of(self.nth_frame as u64)
    }

    /// Attempt to send a frame without blocking
    /// Returns true if sent successfully, false if dropped due to backpressure
    pub fn try_send_frame(&self, data: Vec<u8>) -> bool {
        match self.sender.try_send(data) {
            Ok(_) => {
                self.delivered_frames.fetch_add(1, Ordering::Relaxed);
                self.update_channel_metrics();
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                let dropped = self.dropped_frames.fetch_add(1, Ordering::Relaxed) + 1;
                self.update_channel_metrics();
                self.increment_drop_metric();
                self.maybe_log_drop(dropped);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false, // Receiver closed
        }
    }

    /// Get dropped frame count
    pub fn dropped_count(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    /// Get delivered frame count.
    pub fn delivered_count(&self) -> u64 {
        self.delivered_frames.load(Ordering::Relaxed)
    }

    /// Channel capacity.
    pub fn channel_capacity(&self) -> usize {
        self.channel_capacity
    }

    /// Channel available slots (remaining capacity).
    pub fn channel_available(&self) -> usize {
        self.sender.capacity()
    }

    /// Channel utilization ratio (0.0-1.0).
    pub fn channel_utilization(&self) -> f64 {
        let capacity = self.channel_capacity.max(1);
        let available = self.sender.capacity().min(capacity);
        let used = capacity.saturating_sub(available);
        #[allow(clippy::cast_precision_loss)]
        // SAFETY: precision loss acceptable for channel utilization metric
        let used_f64 = used as f64;
        #[allow(clippy::cast_precision_loss)]
        // SAFETY: precision loss acceptable for channel utilization metric
        let cap_f64 = capacity as f64;
        used_f64 / cap_f64
    }

    fn maybe_log_drop(&self, dropped_total: u64) {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let last = self.last_drop_log_secs.load(Ordering::Relaxed);
        if now_secs.saturating_sub(last) < DROP_LOG_INTERVAL_SECS {
            return;
        }
        if self
            .last_drop_log_secs
            .compare_exchange(last, now_secs, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            tracing::warn!(
                tap_id = %self.id,
                dropped_total = dropped_total,
                channel_capacity = self.channel_capacity,
                channel_available = self.channel_available(),
                "Tap consumer backing up - dropped frame (channel full)"
            );
        }
    }

    fn update_channel_metrics(&self) {
        #[cfg(feature = "metrics")]
        {
            let utilization = self.channel_utilization();
            TAP_CHANNEL_UTILIZATION
                .with_label_values(&[self.metrics_label.as_str()])
                .set(utilization);
        }
    }

    fn increment_drop_metric(&self) {
        #[cfg(feature = "metrics")]
        {
            TAP_FRAMES_DROPPED_TOTAL
                .with_label_values(&[self.metrics_label.as_str()])
                .inc();
        }
    }
}

/// Tap health snapshot for monitoring
#[derive(Debug, Clone)]
pub struct TapHealth {
    pub id: String,
    pub nth_frame: usize,
    pub delivered_frames: u64,
    pub dropped_frames: u64,
    pub channel_capacity: usize,
    pub channel_available: usize,
    pub channel_utilization: f64,
}

impl TapConsumer {
    pub fn health(&self) -> TapHealth {
        TapHealth {
            id: self.id.clone(),
            nth_frame: self.nth_frame,
            delivered_frames: self.delivered_count(),
            dropped_frames: self.dropped_count(),
            channel_capacity: self.channel_capacity(),
            channel_available: self.channel_available(),
            channel_utilization: self.channel_utilization(),
        }
    }
}

fn allocate_metrics_label(id: &str) -> String {
    #[cfg(feature = "metrics")]
    {
        let mut labels = TAP_METRICS_LABELS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if labels.contains(id) {
            return id.to_string();
        }
        if labels.len() < MAX_TAP_METRICS_LABELS {
            labels.insert(id.to_string());
            return id.to_string();
        }
        TAP_METRICS_OTHER_LABEL.to_string()
    }
    #[cfg(not(feature = "metrics"))]
    {
        id.to_string()
    }
}

/// Registry for managing active data taps
#[derive(Debug)]
pub struct TapRegistry {
    taps: RwLock<HashMap<String, Arc<TapConsumer>>>,
    channel_capacity: AtomicUsize,
}

impl TapRegistry {
    pub fn new() -> Self {
        Self::with_channel_capacity(DEFAULT_TAP_CHANNEL_SIZE)
    }

    pub fn with_channel_capacity(channel_capacity: usize) -> Self {
        Self {
            taps: RwLock::new(HashMap::new()),
            channel_capacity: AtomicUsize::new(channel_capacity.max(1)),
        }
    }

    pub fn set_channel_capacity(&self, channel_capacity: usize) {
        self.channel_capacity
            .store(channel_capacity.max(1), Ordering::Relaxed);
    }

    pub fn channel_capacity(&self) -> usize {
        self.channel_capacity.load(Ordering::Relaxed)
    }

    /// Register a new tap
    pub fn register(&self, id: String, nth_frame: usize) -> Result<mpsc::Receiver<Vec<u8>>> {
        let mut taps = self
            .taps
            .write()
            .map_err(|_| anyhow!("Tap registry lock poisoned"))?;

        if taps.contains_key(&id) {
            return Err(anyhow!("Tap with ID '{}' already exists", id));
        }

        let channel_capacity = self.channel_capacity.load(Ordering::Relaxed).max(1);
        let (tx, rx) = mpsc::channel(channel_capacity);
        let tap = Arc::new(TapConsumer::new(
            id.clone(),
            nth_frame,
            channel_capacity,
            tx,
        ));
        taps.insert(id, tap);

        Ok(rx)
    }

    /// Unregister a tap
    pub fn unregister(&self, id: &str) -> Result<bool> {
        let mut taps = self
            .taps
            .write()
            .map_err(|_| anyhow!("Tap registry lock poisoned"))?;
        Ok(taps.remove(id).is_some())
    }

    /// Notify all taps with new data
    pub fn notify_all(&self, data: &[u8]) {
        let taps = match self.taps.read() {
            Ok(guard) => guard,
            Err(_) => return, // Lock poisoned
        };

        if taps.is_empty() {
            return;
        }

        // Clone data once if needed, or per tap?
        // Optimization: only clone if we have at least one consumer that wants it
        // But here we just iterate.

        // Since we need to send owned Vec<u8> to channel, we might need multiple clones
        // if multiple taps want the frame.
        // To avoid cloning for *skipped* frames, we check should_deliver first.

        for tap in taps.values() {
            if tap.should_deliver() {
                // Clone data only when sending
                let frame_data = data.to_vec();
                tap.try_send_frame(frame_data);
            }
        }
    }

    /// Get count of active taps
    pub fn count(&self) -> usize {
        self.taps.read().map(|t| t.len()).unwrap_or(0)
    }

    /// List all taps
    pub fn list(&self) -> Vec<(String, usize)> {
        self.taps
            .read()
            .map(|t| t.values().map(|t| (t.id.clone(), t.nth_frame)).collect())
            .unwrap_or_default()
    }

    /// Snapshot tap health metrics for all active taps
    pub fn tap_health(&self) -> Vec<TapHealth> {
        self.taps
            .read()
            .map(|taps| taps.values().map(|tap| tap.health()).collect())
            .unwrap_or_default()
    }

    /// Snapshot tap health for a specific tap
    pub fn tap_health_for(&self, id: &str) -> Option<TapHealth> {
        self.taps
            .read()
            .ok()
            .and_then(|taps| taps.get(id).map(|tap| tap.health()))
    }
}

impl Default for TapRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tap_drop_counts_when_channel_full() {
        let registry = TapRegistry::with_channel_capacity(1);
        let _rx = registry.register("slow".to_string(), 1).unwrap();

        let frame = vec![1u8, 2, 3, 4];
        for _ in 0..5 {
            registry.notify_all(&frame);
        }

        let health = registry.tap_health_for("slow").expect("tap health missing");
        assert!(health.dropped_frames > 0, "expected dropped frames");
        assert!(
            health.delivered_frames >= 1,
            "expected at least one delivery"
        );
        assert_eq!(health.channel_capacity, 1);
    }

    // =========================================================================
    // Tap Registry Throughput Benchmark (bd-z1zu)
    //
    // Channel evaluation findings:
    //
    // CURRENT IMPLEMENTATION: tokio::sync::mpsc (bounded, per-subscriber)
    //   - Each subscriber gets its own bounded mpsc channel via TapRegistry::register()
    //   - Backpressure: non-blocking try_send(); drops frames when channel is full
    //   - Frame data is cloned (to_vec()) per subscriber that passes nth_frame filter
    //   - nth_frame decimation reduces unnecessary clones and sends
    //
    // WHY NOT tokio::sync::broadcast?
    //   - broadcast requires all receivers to keep up or messages are lost for
    //     ALL receivers (lagging receiver causes RecvError::Lagged for itself)
    //   - mpsc per-subscriber isolates backpressure: a slow GUI tap does not
    //     affect a fast measurement pipeline tap
    //   - broadcast clones data for each recv() call anyway, so no allocation win
    //   - broadcast does not support per-subscriber decimation (nth_frame)
    //
    // VERDICT: The current mpsc-per-subscriber design is the correct choice.
    //   - Independent backpressure per consumer (critical for mixed fast/slow taps)
    //   - try_send() is O(1) and non-blocking
    //   - Frame cloning cost is dominated by memcpy, not channel overhead
    //   - No broadcast alternative provides meaningful improvement here
    //
    // The benchmark below establishes a throughput baseline for regression detection.
    // =========================================================================

    /// Throughput benchmark for tap registry frame delivery.
    ///
    /// Measures send-side throughput (frames/sec, MB/sec) with 1, 2, and 4
    /// subscribers. Uses 1000 frames of 1 KB each. Yields between batches
    /// so async consumers can drain their channels.
    ///
    /// Note: Delivery ratio may be < 100% because `notify_all` uses `try_send`
    /// (non-blocking). Under scheduler contention (e.g., parallel test suite),
    /// consumers may not drain fast enough, causing intentional frame drops.
    /// This is the correct backpressure behavior -- not a bug.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bench_tap_registry_throughput() {
        const FRAME_COUNT: usize = 1000;
        const FRAME_SIZE: usize = 1024; // 1 KB
        const CHANNEL_CAP: usize = 256;
        const BATCH_SIZE: usize = 32;

        let frame_data: Vec<u8> = (0..FRAME_SIZE)
            .map(|i| {
                #[allow(clippy::cast_possible_truncation)]
                // SAFETY: intentional truncation to fill test frame with byte pattern
                let byte = (i % 256) as u8;
                byte
            })
            .collect();

        for subscriber_count in [1usize, 2, 4] {
            let registry = Arc::new(TapRegistry::with_channel_capacity(CHANNEL_CAP));
            let mut receivers = Vec::new();

            for i in 0..subscriber_count {
                let rx = registry
                    .register(format!("bench_sub_{i}"), 1)
                    .expect("register should succeed");
                receivers.push(rx);
            }

            // Spawn consumer tasks that drain their channels
            let mut consumer_handles = Vec::new();
            for mut rx in receivers {
                let handle = tokio::spawn(async move {
                    let mut count = 0u64;
                    while let Some(_frame) = rx.recv().await {
                        count += 1;
                    }
                    count
                });
                consumer_handles.push(handle);
            }

            // Produce frames, yielding between batches so consumers can drain
            let start = std::time::Instant::now();
            for batch_start in (0..FRAME_COUNT).step_by(BATCH_SIZE) {
                let batch_end = (batch_start + BATCH_SIZE).min(FRAME_COUNT);
                for _ in batch_start..batch_end {
                    registry.notify_all(&frame_data);
                }
                tokio::task::yield_now().await;
            }
            let send_elapsed = start.elapsed();

            // Drop registry to close senders, signaling consumers to exit
            drop(registry);

            let mut total_received = 0u64;
            for handle in consumer_handles {
                total_received += handle.await.expect("consumer task should not panic");
            }

            let send_secs = send_elapsed.as_secs_f64();
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: precision loss acceptable for benchmark calculation
            let frames_per_sec = FRAME_COUNT as f64 / send_secs;
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: precision loss acceptable for benchmark calculation
            let total_bytes_attempted = (FRAME_COUNT * FRAME_SIZE * subscriber_count) as f64;
            let mb_per_sec = total_bytes_attempted / (1024.0 * 1024.0) / send_secs;
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: precision loss acceptable for ratio calculation
            let expected_f = (FRAME_COUNT * subscriber_count) as f64;
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: precision loss acceptable for ratio calculation
            let delivery_ratio = total_received as f64 / expected_f;

            // Structured output for CI consumption
            println!("---");
            println!("bench: tap_registry_throughput");
            println!("subscribers: {subscriber_count}");
            println!("frames_sent: {FRAME_COUNT}");
            println!("frame_size_bytes: {FRAME_SIZE}");
            println!("channel_capacity: {CHANNEL_CAP}");
            println!("total_received: {total_received}");
            println!("delivery_ratio: {delivery_ratio:.2}");
            println!("send_time_ms: {:.2}", send_secs * 1000.0);
            println!("frames_per_sec: {frames_per_sec:.0}");
            println!("throughput_mb_sec: {mb_per_sec:.1}");

            // Sanity check: we should receive a meaningful number of frames.
            // The exact ratio depends on scheduler contention; try_send drops
            // frames intentionally under backpressure. We only assert that
            // the pipeline is functional (> 10% delivery).
            assert!(
                total_received > 0,
                "with {subscriber_count} subscribers: received zero frames"
            );
            assert!(
                delivery_ratio > 0.10,
                "with {subscriber_count} subscribers: delivery ratio {delivery_ratio:.2} \
                 is suspiciously low (< 10%), suggesting a channel or scheduling bug"
            );
        }
    }
}
