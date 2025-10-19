//! Multi-instrument concurrent spawning tests
//!
//! Tests for validating that the DAQ system can handle concurrent instrument
//! spawning and shutdown without deadlocks or failures.

use rust_daq::{
    app::DaqApp,
    config::Settings,
    data::registry::ProcessorRegistry,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
    measurement::Measure,
};
use serial_test::serial;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::timeout;

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
fn test_spawn_10_instruments_concurrently() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        let start = Instant::now();
        let mut spawn_times = Vec::new();

        // Spawn 10 instruments concurrently
        for i in 0..10 {
            let instrument_id = format!("mock_{}", i);
            let spawn_start = Instant::now();

            let result = app.with_inner(|inner| inner.spawn_instrument(&instrument_id));

            let spawn_duration = spawn_start.elapsed();
            spawn_times.push(spawn_duration);

            assert!(
                result.is_ok(),
                "Failed to spawn instrument {}: {:?}",
                instrument_id,
                result.err()
            );
        }

        let total_duration = start.elapsed();

        // Verify all instruments are running
        let instrument_count = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(
            instrument_count, 10,
            "Expected 10 instruments, got {}",
            instrument_count
        );

        // Log spawn time statistics
        let avg_spawn_time = spawn_times.iter().sum::<Duration>() / spawn_times.len() as u32;
        let max_spawn_time = spawn_times.iter().max().unwrap();

        println!("Spawn Statistics:");
        println!("  Total time: {:?}", total_duration);
        println!("  Average spawn time: {:?}", avg_spawn_time);
        println!("  Max spawn time: {:?}", max_spawn_time);

        // Verify data is flowing from all instruments
        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        // Collect data points from different channels
        let mut channels_seen = std::collections::HashSet::new();
        let timeout_duration = Duration::from_secs(5);
        let timeout_result = timeout(timeout_duration, async {
            while channels_seen.len() < 10 {
                if let Ok(data_point) = data_rx.recv().await {
                    // Extract instrument ID from channel name
                    if let Some(id) = data_point.channel.split('_').next() {
                        if id == "mock" {
                            channels_seen.insert(data_point.channel.clone());
                        }
                    }
                }
            }
        })
        .await;

        assert!(
            timeout_result.is_ok(),
            "Timed out waiting for data from all instruments"
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_spawn_20_instruments_concurrently() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        let start = Instant::now();

        // Spawn 20 instruments
        for i in 0..20 {
            let instrument_id = format!("mock_{}", i);

            let result = app.with_inner(|inner| inner.spawn_instrument(&instrument_id));

            assert!(
                result.is_ok(),
                "Failed to spawn instrument {}: {:?}",
                instrument_id,
                result.err()
            );
        }

        let total_duration = start.elapsed();

        // Verify all instruments are running
        let instrument_count = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(
            instrument_count, 20,
            "Expected 20 instruments, got {}",
            instrument_count
        );

        println!("20 instruments spawned in {:?}", total_duration);

        // Verify system is responsive
        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());
        let recv_result = timeout(Duration::from_secs(5), data_rx.recv()).await;
        assert!(
            recv_result.is_ok(),
            "System not producing data with 20 instruments"
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_spawn_stop_spawn_cycle() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Cycle: spawn 5, stop 3, spawn 3 more

        // Spawn 5
        for i in 0..5 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        let count = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(count, 5, "Should have 5 instruments");

        // Stop 3
        for i in 0..3 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.stop_instrument(&instrument_id));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;

        let count = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(count, 2, "Should have 2 instruments after stopping 3");

        // Spawn 3 more
        for i in 5..8 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        let count = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(count, 5, "Should have 5 instruments after respawning");
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_no_deadlock_on_concurrent_operations() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn initial instruments
        for i in 0..5 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        // Concurrent operations with timeout to detect deadlocks
        let operations = vec![
            // Subscribe to data
            tokio::spawn({
                let app = app.clone();
                async move {
                    let mut rx = app.with_inner(|inner| inner.data_sender.subscribe());
                    for _ in 0..10 {
                        let _ = rx.recv().await;
                    }
                }
            }),
            // Stop and start instruments
            tokio::spawn({
                let app = app.clone();
                async move {
                    app.with_inner(|inner| inner.stop_instrument("mock_0"));
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    app.with_inner(|inner| inner.spawn_instrument("mock_0"))
                        .unwrap();
                }
            }),
            // Get instrument count multiple times
            tokio::spawn({
                let app = app.clone();
                async move {
                    for _ in 0..10 {
                        let _ = app.with_inner(|inner| inner.instruments.len());
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }),
        ];

        // Wait for all operations with timeout
        let timeout_duration = Duration::from_secs(10);
        let result = timeout(timeout_duration, async {
            for handle in operations {
                handle.await.unwrap();
            }
        })
        .await;

        assert!(
            result.is_ok(),
            "Deadlock detected: operations did not complete within timeout"
        );
    });

    app.shutdown();
}
