//! Integration test demonstrating the Measurement enum eliminating FFT JSON workarounds

use chrono::{TimeZone, Utc};
use rust_daq::{
    core::{
        DataPoint, DataProcessor, FrequencyBin, Measurement, MeasurementProcessor, SpectrumData,
    },
    data::fft::{FFTConfig, FFTProcessor},
};

#[test]
fn test_old_vs_new_fft_approach() {
    // Create a test signal - 50 Hz sine wave
    let config = FFTConfig {
        window_size: 64,
        overlap: 32,
        sampling_rate: 512.0,
    };

    let mut old_fft = FFTProcessor::new(config.clone());
    let mut new_fft = FFTProcessor::new(config);

    // Generate test data
    let mut data_points = Vec::new();
    let mut measurements = Vec::new();

    for i in 0..128 {
        let t = i as f64 / 512.0;
        let value = (2.0 * std::f64::consts::PI * 50.0 * t).sin(); // 50 Hz signal

        let dp = DataPoint {
            timestamp: Utc.timestamp_nanos((t * 1_000_000_000.0) as i64),
            instrument_id: "test_instrument".to_string(),
            channel: "signal".to_string(),
            value,
            unit: "V".to_string(),
            metadata: None,
        };

        data_points.push(dp.clone());
        measurements.push(Measurement::Scalar(dp));
    }

    // OLD APPROACH: DataProcessor with JSON metadata workarounds
    let old_results = old_fft.process(&data_points);

    // NEW APPROACH: MeasurementProcessor with typed spectrum data
    let new_results = new_fft.process_measurements(&measurements);

    // Verify old approach produces multiple DataPoints with JSON metadata
    assert!(!old_results.is_empty());
    let first_old_result = &old_results[0];
    assert_eq!(first_old_result.channel, "signal_fft");
    assert_eq!(first_old_result.unit, "dB");

    // OLD: Frequency data hidden in JSON metadata - hard to access!
    let old_metadata = first_old_result.metadata.as_ref().unwrap();
    let old_frequency = old_metadata["frequency_hz"].as_f64().unwrap();
    let old_magnitude = old_metadata["magnitude_db"].as_f64().unwrap();
    assert_eq!(old_frequency, 0.0); // DC bin
    assert_eq!(old_magnitude, first_old_result.value); // Redundant!

    // Verify new approach produces a single spectrum with structured data
    assert_eq!(new_results.len(), 1);
    match &new_results[0] {
        Measurement::Spectrum(spectrum) => {
            assert_eq!(spectrum.channel, "signal_fft");
            assert_eq!(spectrum.unit, "dB");
            assert!(!spectrum.bins.is_empty());

            // NEW: Frequency data properly typed and accessible!
            let dc_bin = &spectrum.bins[0];
            assert_eq!(dc_bin.frequency, 0.0);

            // Find the 50 Hz peak
            let peak_bin = spectrum
                .bins
                .iter()
                .find(|bin| (bin.frequency - 50.0).abs() < 5.0) // Within 5 Hz
                .expect("Should find 50 Hz peak");

            // The 50 Hz bin should have higher magnitude than DC
            assert!(peak_bin.magnitude > dc_bin.magnitude + 10.0); // At least 10 dB higher

            // Metadata contains processing parameters, not frequency data
            let metadata = spectrum.metadata.as_ref().unwrap();
            assert_eq!(metadata["window_size"], 64);
            assert_eq!(metadata["sampling_rate"], 512.0);
            assert!(metadata.get("frequency_hz").is_none()); // No JSON workaround needed!
        }
        _ => panic!("Expected Spectrum measurement"),
    }

    println!(
        "✅ OLD approach: {} DataPoints with JSON metadata workarounds",
        old_results.len()
    );
    println!(
        "✅ NEW approach: {} Spectrum measurement with typed frequency bins",
        new_results.len()
    );

    // Demonstrate type safety advantage
    if let Measurement::Spectrum(spectrum) = &new_results[0] {
        let total_power: f64 = spectrum
            .bins
            .iter()
            .map(|bin| 10_f64.powf(bin.magnitude / 10.0)) // Convert dB to linear
            .sum();
        println!(
            "✅ Type-safe power calculation: {:.2} (no JSON parsing!)",
            total_power
        );
    }
}

#[test]
fn test_measurement_enum_common_operations() {
    let timestamp = Utc::now();

    // Test scalar measurement
    let scalar = Measurement::Scalar(DataPoint {
        timestamp,
        instrument_id: "test_instrument".to_string(),
        channel: "temperature".to_string(),
        value: 23.5,
        unit: "°C".to_string(),
        metadata: None,
    });

    // Test spectrum measurement
    let spectrum = Measurement::Spectrum(SpectrumData {
        timestamp,
        channel: "audio_fft".to_string(),
        unit: "dB".to_string(),
        bins: vec![
            FrequencyBin {
                frequency: 0.0,
                magnitude: -60.0,
            },
            FrequencyBin {
                frequency: 1000.0,
                magnitude: -20.0,
            },
            FrequencyBin {
                frequency: 2000.0,
                magnitude: -40.0,
            },
        ],
        metadata: None,
    });

    // Test common operations work across all measurement types
    assert_eq!(scalar.channel(), "temperature");
    assert_eq!(scalar.unit(), "°C");
    assert_eq!(scalar.timestamp(), timestamp);

    assert_eq!(spectrum.channel(), "audio_fft");
    assert_eq!(spectrum.unit(), "dB");
    assert_eq!(spectrum.timestamp(), timestamp);

    // Test pattern matching for type-specific operations
    match &spectrum {
        Measurement::Spectrum(data) => {
            let peak_bin = data
                .bins
                .iter()
                .max_by(|a, b| a.magnitude.partial_cmp(&b.magnitude).unwrap())
                .unwrap();
            assert_eq!(peak_bin.frequency, 1000.0);
            assert_eq!(peak_bin.magnitude, -20.0);
        }
        _ => panic!("Expected spectrum"),
    }
}
