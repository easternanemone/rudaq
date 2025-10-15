//! An FFT (Fast Fourier Transform) data processor for frequency analysis.

use crate::core::{DataPoint, DataProcessor};
use chrono::{LocalResult, TimeZone, Utc};
use log::{debug, warn};
use num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::collections::VecDeque;
use std::sync::Arc;

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
///     let sample_rate = 1024.0;
///     let config = FFTConfig {
///         window_size: 1024,
///         overlap: 512,
///         sampling_rate: sample_rate,
///     };
///     let mut fft_processor = FFTProcessor::new(config);
///
///     // Generate a sine wave for testing
///     let frequency = 50.0;
///     let mut sine_wave = Vec::new();
///     for i in 0..2048 {
///         let t = i as f64 / sample_rate;
///         let value = (2.0 * std::f64::consts::PI * frequency * t).sin();
///         sine_wave.push(DataPoint {
///             timestamp: Utc.timestamp_nanos((t * 1_000_000_000.0) as i64),
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
}

impl FFTProcessor {
    /// Creates a new `FFTProcessor`.
    ///
    /// # Arguments
    ///
    /// * `window_size` - The number of samples to use for each FFT. This should be a power of 2 for optimal performance.
    /// * `overlap` - The number of samples to overlap between consecutive windows. Must be less than `window_size`.
    /// * `sampling_rate` - The sampling rate of the incoming data in Hz.
    pub fn new(window_size: usize, overlap: usize, sampling_rate: f64) -> Self {
        assert!(
            overlap < window_size,
            "Overlap must be less than window size"
        );
        assert!(sampling_rate > 0.0, "Sampling rate must be positive");

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(window_size);

        let mut hann_window = Vec::with_capacity(window_size);
        if window_size > 1 {
            for i in 0..window_size {
                // Hann window formula
                let val = 0.5
                    * (1.0
                        - (2.0 * std::f64::consts::PI * i as f64 / (window_size - 1) as f64).cos());
                hann_window.push(val);
            }
        }

        Self {
            window_size,
            overlap,
            sampling_rate,
            buffer: VecDeque::with_capacity(window_size * 2),
            fft_planner: fft,
            hann_window,
            channel: String::from("unknown"),
        }
    }

    fn frequency_to_timestamp(frequency: f64) -> Option<chrono::DateTime<Utc>> {
        let secs = frequency.trunc() as i64;
        let nanos_fraction = (frequency.fract() * 1_000_000_000.0).round();
        let mut secs = secs;
        let mut nanos = nanos_fraction as i64;

        if nanos >= 1_000_000_000 {
            secs += 1;
            nanos -= 1_000_000_000;
        }

        if nanos < 0 {
            return None;
        }

        match Utc.timestamp_opt(secs, nanos as u32) {
            LocalResult::Single(ts) | LocalResult::Ambiguous(ts, _) => Some(ts),
            LocalResult::None => None,
        }
    }
}

impl DataProcessor for FFTProcessor {
    /// Processes a slice of `DataPoint`s, performing an FFT when enough data is available.
    fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> {
        if data.is_empty() {
            return vec![];
        }

        // Update channel from the first data point
        if self.channel == "unknown" {
            self.channel = data[0].channel.clone();
        }

        self.buffer.extend(data.iter().map(|dp| dp.value));
        debug!("Buffer size: {}", self.buffer.len());

        let mut all_fft_results = Vec::new();
        let step_size = self.window_size - self.overlap;

        while self.buffer.len() >= self.window_size {
            debug!("Processing window. Buffer size: {}", self.buffer.len());

            let mut complex_buffer: Vec<Complex<f64>> = self.buffer
                .iter()
                .take(self.window_size)
                .zip(self.hann_window.iter())
                .map(|(&val, &win_val)| Complex::new(val * win_val, 0.0))
                .collect();

            self.fft_planner.process(&mut complex_buffer);

            let freq_resolution = self.sampling_rate / self.window_size as f64;
            let num_bins = self.window_size / 2;

            let mut fft_dps = Vec::with_capacity(num_bins);

            if num_bins > 0 {
                let dc_val = &complex_buffer[0];
                let magnitude = dc_val.norm() / self.window_size as f64;
                let magnitude_db = if magnitude > 1e-6 {
                    20.0 * magnitude.log10()
                } else {
                    -120.0
                };

                if let Some(timestamp) = Self::frequency_to_timestamp(0.0) {
                    fft_dps.push(DataPoint {
                        timestamp,
                        channel: self.channel.clone(),
                        value: magnitude_db,
                        unit: "dB".to_string(),
                        metadata: None,
                    });
                } else {
                    warn!("Skipping DC component: unable to encode frequency as timestamp");
                }
            }

            for i in 1..num_bins {
                let complex_val = &complex_buffer[i];
                let magnitude = (complex_val.norm() * 2.0) / self.window_size as f64;
                let magnitude_db = if magnitude > 1e-6 {
                    20.0 * magnitude.log10()
                } else {
                    -120.0
                };

                let frequency = i as f64 * freq_resolution;

                if let Some(timestamp) = Self::frequency_to_timestamp(frequency) {
                    fft_dps.push(DataPoint {
                        timestamp,
                        channel: self.channel.clone(),
                        value: magnitude_db,
                        unit: "dB".to_string(),
                        metadata: None,
                    });
                } else {
                    warn!(
                        "Skipping frequency bin at {} Hz: unable to encode frequency as timestamp",
                        frequency
                    );
                }
            }

            all_fft_results.extend(fft_dps);
            self.buffer.drain(..step_size);
            debug!("Drained buffer. New size: {}", self.buffer.len());
        }

        all_fft_results
    }
}
