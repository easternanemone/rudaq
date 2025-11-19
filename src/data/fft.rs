//! An FFT (Fast Fourier Transform) data processor for frequency analysis.

use crate::core::{DataPoint, DataProcessor, FrequencyBin, MeasurementProcessor, SpectrumData};
use crate::core_v3::Measurement;
use chrono::Utc;
use log::debug;
use num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde::Deserialize;
use std::collections::VecDeque;
use std::sync::Arc;

// FrequencyBin is now defined in core.rs

/// Configuration for the FFTProcessor.
#[derive(Clone, Debug, Deserialize)]
pub struct FFTConfig {
    pub window_size: usize,
    pub overlap: usize,
    pub sampling_rate: f64,
}

/// A data processor that performs a Fast Fourier Transform (FFT) on a sliding window of data.
///
/// This processor collects time-domain samples into a buffer. When the buffer is full,
/// it applies a Hann window to the samples, performs an FFT, and converts the output
/// to a frequency spectrum.
///
/// The output `DataPoint`s represent the frequency spectrum:
/// - `timestamp`: Encodes the frequency of the bin. This is a workaround to fit into the `DataPoint` struct.
///   The frequency `f` (in Hz) is encoded as a `DateTime` representing `UNIX_EPOCH + f seconds`.
/// - `value`: The magnitude of the frequency bin in decibels (dB).
/// - `unit`: "dB".
/// - `channel`: The channel of the input data.
///
/// # Example
///
/// ```
/// use rust_daq::core::{DataPoint, DataProcessor};
/// use rust_daq::data::fft::{FFTConfig, FFTProcessor};
/// use chrono::{Utc, TimeZone};
/// use std::collections::HashMap;
///
/// // This is a conceptual example. In a real application, you would get DataPoints from an instrument.
/// fn conceptual_example() {
///     let config = FFTConfig {
///         window_size: 1024,
///         overlap: 512,
///         sampling_rate: 1024.0,
///     };
///     let mut fft_processor = FFTProcessor::new(config.clone());
///
///     // Generate a sine wave for testing
///     let frequency = 50.0;
///     let mut sine_wave = Vec::new();
///     for i in 0..2048 {
///         let t = i as f64 / config.sampling_rate;
///         let value = (2.0 * std::f64::consts::PI * frequency * t).sin();
///         sine_wave.push(DataPoint {
///             timestamp: Utc.timestamp_nanos((t * 1_000_000_000.0) as i64),
///             instrument_id: "test_instrument".to_string(),
///             channel: "test".to_string(),
///             value,
///             unit: "V".to_string(),
///             metadata: None,
///         });
///     }
///
///     let spectrum = fft_processor.process(&sine_wave);
///     // The `spectrum` will contain `DataPoint`s representing the frequency spectrum.
///     // There should be a peak around 50 Hz.
/// }
/// ```
#[derive(Clone)]
pub struct FFTProcessor {
    window_size: usize,
    overlap: usize,
    sampling_rate: f64,
    buffer: VecDeque<f64>,
    fft_planner: Arc<dyn Fft<f64>>,
    hann_window: Vec<f64>,
    channel: String,
    instrument_id: String,
}

impl FFTProcessor {
    /// Creates a new `FFTProcessor`.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration for the FFT processor.
    pub fn new(config: FFTConfig) -> Self {
        assert!(
            config.overlap < config.window_size,
            "Overlap must be less than window size"
        );
        assert!(config.sampling_rate > 0.0, "Sampling rate must be positive");

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(config.window_size);

        let mut hann_window = Vec::with_capacity(config.window_size);
        if config.window_size > 1 {
            for i in 0..config.window_size {
                // Hann window formula
                let val = 0.5
                    * (1.0
                        - (2.0 * std::f64::consts::PI * i as f64
                            / (config.window_size - 1) as f64)
                            .cos());
                hann_window.push(val);
            }
        }

        Self {
            window_size: config.window_size,
            overlap: config.overlap,
            sampling_rate: config.sampling_rate,
            buffer: VecDeque::with_capacity(config.window_size * 2),
            fft_planner: fft,
            hann_window,
            channel: String::from("unknown"),
            instrument_id: String::from("unknown"),
        }
    }

