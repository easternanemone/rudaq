//! Meta Instrument Traits
//!
//! Abstract device capabilities that enable type-safe runtime reassignment.
//! Inspired by DynExp's meta instrument system.
//!
//! # Design Philosophy
//!
//! Meta instrument traits are orthogonal to the existing `Instrument` trait.
//! An instrument can implement both:
//! - `Instrument` trait for lifecycle management and data streaming
//! - One or more meta instrument traits for domain-specific capabilities
//!
//! This separation allows existing instruments to opt-in to the module system
//! without breaking changes.
//!
//! # Example
//!
//! ```rust,no_run
//! use rust_daq::instrument::Instrument;
//! use rust_daq::modules::meta_instruments::{MetaInstrument, Camera};
//! use rust_daq::core::ImageData;
//! use async_trait::async_trait;
//!
//! struct MyCamera {
//!     id: String,
//!     // ... fields
//! }
//!
//! // Existing Instrument trait
//! #[async_trait]
//! impl Instrument for MyCamera {
//!     type Measure = rust_daq::measurement::InstrumentMeasurement;
//!     // ... implement methods
//! #   fn name(&self) -> String { self.id.clone() }
//! #   async fn connect(&mut self, id: &str, settings: &std::sync::Arc<rust_daq::config::Settings>) -> anyhow::Result<()> { Ok(()) }
//! #   async fn disconnect(&mut self) -> anyhow::Result<()> { Ok(()) }
//! #   fn measure(&self) -> &Self::Measure { unimplemented!() }
//! }
//!
//! // Add Camera meta trait (opt-in)
//! impl MetaInstrument for MyCamera {
//!     fn instrument_id(&self) -> &str { &self.id }
//!     fn instrument_type(&self) -> &str { "camera" }
//!     fn capabilities(&self) -> Vec<String> { vec!["camera".into()] }
//! }
//!
//! #[async_trait]
//! impl Camera for MyCamera {
//!     async fn capture(&mut self) -> anyhow::Result<ImageData> {
//!         // Implementation
//! #       Ok(ImageData {
//! #           timestamp: chrono::Utc::now(),
//! #           channel: "test".into(),
//! #           width: 640,
//! #           height: 480,
//! #           pixels: vec![0.0; 640 * 480],
//! #           unit: "counts".into(),
//! #           metadata: None,
//! #       })
//!     }
//!
//!     async fn set_exposure(&mut self, ms: f64) -> anyhow::Result<()> {
//!         // Implementation
//! #       Ok(())
//!     }
//!
//!     async fn get_exposure(&self) -> anyhow::Result<f64> {
//!         // Implementation
//! #       Ok(100.0)
//!     }
//!
//!     async fn set_roi(&mut self, x: u32, y: u32, width: u32, height: u32) -> anyhow::Result<()> {
//!         // Implementation
//! #       Ok(())
//!     }
//!
//!     async fn get_sensor_size(&self) -> anyhow::Result<(u32, u32)> {
//!         // Implementation
//! #       Ok((640, 480))
//!     }
//! }
//! ```

use async_trait::async_trait;
use anyhow::Result;
use crate::core::{ImageData, SpectrumData};

/// Base meta instrument trait - all devices can implement this.
///
/// Provides common metadata and capability discovery functionality.
pub trait MetaInstrument: Send + Sync {
    /// Unique identifier for this instrument instance
    fn instrument_id(&self) -> &str;

    /// Type category of this instrument (e.g., "camera", "spectrometer", "power_meter")
    fn instrument_type(&self) -> &str;

    /// List of capability identifiers this instrument provides.
    ///
    /// Common capabilities: "camera", "spectrometer", "power_meter", "positioner", "temperature_controller"
    fn capabilities(&self) -> Vec<String>;
}

/// Camera-specific capabilities.
///
/// Instruments that provide imaging functionality should implement this trait.
/// Supports standard camera operations like capture, exposure control, and ROI selection.
#[async_trait]
pub trait Camera: MetaInstrument {
    /// Capture a single image from the camera.
    ///
    /// Returns an `ImageData` structure containing pixel data, dimensions, and metadata.
    /// This is a synchronous capture - the method blocks until the image is ready.
    async fn capture(&mut self) -> Result<ImageData>;

    /// Set the exposure time in milliseconds.
    ///
    /// # Arguments
    ///
    /// * `ms` - Exposure time in milliseconds
    async fn set_exposure(&mut self, ms: f64) -> Result<()>;

    /// Get the current exposure time in milliseconds
    async fn get_exposure(&self) -> Result<f64>;

    /// Set the region of interest (ROI) for capture.
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate of ROI top-left corner
    /// * `y` - Y coordinate of ROI top-left corner
    /// * `width` - ROI width in pixels
    /// * `height` - ROI height in pixels
    async fn set_roi(&mut self, x: u32, y: u32, width: u32, height: u32) -> Result<()>;

    /// Get the full sensor size (width, height) in pixels
    async fn get_sensor_size(&self) -> Result<(u32, u32)>;
}

