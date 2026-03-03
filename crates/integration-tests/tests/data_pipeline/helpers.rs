// =============================================================================
// Test Helper Functions
// =============================================================================

use common::core::Measurement;

/// Create a scalar measurement for testing
#[allow(dead_code)]
pub fn create_test_scalar(name: &str, value: f64) -> Measurement {
    Measurement::Scalar {
        name: name.to_string(),
        value,
        unit: "V".to_string(),
        timestamp: chrono::Utc::now(),
    }
}

/// Create a vector measurement for testing
#[allow(dead_code)]
pub fn create_test_vector(name: &str, values: Vec<f64>) -> Measurement {
    Measurement::Vector {
        name: name.to_string(),
        values,
        unit: "V".to_string(),
        timestamp: chrono::Utc::now(),
    }
}

/// Create a spectrum measurement for testing
#[allow(dead_code)]
pub fn create_test_spectrum(name: &str, n_bins: usize) -> Measurement {
    let frequencies: Vec<f64> = (0..n_bins).map(|i| i as f64 * 100.0).collect();
    let amplitudes: Vec<f64> = frequencies
        .iter()
        .map(|f| 1.0 / (1.0 + f / 1000.0))
        .collect();
    Measurement::Spectrum {
        name: name.to_string(),
        frequencies,
        amplitudes,
        frequency_unit: Some("Hz".to_string()),
        amplitude_unit: Some("dB".to_string()),
        metadata: None,
        timestamp: chrono::Utc::now(),
    }
}

/// Create an image measurement for testing
#[allow(dead_code)]
pub fn _create_test_image(name: &str, width: u32, height: u32) -> Measurement {
    use common::core::{ImageMetadata, PixelBuffer};

    let pixel_count = (width * height) as usize;
    let pixels: Vec<u16> = (0..pixel_count).map(|i| (i % 65536) as u16).collect();

    Measurement::Image {
        name: name.to_string(),
        width,
        height,
        buffer: PixelBuffer::U16(pixels),
        unit: "counts".to_string(),
        metadata: ImageMetadata {
            exposure_ms: Some(100.0),
            gain: Some(1.0),
            binning: Some((1, 1)),
            temperature_c: Some(-20.0),
            hardware_timestamp_us: None,
            readout_ms: Some(10.0),
            roi_origin: Some((0, 0)),
        },
        timestamp: chrono::Utc::now(),
    }
}
