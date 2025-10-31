// src/measurement/mod.rs

use anyhow::Result;
use async_trait::async_trait;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

use crate::core::DataPoint;

/// Fan-out data distributor for efficient multi-consumer broadcasting without backpressure.
///
/// Uses non-blocking try_send() to prevent slow subscribers from blocking fast ones.
/// Each subscriber gets a dedicated mpsc channel, providing isolation. Messages are dropped
/// if a subscriber's channel is full (logged as warning).
///
/// Uses interior mutability (Mutex) to avoid requiring Arc<Mutex<DataDistributor>> wrapper,
/// following actor model principles by minimizing lock scope.
pub struct DataDistributor<T: Clone> {
    subscribers: Mutex<Vec<SubscriberEntry<T>>>,
    config: DataDistributorConfig,
}

#[derive(Clone, Debug)]
pub struct DataDistributorConfig {
    pub capacity: usize,
    pub warn_drop_rate_percent: f64,
    pub error_saturation_percent: f64,
    pub metrics_window: Duration,
}

impl DataDistributorConfig {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            warn_drop_rate_percent: 1.0,
            error_saturation_percent: 90.0,
            metrics_window: Duration::from_secs(10),
        }
    }

    pub fn with_thresholds(
        capacity: usize,
        warn_drop_rate_percent: f64,
        error_saturation_percent: f64,
        metrics_window: Duration,
    ) -> Self {
        Self {
            capacity,
            warn_drop_rate_percent,
            error_saturation_percent,
            metrics_window,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SubscriberMetricsSnapshot {
    pub subscriber: String,
    pub total_sent: u64,
    pub total_dropped: u64,
    pub drop_rate_percent: f64,
    pub channel_occupancy: usize,
    pub channel_capacity: usize,
}

struct SubscriberEntry<T: Clone> {
    name: String,
    sender: mpsc::Sender<T>,
    metrics: SubscriberMetrics,
    last_occupancy: usize,
}

impl<T: Clone> SubscriberEntry<T> {
    fn new(name: String, sender: mpsc::Sender<T>, now: Instant) -> Self {
        Self {
            name,
            sender,
            metrics: SubscriberMetrics::new(now),
            last_occupancy: 0,
        }
    }
}

struct SubscriberMetrics {
    total_sent: u64,
    total_dropped: u64,
    window_sent: u64,
    window_dropped: u64,
    window_start: Instant,
    drop_warn_emitted: bool,
    saturation_error_emitted: bool,
}

impl SubscriberMetrics {
    fn new(now: Instant) -> Self {
        Self {
            total_sent: 0,
            total_dropped: 0,
            window_sent: 0,
            window_dropped: 0,
            window_start: now,
            drop_warn_emitted: false,
            saturation_error_emitted: false,
        }
    }

    fn record_success(&mut self) {
        self.total_sent = self.total_sent.saturating_add(1);
        self.window_sent = self.window_sent.saturating_add(1);
    }

    fn record_drop(&mut self) {
        self.total_dropped = self.total_dropped.saturating_add(1);
        self.window_dropped = self.window_dropped.saturating_add(1);
    }

    fn check_window(
        &mut self,
        now: Instant,
        name: &str,
        occupancy_percent: f64,
        config: &DataDistributorConfig,
    ) {
        if now.duration_since(self.window_start) >= config.metrics_window {
            let window_total = self.window_sent + self.window_dropped;
            if window_total > 0 {
                let drop_rate = (self.window_dropped as f64 / window_total as f64) * 100.0;
                if drop_rate >= config.warn_drop_rate_percent && !self.drop_warn_emitted {
                    tracing::warn!(
                        "DataDistributor detected sustained drop rate for subscriber '{}' (drop_rate_percent={:.2})",
                        name,
                        drop_rate
                    );
                    self.drop_warn_emitted = true;
                }
            }

            self.window_sent = 0;
            self.window_dropped = 0;
            self.window_start = now;
            self.drop_warn_emitted = false;
            self.saturation_error_emitted = false;
        }

        if occupancy_percent >= config.error_saturation_percent && !self.saturation_error_emitted {
            tracing::error!(
                "DataDistributor subscriber '{}' channel saturated (occupancy_percent={:.2})",
                name,
                occupancy_percent
            );
            self.saturation_error_emitted = true;
        }
    }
}

impl<T: Clone> DataDistributor<T> {
    /// Creates a new DataDistributor with specified channel capacity
    pub fn new(capacity: usize) -> Self {
        Self::with_config(DataDistributorConfig::new(capacity))
    }

    /// Creates a new DataDistributor with detailed observability configuration
    pub fn with_config(config: DataDistributorConfig) -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            config,
        }
    }

    /// Subscribe to the data stream with a named identifier, returns a new mpsc::Receiver
    ///
    /// The name is used for observability - logs and metrics will identify subscribers
    /// by this name when messages are dropped or subscribers disconnect.
    pub async fn subscribe(&self, name: impl Into<String>) -> mpsc::Receiver<T> {
        let name = name.into();
        let (tx, rx) = mpsc::channel(self.config.capacity);
        let mut subscribers = self.subscribers.lock().await;
        tracing::info!(
            "DataDistributor subscriber '{}' registered with capacity {}",
            name,
            self.config.capacity
        );
        subscribers.push(SubscriberEntry::new(name, tx, Instant::now()));
        rx
    }

    /// Broadcast data to all subscribers with automatic dead subscriber cleanup.
    ///
    /// Uses non-blocking try_send() to prevent slow subscribers from blocking fast ones.
    /// Messages are dropped if a subscriber's channel is full. Dead subscribers are
    /// automatically removed.
    pub async fn broadcast(&self, data: T) -> Result<()> {
        let mut subscribers = self.subscribers.lock().await;
        let mut disconnected_indices = Vec::new();
        let now = Instant::now();

        for (i, entry) in subscribers.iter_mut().enumerate() {
            match entry.sender.try_send(data.clone()) {
                Ok(_) => {
                    entry.metrics.record_success();
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    entry.metrics.record_drop();
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::info!("DataDistributor subscriber '{}' disconnected", entry.name);
                    disconnected_indices.push(i);
                    continue;
                }
            }

            let remaining_capacity = entry.sender.capacity();
            let occupancy = self
                .config
                .capacity
                .saturating_sub(remaining_capacity)
                .min(self.config.capacity);
            entry.last_occupancy = occupancy;
            let occupancy_percent = if self.config.capacity == 0 {
                0.0
            } else {
                (occupancy as f64 / self.config.capacity as f64) * 100.0
            };

            entry
                .metrics
                .check_window(now, &entry.name, occupancy_percent, &self.config);
        }

        // Remove disconnected subscribers in reverse order to maintain indices
        for i in disconnected_indices.iter().rev() {
            subscribers.swap_remove(*i);
        }

        Ok(())
    }

    /// Returns the number of active subscribers
    pub async fn subscriber_count(&self) -> usize {
        self.subscribers.lock().await.len()
    }

    /// Returns a snapshot of current metrics for each subscriber
    pub async fn metrics_snapshot(&self) -> Vec<SubscriberMetricsSnapshot> {
        let subscribers = self.subscribers.lock().await;
        subscribers
            .iter()
            .map(|entry| {
                let total = entry.metrics.total_sent + entry.metrics.total_dropped;
                let drop_rate = if total == 0 {
                    0.0
                } else {
                    (entry.metrics.total_dropped as f64 / total as f64) * 100.0
                };

                SubscriberMetricsSnapshot {
                    subscriber: entry.name.clone(),
                    total_sent: entry.metrics.total_sent,
                    total_dropped: entry.metrics.total_dropped,
                    drop_rate_percent: drop_rate,
                    channel_occupancy: entry.last_occupancy,
                    channel_capacity: self.config.capacity,
                }
            })
            .collect()
    }
}

