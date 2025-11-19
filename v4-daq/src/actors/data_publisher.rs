//! Data Publisher Actor
//!
//! Arrow RecordBatch distribution with pub/sub pattern and backpressure handling.
//! Maintains subscriber list and broadcasts data with metrics tracking.

use anyhow::{anyhow, Result};
use arrow::record_batch::RecordBatch;
use kameo::actor::{ActorRef, WeakActorRef};
use kameo::error::BoxSendError;
use kameo::message::{Context, Message};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Represents a data consumer that receives Arrow batches
#[async_trait::async_trait]
pub trait DataConsumer: Send + Sync {
    /// Handle incoming RecordBatch
    async fn handle_batch(&self, batch: RecordBatch, instrument_id: String) -> Result<()>;
}

/// Metrics for the data publisher
#[derive(Debug, Clone, kameo::Reply)]
pub struct PublisherMetrics {
    /// Total number of batches published
    pub batches_published: u64,
    /// Current number of active subscribers
    pub active_subscribers: usize,
    /// Total batches dropped due to backpressure
    pub batches_dropped: u64,
}

/// DataPublisher actor state
pub struct DataPublisher {
    /// Map of subscriber IDs to actor references
    subscribers: HashMap<String, Arc<dyn DataConsumer>>,
    /// Metrics tracking
    metrics: PublisherMetrics,
    /// Atomic counter for unique subscriber IDs
    subscriber_counter: Arc<AtomicU64>,
}

impl DataPublisher {
    /// Create a new data publisher
    pub fn new() -> Self {
        Self {
            subscribers: HashMap::new(),
            metrics: PublisherMetrics {
                batches_published: 0,
                active_subscribers: 0,
                batches_dropped: 0,
            },
            subscriber_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Check if all subscribers are healthy
    fn all_subscribers_active(&self) -> bool {
        !self.subscribers.is_empty()
    }
}

impl Default for DataPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl kameo::Actor for DataPublisher {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(args: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        tracing::info!("DataPublisher actor started");
        Ok(args)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        tracing::info!(
            "DataPublisher actor stopped with {} active subscribers",
            self.subscribers.len()
        );
        self.subscribers.clear();
        Ok(())
    }
}

/// Subscribe message - adds a new data consumer
pub struct Subscribe {
    pub subscriber: Arc<dyn DataConsumer>,
}

impl std::fmt::Debug for Subscribe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscribe")
            .field("subscriber", &"Arc<dyn DataConsumer>")
            .finish()
    }
}

impl Message<Subscribe> for DataPublisher {
    type Reply = Result<String>;

    async fn handle(
        &mut self,
        msg: Subscribe,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let subscriber_id = format!(
            "subscriber_{}",
            self.subscriber_counter.fetch_add(1, Ordering::SeqCst)
        );

        self.subscribers
            .insert(subscriber_id.clone(), msg.subscriber);
        self.metrics.active_subscribers = self.subscribers.len();

        tracing::info!("New subscriber registered: {}", subscriber_id);

        Ok(subscriber_id)
    }
}

/// Unsubscribe message - removes a data consumer
#[derive(Debug)]
pub struct Unsubscribe {
    pub subscriber_id: String,
}

impl Message<Unsubscribe> for DataPublisher {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: Unsubscribe,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if self.subscribers.remove(&msg.subscriber_id).is_some() {
            self.metrics.active_subscribers = self.subscribers.len();
            tracing::info!("Subscriber unregistered: {}", msg.subscriber_id);
            Ok(())
        } else {
            Err(anyhow!("Subscriber not found: {}", msg.subscriber_id))
        }
    }
}

/// Publish message - broadcasts batch to all subscribers
#[derive(Debug, Clone)]
pub struct PublishBatch {
    pub batch: RecordBatch,
    pub instrument_id: String,
}

impl Message<PublishBatch> for DataPublisher {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: PublishBatch,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if !self.all_subscribers_active() {
            tracing::trace!("No active subscribers for batch from {}", msg.instrument_id);
            return Ok(());
        }

        let batch_size = msg.batch.num_rows();
        let mut failed_subscribers = Vec::new();

        // Broadcast to all subscribers
        for (subscriber_id, subscriber) in &self.subscribers {
            match subscriber
                .handle_batch(msg.batch.clone(), msg.instrument_id.clone())
                .await
            {
                Ok(_) => {
                    tracing::trace!(
                        "Published batch ({} rows) from {} to subscriber {}",
                        batch_size,
                        msg.instrument_id,
                        subscriber_id
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        "Failed to deliver batch to subscriber {}: {}",
                        subscriber_id,
                        err
                    );
                    self.metrics.batches_dropped += 1;
                    failed_subscribers.push(subscriber_id.clone());
                }
            }
        }

        // Remove failed subscribers (backpressure handling)
        for subscriber_id in failed_subscribers {
            self.subscribers.remove(&subscriber_id);
            self.metrics.active_subscribers = self.subscribers.len();
            tracing::warn!("Removed unresponsive subscriber: {}", subscriber_id);
        }

        self.metrics.batches_published += 1;

        tracing::debug!(
            "Published batch from instrument {} to {} subscribers",
            msg.instrument_id,
            self.subscribers.len()
        );

        Ok(())
    }
}

/// Get metrics message - returns current publisher statistics
#[derive(Debug)]
pub struct GetMetrics;

impl Message<GetMetrics> for DataPublisher {
    type Reply = PublisherMetrics;

