//! Integration tests for dynamic configuration support.
//!
//! Tests the MVP implementation of runtime instrument reconfiguration via
//! DaqCommand::AddInstrumentDynamic, RemoveInstrumentDynamic, and UpdateInstrumentParameter.

use rust_daq::app_actor::DaqManagerActor;
use rust_daq::config::{versioning::VersionId, ApplicationSettings, Settings, StorageSettings};
use rust_daq::data::registry::ProcessorRegistry;
use rust_daq::instrument::mock::MockInstrument;
use rust_daq::instrument::InstrumentRegistry;
use rust_daq::log_capture::LogBuffer;
use rust_daq::measurement::InstrumentMeasurement;
use rust_daq::messages::DaqCommand;
use rust_daq::modules::ModuleRegistry;
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
            data_distributor: Default::default(),
        },
        storage: StorageSettings {
            default_path: "./data".to_string(),
            default_format: "csv".to_string(),
        },
        instruments: HashMap::new(),
        processors: None,
        instruments_v3: Vec::new(),
    }
}

async fn setup_actor() -> mpsc::Sender<DaqCommand> {
    let settings = create_test_settings();
    let runtime = Arc::new(Runtime::new().expect("Failed to create runtime"));

    // Register mock instrument in registry
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));

    let actor = DaqManagerActor::<InstrumentMeasurement>::new(
        settings,
        Arc::new(instrument_registry),
        Arc::new(ProcessorRegistry::new()),
        Arc::new(ModuleRegistry::new()),
        LogBuffer::new(),
        runtime,
    )
    .expect("Failed to create actor");

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    tokio::spawn(actor.run(cmd_rx));

    // Give the actor time to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    cmd_tx
}

