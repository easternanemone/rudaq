//! Integration test for V2InstrumentAdapter (bd-49 Phase 1 validation)
//!
//! This test proves that V2 instruments can be wrapped with V2InstrumentAdapter
//! and work seamlessly in the existing V1 architecture.
//!
//! ## Known Limitations
//!
//! V1's InstrumentCommand enum doesn't include StartAcquisition, which means
//! V2 instruments wrapped in the adapter cannot be triggered to start streaming
//! via V1's command interface. This limitation will be resolved when app.rs is
//! migrated to native V2 architecture in Phase 3 of bd-49.

use rust_daq::{
    config::Settings,
    core::{Instrument, V2InstrumentAdapter},
    instruments_v2::mock_instrument::MockInstrumentV2,
};
use std::sync::Arc;

#[tokio::test]
async fn test_v2_adapter_basic_lifecycle() {
    // Create V2 instrument wrapped in adapter
    let v2_instrument = Box::new(MockInstrumentV2::new("test_v2_mock".to_string()));
    let mut adapter = V2InstrumentAdapter::new(v2_instrument);

    // Verify initial state
    assert_eq!(adapter.name(), "test_v2_mock");

    // Connect (calls V2 initialize())
    let settings = Arc::new(Settings::new(None).unwrap());
    let connect_result = adapter.connect("test_v2_mock", &settings).await;
    assert!(
        connect_result.is_ok(),
        "Adapter should connect successfully"
    );

    // Verify data stream can be subscribed to
    let data_stream_result = adapter.data_stream().await;
    assert!(
        data_stream_result.is_ok(),
        "Should be able to subscribe to data stream"
    );

    // Clean shutdown
    let disconnect_result = adapter.disconnect().await;
    assert!(
        disconnect_result.is_ok(),
        "Adapter should disconnect cleanly"
    );
}

#[tokio::test]
async fn test_v2_adapter_command_translation() {
    use rust_daq::core::InstrumentCommand;

    // Create V2 instrument wrapped in adapter
    let v2_instrument = Box::new(MockInstrumentV2::new("test_cmd".to_string()));
    let mut adapter = V2InstrumentAdapter::new(v2_instrument);

    let settings = Arc::new(Settings::new(None).unwrap());
    adapter.connect("test_cmd", &settings).await.unwrap();

    // Test SetParameter command translation (V1 → V2)
    let result = adapter
        .handle_command(InstrumentCommand::SetParameter(
            "exposure_ms".to_string(),
            "200.0".to_string(),
        ))
        .await;

    // V2 adapter should translate V1 string values to V2 JSON values
    assert!(
        result.is_ok(),
        "SetParameter should translate V1→V2 successfully: {:?}",
        result.err()
    );

    adapter.disconnect().await.unwrap();
}

#[tokio::test]
async fn test_v2_adapter_multiple_connections() {
    // Test that adapter can be connected/disconnected multiple times
    let v2_instrument = Box::new(MockInstrumentV2::new("test_reconnect".to_string()));
    let mut adapter = V2InstrumentAdapter::new(v2_instrument);

    let settings = Arc::new(Settings::new(None).unwrap());

    // First connection
    adapter.connect("test_reconnect", &settings).await.unwrap();
    adapter.disconnect().await.unwrap();

    // Second connection (tests state machine recovery)
    let reconnect_result = adapter.connect("test_reconnect", &settings).await;
    assert!(
        reconnect_result.is_ok(),
        "Adapter should support reconnection"
    );

    adapter.disconnect().await.unwrap();
}

#[tokio::test]
async fn test_v2_adapter_data_flow_end_to_end() {
    use daq_core::Instrument as V2Instrument;
    use rust_daq::instruments_v2::mock_instrument::MockInstrumentV2;
    use std::time::Duration;
    use tokio::time::timeout;

    // Create V2 instrument with direct access (not wrapped yet)
    let mut v2_mock = MockInstrumentV2::new("test_data_flow".to_string());

    // Initialize the V2 instrument
    v2_mock.initialize().await.unwrap();

    // Subscribe to V2 measurement stream
    let mut v2_rx = v2_mock.measurement_stream();

    // Start acquisition using V2 interface (V1 doesn't have this command)
    use daq_core::InstrumentCommand;
    v2_mock
        .handle_command(InstrumentCommand::StartAcquisition)
        .await
        .unwrap();

    // Verify V2 instrument produces data
    let v2_result = timeout(Duration::from_secs(2), v2_rx.recv()).await;
    assert!(
        v2_result.is_ok(),
        "V2 instrument should produce measurements"
    );

    let measurement = v2_result.unwrap().unwrap();

    // MockInstrumentV2 produces Image data during live acquisition
    match measurement.as_ref() {
        daq_core::Measurement::Image(img) => {
            assert_eq!(img.channel, "test_data_flow_image");
            assert!(
                img.width > 0 && img.height > 0,
                "Image should have dimensions"
            );
            assert_eq!(img.pixels.len(), (img.width * img.height) as usize);
        }
        _ => panic!("Expected Image measurement from MockInstrumentV2::start_live()"),
    }

    // Clean shutdown
    v2_mock.shutdown().await.unwrap();

    // Note: This test proves V2 measurements are correctly produced and typed.
    // The adapter will log and drop Image/Spectrum data (V1 limitation).
    // Full scalar data conversion testing requires instruments that produce Scalar measurements.
}
