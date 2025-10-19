//! Command flood tests
//!
//! Tests for validating that the system can handle high-frequency command
//! bursts from the GUI without dropping commands or blocking.

use rust_daq::{
    app::DaqApp,
    config::Settings,
    core::InstrumentCommand,
    data::registry::ProcessorRegistry,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
    measurement::Measure,
};
use serial_test::serial;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
fn test_command_flood_single_instrument() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn single instrument
        app.with_inner(|inner| inner.spawn_instrument("mock_0"))
            .unwrap();

        const COMMANDS_PER_SECOND: usize = 1000;
        const DURATION_SECONDS: u64 = 1;
        const TOTAL_COMMANDS: usize = COMMANDS_PER_SECOND * DURATION_SECONDS as usize;

        let start = Instant::now();
        let mut sent_count = 0;
        let mut failed_count = 0;
        let mut latencies = Vec::new();

        // Send commands as fast as possible
        for _ in 0..TOTAL_COMMANDS {
            let cmd_start = Instant::now();

            let result = app.with_inner(|inner| {
                inner.send_instrument_command("mock_0", InstrumentCommand::Shutdown)
            });

            let latency = cmd_start.elapsed();
            latencies.push(latency);

            if result.is_ok() {
                sent_count += 1;
            } else {
                failed_count += 1;
            }

            // Target 1000 commands/sec
            let target_interval = Duration::from_micros(1000); // 1ms between commands
            if let Some(sleep_time) = target_interval.checked_sub(latency) {
                tokio::time::sleep(sleep_time).await;
            }
        }

        let elapsed = start.elapsed();

        // Calculate statistics
        let avg_latency = latencies.iter().sum::<Duration>() / latencies.len() as u32;
        let max_latency = latencies.iter().max().unwrap();
        let p95_index = (latencies.len() as f64 * 0.95) as usize;
        let mut sorted_latencies = latencies.clone();
        sorted_latencies.sort();
        let p95_latency = sorted_latencies[p95_index];

        println!("\nCommand Flood Test Results:");
        println!("  Total commands sent: {}", sent_count);
        println!("  Failed commands: {}", failed_count);
        println!("  Duration: {:?}", elapsed);
        println!(
            "  Actual rate: {:.0} cmd/sec",
            sent_count as f64 / elapsed.as_secs_f64()
        );
        println!("\nLatency Statistics:");
        println!("  Average: {:?}", avg_latency);
        println!("  P95: {:?}", p95_latency);
        println!("  Max: {:?}", max_latency);

        // Assertions
        assert_eq!(
            sent_count, TOTAL_COMMANDS,
            "All commands should be sent successfully"
        );
        assert_eq!(failed_count, 0, "No commands should fail");
        assert!(
            avg_latency < Duration::from_millis(10),
            "Average command latency should be < 10ms, got {:?}",
            avg_latency
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_command_flood_multiple_instruments() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        const INSTRUMENT_COUNT: usize = 5;

        // Spawn instruments
        for i in 0..INSTRUMENT_COUNT {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        const COMMANDS_PER_INSTRUMENT: usize = 200;
        let start = Instant::now();

        // Spawn concurrent command senders for each instrument
        let mut handles = Vec::new();

        for i in 0..INSTRUMENT_COUNT {
            let app_clone = app.clone();
            let instrument_id = format!("mock_{}", i);

            let handle = tokio::spawn(async move {
                let mut sent = 0;
                let mut failed = 0;

                for _ in 0..COMMANDS_PER_INSTRUMENT {
                    let result = app_clone.with_inner(|inner| {
                        inner.send_instrument_command(&instrument_id, InstrumentCommand::Shutdown)
                    });

                    if result.is_ok() {
                        sent += 1;
                    } else {
                        failed += 1;
                    }

                    tokio::time::sleep(Duration::from_micros(1000)).await;
                }

                (sent, failed)
            });

            handles.push(handle);
        }

        // Wait for all senders to complete
        let mut total_sent = 0;
        let mut total_failed = 0;

        for handle in handles {
            let (sent, failed) = handle.await.unwrap();
            total_sent += sent;
            total_failed += failed;
        }

        let elapsed = start.elapsed();

        println!("\nMulti-Instrument Command Flood Results:");
        println!("  Instruments: {}", INSTRUMENT_COUNT);
        println!("  Commands per instrument: {}", COMMANDS_PER_INSTRUMENT);
        println!("  Total commands sent: {}", total_sent);
        println!("  Failed commands: {}", total_failed);
        println!("  Duration: {:?}", elapsed);
        println!(
            "  Overall rate: {:.0} cmd/sec",
            total_sent as f64 / elapsed.as_secs_f64()
        );

        assert_eq!(
            total_sent,
            INSTRUMENT_COUNT * COMMANDS_PER_INSTRUMENT,
            "All commands should be sent"
        );
        assert_eq!(total_failed, 0, "No commands should fail");
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_command_bursts_dont_block_data_flow() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn instrument
        app.with_inner(|inner| inner.spawn_instrument("mock_0"))
            .unwrap();

        // Subscribe to data
        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        // Spawn data counter task
        let data_counter = tokio::spawn(async move {
            let mut count = 0;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(2) {
                if data_rx.recv().await.is_ok() {
                    count += 1;
                }
            }
            count
        });

        // Send command bursts while data is flowing
        for _ in 0..10 {
            for _ in 0..100 {
                let _ = app.with_inner(|inner| {
                    inner.send_instrument_command("mock_0", InstrumentCommand::Shutdown)
                });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let data_points_received = data_counter.await.unwrap();

        println!("\nCommand Bursts vs Data Flow:");
        println!("  Commands sent: 1000 (10 bursts of 100)");
        println!("  Data points received: {}", data_points_received);

        assert!(
            data_points_received > 0,
            "Data should continue flowing during command bursts"
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_command_queue_recovery() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn instrument
        app.with_inner(|inner| inner.spawn_instrument("mock_0"))
            .unwrap();

        // Flood with commands to fill the queue
        const FLOOD_SIZE: usize = 1000;
        for _ in 0..FLOOD_SIZE {
            let _ = app.with_inner(|inner| {
                inner.send_instrument_command("mock_0", InstrumentCommand::Shutdown)
            });
        }

        // Wait for queue to drain
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Send new command and verify system recovered
        let result = app.with_inner(|inner| {
            inner.send_instrument_command("mock_0", InstrumentCommand::Shutdown)
        });

        assert!(
            result.is_ok(),
            "Command should succeed after queue recovery: {:?}",
            result.err()
        );

        // Verify data is still flowing
        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());
        let recv_result = tokio::time::timeout(Duration::from_secs(2), data_rx.recv()).await;

        assert!(
            recv_result.is_ok(),
            "Data should continue flowing after command flood recovery"
        );
    });

    app.shutdown();
}
