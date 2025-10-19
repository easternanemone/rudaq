//! Integration tests for storage writer graceful shutdown behavior.

use rust_daq::{
    app::DaqApp,
    config::Settings,
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
fn test_storage_writer_graceful_shutdown() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Start recording
        app.with_inner(|inner| {
            inner.start_recording().expect("Failed to start recording");
        });

        // Let it record some data
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Stop recording (should use graceful shutdown)
        let start_time = std::time::Instant::now();
        app.with_inner(|inner| {
            inner.stop_recording();
        });

        // Give it time to complete graceful shutdown
        tokio::time::sleep(Duration::from_millis(200)).await;

        let elapsed = start_time.elapsed();

        // Should complete quickly with graceful shutdown (much less than timeout)
        assert!(
            elapsed < Duration::from_secs(1),
            "Graceful shutdown took too long: {:?}",
            elapsed
        );
    });

    app.shutdown();
}

#[test]
fn test_stop_recording_when_not_recording() {
    let app = create_test_app();

    // Should be safe to call stop_recording when not recording
    app.with_inner(|inner| {
        inner.stop_recording(); // Should be no-op
    });

    app.shutdown();
}

#[test]
fn test_multiple_start_stop_cycles() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        for i in 0..3 {
            // Start recording
            app.with_inner(|inner| {
                inner
                    .start_recording()
                    .expect(&format!("Failed to start recording cycle {}", i));
            });

            // Record briefly
            tokio::time::sleep(Duration::from_millis(50)).await;

            // Stop recording
            app.with_inner(|inner| {
                inner.stop_recording();
            });

            // Brief pause between cycles
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    app.shutdown();
}
