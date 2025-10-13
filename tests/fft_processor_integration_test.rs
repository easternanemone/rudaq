use rust_daq::{
    app::DaqApp,
    config::Settings,
    core::DataProcessor,
    data::fft::FFTProcessor,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// This test verifies that the FFTProcessor correctly identifies a sine wave frequency
/// when placed in a data processing pipeline.
#[test]
fn test_fft_processor_in_pipeline() {
    // --- Setup ---
    let _ = env_logger::builder().is_test(true).try_init();
    let settings = Arc::new(Settings::new().unwrap());
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", || Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);
    let log_buffer = LogBuffer::new();

    let app = DaqApp::new(settings.clone(), instrument_registry, log_buffer).unwrap();
    let runtime = app.get_runtime();

    runtime.block_on(async {
        let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

        // Create a new pipeline with the FFTProcessor
        let sampling_rate = settings.instruments.get("mock").unwrap().get("sample_rate_hz").unwrap().as_float().unwrap();
        let window_size = 1024;
        let mut fft_processor = FFTProcessor::new(window_size, window_size / 2, sampling_rate);

        // --- Act ---
        // Spawn the instrument, which starts data generation
        app.with_inner(|inner| {
            inner.spawn_instrument("mock").unwrap();
        });

        let collection_duration = Duration::from_secs(5);
        let mut spectrum_points = Vec::new();

        let collection_future = async {
            loop {
                match data_rx.recv().await {
                    Ok(data_point) => {
                        if data_point.channel == "sine_wave" {
                            let processed = fft_processor.process(&[data_point]);
                            if !processed.is_empty() {
                                spectrum_points = processed;
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        // Channel closed, which is expected after the mock instrument finishes
                        break;
                    }
                }
            }
        };

        let res = timeout(collection_duration, collection_future).await;
        assert!(res.is_ok(), "Test timed out before producing a spectrum");

        // --- Assert ---
        assert!(!spectrum_points.is_empty(), "FFT processor did not produce any output.");

        // Find the frequency with the highest magnitude
        let mut peak_freq = 0.0;
        let mut max_mag = -f64::INFINITY;

        for dp in spectrum_points {
            if dp.value > max_mag {
                max_mag = dp.value;
                let secs = dp.timestamp.timestamp();
                let nsecs = dp.timestamp.timestamp_subsec_nanos();
                peak_freq = secs as f64 + (nsecs as f64 / 1_000_000_000.0);
            }
        }

        // The mock instrument's sine wave frequency is approximately 15.9 Hz.
        let expected_freq = (0.1 * sampling_rate) / (2.0 * std::f64::consts::PI);
        let freq_resolution = sampling_rate / window_size as f64;

        assert!(
            (peak_freq - expected_freq).abs() <= freq_resolution,
            "Peak frequency {:.2} Hz is not close to the expected frequency {:.2} Hz. Resolution: {:.2} Hz",
            peak_freq,
            expected_freq,
            freq_resolution
        );
    });

    // --- Teardown ---
    app.shutdown();
}