/// Spectrometer-specific capabilities.
///
/// Instruments that perform spectral analysis should implement this trait.
#[async_trait]
pub trait Spectrometer: MetaInstrument {
    /// Acquire a spectrum measurement.
    ///
    /// Returns a `SpectrumData` structure containing wavelengths and intensities.
    async fn acquire_spectrum(&mut self) -> Result<SpectrumData>;

    /// Set the integration time in milliseconds.
    ///
    /// Longer integration times increase SNR but reduce acquisition rate.
    ///
    /// # Arguments
    ///
    /// * `ms` - Integration time in milliseconds
    async fn set_integration_time(&mut self, ms: f64) -> Result<()>;

    /// Get the wavelength range covered by this spectrometer.
    ///
    /// Returns (min_wavelength_nm, max_wavelength_nm)
    async fn get_wavelength_range(&self) -> Result<(f64, f64)>;

    /// Get the wavelength calibration for each pixel/bin.
    ///
    /// Returns a vector of wavelengths in nm, one per detector element.
    async fn get_wavelength_calibration(&self) -> Result<Vec<f64>>;
}

/// Power meter capabilities.
///
/// Instruments that measure optical or RF power should implement this trait.
#[async_trait]
pub trait PowerMeter: MetaInstrument {
    /// Read the current power measurement.
    ///
    /// Returns power in watts. Use `get_range()` to determine the measurement range.
    async fn read_power(&mut self) -> Result<f64>;

    /// Set the wavelength for calibrated measurements.
    ///
    /// Many power meters have wavelength-dependent responsivity.
    ///
    /// # Arguments
    ///
    /// * `nm` - Wavelength in nanometers
    async fn set_wavelength(&mut self, nm: f64) -> Result<()>;

    /// Set the measurement range in watts.
    ///
    /// Auto-ranging can be achieved by setting a very large range.
    ///
    /// # Arguments
    ///
    /// * `watts` - Maximum expected power in watts
    async fn set_range(&mut self, watts: f64) -> Result<()>;

    /// Get the current measurement range in watts
    async fn get_range(&self) -> Result<f64>;

    /// Perform a zero calibration.
    ///
    /// Should be done with no input power to subtract dark current/offset.
    async fn zero(&mut self) -> Result<()>;
}

/// Position control capabilities (stages, mirrors, rotation mounts).
///
/// Instruments that provide motorized position control should implement this trait.
#[async_trait]
pub trait Positioner: MetaInstrument {
    /// Move to an absolute position.
    ///
    /// The unit depends on the device (mm for linear stages, degrees for rotation mounts).
    /// Blocks until motion completes or returns immediately with motion in progress.
    ///
    /// # Arguments
    ///
    /// * `position` - Target position in device units
    async fn move_absolute(&mut self, position: f64) -> Result<()>;

    /// Move by a relative offset from current position.
    ///
    /// # Arguments
    ///
    /// * `delta` - Distance to move (positive or negative) in device units
    async fn move_relative(&mut self, delta: f64) -> Result<()>;

    /// Get the current position in device units
    async fn get_position(&self) -> Result<f64>;

    /// Home the positioner to its reference position.
    ///
    /// Most stages require homing after power-on to establish absolute positioning.
    async fn home(&mut self) -> Result<()>;

    /// Stop any motion in progress (emergency stop)
    async fn stop_motion(&mut self) -> Result<()>;

    /// Check if the positioner is currently moving
    async fn is_moving(&self) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock Camera for testing
    struct MockCamera {
        id: String,
        exposure_ms: f64,
    }

    impl MetaInstrument for MockCamera {
        fn instrument_id(&self) -> &str {
            &self.id
        }

        fn instrument_type(&self) -> &str {
            "camera"
        }

        fn capabilities(&self) -> Vec<String> {
            vec!["camera".to_string()]
        }
    }

    #[async_trait]
    impl Camera for MockCamera {
        async fn capture(&mut self) -> Result<ImageData> {
            Ok(ImageData {
                timestamp: chrono::Utc::now(),
                channel: format!("{}_image", self.id),
                width: 640,
                height: 480,
                pixels: vec![0.0; 640 * 480],
                unit: "counts".to_string(),
                metadata: None,
            })
        }

        async fn set_exposure(&mut self, ms: f64) -> Result<()> {
            self.exposure_ms = ms;
            Ok(())
        }

        async fn get_exposure(&self) -> Result<f64> {
            Ok(self.exposure_ms)
        }

        async fn set_roi(&mut self, _x: u32, _y: u32, _width: u32, _height: u32) -> Result<()> {
            Ok(())
        }

        async fn get_sensor_size(&self) -> Result<(u32, u32)> {
            Ok((640, 480))
        }
    }

    #[tokio::test]
    async fn test_camera_trait() {
        let mut camera = MockCamera {
            id: "test_cam".to_string(),
            exposure_ms: 100.0,
        };

        // Test MetaInstrument
        assert_eq!(camera.instrument_id(), "test_cam");
        assert_eq!(camera.instrument_type(), "camera");
        assert!(camera.capabilities().contains(&"camera".to_string()));

        // Test Camera
        camera.set_exposure(50.0).await.unwrap();
        assert_eq!(camera.get_exposure().await.unwrap(), 50.0);

        let image = camera.capture().await.unwrap();
        assert_eq!(image.width, 640);
        assert_eq!(image.height, 480);
    }
}
