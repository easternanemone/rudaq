//! Session round-trip tests
//!
//! Tests for validating that sessions can be saved and restored correctly
//! with all instrument state preserved across multiple iterations.

use rust_daq::{
    app::DaqApp,
    config::Settings,
    data::registry::ProcessorRegistry,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
    measurement::Measure,
    session::GuiState,
};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use tempfile::NamedTempFile;

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
fn test_single_session_roundtrip() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn 10 instruments
        for i in 0..10 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        // Verify initial state
        let initial_count = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(initial_count, 10, "Should have 10 instruments initially");

        // Create temporary file for session
        let temp_file = NamedTempFile::new().unwrap();
        let session_path = temp_file.path();

        // Save session
        let gui_state = GuiState::default();
        app.save_session(session_path, gui_state.clone()).unwrap();

        // Stop all instruments
        let instrument_ids: Vec<String> =
            app.with_inner(|inner| inner.instruments.keys().cloned().collect());
        for id in &instrument_ids {
            app.with_inner(|inner| inner.stop_instrument(id));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;

        let count_after_stop = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(count_after_stop, 0, "All instruments should be stopped");

        // Load session
        let loaded_gui_state = app.load_session(session_path).unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify instruments are restored
        let restored_count = app.with_inner(|inner| inner.instruments.len());
        assert_eq!(
            restored_count, initial_count,
            "Restored instrument count should match initial count"
        );

        // Verify GUI state
        assert_eq!(
            loaded_gui_state.log_panel_visible, gui_state.log_panel_visible,
            "GUI state should be preserved"
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_100_iteration_session_roundtrip() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        const ITERATIONS: usize = 100;
        const INSTRUMENT_COUNT: usize = 10;

        let mut success_count = 0;
        let mut failures = Vec::new();

        for iteration in 0..ITERATIONS {
            // Spawn instruments
            for i in 0..INSTRUMENT_COUNT {
                let instrument_id = format!("mock_{}", i);
                let result = app.with_inner(|inner| inner.spawn_instrument(&instrument_id));
                if result.is_err() {
                    failures.push(format!(
                        "Iteration {}: Failed to spawn instrument {}: {:?}",
                        iteration,
                        i,
                        result.err()
                    ));
                    continue;
                }
            }

            // Verify spawn
            let spawned_count = app.with_inner(|inner| inner.instruments.len());
            if spawned_count != INSTRUMENT_COUNT {
                failures.push(format!(
                    "Iteration {}: Expected {} instruments, got {}",
                    iteration, INSTRUMENT_COUNT, spawned_count
                ));
                continue;
            }

            // Create temp file
            let temp_file = NamedTempFile::new().unwrap();
            let session_path = temp_file.path();

            // Save session
            let gui_state = GuiState {
                log_panel_visible: iteration % 2 == 0,
                storage_panel_visible: iteration % 3 == 0,
                plot_panel_visible: true,
            };

            if let Err(e) = app.save_session(session_path, gui_state.clone()) {
                failures.push(format!(
                    "Iteration {}: Failed to save session: {:?}",
                    iteration, e
                ));
                continue;
            }

            // Stop all
            let instrument_ids: Vec<String> =
                app.with_inner(|inner| inner.instruments.keys().cloned().collect());
            for id in &instrument_ids {
                app.with_inner(|inner| inner.stop_instrument(id));
            }

            tokio::time::sleep(Duration::from_millis(50)).await;

            // Load session
            match app.load_session(session_path) {
                Ok(loaded_gui_state) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;

                    // Verify restoration
                    let restored_count = app.with_inner(|inner| inner.instruments.len());
                    if restored_count != INSTRUMENT_COUNT {
                        failures.push(format!(
                            "Iteration {}: Restored {} instruments, expected {}",
                            iteration, restored_count, INSTRUMENT_COUNT
                        ));
                        continue;
                    }

                    // Verify GUI state
                    if loaded_gui_state.log_panel_visible != gui_state.log_panel_visible {
                        failures.push(format!("Iteration {}: GUI state mismatch", iteration));
                        continue;
                    }

                    success_count += 1;
                }
                Err(e) => {
                    failures.push(format!(
                        "Iteration {}: Failed to load session: {:?}",
                        iteration, e
                    ));
                    continue;
                }
            }

            // Clean up for next iteration
            let instrument_ids: Vec<String> =
                app.with_inner(|inner| inner.instruments.keys().cloned().collect());
            for id in &instrument_ids {
                app.with_inner(|inner| inner.stop_instrument(id));
            }

            tokio::time::sleep(Duration::from_millis(50)).await;

            if (iteration + 1) % 10 == 0 {
                println!(
                    "Completed {} iterations, {} successes",
                    iteration + 1,
                    success_count
                );
            }
        }

        // Report results
        println!("\nSession Round-Trip Test Results:");
        println!("  Total iterations: {}", ITERATIONS);
        println!("  Successful: {}", success_count);
        println!("  Failed: {}", failures.len());

        if !failures.is_empty() {
            println!("\nFailures:");
            for (i, failure) in failures.iter().enumerate().take(10) {
                println!("  {}: {}", i + 1, failure);
            }
            if failures.len() > 10 {
                println!("  ... and {} more", failures.len() - 10);
            }
        }

        assert_eq!(
            success_count, ITERATIONS,
            "Expected 100/100 successful iterations, got {}/{}",
            success_count, ITERATIONS
        );
    });

    app.shutdown();
}

#[test]
#[serial]
fn test_session_preserves_storage_format() {
    let app = create_test_app();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        // Spawn instruments
        for i in 0..5 {
            let instrument_id = format!("mock_{}", i);
            app.with_inner(|inner| inner.spawn_instrument(&instrument_id))
                .unwrap();
        }

        // Set storage format
        app.with_inner(|inner| {
            inner.storage_format = "hdf5".to_string();
        });

        let temp_file = NamedTempFile::new().unwrap();
        let session_path = temp_file.path();

        // Save and load
        app.save_session(session_path, GuiState::default()).unwrap();

        let storage_format_before = app.with_inner(|inner| inner.storage_format.clone());

        app.with_inner(|inner| {
            inner.storage_format = "csv".to_string(); // Change it
        });

        app.load_session(session_path).unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;

        let storage_format_after = app.with_inner(|inner| inner.storage_format.clone());

        assert_eq!(
            storage_format_after, storage_format_before,
            "Storage format should be restored from session"
        );
    });

    app.shutdown();
}
