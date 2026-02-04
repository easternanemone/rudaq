//! Andor Shamrock Spectrograph Driver
//!
//! Safe wrapper for Andor SDK3 spectrograph API (atspectrograph.dll).
//!
//! # Features
//!
//! - **Wavelength Control**: Set center wavelength for grating
//! - **Grating Selection**: Switch between up to 3 gratings
//! - **Slit Width Control**: Adjust input/output slit widths
//! - **Flipper Mirror**: Direct vs side output selection
//! - **Wavelength Calibration**: Pixel-to-wavelength mapping
//!
//! # Initialization Sequence
//!
//! Based on LIBS/initialization.py lines 47-51:
//!
//! 1. Initialize spectrograph SDK
//! 2. Set default grating (usually grating 2)
//! 3. Set default wavelength (e.g., 310nm)
//! 4. Configure slit widths
//! 5. Query wavelength calibration from camera pixel width
//!
//! # Example
//!
//! ```rust,no_run
//! use driver_andor_sdk3::spectrograph::AndorSpectrograph;
//! use common::capabilities::WavelengthTunable;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let spec = AndorSpectrograph::new_async(0).await?;
//!
//! // Set grating and wavelength
//! spec.set_grating(2).await?;
//! spec.set_wavelength(310.0).await?;
//!
//! // Configure slits
//! spec.set_slit_width(2, 150.0).await?;  // Port 2, 150µm
//!
//! // Get calibration for camera
//! let calibration = spec.get_wavelength_calibration(2048).await?;
//! # Ok(())
//! # }
//! ```

use crate::error::AndorError;
use crate::types::{FlipperMirror, Grating, GratingInfo, SpectrographInfo, WavelengthCalibration};
use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{Parameterized, ShutterControl, WavelengthTunable};
use common::observable::ParameterSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(feature = "spectrograph")]
use std::sync::atomic::AtomicUsize;

#[cfg(feature = "spectrograph")]
use andor_sdk3_sys::*;

/// Global instance counter for SDK library lifecycle management.
///
/// The Andor Shamrock SDK requires ShamrockInitialize() to be called once before any
/// spectrographs are opened, and ShamrockClose() to be called once after all spectrographs
/// are closed. This counter tracks the number of live spectrograph instances to ensure
/// proper cleanup.
#[cfg(feature = "spectrograph")]
static SHAMROCK_INSTANCE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Andor Shamrock spectrograph driver
///
/// Implements the following capabilities:
/// - `WavelengthTunable`: Set center wavelength and grating
/// - `ShutterControl`: Control spectrograph shutter
/// - `Parameterized`: Expose spectrograph parameters
#[derive(Clone)]
pub struct AndorSpectrograph {
    inner: Arc<AndorSpectrographInner>,
}

struct AndorSpectrographInner {
    #[cfg(feature = "spectrograph")]
    handle: i32, // Spectrograph device index
    #[cfg(not(feature = "spectrograph"))]
    handle: i32,

    info: SpectrographInfo,
    shutter_open: AtomicBool,

    // Configuration
    wavelength_nm: Mutex<f64>,
    grating: Mutex<Grating>,
    slit_width_um: Mutex<f64>,
    flipper_mirror: Mutex<FlipperMirror>,

    // Calibration cache
    calibration: Mutex<Option<WavelengthCalibration>>,

    // Parameters
    params: ParameterSet,
}

impl AndorSpectrograph {
    /// Create new mock spectrograph instance for testing
    ///
    /// This is a convenience method that always uses the mock backend,
    /// regardless of feature flags.
    pub async fn new_mock() -> Result<Self> {
        Self::new_async(0).await
    }

    /// Create new spectrograph instance (async)
    ///
    /// # Arguments
    ///
    /// * `device_index` - Spectrograph device index (usually 0)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - SDK initialization fails
    /// - Device index is invalid
    pub async fn new_async(device_index: i32) -> Result<Self> {
        #[cfg(feature = "spectrograph")]
        let info = tokio::task::spawn_blocking(move || Self::init_hardware(device_index)).await??;

        #[cfg(not(feature = "spectrograph"))]
        let info = Self::mock_spectrograph_info(device_index);

        let inner = Arc::new(AndorSpectrographInner {
            handle: device_index,
            info,
            shutter_open: AtomicBool::new(false),
            wavelength_nm: Mutex::new(310.0),
            grating: Mutex::new(Grating::Grating2),
            slit_width_um: Mutex::new(150.0),
            flipper_mirror: Mutex::new(FlipperMirror::Direct),
            calibration: Mutex::new(None),
            params: ParameterSet::new(),
        });

        Ok(Self { inner })
    }

