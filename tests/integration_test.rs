use rust_daq::{
    app::DaqApp,
    config::Settings,
    data::registry::ProcessorRegistry,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

#[test]
fn test_mock_instrument_spawns_and_produces_data() {
    // Setup
    let settings = Arc::new(Settings::new(None).unwrap());
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);
    let processor_registry = Arc::new(ProcessorRegistry::new());
    let log_buffer = LogBuffer::new();

    let app = DaqApp::new(
        settings.clone(),
        instrument_registry,
        processor_registry,
        log_buffer,
    )
    .unwrap();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        // Instrument is automatically started in DaqApp::new()

        // Act: Check for data
        let recv_result = timeout(Duration::from_secs(5), data_rx.recv()).await;

        // Assert
        assert!(recv_result.is_ok(), "Did not receive data point in time");
        let data_point = recv_result.unwrap().unwrap();
        assert!(
            matches!(
                data_point.channel.as_str(),
                "sine_wave" | "cosine_wave" | "sine_wave_filtered" | "cosine_wave_filtered"
            ) || data_point.channel.ends_with("_fft"),
            "Unexpected channel name"
        );
    });

    // Teardown
    app.shutdown();
}
