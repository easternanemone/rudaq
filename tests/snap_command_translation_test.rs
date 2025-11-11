use std::sync::Arc;
use tokio::runtime::Runtime;

use rust_daq::{
    app_actor::DaqManagerActor,
    config::{ApplicationSettings, Settings, StorageSettings, TimeoutSettings},
    instrument::{InstrumentRegistry, InstrumentRegistryV2},
    instruments_v2::mock_instrument::MockInstrumentV2,
    measurement::Measure,
    messages::DaqCommand,
};

#[tokio::test]
async fn test_snap_command_translation_fails() {
    let settings = Settings {
        log_level: "info".to_string(),
        application: ApplicationSettings {
            broadcast_channel_capacity: 1024,
            command_channel_capacity: 128,
            data_distributor: Default::default(),
            timeouts: TimeoutSettings::default(),
        },
        storage: StorageSettings {
            default_path: "./data".to_string(),
            default_format: "csv".to_string(),
        },
        instruments: std::collections::HashMap::new(),
        processors: None,
        instruments_v3: Vec::new(),
    };

    let runtime = Arc::new(Runtime::new().unwrap());
    let mut instrument_registry_v2 = InstrumentRegistryV2::new();

    // The mock instrument needs a way to communicate back what command it received.
    // We'll use a channel for this.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    instrument_registry_v2.register("mock_v2_snap_test", move |id| {
        let mut inst = MockInstrumentV2::new(id.to_string());
        // This is a bit of a hack for testing, but we are replacing the handle_command
        // with a version that sends the received command to our channel.
        // A better solution would be to enhance the mock instrument for testability.
        let tx = tx.clone();
        Box::pin(async move {
            let original_handle_command = inst.handle_command_mut();
            move |cmd| {
                let tx = tx.clone();
                async move {
                    tx.send(cmd.clone()).unwrap();
                    (original_handle_command)(cmd).await
                }
            }
        })
    });
    let instrument_registry_v2 = Arc::new(instrument_registry_v2);

    let (command_tx, command_rx) = tokio::sync::mpsc::channel(32);
    let actor = DaqManagerActor::<rust_daq::measurement::InstrumentMeasurement>::new(
        settings.clone(),
        Arc::new(InstrumentRegistry::new()),
        instrument_registry_v2,
        Arc::new(rust_daq::data::registry::ProcessorRegistry::new()),
        Arc::new(rust_daq::modules::ModuleRegistry::new()),
        runtime.clone(),
    )
    .unwrap();

    runtime.spawn(actor.run(command_rx));

    // Spawn the instrument
    let (spawn_cmd, spawn_rx) = DaqCommand::spawn_instrument("test_snap".to_string());
    command_tx.send(spawn_cmd).await.unwrap();
    spawn_rx.await.unwrap().unwrap();

    // Send the snap command
    let snap_command = rust_daq::core::InstrumentCommand::Execute("snap".to_string(), vec![]);
    let (send_cmd, send_rx) =
        DaqCommand::send_instrument_command("test_snap".to_string(), snap_command);
    command_tx.send(send_cmd).await.unwrap();
    send_rx.await.unwrap().unwrap();

    // Check what command the mock instrument received
    let received_cmd = rx.recv().await;

    // This assertion should fail, because the "snap" command is currently ignored.
    // We expect to receive nothing, or at least not SnapFrame.
    assert!(
        matches!(received_cmd, Some(daq_core::InstrumentCommand::SnapFrame)),
        "Expected SnapFrame command, but received: {:?}",
        received_cmd
    );
}