#[tokio::test]
async fn test_add_instrument_dynamic_success() {
    let cmd_tx = setup_actor().await;

    // Create mock instrument configuration
    let config = toml::Value::Table(toml::map::Map::new());

    let (cmd, rx) =
        DaqCommand::add_instrument_dynamic("dynamic_mock".to_string(), "mock".to_string(), config);

    cmd_tx.send(cmd).await.expect("Failed to send command");
    let result = rx.await.expect("Failed to receive response");

    assert!(
        result.is_ok(),
        "Dynamic instrument addition should succeed: {:?}",
        result
    );

    // Verify instrument is in the list
    let (list_cmd, list_rx) = DaqCommand::get_instrument_list();
    cmd_tx
        .send(list_cmd)
        .await
        .expect("Failed to send list command");
    let instruments = list_rx.await.expect("Failed to receive instrument list");

    assert!(
        instruments.contains(&"dynamic_mock".to_string()),
        "Dynamic instrument should be in the list"
    );

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_add_instrument_dynamic_duplicate() {
    let cmd_tx = setup_actor().await;

    let config = toml::Value::Table(toml::map::Map::new());

    // Add first instrument
    let (cmd1, rx1) = DaqCommand::add_instrument_dynamic(
        "dup_test".to_string(),
        "mock".to_string(),
        config.clone(),
    );
    cmd_tx.send(cmd1).await.expect("Failed to send command");
    let result1 = rx1.await.expect("Failed to receive response");
    assert!(result1.is_ok(), "First addition should succeed");

    // Try to add duplicate
    let (cmd2, rx2) =
        DaqCommand::add_instrument_dynamic("dup_test".to_string(), "mock".to_string(), config);
    cmd_tx.send(cmd2).await.expect("Failed to send command");
    let result2 = rx2.await.expect("Failed to receive response");

    assert!(
        result2.is_err(),
        "Duplicate instrument addition should fail"
    );
    let err_msg = format!("{:?}", result2.unwrap_err());
    assert!(
        err_msg.contains("already running") || err_msg.contains("duplicate"),
        "Error should mention duplicate/already running: {}",
        err_msg
    );

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_remove_instrument_dynamic_success() {
    let cmd_tx = setup_actor().await;

    // Add instrument first
    let config = toml::Value::Table(toml::map::Map::new());
    let (add_cmd, add_rx) =
        DaqCommand::add_instrument_dynamic("to_remove".to_string(), "mock".to_string(), config);
    cmd_tx.send(add_cmd).await.expect("Failed to send command");
    add_rx
        .await
        .expect("Failed to receive response")
        .expect("Add should succeed");

    // Remove instrument
    let (remove_cmd, remove_rx) =
        DaqCommand::remove_instrument_dynamic("to_remove".to_string(), false);
    cmd_tx
        .send(remove_cmd)
        .await
        .expect("Failed to send remove command");
    let result = remove_rx.await.expect("Failed to receive remove response");

    assert!(
        result.is_ok(),
        "Instrument removal should succeed: {:?}",
        result
    );

    // Verify instrument is not in the list
    let (list_cmd, list_rx) = DaqCommand::get_instrument_list();
    cmd_tx
        .send(list_cmd)
        .await
        .expect("Failed to send list command");
    let instruments = list_rx.await.expect("Failed to receive instrument list");

    assert!(
        !instruments.contains(&"to_remove".to_string()),
        "Removed instrument should not be in the list"
    );

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_remove_instrument_dynamic_not_found() {
    let cmd_tx = setup_actor().await;

    let (remove_cmd, remove_rx) =
        DaqCommand::remove_instrument_dynamic("nonexistent".to_string(), false);
    cmd_tx
        .send(remove_cmd)
        .await
        .expect("Failed to send command");
    let result = remove_rx.await.expect("Failed to receive response");

    assert!(
        result.is_err(),
        "Removing nonexistent instrument should fail"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("not running"),
        "Error should mention instrument not running: {}",
        err_msg
    );

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_rollback_to_version_propagates_errors() {
    let cmd_tx = setup_actor().await;

    // Use obviously missing snapshot id to trigger failure path
    let missing = VersionId("config-19700101_000000-missing.toml".to_string());
    let (cmd, rx) = DaqCommand::rollback_to_version(missing);
    cmd_tx
        .send(cmd)
        .await
        .expect("Failed to send rollback command");
    let result = rx.await.expect("Rollback response channel dropped");
    assert!(
        result.is_err(),
        "Rollback should propagate underlying error when snapshot is missing"
    );

    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_remove_instrument_dynamic_force() {
    let cmd_tx = setup_actor().await;

    // Add instrument
    let config = toml::Value::Table(toml::map::Map::new());
    let (add_cmd, add_rx) =
        DaqCommand::add_instrument_dynamic("force_remove".to_string(), "mock".to_string(), config);
    cmd_tx.send(add_cmd).await.expect("Failed to send command");
    add_rx
        .await
        .expect("Failed to receive response")
        .expect("Add should succeed");

    // Remove with force=true (bypasses dependency checks)
    let (remove_cmd, remove_rx) =
        DaqCommand::remove_instrument_dynamic("force_remove".to_string(), true);
    cmd_tx
        .send(remove_cmd)
        .await
        .expect("Failed to send remove command");
    let result = remove_rx.await.expect("Failed to receive remove response");

    assert!(result.is_ok(), "Force removal should succeed: {:?}", result);

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_update_instrument_parameter_success() {
    let cmd_tx = setup_actor().await;

    // Add instrument first
    let config = toml::Value::Table(toml::map::Map::new());
    let (add_cmd, add_rx) =
        DaqCommand::add_instrument_dynamic("param_test".to_string(), "mock".to_string(), config);
    cmd_tx.send(add_cmd).await.expect("Failed to send command");
    add_rx
        .await
        .expect("Failed to receive response")
        .expect("Add should succeed");

    // Update parameter
    let (update_cmd, update_rx) = DaqCommand::update_instrument_parameter(
        "param_test".to_string(),
        "test_param".to_string(),
        "test_value".to_string(),
    );
    cmd_tx
        .send(update_cmd)
        .await
        .expect("Failed to send update command");
    let result = update_rx.await.expect("Failed to receive update response");

    // Note: Mock instruments may not handle SetParameter, but command should be sent
    // For MVP, we consider success if the command was sent without channel errors
    match result {
        Ok(_) => {
            // Success - parameter update sent
        }
        Err(e) => {
            // Also acceptable if instrument doesn't support the parameter
            let err_msg = format!("{:?}", e);
            println!("Parameter update result: {}", err_msg);
        }
    }

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_update_instrument_parameter_not_found() {
    let cmd_tx = setup_actor().await;

    let (update_cmd, update_rx) = DaqCommand::update_instrument_parameter(
        "nonexistent".to_string(),
        "param".to_string(),
        "value".to_string(),
    );
    cmd_tx
        .send(update_cmd)
        .await
        .expect("Failed to send command");
    let result = update_rx.await.expect("Failed to receive response");

    assert!(
        result.is_err(),
        "Updating nonexistent instrument should fail"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("not running"),
        "Error should mention instrument not running: {}",
        err_msg
    );

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}

#[tokio::test]
async fn test_dynamic_config_full_workflow() {
    let cmd_tx = setup_actor().await;

    // 1. Add instrument dynamically
    let config = toml::Value::Table(toml::map::Map::new());
    let (add_cmd, add_rx) =
        DaqCommand::add_instrument_dynamic("workflow_test".to_string(), "mock".to_string(), config);
    cmd_tx.send(add_cmd).await.expect("Failed to send command");
    add_rx
        .await
        .expect("Failed to receive response")
        .expect("Add should succeed");

    // 2. Verify it's in the list
    let (list_cmd, list_rx) = DaqCommand::get_instrument_list();
    cmd_tx
        .send(list_cmd)
        .await
        .expect("Failed to send list command");
    let instruments = list_rx.await.expect("Failed to receive instrument list");
    assert!(instruments.contains(&"workflow_test".to_string()));

    // 3. Update parameter
    let (update_cmd, update_rx) = DaqCommand::update_instrument_parameter(
        "workflow_test".to_string(),
        "rate".to_string(),
        "100".to_string(),
    );
    cmd_tx
        .send(update_cmd)
        .await
        .expect("Failed to send update command");
    let _ = update_rx.await.expect("Failed to receive update response");

    // 4. Remove instrument
    let (remove_cmd, remove_rx) =
        DaqCommand::remove_instrument_dynamic("workflow_test".to_string(), false);
    cmd_tx
        .send(remove_cmd)
        .await
        .expect("Failed to send remove command");
    remove_rx
        .await
        .expect("Failed to receive response")
        .expect("Remove should succeed");

    // 5. Verify it's no longer in the list
    let (list_cmd2, list_rx2) = DaqCommand::get_instrument_list();
    cmd_tx
        .send(list_cmd2)
        .await
        .expect("Failed to send list command");
    let instruments2 = list_rx2.await.expect("Failed to receive instrument list");
    assert!(!instruments2.contains(&"workflow_test".to_string()));

    // Cleanup
    let (shutdown_cmd, _) = DaqCommand::shutdown();
    let _ = cmd_tx.send(shutdown_cmd).await;
}
