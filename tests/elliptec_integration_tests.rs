//! Hardware integration tests for Elliptec ELL14 Rotation Mounts - Graceful Shutdown
//!
//! Run with: cargo test --test elliptec_integration_tests --features instrument_serial -- --ignored --nocapture

use rust_daq::{
    app_actor::DaqManagerActor,
    config::Settings,
    core::InstrumentCommand,
    instrument::{InstrumentRegistry, InstrumentRegistryV2},
    measurement::InstrumentMeasurement,
    messages::DaqCommand,
    data::registry::ProcessorRegistry,
    modules::ModuleRegistry,
};
use std::{sync::Arc, time::Duration};
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

/// Creates a temporary settings file for the Elliptec instrument.
fn create_temp_settings_file() -> NamedTempFile {
    let content = r#"
[application]
broadcast_channel_capacity = 1024
command_channel_capacity = 128

[application.data_distributor]
subscriber_capacity = 256
warn_drop_rate_percent = 1.0
error_saturation_percent = 10.0
metrics_window_secs = 5

[application.timeouts]
connect_secs = 10
disconnect_secs = 5
command_secs = 5
shutdown_secs = 10

[instruments.elliptec_rotators]
type = "elliptec"
port = "/dev/ttyUSB0"
baud_rate = 9600
device_addresses = [2]
polling_rate_hz = 10.0
"#;
    let file = NamedTempFile::new().unwrap();
    std::fs::write(file.path(), content).unwrap();
    file
}

#[tokio::test]
#[ignore] // Hardware-only test
async fn test_graceful_shutdown_and_disconnect() {
    println!("\n=== Elliptec Graceful Shutdown and Disconnect Test (V5 Arch) ===");
    println!("Purpose: Verify instrument completes in-flight commands and state is preserved across a graceful shutdown.");
    println!();

    let settings_file = create_temp_settings_file();
    let settings = Arc::new(Settings::from_path(settings_file.path()).unwrap());
    let runtime = Arc::new(Runtime::new().unwrap());

    // --- Step 1: Initial connection and issue move command ---
    println!("Step 1: Connecting and issuing a move command...");
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(128);
    let actor = DaqManagerActor::<InstrumentMeasurement>::new(
        settings.as_ref().clone(),
        Arc::new(InstrumentRegistry::new()),
        Arc::new(InstrumentRegistryV2::new()),
        Arc::new(ProcessorRegistry::new()),
        Arc::new(ModuleRegistry::new()),
        runtime.clone(),
    ).unwrap();
    runtime.spawn(actor.run(cmd_rx));

    // Spawn the instrument
    let (spawn_cmd, spawn_rx) = DaqCommand::spawn_instrument("elliptec_rotators".to_string());
    cmd_tx.send(spawn_cmd).await.unwrap();
    spawn_rx.await.unwrap().unwrap();

    // Subscribe to data to get position
    let (sub_cmd, sub_rx) = DaqCommand::subscribe_to_data();
    cmd_tx.send(sub_cmd).await.unwrap();
    let mut data_rx = sub_rx.await.unwrap();

    // Read initial position
    let initial_dp = tokio::time::timeout(Duration::from_secs(2), data_rx.recv())
        .await
        .expect("Timeout waiting for initial position")
        .unwrap();
    let initial_pos = initial_dp.as_scalar().unwrap().value;
    println!("  - Initial position: {:.2} degrees", initial_pos);

    // Issue a move command
    let target_pos = initial_pos + 10.0;
    let set_pos_v1_cmd = InstrumentCommand::SetParameter(
        "2:position".to_string(),
        rust_daq::core::ParameterValue::Float(target_pos),
    );
    let (move_cmd, move_rx) = DaqCommand::send_instrument_command("elliptec_rotators".to_string(), set_pos_v1_cmd);
    cmd_tx.send(move_cmd).await.unwrap();
    move_rx.await.unwrap().unwrap();
    println!("  - Move command sent to {:.2} degrees.", target_pos);
    println!("Step 1: Complete.");
    println!();

    // --- Step 2: Graceful shutdown (IMMEDIATELY after command) ---
    println!("Step 2: Shutting down the application immediately...");
    let (shutdown_cmd, shutdown_rx) = DaqCommand::shutdown();
    cmd_tx.send(shutdown_cmd).await.unwrap();
    shutdown_rx.await.unwrap().unwrap();
    println!("Step 2: Complete. Now waiting for physical move to finish...");

    // Wait for the physical move to complete while the app is offline.
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("         Physical move should now be complete.");
    println!();

    // --- Step 3: Reconnect and verify position ---
    println!("Step 3: Reconnecting and verifying position...");
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(128);
    let actor = DaqManagerActor::<InstrumentMeasurement>::new(
        settings.as_ref().clone(),
        Arc::new(InstrumentRegistry::new()),
        Arc::new(InstrumentRegistryV2::new()),
        Arc::new(ProcessorRegistry::new()),
        Arc::new(ModuleRegistry::new()),
        runtime.clone(),
    ).unwrap();
    runtime.spawn(actor.run(cmd_rx));

    // Spawn the instrument again
    let (spawn_cmd, spawn_rx) = DaqCommand::spawn_instrument("elliptec_rotators".to_string());
    cmd_tx.send(spawn_cmd).await.unwrap();
    spawn_rx.await.unwrap().unwrap();

    // Subscribe to data
    let (sub_cmd, sub_rx) = DaqCommand::subscribe_to_data();
    cmd_tx.send(sub_cmd).await.unwrap();
    let mut data_rx = sub_rx.await.unwrap();

    // The first read might be stale, so we read a few times.
    let mut final_pos = 0.0;
    for _ in 0..5 {
        let dp = tokio::time::timeout(Duration::from_secs(2), data_rx.recv())
            .await
            .expect("Timeout waiting for position after reconnect")
            .unwrap();
        final_pos = dp.as_scalar().unwrap().value;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("  - Position after reconnect: {:.2} degrees", final_pos);

    assert!(
        (final_pos - target_pos).abs() < 0.1,
        "Position was not preserved after shutdown! Expected {:.2}, got {:.2}",
        target_pos,
        final_pos
    );
    println!("Step 3: Complete. Position successfully preserved.");
    println!();

    println!("✅ Test Passed: Elliptec completes in-flight commands and state is preserved.");
}