    /// Processes a slice of `DataPoint`s, performing an FFT when enough data is available.
    pub fn process_fft(&mut self, data: &[DataPoint]) -> Vec<FrequencyBin> {
        if data.is_empty() {
            return vec![];
        }

        // Update channel and instrument_id from the first data point
        if self.channel == "unknown" {
            self.channel = data[0].channel.clone();
            self.instrument_id = data[0].instrument_id.clone();
        }

        self.buffer.extend(data.iter().map(|dp| dp.value));
        debug!("Buffer size: {}", self.buffer.len());

        let mut all_fft_results = Vec::new();
        let step_size = self.window_size - self.overlap;

        while self.buffer.len() >= self.window_size {
            debug!("Processing window. Buffer size: {}", self.buffer.len());

            let mut complex_buffer: Vec<Complex<f64>> = self
                .buffer
                .iter()
                .take(self.window_size)
                .zip(self.hann_window.iter())
                .map(|(&val, &win_val)| Complex::new(val * win_val, 0.0))
                .collect();

            self.fft_planner.process(&mut complex_buffer);

            let freq_resolution = self.sampling_rate / self.window_size as f64;
            let num_bins = self.window_size / 2;

            let mut fft_bins = Vec::with_capacity(num_bins);

            if num_bins > 0 {
                let magnitude = complex_buffer[0].norm() / self.window_size as f64;
                let magnitude_db = if magnitude > 1e-6 {
                    20.0 * magnitude.log10()
                } else {
                    -120.0
                };
                fft_bins.push(FrequencyBin {
                    frequency: 0.0,
                    magnitude: magnitude_db,
                });
            }

            for (i, complex_val) in complex_buffer.iter().enumerate().take(num_bins).skip(1) {
                let magnitude = (complex_val.norm() * 2.0) / self.window_size as f64;
                let magnitude_db = if magnitude > 1e-6 {
                    20.0 * magnitude.log10()
                } else {
                    -120.0
                };

                let frequency = i as f64 * freq_resolution;

                fft_bins.push(FrequencyBin {
                    frequency,
                    magnitude: magnitude_db,
                });
            }

            all_fft_results.extend(fft_bins);
            self.buffer.drain(0..step_size);
            debug!("Drained buffer. New size: {}", self.buffer.len());
        }

        all_fft_results
    }
}

impl DataProcessor for FFTProcessor {
    /// Processes a slice of `DataPoint`s, performing an FFT when enough data is available.
    fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> {
        let fft_bins = self.process_fft(data);
        let timestamp = data.last().map_or_else(Utc::now, |dp| dp.timestamp);

        fft_bins
            .into_iter()
            .map(|bin| {
                let metadata = serde_json::json!({
                    "frequency_hz": bin.frequency,
                    "magnitude_db": bin.magnitude,
                });

                DataPoint {
                    timestamp,
                    instrument_id: self.instrument_id.clone(),
                    channel: format!("{}_fft", self.channel),
                    value: bin.magnitude,
                    unit: "dB".to_string(),
                    metadata: Some(metadata),
                }
            })
            .collect()
    }
}

