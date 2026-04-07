//! Mock Spectrograph for scripting and testing.
//!
//! Provides a minimal `SpectrometerControl` + `WavelengthTunable` implementation
//! for use in Rhai scripts and tests without requiring the Andor Shamrock SDK.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{SpectrometerControl, WavelengthTunable};
use tokio::sync::Mutex;

/// Mock spectrograph with grating, wavelength, and slit control.
pub struct MockSpectrograph {
    grating: Mutex<i32>,
    wavelength_nm: Mutex<f64>,
    slit_width_um: Mutex<f64>,
    num_gratings: i32,
}

impl MockSpectrograph {
    /// Create a new mock spectrograph with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            grating: Mutex::new(1),
            wavelength_nm: Mutex::new(310.0),
            slit_width_um: Mutex::new(100.0),
            num_gratings: 3,
        }
    }
}

impl Default for MockSpectrograph {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WavelengthTunable for MockSpectrograph {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        *self.wavelength_nm.lock().await = wavelength_nm;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        Ok(*self.wavelength_nm.lock().await)
    }
}

#[async_trait]
impl SpectrometerControl for MockSpectrograph {
    async fn set_grating(&self, grating_num: i32) -> Result<()> {
        if grating_num < 1 || grating_num > self.num_gratings {
            anyhow::bail!("Grating {grating_num} out of range 1-{}", self.num_gratings);
        }
        *self.grating.lock().await = grating_num;
        Ok(())
    }

    async fn get_grating(&self) -> Result<i32> {
        Ok(*self.grating.lock().await)
    }

    async fn set_wavelength(&self, nm: f64) -> Result<()> {
        WavelengthTunable::set_wavelength(self, nm).await
    }

    async fn get_wavelength(&self) -> Result<f64> {
        WavelengthTunable::get_wavelength(self).await
    }

    async fn set_slit_width(&self, _slit_id: i32, width_um: f64) -> Result<()> {
        *self.slit_width_um.lock().await = width_um;
        Ok(())
    }

    async fn get_calibration(&self, num_pixels: usize) -> Result<Vec<f64>> {
        let center = *self.wavelength_nm.lock().await;
        let dispersion = 0.05; // nm per pixel
        #[expect(
            clippy::cast_precision_loss,
            reason = "pixel count is small enough that f64 precision is sufficient for wavelength calculation"
        )]
        let half = num_pixels as f64 / 2.0;
        let wavelengths: Vec<f64> = (0..num_pixels)
            .map(|i| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "pixel index is small enough for exact f64 representation"
                )]
                let fi = i as f64;
                center + (fi - half) * dispersion
            })
            .collect();
        Ok(wavelengths)
    }

    async fn is_at_zero_order(&self) -> Result<bool> {
        let wl = *self.wavelength_nm.lock().await;
        Ok(wl.abs() < f64::EPSILON)
    }

    async fn set_shutter(&self, _open: bool) -> Result<()> {
        Ok(())
    }
}
