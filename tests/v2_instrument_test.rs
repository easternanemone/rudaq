
use rust_daq::{
    app_actor::DaqManagerActor,
    config::Settings,
    instrument::{InstrumentRegistry, InstrumentRegistryV2},
    measurement::InstrumentMeasurement,
    messages::DaqCommand,
};
use std::sync::Arc;
use tokio::runtime::Runtime;

#[test]
fn test_v2_snap_command() {
    let runtime = Arc::new(Runtime::new().unwrap());
    let settings = Settings::new(None).unwrap();
    let instrument_registry = Arc::new(InstrumentRegistry::<InstrumentMeasurement>::new());
    let mut instrument_registry_v2 = InstrumentRegistryV2::new();
    instrument_registry_v2.register("mock_camera_v3", |id| {
        Box::pin(rust_daq::instrument::mock_v3::MockCameraV3::new(id.to_string()))
    });
    let instrument_registry_v2 = Arc::new(instrument_registry_v2);
    let processor_registry = Arc::new(rust_daq::data::registry::ProcessorRegistry::new());
    let module_registry = Arc::new(rust_daq::modules::ModuleRegistry::new());

    let actor = DaqManagerActor::<InstrumentMeasurement>::new(
        settings.clone(),
        instrument_registry,
        instrument_registry_v2,
        processor_registry,
        module_registry,
        runtime.clone(),
    )
    .unwrap();

    let (command_tx, command_rx) = tokio::sync::mpsc::channel(32);
    runtime.spawn(async move {
        actor.run(command_rx).await;
    });

    runtime.block_on(async move {
        // Corrected instrument ID and added a small delay to ensure the instrument is ready
        let (cmd, rx) = DaqCommand::spawn_instrument("mock_camera_v3".to_string());
        command_tx.send(cmd).await.unwrap();
        rx.await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let (cmd, mut data_rx) = DaqCommand::subscribe_to_data();
        command_tx.send(cmd).await.unwrap();
        let mut data_rx = data_rx.await.unwrap();

        let (cmd, rx) = DaqCommand::send_instrument_command(
            "mock_camera_v3".to_string(),
            rust_daq::core::InstrumentCommand::Execute("snap".to_string(), vec![]),
        );
        command_tx.send(cmd).await.unwrap();
        rx.await.unwrap().unwrap();

        let measurement = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            data_rx.recv(),
        )
        .await
        .unwrap();

        assert!(measurement.is_some());
    });
}