impl MeasurementProcessor for FFTProcessor {
    /// Processes measurements, converting scalar time-series data to frequency spectra.
    ///
    /// This implementation filters for `Measurement::Scalar` data points, performs FFT
    /// analysis, and returns `Measurement::Spectrum` containing properly typed frequency
    /// bins instead of JSON metadata workarounds.
    fn process_measurements(&mut self, data: &[Arc<Measurement>]) -> Vec<Arc<Measurement>> {
        // Extract scalar data points from Arc<Measurement> and convert to core::DataPoint
        let scalars: Vec<DataPoint> = data
            .iter()
            .filter_map(|m| {
                if let Measurement::Scalar(dp) = m.as_ref() {
                    // Convert daq_core::DataPoint to core::DataPoint
                    Some(DataPoint {
                        timestamp: dp.timestamp,
                        instrument_id: String::new(), // daq_core doesn't have instrument_id
                        channel: dp.channel.clone(),
                        value: dp.value,
                        unit: dp.unit.clone(),
                        metadata: None,
                    })
                } else {
                    None
                }
            })
            .collect();

        if scalars.is_empty() {
            return Vec::new();
        }

        // Use the existing FFT processing logic
        let fft_bins = self.process_fft(&scalars);
        if fft_bins.is_empty() {
            return Vec::new();
        }

        // Update channel from the first data point
        if self.channel == "unknown" {
            self.channel = scalars[0].channel.clone();
        }

        // Create a single spectrum measurement instead of multiple scalar DataPoints
        let spectrum_v1 = SpectrumData {
            timestamp: scalars.last().map_or_else(Utc::now, |dp| dp.timestamp),
            channel: format!("{}_fft", self.channel),
            unit: "dB".to_string(),
            bins: fft_bins.clone(),
            metadata: Some(serde_json::json!({
                "window_size": self.window_size,
                "overlap": self.overlap,
                "sampling_rate": self.sampling_rate,
            })),
        };

        // Convert from core::SpectrumData to daq_core::SpectrumData
        let wavelengths: Vec<f64> = fft_bins.iter().map(|bin| bin.frequency).collect();
        let intensities: Vec<f64> = fft_bins.iter().map(|bin| bin.magnitude).collect();

        let spectrum_v2 = daq_core::SpectrumData {
            timestamp: spectrum_v1.timestamp,
            channel: spectrum_v1.channel,
            wavelengths,
            intensities,
            unit_x: "Hz".to_string(),
            unit_y: spectrum_v1.unit,
            metadata: spectrum_v1.metadata,
        };

        vec![Arc::new(Measurement::Spectrum(spectrum_v2))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_measurement_processor_fft() {
        let config = FFTConfig {
            window_size: 8,
            overlap: 4,
            sampling_rate: 8.0,
        };
        let mut fft_processor = FFTProcessor::new(config);

        // Create test data - a simple sine wave
        let mut measurements = Vec::new();
        for i in 0..16 {
            let t = i as f64 / 8.0;
            let value = (2.0 * std::f64::consts::PI * 1.0 * t).sin(); // 1 Hz sine wave
            measurements.push(Arc::new(Measurement::Scalar(daq_core::DataPoint {
                timestamp: Utc.timestamp_nanos((t * 1_000_000_000.0) as i64),
                channel: "test_signal".to_string(),
                value,
                unit: "V".to_string(),
            })));
        }

        // Process with new MeasurementProcessor interface
        let result = fft_processor.process_measurements(&measurements);

        // Should get spectrum measurements
        assert_eq!(result.len(), 1);
        match result[0].as_ref() {
            Measurement::Spectrum(spectrum) => {
                assert_eq!(spectrum.channel, "test_signal_fft");
                assert_eq!(spectrum.unit_y, "dB");
                assert!(!spectrum.wavelengths.is_empty());
                assert!(!spectrum.intensities.is_empty());

                // Verify frequency bins are properly structured
                assert_eq!(spectrum.wavelengths[0], 0.0); // DC component

                // Should have metadata about FFT parameters
                assert!(spectrum.metadata.is_some());
                let metadata = spectrum.metadata.as_ref().unwrap();
                assert_eq!(metadata["window_size"], 8);
                assert_eq!(metadata["sampling_rate"], 8.0);
            }
            _ => panic!("Expected Spectrum measurement, got {:?}", result[0]),
        }
    }

    #[test]
    fn test_measurement_processor_filters_non_scalar() {
        let config = FFTConfig {
            window_size: 4,
            overlap: 2,
            sampling_rate: 4.0,
        };
        let mut fft_processor = FFTProcessor::new(config);

        // Mix of measurement types - only scalars should be processed
        let measurements = vec![
            Arc::new(Measurement::Spectrum(daq_core::SpectrumData {
                timestamp: Utc::now(),
                channel: "existing_spectrum".to_string(),
                wavelengths: vec![],
                intensities: vec![],
                unit_x: "Hz".to_string(),
                unit_y: "dB".to_string(),
                metadata: None,
            })),
            Arc::new(Measurement::Scalar(daq_core::DataPoint {
                timestamp: Utc::now(),
                channel: "scalar_data".to_string(),
                value: 1.0,
                unit: "V".to_string(),
            })),
        ];

        let result = fft_processor.process_measurements(&measurements);

        // Should return empty because we don't have enough scalar data for FFT window
        assert!(result.is_empty());
    }
}
