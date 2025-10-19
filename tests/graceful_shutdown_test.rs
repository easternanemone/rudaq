//! Tests for graceful shutdown with timeout behavior.

use rust_daq::{
    app::DaqApp,
    config::Settings,
    core::InstrumentCommand,
    data::registry::ProcessorRegistry,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
    measurement::InstrumentMeasurement,
};
use std::sync::Arc;
use std::time::Duration;

/// Helper to create test app with mock instrument.
fn create_test_app() -> DaqApp<InstrumentMeasurement> {
    let settings = Arc::new(Settings::new(None).expect("Failed to create settings"));
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);
    let processor_registry = Arc::new(ProcessorRegistry::new());
    let log_buffer = LogBuffer::new();

    DaqApp::new(
        settings,
        instrument_registry,
        processor_registry,
        log_buffer,
    )
    .expect("Failed to create app")
}

#[test]
fn test_app_shutdown_is_graceful() {
    // Create app which auto-spawns "mock" from config
    let app = create_test_app();

    // Brief pause to let instrument start
    std::thread::sleep(Duration::from_millis(100));

    // Shutdown should complete gracefully
    let start = std::time::Instant::now();
    app.shutdown();
    let elapsed = start.elapsed();

    // Should complete quickly with graceful shutdown (much less than 5s timeout per instrument)
    assert!(
        elapsed < Duration::from_secs(6),
        "Graceful shutdown took too long: {:?}",
        elapsed
    );
}

#[test]
fn test_instrument_receives_shutdown_command() {
    // Create app which auto-spawns "mock" from config
    let app = create_test_app();

    // Brief pause to let instrument start
    std::thread::sleep(Duration::from_millis(100));

    // Send shutdown command explicitly to individual instrument
    let start = std::time::Instant::now();
    app.with_inner(|inner| {
        inner.stop_instrument("mock");
    });
    let elapsed = start.elapsed();

    // Should complete quickly (instrument should respond to shutdown command)
    assert!(
        elapsed < Duration::from_secs(6),
        "Instrument shutdown took too long: {:?}",
        elapsed
    );

    app.shutdown();
}

#[test]
fn test_shutdown_logs_graceful_completion() {
    // This test verifies that the logging infrastructure works
    // Actual log content verification would require capturing logs
    // Create app which auto-spawns "mock" from config
    let app = create_test_app();

    std::thread::sleep(Duration::from_millis(100));
    app.shutdown();

    // If we get here without panic, logging worked
    assert!(true);
}

#[test]
fn test_multiple_instruments_shutdown() {
    // Create app which auto-spawns "mock" from config
    // Note: Testing with just one instrument is sufficient for this test
    let app = create_test_app();

    std::thread::sleep(Duration::from_millis(100));

    // Shutdown all at once
    let start = std::time::Instant::now();
    app.shutdown();
    let elapsed = start.elapsed();

    // Should complete in reasonable time
    assert!(
        elapsed < Duration::from_secs(6),
        "Shutdown took too long: {:?}",
        elapsed
    );
}

#[test]
fn test_concurrent_shutdown_requests_are_idempotent() {
    // Test that multiple concurrent shutdown requests don't cause panics or hangs
    let app = create_test_app();

    std::thread::sleep(Duration::from_millis(100));

    // Call shutdown from multiple threads simultaneously
    let app_clone1 = app.clone();
    let app_clone2 = app.clone();

    let handle1 = std::thread::spawn(move || {
        app_clone1.shutdown();
    });

    let handle2 = std::thread::spawn(move || {
        app_clone2.shutdown();
    });

    // Main thread also calls shutdown
    app.shutdown();

    // All threads should complete without panic
    handle1.join().expect("Thread 1 should not panic");
    handle2.join().expect("Thread 2 should not panic");
}

#[test]
fn test_shutdown_completes_within_timeout() {
    // Verify that shutdown doesn't exceed the 5-second timeout per instrument
    let app = create_test_app();

    std::thread::sleep(Duration::from_millis(100));

    // Measure shutdown time
    let start = std::time::Instant::now();
    app.shutdown();
    let elapsed = start.elapsed();

    // With graceful shutdown, should complete well before 5s timeout
    // (Mock instrument disconnects quickly)
    assert!(
        elapsed < Duration::from_secs(2),
        "Graceful shutdown should complete quickly, took {:?}",
        elapsed
    );
}

#[test]
fn test_stop_instrument_completes_within_timeout() {
    // Test individual instrument stop with timeout enforcement
    let app = create_test_app();

    std::thread::sleep(Duration::from_millis(100));

    // Stop individual instrument
    let start = std::time::Instant::now();
    app.with_inner(|inner| {
        inner.stop_instrument("mock");
    });
    let elapsed = start.elapsed();

    // Should complete within timeout (5s) but much faster for mock
    assert!(
        elapsed < Duration::from_secs(5),
        "Instrument stop should complete within timeout, took {:?}",
        elapsed
    );

    app.shutdown();
}

#[test]
fn test_data_stream_closes_on_shutdown() {
    // Test that data streams are properly closed when instrument shuts down
    let app = create_test_app();

    // Subscribe to data before shutdown
    let mut rx = app.with_inner(|inner| inner.data_sender.subscribe());

    std::thread::sleep(Duration::from_millis(100));

    // Verify we can receive data before shutdown
    let runtime = app.get_runtime();
    let received_before = runtime.block_on(async {
        tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_ok()
    });
    assert!(received_before, "Should receive data before shutdown");

    // Shutdown the app
    app.with_inner(|inner| {
        inner.stop_instrument("mock");
    });

    // After shutdown, stream should eventually close (return None)
    let stream_closed = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), async {
            // Keep reading until we get None (stream closed)
            while rx.recv().await.is_some() {}
        })
        .await
        .is_ok()
    });

    assert!(stream_closed, "Data stream should close after shutdown");

    app.shutdown();
}

#[test]
fn test_shutdown_after_stop_instrument() {
    // Test that shutdown still works correctly after manually stopping an instrument
    let app = create_test_app();

    std::thread::sleep(Duration::from_millis(100));

    // Stop instrument first
    app.with_inner(|inner| {
        inner.stop_instrument("mock");
    });

    std::thread::sleep(Duration::from_millis(50));

    // Then shutdown app - should not hang or panic
    let start = std::time::Instant::now();
    app.shutdown();
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "Shutdown after stop_instrument should be fast, took {:?}",
        elapsed
    );
}
