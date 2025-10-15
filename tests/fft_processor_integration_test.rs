use chrono::{TimeZone, Utc};
use rust_daq::{
    app::DaqApp,
    config::Settings,
    core::{DataPoint, DataProcessor},
    data::{
        fft::{FFTConfig, FFTProcessor},
        registry::ProcessorRegistry,
    },
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::LogBuffer,
};
use serial_test::serial;
use std::sync::Arc;

#[test]
#[serial]
fn test_fft_processor_in_pipeline() {
    // --- Setup ---
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

        // Create a new pipeline with the FFTProcessor
        let sampling_rate = 1024.0;
        let window_size = 1024;
        let config = FFTConfig {
            window_size,
            overlap: window_size / 2,
            sampling_rate,
        };
        let mut fft_processor = FFTProcessor::new(config);

        // Collect some data points
        let mut collected_data = Vec::new();
        for _ in 0..window_size {
            if let Ok(dp) = data_rx.recv().await {
                collected_data.push(dp);
            }
        }

        // Process the data
        let spectrum = fft_processor.process(&collected_data);
        assert!(
            !spectrum.is_empty(),
            "FFT processor did not produce any output."
        );
    });

    // Teardown
    app.shutdown();
}

#[test]
#[serial]
fn test_fft_processor_sine_wave() {
    let sample_rate = 1024.0;
    let window_size = 1024;
    let frequency = 50.0;
    let config = FFTConfig {
        window_size,
        overlap: window_size / 2,
        sampling_rate: sample_rate,
    };
    let mut fft_processor = FFTProcessor::new(config);

    // Generate a sine wave
    let mut sine_wave = Vec::new();
    for i in 0..window_size {
        let t = i as f64 / sample_rate;
        let value = (2.0 * std::f64::consts::PI * frequency * t).sin();
        sine_wave.push(DataPoint {
            timestamp: Utc.timestamp_nanos((t * 1_000_000_000.0) as i64),
            channel: "test".to_string(),
            value,
            unit: "V".to_string(),
            metadata: None,
        });
    }

    let spectrum = fft_processor.process_fft(&sine_wave);
    assert!(
        !spectrum.is_empty(),
        "FFT processor did not produce any output."
    );

    let peak_freq = spectrum
        .iter()
        .max_by(|a, b| a.magnitude.partial_cmp(&b.magnitude).unwrap())
        .map(|bin| bin.frequency)
        .unwrap();

    let freq_resolution = sample_rate / window_size as f64;
    assert!(
        (peak_freq - frequency).abs() <= freq_resolution,
        "Peak frequency {:.2} Hz is not close to the expected frequency {:.2} Hz. Resolution: {:.2} Hz",
        peak_freq,
        frequency,
        freq_resolution
    );
}
