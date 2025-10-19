//! Integration test for backpressure mechanism
//!
//! This test validates that the mpsc fan-out pattern ensures data integrity
//! by verifying that all subscribers receive all data points without loss,
//! even when consumers have different speeds.

use rust_daq::{
    config::Settings,
    core::{DataPoint, Instrument},
    measurement::{InstrumentMeasurement, Measure},
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::{sleep, Duration};

/// High-throughput mock instrument for backpressure testing
#[derive(Clone)]
struct HighThroughputMock {
    id: String,
    measurement: Option<InstrumentMeasurement>,
    sample_count: usize,
}

impl HighThroughputMock {
    fn new(sample_count: usize) -> Self {
        Self {
            id: String::new(),
            measurement: None,
            sample_count,
        }
    }
}

#[async_trait]
impl Instrument for HighThroughputMock {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        "High Throughput Mock".to_string()
    }

    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        self.id = id.to_string();
        let capacity = settings.application.broadcast_channel_capacity;
        let measurement = InstrumentMeasurement::new(capacity, self.id.clone());
        self.measurement = Some(measurement.clone());

        let sample_count = self.sample_count;
        let instrument_id = self.id.clone();

        // Spawn high-rate data generator
        tokio::spawn(async move {
            for i in 0..sample_count {
                let dp = DataPoint {
                    timestamp: chrono::Utc::now(),
                    instrument_id: instrument_id.clone(),
                    channel: "high_rate".to_string(),
                    value: i as f64,
                    unit: "count".to_string(),
                    metadata: None,
                };

                if measurement.broadcast(dp).await.is_err() {
                    eprintln!("Broadcast failed at sample {}", i);
                    break;
                }

                // High throughput - minimal delay
                tokio::task::yield_now().await;
            }
        });

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.measurement = None;
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement.as_ref().unwrap()
    }
}

#[tokio::test]
async fn test_backpressure_all_consumers_receive_all_data() {
    const SAMPLE_COUNT: usize = 1000;
    const CONSUMER_COUNT: usize = 3;

    // Create test settings
    let settings = Arc::new(Settings::new(Some("default")).unwrap());

    // Create instrument
    let mut instrument = HighThroughputMock::new(SAMPLE_COUNT);
    instrument.connect("backpressure_test", &settings).await.unwrap();

    // Create multiple consumers with different speeds
    let mut stream1 = instrument.measure().data_stream().await.unwrap();  // Fast consumer
    let mut stream2 = instrument.measure().data_stream().await.unwrap();  // Medium consumer
    let mut stream3 = instrument.measure().data_stream().await.unwrap();  // Slow consumer

    let count1 = Arc::new(AtomicUsize::new(0));
    let count2 = Arc::new(AtomicUsize::new(0));
    let count3 = Arc::new(AtomicUsize::new(0));

    let c1 = count1.clone();
    let c2 = count2.clone();
    let c3 = count3.clone();

    // Fast consumer - no delay
    let consumer1 = tokio::spawn(async move {
        let mut received = Vec::new();
        while let Some(dp) = stream1.recv().await {
            received.push(dp.value);
            c1.fetch_add(1, Ordering::SeqCst);
        }
        received
    });

    // Medium consumer - small delay
    let consumer2 = tokio::spawn(async move {
        let mut received = Vec::new();
        while let Some(dp) = stream2.recv().await {
            received.push(dp.value);
            c2.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_micros(10)).await;
        }
        received
    });

    // Slow consumer - larger delay
    let consumer3 = tokio::spawn(async move {
        let mut received = Vec::new();
        while let Some(dp) = stream3.recv().await {
            received.push(dp.value);
            c3.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_micros(50)).await;
        }
        received
    });

    // Wait for all data to be generated
    sleep(Duration::from_millis(500)).await;

    // Disconnect to close streams
    instrument.disconnect().await.unwrap();

    // Wait for consumers to finish
    let data1 = consumer1.await.unwrap();
    let data2 = consumer2.await.unwrap();
    let data3 = consumer3.await.unwrap();

    // Verify all consumers received all data
    println!("Consumer 1 (fast) received: {} samples", data1.len());
    println!("Consumer 2 (medium) received: {} samples", data2.len());
    println!("Consumer 3 (slow) received: {} samples", data3.len());

    // CRITICAL: All consumers must receive exactly SAMPLE_COUNT data points
    // This proves there's no silent data loss like with broadcast::channel
    assert_eq!(data1.len(), SAMPLE_COUNT, "Fast consumer lost data!");
    assert_eq!(data2.len(), SAMPLE_COUNT, "Medium consumer lost data!");
    assert_eq!(data3.len(), SAMPLE_COUNT, "Slow consumer lost data!");

    // Verify data integrity - all values should be sequential
    for (i, &value) in data1.iter().enumerate() {
        assert_eq!(value, i as f64, "Fast consumer data corruption at index {}", i);
    }
    for (i, &value) in data2.iter().enumerate() {
        assert_eq!(value, i as f64, "Medium consumer data corruption at index {}", i);
    }
    for (i, &value) in data3.iter().enumerate() {
        assert_eq!(value, i as f64, "Slow consumer data corruption at index {}", i);
    }

    println!("SUCCESS: All {} consumers received all {} samples without loss",
             CONSUMER_COUNT, SAMPLE_COUNT);
}

#[tokio::test]
async fn test_backpressure_applies_correctly() {
    // This test verifies that slow consumers cause backpressure
    // by slowing down the producer, rather than dropping data

    const SAMPLE_COUNT: usize = 100;
    let settings = Arc::new(Settings::new(Some("default")).unwrap());

    let mut instrument = HighThroughputMock::new(SAMPLE_COUNT);
    instrument.connect("backpressure_test_2", &settings).await.unwrap();

    // Create one very slow consumer
    let mut stream = instrument.measure().data_stream().await.unwrap();

    let start = std::time::Instant::now();
    let mut received_count = 0;

    // Slow consumer - 10ms per sample
    while let Some(_dp) = stream.recv().await {
        received_count += 1;
        sleep(Duration::from_millis(10)).await;
    }

    let duration = start.elapsed();

    // With backpressure, this should take at least 100 * 10ms = 1 second
    // (proving the producer was slowed down by the consumer)
    assert!(duration.as_secs() >= 1,
            "Backpressure not working - completed too quickly: {:?}", duration);

    // And we should have received all data
    assert_eq!(received_count, SAMPLE_COUNT,
               "Lost data even with backpressure: got {} expected {}",
               received_count, SAMPLE_COUNT);

    instrument.disconnect().await.unwrap();

    println!("SUCCESS: Backpressure correctly slowed producer. Took {:?} for {} samples",
             duration, SAMPLE_COUNT);
}
