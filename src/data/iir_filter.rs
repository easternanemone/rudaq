//! An IIR (Infinite Impulse Response) filter data processor.
use crate::core::{DataPoint, DataProcessor};
use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Q_BUTTERWORTH_F64};
use serde::Deserialize;

/// The type of IIR filter to apply.
#[derive(Debug, Deserialize, Clone)]
pub enum FilterType {
    /// A low-pass filter allows frequencies below the cutoff frequency to pass through.
    Lowpass,
    /// A high-pass filter allows frequencies above the cutoff frequency to pass through.
    Highpass,
    /// A band-pass filter allows frequencies within a certain range to pass through.
    Bandpass,
    /// A band-stop (or notch) filter rejects frequencies within a certain range.
    Bandstop,
}

/// Configuration for the `IirFilter`.
///
/// This struct is typically deserialized from a configuration file.
#[derive(Debug, Deserialize, Clone)]
pub struct IirFilterConfig {
    /// The type of filter to apply.
    pub filter_type: FilterType,
    /// The cutoff or center frequency of the filter in Hz.
    pub f0: f64,
    /// The sample rate of the data in Hz.
    pub fs: f64,
    /// The quality factor (Q) of the filter. Determines the sharpness of the filter's transition.
    /// If not provided, a default Butterworth Q value is used.
    pub q: Option<f64>,
}

/// A data processor that applies an IIR filter to a stream of `DataPoint`s.
///
/// This processor uses the `biquad` crate to implement a second-order IIR filter.
/// It supports low-pass, high-pass, band-pass, and band-stop (notch) filters.
///
/// # Example Configuration (`.toml`)
///
/// ```toml
/// [[processors.my_instrument]]
/// type = "iir"
/// filter_type = "Lowpass"
/// f0 = 50.0  # Cutoff frequency in Hz
/// fs = 1000.0 # Sample rate in Hz
/// q = 0.707  # Optional quality factor
/// ```
pub struct IirFilter {
    filter: DirectForm1<f64>,
}

impl IirFilter {
    pub fn new(config: IirFilterConfig) -> Result<Self, &'static str> {
        let coeffs = Self::design_filter(&config)?;
        Ok(Self {
            filter: DirectForm1::<f64>::new(coeffs),
        })
    }

    fn design_filter(config: &IirFilterConfig) -> Result<Coefficients<f64>, &'static str> {
        let f0 = config.f0.hz();
        let fs = config.fs.hz();
        let q = config.q.unwrap_or(Q_BUTTERWORTH_F64);

        let result = match config.filter_type {
            FilterType::Lowpass => {
                Coefficients::<f64>::from_params(biquad::Type::LowPass, fs, f0, q)
            }
            FilterType::Highpass => {
                Coefficients::<f64>::from_params(biquad::Type::HighPass, fs, f0, q)
            }
            FilterType::Bandpass => {
                Coefficients::<f64>::from_params(biquad::Type::BandPass, fs, f0, q)
            }
            FilterType::Bandstop => {
                Coefficients::<f64>::from_params(biquad::Type::Notch, fs, f0, q)
            }
        };
        result.map_err(|_| "Failed to create IIR filter coefficients")
    }
}

impl DataProcessor for IirFilter {
    fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> {
        data.iter()
            .map(|dp| {
                let mut new_dp = dp.clone();
                new_dp.value = self.filter.run(dp.value);
                new_dp.channel = format!("{}_filtered", dp.channel);
                new_dp
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DataPoint;
    use chrono::Utc;

    fn create_test_data(len: usize) -> Vec<DataPoint> {
        (0..len)
            .map(|i| DataPoint {
                timestamp: Utc::now(),
                channel: "test".to_string(),
                value: i as f64,
                unit: "V".to_string(),
                metadata: None,
            })
            .collect()
    }

    #[test]
    fn test_lowpass_filter_creation() {
        let config = IirFilterConfig {
            filter_type: FilterType::Lowpass,
            f0: 100.0,
            fs: 1000.0,
            q: None,
        };
        assert!(IirFilter::new(config).is_ok());
    }

    #[test]
    fn test_highpass_filter_creation() {
        let config = IirFilterConfig {
            filter_type: FilterType::Highpass,
            f0: 100.0,
            fs: 1000.0,
            q: None,
        };
        assert!(IirFilter::new(config).is_ok());
    }

    #[test]
    fn test_bandpass_filter_creation() {
        let config = IirFilterConfig {
            filter_type: FilterType::Bandpass,
            f0: 100.0,
            fs: 1000.0,
            q: Some(1.0),
        };
        assert!(IirFilter::new(config).is_ok());
    }

    #[test]
    fn test_bandstop_filter_creation() {
        let config = IirFilterConfig {
            filter_type: FilterType::Bandstop,
            f0: 100.0,
            fs: 1000.0,
            q: Some(1.0),
        };
        assert!(IirFilter::new(config).is_ok());
    }

    #[test]
    fn test_invalid_filter_params() {
        // f0 > fs / 2
        let config = IirFilterConfig {
            filter_type: FilterType::Lowpass,
            f0: 600.0,
            fs: 1000.0,
            q: None,
        };
        assert!(IirFilter::new(config).is_err());
    }

    #[test]
    fn test_filter_processes_data() {
        let config = IirFilterConfig {
            filter_type: FilterType::Lowpass,
            f0: 100.0,
            fs: 1000.0,
            q: None,
        };
        let mut filter = IirFilter::new(config).unwrap();
        let data = create_test_data(10);
        let original_values: Vec<f64> = data.iter().map(|dp| dp.value).collect();

        let processed_data = filter.process(&data);
        let processed_values: Vec<f64> = processed_data.iter().map(|dp| dp.value).collect();

        assert_ne!(original_values, processed_values);
        assert_eq!(processed_data[0].channel, "test_filtered");
    }
}
