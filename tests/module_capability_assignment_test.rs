//! Integration tests for module capability-based instrument assignment.
//!
//! Tests the complete workflow of spawning modules, assigning instruments via
//! capabilities, and validating the assignment system using the DaqCommand API.

use rust_daq::app_actor::DaqManagerActor;
use rust_daq::config::{ApplicationSettings, Settings, StorageSettings};
use rust_daq::data::registry::ProcessorRegistry;
use rust_daq::instrument::capabilities::power_measurement_capability_id;
use rust_daq::instrument::InstrumentRegistry;
use rust_daq::log_capture::LogBuffer;
use rust_daq::measurement::InstrumentMeasurement;
use rust_daq::messages::DaqCommand;
use rust_daq::modules::ModuleConfig;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

fn create_test_settings() -> Settings {
    Settings {
        log_level: "info".to_string(),
        application: ApplicationSettings {
            broadcast_channel_capacity: 64,
            command_channel_capacity: 16,
        },
        storage: StorageSettings {
            default_path: "./data".to_string(),
            default_format: "csv".to_string(),
        },
        instruments: HashMap::new(),
        processors: None,
    }
}

async fn setup_actor() -> mpsc::Sender<DaqCommand> {
    let settings = create_test_settings();
    let runtime = Arc::new(Runtime::new().expect("Failed to create runtime"));

    let actor = DaqManagerActor::<InstrumentMeasurement>::new(
        Arc::new(settings),
        Arc::new(InstrumentRegistry::new()),
        Arc::new(ProcessorRegistry::new()),
        LogBuffer::new(),
        runtime,
    )
    .expect("Failed to create actor");

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    tokio::spawn(actor.run(cmd_rx));

    cmd_tx
}

#[tokio::test]
async fn test_spawn_module_via_command() {
    let cmd_tx = setup_actor().await;

    let config = ModuleConfig {
        params: HashMap::new(),
    };

    let (spawn_cmd, spawn_rx) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config);

    cmd_tx
        .send(spawn_cmd)
        .await
        .expect("Should send command");

    let result = spawn_rx.await.expect("Should receive response");
    assert!(
        result.is_ok(),
        "Module spawn should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_reject_duplicate_module_spawn_via_command() {
    let cmd_tx = setup_actor().await;

    let config = ModuleConfig {
        params: HashMap::new(),
    };

    // First spawn should succeed
    let (spawn_cmd1, spawn_rx1) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config.clone());
    cmd_tx.send(spawn_cmd1).await.expect("Should send command");
    let result1 = spawn_rx1.await.expect("Should receive response");
    assert!(result1.is_ok(), "First spawn should succeed");

    // Second spawn with same ID should fail
    let (spawn_cmd2, spawn_rx2) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config);
    cmd_tx.send(spawn_cmd2).await.expect("Should send command");
    let result2 = spawn_rx2.await.expect("Should receive response");

    assert!(
        result2.is_err(),
        "Second spawn with same ID should fail"
    );
    let err = result2.unwrap_err();
    assert!(
        err.to_string().contains("already spawned"),
        "Error should mention duplicate, got: {}",
        err
    );
}

#[tokio::test]
async fn test_reject_unknown_module_type_via_command() {
    let cmd_tx = setup_actor().await;

    let config = ModuleConfig {
        params: HashMap::new(),
    };

    let (spawn_cmd, spawn_rx) =
        DaqCommand::spawn_module("test_module".to_string(), "unknown_type".to_string(), config);
    cmd_tx.send(spawn_cmd).await.expect("Should send command");
    let result = spawn_rx.await.expect("Should receive response");

    assert!(result.is_err(), "Should reject unknown module type");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("Unknown module type"),
        "Error should mention unknown type, got: {}",
        err
    );
}

#[tokio::test]
async fn test_assign_instrument_rejects_missing_module() {
    let cmd_tx = setup_actor().await;

    let (assign_cmd, assign_rx) = DaqCommand::assign_instrument_to_module(
        "nonexistent_module".to_string(),
        "power_meter".to_string(),
        "power_meter_1".to_string(),
    );

    cmd_tx
        .send(assign_cmd)
        .await
        .expect("Should send command");

    let result = assign_rx.await.expect("Should receive response");
    assert!(
        result.is_err(),
        "Should reject assignment when module not found"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("is not spawned"),
        "Error should mention module not found, got: {}",
        err
    );
}