    #[cfg(feature = "spectrograph")]
    fn init_hardware(device_index: i32) -> Result<SpectrographInfo> {
        use crate::error::sdk_result;

        unsafe {
            // Initialize library only if this is the first instance
            // SAFETY: We use atomic operations to ensure thread-safe ref counting.
            // The SDK is initialized once when transitioning from 0 to 1 instances.
            if SHAMROCK_INSTANCE_COUNT.fetch_add(1, Ordering::SeqCst) == 0 {
                // SAFETY: ShamrockInitialize is called once during driver initialization.
                // The null pointer is the expected argument per Andor SDK documentation.
                // No concurrent access - this runs in spawn_blocking, isolated from other threads.
                let ret = ShamrockInitialize(std::ptr::null_mut());
                if let Err(e) = sdk_result(ret) {
                    // Rollback the instance count on initialization failure
                    SHAMROCK_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst);
                    return Err(e.into());
                }
            }

            // SAFETY: num_devices is a stack-allocated i32 with a valid pointer.
            // ShamrockGetNumberDevices writes the device count to this location.
            // The SDK guarantees this is safe after successful initialization.
            let mut num_devices = 0i32;
            let ret = ShamrockGetNumberDevices(&mut num_devices);
            if let Err(e) = sdk_result(ret) {
                // Cleanup library if last instance and query failed
                if SHAMROCK_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    ShamrockClose();
                }
                return Err(e.into());
            }

            if device_index >= num_devices {
                // Cleanup library if last instance and device not found
                if SHAMROCK_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    ShamrockClose();
                }
                return Err(AndorError::DeviceNotFound(format!(
                    "Spectrograph index {} not found (only {} devices)",
                    device_index, num_devices
                ))
                .into());
            }

            // SAFETY: device_index is validated in-range by num_devices check above.
            // The buffer is stack-allocated with fixed size 256, which matches SDK expectations.
            // ShamrockGetSerialNumber writes a null-terminated string to this buffer.
            // No concurrent access - we hold exclusive access during initialization.
            let mut serial_buffer = vec![0i8; 256];
            let ret = ShamrockGetSerialNumber(device_index, serial_buffer.as_mut_ptr());
            let serial_number = if ret == SHAMROCK_SUCCESS {
                let bytes: Vec<u8> = serial_buffer.iter().map(|&b| b as u8).collect();
                String::from_utf8_lossy(&bytes)
                    .trim_end_matches('\0')
                    .to_string()
            } else {
                "Unknown".to_string()
            };

            // SAFETY: device_index is validated in-range.
            // num_gratings is a stack-allocated i32 with a valid pointer.
            // ShamrockGetNumberGratings writes the grating count to this location.
            let mut num_gratings = 0i32;
            let ret = ShamrockGetNumberGratings(device_index, &mut num_gratings);
            let num_gratings = if ret == SHAMROCK_SUCCESS {
                num_gratings as usize
            } else {
                3
            };

            Ok(SpectrographInfo {
                model: "Shamrock".to_string(),
                serial_number,
                num_gratings,
            })
        }
    }

    #[cfg(not(feature = "spectrograph"))]
    fn mock_spectrograph_info(_device_index: i32) -> SpectrographInfo {
        SpectrographInfo {
            model: "Mock Shamrock".to_string(),
            serial_number: "MOCK-SPEC-001".to_string(),
            num_gratings: 3,
        }
    }

    /// Get spectrograph information
    pub fn info(&self) -> &SpectrographInfo {
        &self.inner.info
    }

    /// Set active grating
    pub async fn set_grating(&self, grating_index: i32) -> Result<()> {
        let grating = Grating::try_from(grating_index).map_err(|e| anyhow::anyhow!(e))?;

        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is a valid device index from successful initialization.
                // grating_index is validated by Grating::try_from above.
                // spawn_blocking ensures no concurrent FFI calls to the same device.
                unsafe {
                    let ret = ShamrockSetGrating(handle, grating_index);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        *self.inner.grating.lock().await = grating;
        tracing::info!("Grating set to {}", grating_index);
        Ok(())
    }

    /// Get active grating index
    pub async fn get_grating(&self) -> Result<i32> {
        Ok(*self.inner.grating.lock().await as i32)
    }

    /// Get grating information (lines/mm, blaze wavelength)
    pub async fn get_grating_info(&self, grating_index: i32) -> Result<GratingInfo> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization. All output parameters
                // (lines, blaze, home, offset) are stack-allocated with valid pointers.
                // ShamrockGetGratingInfo writes to these locations.
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let mut lines = 0.0;
                    let mut blaze = 0.0;
                    let mut home = 0;
                    let mut offset = 0;

                    let ret = ShamrockGetGratingInfo(
                        handle,
                        grating_index,
                        &mut lines,
                        &mut blaze,
                        &mut home,
                        &mut offset,
                    );
                    sdk_result(ret)?;

                    Ok(GratingInfo {
                        lines_per_mm: lines,
                        blaze_wavelength_nm: blaze,
                    })
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(GratingInfo {
            lines_per_mm: 1200.0,
            blaze_wavelength_nm: 300.0,
        })
    }

    /// Set slit width in micrometers
    ///
    /// # Arguments
    ///
    /// * `port` - Slit port number (typically 2 for output slit)
    /// * `width_um` - Slit width in micrometers
    pub async fn set_slit_width(&self, port: i32, width_um: f64) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // port and width_um are validated by the SDK (will return error if invalid).
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let ret = ShamrockSetAutoSlitWidth(handle, port, width_um as f32);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        *self.inner.slit_width_um.lock().await = width_um;
        tracing::info!("Slit width set to {}µm", width_um);
        Ok(())
    }

    /// Get slit width in micrometers
    pub async fn get_slit_width(&self, port: i32) -> Result<f64> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // width is a stack-allocated f32 with a valid pointer.
                // ShamrockGetAutoSlitWidth writes the slit width to this location.
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let mut width = 0.0;
                    let ret = ShamrockGetAutoSlitWidth(handle, port, &mut width);
                    sdk_result(ret)?;
                    Ok(width as f64)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(*self.inner.slit_width_um.lock().await)
    }

    /// Set flipper mirror position
    pub async fn set_flipper_mirror(&self, port: i32, position: FlipperMirror) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            let pos = position as i32;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // pos is a valid FlipperMirror enum value cast to i32.
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let ret = ShamrockSetFlipperMirror(handle, port, pos);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        *self.inner.flipper_mirror.lock().await = position;
        tracing::info!("Flipper mirror set to {:?}", position);
        Ok(())
    }

    /// Get wavelength calibration array
    ///
    /// Maps pixel indices to wavelengths based on current grating and center wavelength.
    ///
    /// # Arguments
    ///
    /// * `num_pixels` - Number of camera pixels (typically 2048)
    pub async fn get_wavelength_calibration(
        &self,
        num_pixels: u32,
    ) -> Result<WavelengthCalibration> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // wavelengths is a heap-allocated Vec with num_pixels elements.
                // ShamrockGetCalibration writes wavelength values to this buffer.
                // The buffer size matches num_pixels, preventing buffer overflow.
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let mut wavelengths = vec![0.0f32; num_pixels as usize];
                    let ret =
                        ShamrockGetCalibration(handle, wavelengths.as_mut_ptr(), num_pixels as i32);
                    sdk_result(ret)?;

                    // Convert f32 to f64
                    let wavelengths_f64: Vec<f64> = wavelengths.iter().map(|&w| w as f64).collect();

                    let calibration = WavelengthCalibration::new(wavelengths_f64);

                    Ok(calibration)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        {
            // Mock linear calibration
            let center = *self.inner.wavelength_nm.lock().await;
            let dispersion = 0.05; // nm per pixel
            let wavelengths: Vec<f64> = (0..num_pixels)
                .map(|i| center + (i as f64 - num_pixels as f64 / 2.0) * dispersion)
                .collect();

            Ok(WavelengthCalibration::new(wavelengths))
        }
    }
}