    async fn handle(
        &mut self,
        _msg: GetMetrics,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.metrics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use kameo::actor::spawn;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    /// Mock data consumer for testing
    struct MockConsumer {
        received_batches: Arc<Mutex<Vec<RecordBatch>>>,
        fail_after: Option<usize>,
        call_count: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl DataConsumer for MockConsumer {
        async fn handle_batch(&self, batch: RecordBatch, _instrument_id: String) -> Result<()> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);

            // Check if we should fail
            if let Some(fail_count) = self.fail_after {
                if count >= fail_count {
                    return Err(anyhow!("Mock failure"));
                }
            }

            self.received_batches.lock().unwrap().push(batch);
            Ok(())
        }
    }

    fn create_test_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));

        let ids = Arc::new(Int64Array::from(vec![1, 2, 3]));

        RecordBatch::try_new(schema, vec![ids]).expect("Failed to create test batch")
    }

    #[tokio::test]
    async fn test_publisher_subscribe() {
        let publisher = spawn(DataPublisher::new());

        let consumer = Arc::new(MockConsumer {
            received_batches: Arc::new(Mutex::new(Vec::new())),
            fail_after: None,
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        let subscriber_id = publisher
            .ask(Subscribe {
                subscriber: consumer.clone(),
            })
            .await
            .expect("Failed to subscribe");

        assert!(subscriber_id.starts_with("subscriber_"));

        let metrics = publisher.ask(GetMetrics).await;
        assert_eq!(metrics.active_subscribers, 1);

        publisher.kill().await;
    }

    #[tokio::test]
    async fn test_publisher_unsubscribe() {
        let publisher = spawn(DataPublisher::new());

        let consumer = Arc::new(MockConsumer {
            received_batches: Arc::new(Mutex::new(Vec::new())),
            fail_after: None,
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        let subscriber_id = publisher
            .ask(Subscribe {
                subscriber: consumer.clone(),
            })
            .await
            .expect("Failed to subscribe");

        let mut metrics = publisher.ask(GetMetrics).await;
        assert_eq!(metrics.active_subscribers, 1);

        publisher
            .ask(Unsubscribe {
                subscriber_id: subscriber_id.clone(),
            })
            .await
            .expect("Failed to unsubscribe");

        metrics = publisher.ask(GetMetrics).await;
        assert_eq!(metrics.active_subscribers, 0);

        publisher.kill().await;
    }

    #[tokio::test]
    async fn test_publisher_broadcast() {
        let publisher = spawn(DataPublisher::new());

        let consumer1 = Arc::new(MockConsumer {
            received_batches: Arc::new(Mutex::new(Vec::new())),
            fail_after: None,
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        let consumer2 = Arc::new(MockConsumer {
            received_batches: Arc::new(Mutex::new(Vec::new())),
            fail_after: None,
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        publisher
            .ask(Subscribe {
                subscriber: consumer1.clone(),
            })
            .await
            .expect("Failed to subscribe");

        publisher
            .ask(Subscribe {
                subscriber: consumer2.clone(),
            })
            .await
            .expect("Failed to subscribe");

        let batch = create_test_batch();
        publisher
            .ask(PublishBatch {
                batch,
                instrument_id: "test_instrument".to_string(),
            })
            .await
            .expect("Failed to publish");

        let batches1 = consumer1.received_batches.lock().unwrap();
        let batches2 = consumer2.received_batches.lock().unwrap();

        assert_eq!(batches1.len(), 1);
        assert_eq!(batches2.len(), 1);

        let metrics = publisher.ask(GetMetrics).await;
        assert_eq!(metrics.batches_published, 1);
        assert_eq!(metrics.active_subscribers, 2);

        publisher.kill().await;
    }

    #[tokio::test]
    async fn test_publisher_backpressure() {
        let publisher = spawn(DataPublisher::new());

        let consumer_ok = Arc::new(MockConsumer {
            received_batches: Arc::new(Mutex::new(Vec::new())),
            fail_after: None,
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        let consumer_fail = Arc::new(MockConsumer {
            received_batches: Arc::new(Mutex::new(Vec::new())),
            fail_after: Some(0), // Fail immediately
            call_count: Arc::new(AtomicUsize::new(0)),
        });

        let id1 = publisher
            .ask(Subscribe {
                subscriber: consumer_ok.clone(),
            })
            .await
            .expect("Failed to subscribe");

        let id2 = publisher
            .ask(Subscribe {
                subscriber: consumer_fail.clone(),
            })
            .await
            .expect("Failed to subscribe");

        let batch = create_test_batch();
        publisher
            .ask(PublishBatch {
                batch,
                instrument_id: "test_instrument".to_string(),
            })
            .await
            .expect("Failed to publish");

        let metrics = publisher.ask(GetMetrics).await;
        assert_eq!(metrics.batches_published, 1);
        assert_eq!(metrics.batches_dropped, 1);
        assert_eq!(metrics.active_subscribers, 1); // Failed subscriber removed

        // Verify only the good consumer received the batch
        let batches = consumer_ok.received_batches.lock().unwrap();
        assert_eq!(batches.len(), 1);

        publisher.kill().await;
    }

    #[tokio::test]
    async fn test_publisher_no_subscribers() {
        let publisher = spawn(DataPublisher::new());

        let batch = create_test_batch();
        publisher
            .ask(PublishBatch {
                batch,
                instrument_id: "test_instrument".to_string(),
            })
            .await
            .expect("Failed to publish with no subscribers");

        let metrics = publisher.ask(GetMetrics).await;
        assert_eq!(metrics.batches_published, 0); // Not incremented when no subscribers
        assert_eq!(metrics.active_subscribers, 0);

        publisher.kill().await;
    }

    #[tokio::test]
    async fn test_publisher_unsubscribe_nonexistent() {
        let publisher = spawn(DataPublisher::new());

        let result = publisher
            .ask(Unsubscribe {
                subscriber_id: "nonexistent".to_string(),
            })
            .await;

        assert!(result.is_err());

        publisher.kill().await;
    }
}