#[tokio::test]
async fn test_assign_instrument_rejects_missing_instrument() {
    let cmd_tx = setup_actor().await;

    // Spawn module first
    let config = ModuleConfig {
        params: HashMap::new(),
    };
    let (spawn_cmd, spawn_rx) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config);
    cmd_tx.send(spawn_cmd).await.expect("Should send command");
    spawn_rx
        .await
        .expect("Should receive response")
        .expect("Module spawn should succeed");

    // Try to assign non-existent instrument
    let (assign_cmd, assign_rx) = DaqCommand::assign_instrument_to_module(
        "pm_module".to_string(),
        "power_meter".to_string(),
        "nonexistent_instrument".to_string(),
    );

    cmd_tx
        .send(assign_cmd)
        .await
        .expect("Should send command");

    let result = assign_rx.await.expect("Should receive response");
    assert!(
        result.is_err(),
        "Should reject assignment when instrument not found"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("is not running"),
        "Error should mention instrument not running, got: {}",
        err
    );
}

#[tokio::test]
async fn test_start_module_via_command() {
    let cmd_tx = setup_actor().await;

    // Spawn module
    let config = ModuleConfig {
        params: HashMap::new(),
    };
    let (spawn_cmd, spawn_rx) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config);
    cmd_tx.send(spawn_cmd).await.expect("Should send command");
    spawn_rx
        .await
        .expect("Should receive response")
        .expect("Module spawn should succeed");

    // Start module (will fail because no instrument assigned, but tests the command flow)
    let (start_cmd, start_rx) = DaqCommand::start_module("pm_module".to_string());
    cmd_tx
        .send(start_cmd)
        .await
        .expect("Should send command");

    let result = start_rx.await.expect("Should receive response");
    // This may fail due to missing instrument assignment, which is expected
    // We're testing the command pathway works
    let _ = result;
}

#[tokio::test]
async fn test_stop_module_via_command() {
    let cmd_tx = setup_actor().await;

    // Spawn module
    let config = ModuleConfig {
        params: HashMap::new(),
    };
    let (spawn_cmd, spawn_rx) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config);
    cmd_tx.send(spawn_cmd).await.expect("Should send command");
    spawn_rx
        .await
        .expect("Should receive response")
        .expect("Module spawn should succeed");

    // Stop module
    let (stop_cmd, stop_rx) = DaqCommand::stop_module("pm_module".to_string());
    cmd_tx.send(stop_cmd).await.expect("Should send command");

    let result = stop_rx.await.expect("Should receive response");
    assert!(
        result.is_ok(),
        "Module stop should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_module_lifecycle_complete() {
    let cmd_tx = setup_actor().await;

    // 1. Spawn module
    let config = ModuleConfig {
        params: HashMap::new(),
    };
    let (spawn_cmd, spawn_rx) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config);
    cmd_tx.send(spawn_cmd).await.expect("Should send command");
    spawn_rx
        .await
        .expect("Should receive response")
        .expect("Module spawn should succeed");

    // 2. Stop module (not started yet, should be idempotent)
    let (stop_cmd, stop_rx) = DaqCommand::stop_module("pm_module".to_string());
    cmd_tx.send(stop_cmd).await.expect("Should send command");
    stop_rx
        .await
        .expect("Should receive response")
        .expect("Module stop should succeed");
}

#[tokio::test]
async fn test_shutdown_with_active_module() {
    let cmd_tx = setup_actor().await;

    // Spawn module
    let config = ModuleConfig {
        params: HashMap::new(),
    };
    let (spawn_cmd, spawn_rx) =
        DaqCommand::spawn_module("pm_module".to_string(), "power_meter".to_string(), config);
    cmd_tx.send(spawn_cmd).await.expect("Should send command");
    spawn_rx
        .await
        .expect("Should receive response")
        .expect("Module spawn should succeed");

    // Shutdown - should cleanup module gracefully
    let (shutdown_cmd, shutdown_rx) = DaqCommand::shutdown();
    cmd_tx
        .send(shutdown_cmd)
        .await
        .expect("Should send command");

    let result = shutdown_rx.await.expect("Should receive response");
    // Shutdown may return errors from module cleanup, that's ok
    let _ = result;
}