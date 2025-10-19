//! Data flow validation tests
//!
//! Tests for validating that data from multiple instruments flows correctly
//! through the system without loss or lag.

use rust_daq::{
    app::DaqApp,
    config::Settings,
    data::registry::ProcessorRegistry,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
    measurement::Measure,
};
use serial_test::serial;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Helper to create a test DaqApp with mock instruments
fn create_test_app() -> DaqApp<impl Measure> {
    let settings = Arc::new(Settings::new(None).unwrap());
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);
    let processor_registry = Arc::new(ProcessorRegistry::new());
    let log_buffer = LogBuffer::new();

    DaqApp::new(
        settings.clone(),
        instrument_registry,
        processor_registry,
        log_buffer,
    )
    .unwrap()
}

#[test]
#[serial]
fn test_data_flow_from_10_instruments() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        const INSTRUMENT_COUNT: usize = 10;

        // Spawn instruments
        for i in 0..INSTRUMENT_COUNT {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        // Subscribe to data
        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        // Collect data points by channel for 2 seconds
        let start = Instant::now();
        let mut channel_counts = HashMap::new();
        let mut total_data_points = 0;

        while start.elapsed() < Duration::from_secs(2) {
            match tokio::time::timeout(Duration::from_millis(100), data_rx.recv()).await {
                Ok(Ok(data_point)) => {
                    *channel_counts
                        .entry(data_point.channel.clone())
                        .or_insert(0) += 1;
                    total_data_points += 1;
                }
                Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                    eprintln!("WARNING: Receiver lagged by {} messages", n);
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    panic!("Data channel closed unexpectedly");
                }
                Err(_) => {} // Timeout, continue
            }
        }

        println!("\nData Flow Test Results:");
        println!("  Instruments: {}", INSTRUMENT_COUNT);
        println!("  Total data points: {}", total_data_points);
        println!("  Unique channels: {}", channel_counts.len());
        println!("\nData points per channel:");
        for (channel, count) in &channel_counts {
            println!("  {}: {}", channel, count);
        }

        // Each mock instrument should produce some data
        assert!(
            total_data_points > 0,
            "Should receive data from instruments"
        );
        assert!(
            channel_counts.len() >= INSTRUMENT_COUNT,
            "Should see data from at least {} instruments, got {} channels",
            INSTRUMENT_COUNT,
            channel_counts.len()
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_detect_broadcast_lag() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn multiple instruments to generate high data rate
        for i in 0..20 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        // Subscribe but don't consume data (creates lag)
        let mut slow_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        // Let data accumulate
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Now try to receive and detect lag
        let mut lag_detected = false;
        let mut lag_count = 0;

        for _ in 0..10 {
            match slow_rx.recv().await {
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    lag_detected = true;
                    lag_count = n;
                    eprintln!("Detected lag: {} messages", n);
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("Channel closed unexpectedly");
                }
            }
        }

        println!("\nBroadcast Lag Detection:");
        println!("  Lag detected: {}", lag_detected);
        if lag_detected {
            println!("  Messages lagged: {}", lag_count);
        }

        // This test verifies that lag is properly detected
        // With 20 instruments and slow consumption, lag should occur
        assert!(
            lag_detected,
            "Expected to detect lag with 20 instruments and slow receiver"
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_multiple_subscribers_receive_same_data() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn instruments
        for i in 0..5 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        // Create multiple subscribers
        let mut rx1 = app.with_inner(|inner| inner.data_sender.subscribe());
        let mut rx2 = app.with_inner(|inner| inner.data_sender.subscribe());
        let mut rx3 = app.with_inner(|inner| inner.data_sender.subscribe());

        // Collect data from all subscribers concurrently
        let collector1 = tokio::spawn(async move {
            let mut count = 0;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(1) {
                if rx1.recv().await.is_ok() {
                    count += 1;
                }
            }
            count
        });

        let collector2 = tokio::spawn(async move {
            let mut count = 0;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(1) {
                if rx2.recv().await.is_ok() {
                    count += 1;
                }
            }
            count
        });

        let collector3 = tokio::spawn(async move {
            let mut count = 0;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(1) {
                if rx3.recv().await.is_ok() {
                    count += 1;
                }
            }
            count
        });

        let count1 = collector1.await.unwrap();
        let count2 = collector2.await.unwrap();
        let count3 = collector3.await.unwrap();

        println!("\nMultiple Subscribers Test:");
        println!("  Subscriber 1: {} data points", count1);
        println!("  Subscriber 2: {} data points", count2);
        println!("  Subscriber 3: {} data points", count3);

        // All subscribers should receive roughly the same amount of data
        assert!(count1 > 0, "Subscriber 1 should receive data");
        assert!(count2 > 0, "Subscriber 2 should receive data");
        assert!(count3 > 0, "Subscriber 3 should receive data");

        // Allow for some variance due to timing, but should be within 20% of each other
        let max_count = count1.max(count2).max(count3) as f64;
        let min_count = count1.min(count2).min(count3) as f64;
        let variance = (max_count - min_count) / max_count;

        assert!(
            variance < 0.3,
            "Variance between subscribers should be < 30%, got {:.1}%",
            variance * 100.0
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_data_continues_during_instrument_lifecycle() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Start with 5 instruments
        for i in 0..5 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        // Collector task
        let collector = tokio::spawn(async move {
            let mut counts_by_second = Vec::new();
            let start = Instant::now();

            for second in 0..5 {
                let second_start = Instant::now();
                let mut count = 0;

                while second_start.elapsed() < Duration::from_secs(1) {
                    if let Ok(Ok(_)) =
                        tokio::time::timeout(Duration::from_millis(10), data_rx.recv()).await
                    {
                        count += 1;
                    }
                }

                counts_by_second.push((second, count));
            }

            counts_by_second
        });

        // Perform lifecycle operations while data flows
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Stop 2 instruments
        app.with_inner(|inner| inner.stop_instrument("mock_0"));
        app.with_inner(|inner| inner.stop_instrument("mock_1"));

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Add 3 new instruments
        for i in 5..8 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        // Wait for collector to finish
        let counts = collector.await.unwrap();

        println!("\nData Flow During Lifecycle Operations:");
        for (second, count) in &counts {
            println!("  Second {}: {} data points", second, count);
        }

        // Verify data flowed in every second
        for (second, count) in &counts {
            assert!(
                *count > 0,
                "Should receive data in second {}, got {}",
                second,
                count
            );
        }
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_no_data_loss_under_normal_load() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn 10 instruments
        for i in 0..10 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        let mut total_received = 0;
        let mut lag_events = 0;
        let start = Instant::now();

        while start.elapsed() < Duration::from_secs(3) {
            match data_rx.recv().await {
                Ok(_) => {
                    total_received += 1;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    lag_events += 1;
                    eprintln!("Lag event {}: {} messages dropped", lag_events, n);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("Channel closed unexpectedly");
                }
            }
        }

        println!("\nData Loss Test (Normal Load):");
        println!("  Total received: {}", total_received);
        println!("  Lag events: {}", lag_events);
        println!("  Rate: {:.0} data points/sec", total_received as f64 / 3.0);

        assert!(total_received > 0, "Should receive data points");

        // Under normal load with 10 instruments, there should be no lag
        assert_eq!(
            lag_events, 0,
            "Should not experience lag under normal load with 10 instruments"
        );
    });

    app.shutdown();
}