#[async_trait]
pub trait Measure: Send + Sync {
    type Data: Send + Sync + Clone;

    async fn measure(&mut self) -> Result<Self::Data>;
    async fn data_stream(&self) -> Result<tokio::sync::mpsc::Receiver<std::sync::Arc<Self::Data>>>;
}

pub mod datapoint;
pub mod instrument_measurement;
pub mod power;

pub use instrument_measurement::InstrumentMeasurement;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    // Using Arc<T> is a common pattern for broadcast data to make clones cheap.
    type TestData = Arc<u32>;

    #[tokio::test]
    async fn new_and_subscribe_updates_subscriber_count() {
        // Arrange
        let distributor = DataDistributor::<TestData>::new(10);
        assert_eq!(
            distributor.subscriber_count().await,
            0,
            "Initial subscriber count should be 0"
        );

        // Act
        let _rx1 = distributor.subscribe("sub1").await;

        // Assert
        assert_eq!(
            distributor.subscriber_count().await,
            1,
            "Subscriber count should be 1 after one subscription"
        );

        // Act
        let _rx2 = distributor.subscribe("sub2").await;

        // Assert
        assert_eq!(
            distributor.subscriber_count().await,
            2,
            "Subscriber count should be 2 after a second subscription"
        );
    }

    #[tokio::test]
    async fn broadcast_delivers_data_to_all_subscribers() {
        // Arrange
        let distributor = DataDistributor::<TestData>::new(10);
        let mut rx1 = distributor.subscribe("sub1").await;
        let mut rx2 = distributor.subscribe("sub2").await;
        let data = Arc::new(42);

        // Act
        distributor.broadcast(data.clone()).await.unwrap();

        // Assert: Both subscribers should receive the exact same data.
        let received1 = timeout(Duration::from_millis(20), rx1.recv())
            .await
            .expect("rx1 should receive data within timeout")
            .expect("rx1 channel should not be empty");
        let received2 = timeout(Duration::from_millis(20), rx2.recv())
            .await
            .expect("rx2 should receive data within timeout")
            .expect("rx2 channel should not be empty");

        assert_eq!(received1, data);
        assert_eq!(received2, data);
    }

    #[tokio::test]
    async fn dead_subscriber_is_cleaned_up_on_broadcast() {
        // Arrange
        let distributor = DataDistributor::<TestData>::new(10);
        let mut rx1 = distributor.subscribe("surviving_subscriber").await;
        let rx2 = distributor.subscribe("dead_subscriber").await;

        assert_eq!(distributor.subscriber_count().await, 2);

        // Act: Drop one receiver to simulate a disconnected client.
        drop(rx2);

        // Broadcast something to trigger the cleanup logic for the closed channel.
        distributor.broadcast(Arc::new(1)).await.unwrap();

        // Consume the first message from the surviving subscriber
        let first_msg = timeout(Duration::from_millis(20), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*first_msg, 1);

        // Assert: The dead subscriber should be removed.
        assert_eq!(distributor.subscriber_count().await, 1);

        // The remaining subscriber should still receive subsequent data.
        distributor.broadcast(Arc::new(2)).await.unwrap();
        let received = timeout(Duration::from_millis(20), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            *received, 2,
            "Surviving subscriber should still receive data after cleanup"
        );
    }

    #[tokio::test]
    async fn multiple_dead_subscribers_are_removed_correctly() {
        // Arrange
        let distributor = DataDistributor::<TestData>::new(10);
        let rx1 = distributor.subscribe("sub1_dead").await;
        let mut rx2 = distributor.subscribe("sub2_survivor").await;
        let rx3 = distributor.subscribe("sub3_dead").await;
        let rx4 = distributor.subscribe("sub4_dead").await;
        assert_eq!(distributor.subscriber_count().await, 4);

        // Act: Drop subscribers at the beginning, middle, and end of the internal list.
        // This tests the reverse-iteration and swap_remove logic.
        drop(rx1);
        drop(rx3);
        drop(rx4);

        // Broadcast to trigger cleanup.
        distributor.broadcast(Arc::new(100)).await.unwrap();

        // Assert
        assert_eq!(
            distributor.subscriber_count().await,
            1,
            "Only one subscriber should remain"
        );

        // The only remaining subscriber should receive the data.
        let received = timeout(Duration::from_millis(20), rx2.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*received, 100);
    }

    #[tokio::test]
    async fn non_blocking_broadcast_drops_messages_for_full_channel() {
        // Arrange: Use a small capacity to easily fill the channel.
        let distributor = DataDistributor::<TestData>::new(1);
        let mut rx = distributor.subscribe("slow_consumer").await;

        // Act: Send two messages without reading. The first fills the channel, the second is dropped.
        distributor.broadcast(Arc::new(1)).await.unwrap(); // Fills the channel's buffer.
        distributor.broadcast(Arc::new(2)).await.unwrap(); // Should be dropped due to TrySendError::Full.

        // Assert: The receiver only gets the first message.
        let received1 = timeout(Duration::from_millis(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*received1, 1);

        // Assert: The channel is now empty. A subsequent receive times out, proving the second message was dropped.
        let recv_result = timeout(Duration::from_millis(20), rx.recv()).await;
        assert!(
            recv_result.is_err(),
            "Channel should be empty; second message should have been dropped"
        );
    }

    #[tokio::test]
    async fn slow_subscriber_does_not_block_fast_subscriber() {
        // Arrange: A distributor with a small channel capacity.
        let distributor = DataDistributor::<TestData>::new(1);
        let mut fast_rx = distributor.subscribe("fast_subscriber").await;
        // The slow subscriber's receiver is created but never read from.
        let _slow_rx = distributor.subscribe("slow_subscriber").await;

        // Act & Assert

        // 1. Broadcast a message. Both channels receive it. The slow channel is now full.
        distributor.broadcast(Arc::new(1)).await.unwrap();
        let received_fast = timeout(Duration::from_millis(20), fast_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*received_fast, 1);

        // 2. Broadcast another message. The fast subscriber's channel is empty and should
        // receive it. The slow one is full, so the message is dropped for it.
        // This broadcast call must complete quickly, proving it is non-blocking.
        let broadcast_future = distributor.broadcast(Arc::new(2));
        let result = timeout(Duration::from_millis(50), broadcast_future).await;
        assert!(
            result.is_ok(),
            "Broadcast should not block even with a full subscriber channel"
        );
        result.unwrap().unwrap();

        // 3. Verify the fast subscriber received the second message, proving isolation.
        let received_fast_2 = timeout(Duration::from_millis(20), fast_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*received_fast_2, 2);

        // 4. Verify the slow subscriber is still counted, as its channel is not closed.
        assert_eq!(distributor.subscriber_count().await, 2);
    }

    #[tokio::test]
    async fn broadcast_with_no_subscribers_is_a_safe_no_op() {
        // Arrange
        let distributor = DataDistributor::<TestData>::new(10);
        assert_eq!(distributor.subscriber_count().await, 0);

        // Act: Broadcast data. This should not panic or error.
        let result = distributor.broadcast(Arc::new(99)).await;

        // Assert
        assert!(result.is_ok());
        assert_eq!(distributor.subscriber_count().await, 0);
    }

    #[tokio::test]
    async fn metrics_snapshot_reports_counters() {
        let config =
            DataDistributorConfig::with_thresholds(1, 50.0, 90.0, Duration::from_millis(5));
        let distributor = DataDistributor::with_config(config);

        let mut fast_rx = distributor.subscribe("fast").await;
        let slow_rx = distributor.subscribe("slow").await;

        distributor.broadcast(Arc::new(1)).await.unwrap();
        assert!(fast_rx.recv().await.is_some());

        for _ in 0..3 {
            distributor.broadcast(Arc::new(2)).await.unwrap();
        }

        let metrics = distributor.metrics_snapshot().await;
        let slow_metrics = metrics
            .iter()
            .find(|m| m.subscriber == "slow")
            .expect("slow subscriber metrics present");
        assert!(slow_metrics.total_dropped > 0);
        assert!(slow_metrics.channel_occupancy > 0);
        drop(slow_rx);
    }

    #[tokio::test]
    #[traced_test]
    async fn slow_subscriber_triggers_alerts() {
        let config = DataDistributorConfig::with_thresholds(1, 0.0, 0.0, Duration::from_millis(1));
        let distributor = DataDistributor::with_config(config);

        let mut fast_rx = distributor.subscribe("fast").await;
        let slow_rx = distributor.subscribe("slow").await;

        distributor.broadcast(Arc::new(10)).await.unwrap();
        assert!(fast_rx.recv().await.is_some());

        tokio::time::sleep(Duration::from_millis(2)).await;

        distributor.broadcast(Arc::new(11)).await.unwrap();

        assert!(logs_contain("DataDistributor detected sustained drop rate"));
        assert!(logs_contain("channel saturated"));

        drop(slow_rx);
    }
}