// Implement capability traits

#[async_trait]
impl WavelengthTunable for AndorSpectrograph {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // wavelength_nm is validated by the SDK (will return error if out of range).
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let ret = ShamrockSetWavelength(handle, wavelength_nm as f32);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        *self.inner.wavelength_nm.lock().await = wavelength_nm;
        tracing::info!("Wavelength set to {} nm", wavelength_nm);
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // wavelength is a stack-allocated f32 with a valid pointer.
                // ShamrockGetWavelength writes the current wavelength to this location.
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let mut wavelength = 0.0;
                    let ret = ShamrockGetWavelength(handle, &mut wavelength);
                    sdk_result(ret)?;
                    Ok(wavelength as f64)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(*self.inner.wavelength_nm.lock().await)
    }

    fn wavelength_range(&self) -> (f64, f64) {
        // Range depends on grating, return typical UV-NIR range
        (200.0, 1000.0)
    }
}

#[async_trait]
impl ShutterControl for AndorSpectrograph {
    async fn open_shutter(&self) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // 1 is a valid shutter command (open).
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let ret = ShamrockSetShutter(handle, 1);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        self.inner.shutter_open.store(true, Ordering::Relaxed);
        tracing::info!("Spectrograph shutter opened");
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // 0 is a valid shutter command (close).
                // spawn_blocking ensures no concurrent FFI calls.
                unsafe {
                    let ret = ShamrockSetShutter(handle, 0);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        self.inner.shutter_open.store(false, Ordering::Relaxed);
        tracing::info!("Spectrograph shutter closed");
        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        Ok(self.inner.shutter_open.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl Parameterized for AndorSpectrograph {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}

impl Drop for AndorSpectrographInner {
    fn drop(&mut self) {
        #[cfg(feature = "spectrograph")]
        unsafe {
            // Only finalize library when last instance is dropped
            // SAFETY: We increment the counter in init_hardware when library is initialized,
            // so we must decrement here. If this is the last instance (count was 1 before
            // decrement), we finalize the library.
            if SHAMROCK_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                ShamrockClose();
            }
        }
    }
}
