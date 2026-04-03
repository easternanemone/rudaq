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
use crate::types::{
    FilterPosition, FlipperMirror, Grating, GratingInfo, SlitPort, SpectrographInfo,
    WavelengthCalibration, WavelengthLimits,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use common::capabilities::{Parameterized, ShutterControl, WavelengthTunable};
use common::error::DaqError;
use common::observable::ParameterSet;
use common::parameter::Parameter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(feature = "spectrograph")]
use std::sync::atomic::AtomicUsize;
#[cfg(feature = "spectrograph")]
use std::sync::Mutex as StdMutex;

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
#[cfg(feature = "spectrograph")]
static SHAMROCK_INIT_MUTEX: StdMutex<()> = StdMutex::new(());
#[cfg(feature = "spectrograph")]
const SHAMROCK_RECOVERY_ROOTS: &[&str] = &["/tmp", "/dev/shm"];
#[cfg(feature = "spectrograph")]
const SHAMROCK_RECOVERY_PREFIXES: &[&str] = &[
    "shamrock",
    "atspectrograph",
    "andor",
    "atdebug",
    "sem.shamrock",
    "sem.atspectrograph",
    "sem.andor",
    "sem.atdebug",
];

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
    wavelength_nm: Parameter<f64>,
    grating: Parameter<Grating>,
    slit_width_um: Parameter<f64>,
    flipper_mirror: Parameter<FlipperMirror>,

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
        let info = crate::ffi_timeout::ffi_call(
            move || Self::init_hardware(device_index),
            crate::ffi_timeout::FFI_INIT_TIMEOUT,
            "AndorSpectrograph::init_hardware",
        )
        .await??;

        #[cfg(not(feature = "spectrograph"))]
        let info = Self::mock_spectrograph_info(device_index);

        // Create parameters with descriptive metadata
        #[allow(unused_mut)]
        let mut wavelength_nm = Parameter::new("wavelength_nm", 310.0)
            .with_unit("nm")
            .with_description("Center wavelength");

        #[allow(unused_mut)]
        let mut grating =
            Parameter::new("grating", Grating::Grating2).with_description("Active grating");

        #[allow(unused_mut)]
        let mut slit_width_um = Parameter::new("slit_width_um", 150.0)
            .with_unit("µm")
            .with_description("Slit width");

        #[allow(unused_mut)]
        let mut flipper_mirror = Parameter::new("flipper_mirror", FlipperMirror::Direct)
            .with_description("Flipper mirror position");

        // Connect hardware callbacks when SDK is available
        #[cfg(feature = "spectrograph")]
        {
            Self::attach_wavelength_callback(&mut wavelength_nm, device_index);
            Self::attach_grating_callback(&mut grating, device_index);
            Self::attach_wavelength_reader(&mut wavelength_nm, device_index);
            Self::attach_slit_width_callback(&mut slit_width_um, device_index);
            Self::attach_flipper_mirror_callback(&mut flipper_mirror, device_index);
        }

        // Register all parameters for GUI/API exposure
        let mut params = ParameterSet::new();
        params.register(wavelength_nm.clone());
        params.register(grating.clone());
        params.register(slit_width_um.clone());
        params.register(flipper_mirror.clone());

        let inner = Arc::new(AndorSpectrographInner {
            handle: device_index,
            info,
            shutter_open: AtomicBool::new(false),
            wavelength_nm,
            grating,
            slit_width_um,
            flipper_mirror,
            calibration: Mutex::new(None),
            params,
        });

        Ok(Self { inner })
    }

    #[cfg(feature = "spectrograph")]
    fn init_hardware(device_index: i32) -> Result<SpectrographInfo> {
        let _guard = match SHAMROCK_INIT_MUTEX.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("Shamrock init mutex poisoned during recovery, proceeding");
                poisoned.into_inner()
            }
        };

        // SAFETY: `init_hardware_once` encapsulates the raw Shamrock startup sequence; access is
        // serialized by `SHAMROCK_INIT_MUTEX`, and this caller handles cleanup/error propagation.
        match unsafe { Self::init_hardware_once(device_index) } {
            Ok(info) => Ok(info),
            Err(err) => {
                let err_text = err.to_string();
                let Some(andor_error) = err.downcast_ref::<AndorError>() else {
                    return Err(err);
                };
                if !matches!(
                    andor_error,
                    AndorError::SdkError { code, .. } if *code == 20201
                ) || !err_text.contains("ShamrockInitialize")
                {
                    return Err(err);
                }

                let cleanup_report = crate::cleanup_runtime_artifacts(
                    SHAMROCK_RECOVERY_ROOTS,
                    SHAMROCK_RECOVERY_PREFIXES,
                );
                tracing::warn!(
                    error = %andor_error,
                    removed = cleanup_report.removed_count(),
                    failed = cleanup_report.failed_count(),
                    artifacts = ?cleanup_report,
                    "Shamrock init reported a stale runtime conflict; cleaning artifacts and retrying once"
                );

                // SAFETY: This is the same serialized startup path as above, retried once after
                // cleaning stale runtime artifacts reported by the SDK.
                unsafe { Self::init_hardware_once(device_index) }.map_err(|retry_err| {
                    retry_err.context(format!(
                        "Shamrock recovery retry failed after cleanup ({})",
                        cleanup_report.summary()
                    ))
                })
            }
        }
    }

    #[cfg(feature = "spectrograph")]
    unsafe fn init_hardware_once(device_index: i32) -> Result<SpectrographInfo> {
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
                    return Err(e.context("ShamrockInitialize failed"));
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
                return Err(e.context("ShamrockGetNumberDevices failed"));
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
        self.inner.grating.set(grating).await?;
        tracing::info!("Grating set to {}", grating_index);
        Ok(())
    }

    /// Get active grating index
    pub async fn get_grating(&self) -> Result<i32> {
        Ok(self.inner.grating.get() as i32)
    }

    /// Get grating information (lines/mm, blaze wavelength)
    pub async fn get_grating_info(&self, grating_index: i32) -> Result<GratingInfo> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            crate::ffi_timeout::ffi_call(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization. All output parameters
                // (lines, blaze, home, offset) are stack-allocated with valid pointers.
                // ShamrockGetGratingInfo writes to these locations.
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
            }, crate::ffi_timeout::FFI_QUERY_TIMEOUT, "get_grating_info")
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
        // Update parameter (triggers hardware callback for default port 2)
        self.inner.slit_width_um.set(width_um).await?;

        // If a non-default port is specified, also write to that port directly
        #[cfg(feature = "spectrograph")]
        if port != 2 {
            let handle = self.inner.handle;
            crate::ffi_timeout::ffi_call(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // port and width_um are validated by the SDK (will return error if invalid).
                unsafe {
                    let ret = ShamrockSetAutoSlitWidth(handle, port, width_um as f32);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            }, crate::ffi_timeout::FFI_MOTION_TIMEOUT, "set_slit_width")
            .await??;
        }

        tracing::info!("Slit width set to {}µm (port {})", width_um, port);
        Ok(())
    }

    /// Get slit width in micrometers
    pub async fn get_slit_width(&self, port: i32) -> Result<f64> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            crate::ffi_timeout::ffi_call(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // width is a stack-allocated f32 with a valid pointer.
                // ShamrockGetAutoSlitWidth writes the slit width to this location.
                unsafe {
                    let mut width = 0.0;
                    let ret = ShamrockGetAutoSlitWidth(handle, port, &mut width);
                    sdk_result(ret)?;
                    Ok(width as f64)
                }
            }, crate::ffi_timeout::FFI_QUERY_TIMEOUT, "get_slit_width")
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(self.inner.slit_width_um.get())
    }

    /// Set flipper mirror position
    pub async fn set_flipper_mirror(&self, port: i32, position: FlipperMirror) -> Result<()> {
        // Update parameter (triggers hardware callback for default port 1)
        self.inner.flipper_mirror.set(position).await?;

        // If a non-default port is specified, also write to that port directly
        #[cfg(feature = "spectrograph")]
        if port != 1 {
            let handle = self.inner.handle;
            let pos = position as i32;
            crate::ffi_timeout::ffi_call(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // pos is a valid FlipperMirror enum value cast to i32.
                unsafe {
                    let ret = ShamrockSetFlipperMirror(handle, port, pos);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            }, crate::ffi_timeout::FFI_CONFIG_TIMEOUT, "set_flipper_mirror")
            .await??;
        }

        tracing::info!("Flipper mirror set to {:?} (port {})", position, port);
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
            crate::ffi_timeout::ffi_call(move || {
                use crate::error::sdk_result;
                // SAFETY: handle is valid from initialization.
                // wavelengths is a heap-allocated Vec with num_pixels elements.
                // ShamrockGetCalibration writes wavelength values to this buffer.
                // The buffer size matches num_pixels, preventing buffer overflow.
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
            }, crate::ffi_timeout::FFI_QUERY_TIMEOUT, "get_wavelength_calibration")
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        {
            // Mock linear calibration
            let center = self.inner.wavelength_nm.get();
            let dispersion = 0.05; // nm per pixel
            let wavelengths: Vec<f64> = (0..num_pixels)
                .map(|i| center + (f64::from(i) - f64::from(num_pixels) / 2.0) * dispersion)
                .collect();

            Ok(WavelengthCalibration::new(wavelengths))
        }
    }

    // ── Wavelength limits per grating [bd-p1zz] ──────────────────────────

    /// Get wavelength limits for a specific grating.
    ///
    /// Returns the minimum and maximum center wavelength allowed by the SDK
    /// for the given grating, accounting for its line density and blaze angle.
    ///
    /// # Arguments
    ///
    /// * `grating_index` - Grating number (1-3)
    pub async fn get_wavelength_limits(&self, grating_index: i32) -> Result<WavelengthLimits> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let mut min: f32 = 0.0;
                    let mut max: f32 = 0.0;
                    let ret =
                        ShamrockGetWavelengthLimits(handle, grating_index, &mut min, &mut max);
                    sdk_result(ret)?;
                    Ok(WavelengthLimits {
                        min_nm: min as f64,
                        max_nm: max as f64,
                    })
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(WavelengthLimits {
            min_nm: 200.0,
            max_nm: 1000.0,
        })
    }

    // ── Detector offset [bd-1rfx] ──────────────────────────────────────

    /// Get detector offset in pixels.
    ///
    /// The detector offset compensates for physical misalignment between the
    /// detector and the spectrograph focal plane.
    pub async fn get_detector_offset(&self) -> Result<i32> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let mut offset: i32 = 0;
                    let ret = ShamrockGetDetectorOffset(handle, &mut offset);
                    sdk_result(ret)?;
                    Ok(offset)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(0)
    }

    /// Set detector offset in pixels.
    ///
    /// # Arguments
    ///
    /// * `offset` - Offset in pixels (positive shifts toward red)
    pub async fn set_detector_offset(&self, offset: i32) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let ret = ShamrockSetDetectorOffset(handle, offset);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        tracing::info!("Detector offset set to {} pixels", offset);
        Ok(())
    }

    // ── Filter wheel control [bd-8n55] ─────────────────────────────────

    /// Check if a filter wheel is installed.
    pub async fn filter_is_present(&self) -> Result<bool> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || unsafe {
                let mut present: i32 = 0;
                let ret = ShamrockFilterIsPresent(handle, &mut present);
                if ret == SHAMROCK_SUCCESS {
                    Ok(present != 0)
                } else {
                    Ok(false)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(true) // Mock has filter wheel
    }

    /// Get current filter wheel position.
    pub async fn get_filter(&self) -> Result<FilterPosition> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let mut filter: i32 = 0;
                    let ret = ShamrockGetFilter(handle, &mut filter);
                    sdk_result(ret)?;
                    Ok(FilterPosition(filter))
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(FilterPosition(1))
    }

    /// Set filter wheel position.
    ///
    /// # Arguments
    ///
    /// * `position` - Filter position (1-indexed, typically 1-6)
    pub async fn set_filter(&self, position: FilterPosition) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            let pos = position.0;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let ret = ShamrockSetFilter(handle, pos);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        tracing::info!("Filter wheel set to position {}", position);
        Ok(())
    }

    /// Get description string for a filter position.
    ///
    /// # Arguments
    ///
    /// * `position` - Filter position (1-indexed)
    pub async fn get_filter_info(&self, position: FilterPosition) -> Result<String> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            let pos = position.0;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let mut buffer = vec![0i8; 256];
                    let ret = ShamrockGetFilterInfo(handle, pos, buffer.as_mut_ptr());
                    sdk_result(ret)?;
                    let bytes: Vec<u8> = buffer.iter().map(|&b| b as u8).collect();
                    Ok(String::from_utf8_lossy(&bytes)
                        .trim_end_matches('\0')
                        .to_string())
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(format!("Mock Filter {}", position.0))
    }

    // ── Focus mirror control [bd-2in1] ─────────────────────────────────

    /// Check if a motorized focus mirror is installed.
    pub async fn focus_mirror_is_present(&self) -> Result<bool> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || unsafe {
                let mut present: i32 = 0;
                let ret = ShamrockFocusMirrorIsPresent(handle, &mut present);
                if ret == SHAMROCK_SUCCESS {
                    Ok(present != 0)
                } else {
                    Ok(false)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(true) // Mock has focus mirror
    }

    /// Get current focus mirror position in steps.
    pub async fn get_focus_mirror(&self) -> Result<i32> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let mut focus: i32 = 0;
                    let ret = ShamrockGetFocusMirror(handle, &mut focus);
                    sdk_result(ret)?;
                    Ok(focus)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(0)
    }

    /// Set focus mirror position in steps.
    ///
    /// # Arguments
    ///
    /// * `focus` - Focus mirror position in steps (0 to max_steps)
    pub async fn set_focus_mirror(&self, focus: i32) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let ret = ShamrockSetFocusMirror(handle, focus);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        tracing::info!("Focus mirror set to step {}", focus);
        Ok(())
    }

    /// Get maximum number of focus mirror steps.
    pub async fn get_focus_mirror_max_steps(&self) -> Result<i32> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let mut steps: i32 = 0;
                    let ret = ShamrockGetFocusMirrorMaxSteps(handle, &mut steps);
                    sdk_result(ret)?;
                    Ok(steps)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        Ok(1000)
    }

    // ── Multi-port slit management [bd-2qd1] ──────────────────────────

    /// Check if a specific auto-slit port is present.
    ///
    /// # Arguments
    ///
    /// * `port` - Slit port to check
    pub async fn auto_slit_is_present(&self, port: SlitPort) -> Result<bool> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            let port_id = port as i32;
            tokio::task::spawn_blocking(move || unsafe {
                let mut present: i32 = 0;
                let ret = ShamrockAutoSlitIsPresent(handle, port_id, &mut present);
                if ret == SHAMROCK_SUCCESS {
                    Ok(present != 0)
                } else {
                    Ok(false)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        {
            let _ = port;
            Ok(true) // Mock has all slit ports
        }
    }

    /// Set slit width for a specific port.
    ///
    /// # Arguments
    ///
    /// * `port` - Slit port to configure
    /// * `width_um` - Slit width in micrometers
    pub async fn set_slit_width_port(&self, port: SlitPort, width_um: f64) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            let port_id = port as i32;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let ret = ShamrockSetAutoSlitWidth(handle, port_id, width_um as f32);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        tracing::info!("Slit {} set to {}µm", port, width_um);
        Ok(())
    }

    /// Get slit width for a specific port.
    ///
    /// # Arguments
    ///
    /// * `port` - Slit port to query
    pub async fn get_slit_width_port(&self, port: SlitPort) -> Result<f64> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            let port_id = port as i32;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let mut width: f32 = 0.0;
                    let ret = ShamrockGetAutoSlitWidth(handle, port_id, &mut width);
                    sdk_result(ret)?;
                    Ok(width as f64)
                }
            })
            .await?
        }

        #[cfg(not(feature = "spectrograph"))]
        {
            let _ = port;
            Ok(self.inner.slit_width_um.get())
        }
    }

    /// Reset a slit port to its home position.
    ///
    /// # Arguments
    ///
    /// * `port` - Slit port to reset
    pub async fn reset_slit(&self, port: SlitPort) -> Result<()> {
        #[cfg(feature = "spectrograph")]
        {
            let handle = self.inner.handle;
            let port_id = port as i32;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let ret = ShamrockAutoSlitReset(handle, port_id);
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        tracing::info!("Slit {} reset to home", port);
        Ok(())
    }

    // --- Hardware callback attachment methods ---

    #[cfg(feature = "spectrograph")]
    fn attach_wavelength_callback(param: &mut Parameter<f64>, handle: i32) {
        param.connect_to_hardware_write(move |val: f64| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    use crate::error::sdk_result;
                    // SAFETY: handle is valid from initialization.
                    // val is cast to f32 for the SDK API.
                    // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
                    // It does not serialize concurrent calls.
                    unsafe {
                        let ret = ShamrockSetWavelength(handle, val as f32);
                        sdk_result(ret)?;
                        Ok::<(), anyhow::Error>(())
                    }
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "spectrograph")]
    fn attach_grating_callback(param: &mut Parameter<Grating>, handle: i32) {
        param.connect_to_hardware_write(move |grating: Grating| {
            Box::pin(async move {
                let grating_index = grating as i32;
                tokio::task::spawn_blocking(move || {
                    use crate::error::sdk_result;
                    // SAFETY: handle is valid from initialization.
                    // grating_index is a valid Grating enum value cast to i32.
                    // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
                    // It does not serialize concurrent calls.
                    unsafe {
                        let ret = ShamrockSetGrating(handle, grating_index);
                        sdk_result(ret)?;
                        Ok::<(), anyhow::Error>(())
                    }
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "spectrograph")]
    fn attach_wavelength_reader(param: &mut Parameter<f64>, handle: i32) {
        param.connect_to_hardware_read(move || {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    use crate::error::sdk_result;
                    // SAFETY: handle is valid from initialization.
                    // wavelength is a stack-allocated f32 with a valid pointer.
                    // ShamrockGetWavelength writes the current wavelength to this location.
                    // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
                    // It does not serialize concurrent calls.
                    unsafe {
                        let mut wavelength: f32 = 0.0;
                        let ret = ShamrockGetWavelength(handle, &mut wavelength);
                        sdk_result(ret)?;
                        Ok::<f64, anyhow::Error>(wavelength as f64)
                    }
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "spectrograph")]
    fn attach_slit_width_callback(param: &mut Parameter<f64>, handle: i32) {
        param.connect_to_hardware_write(move |width_um: f64| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    use crate::error::sdk_result;
                    // SAFETY: handle is valid from initialization.
                    // Bound to default port 2 (Input Direct).
                    // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
                    // It does not serialize concurrent calls.
                    unsafe {
                        let ret = ShamrockSetAutoSlitWidth(handle, 2, width_um as f32);
                        sdk_result(ret)?;
                        Ok::<(), anyhow::Error>(())
                    }
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "spectrograph")]
    fn attach_flipper_mirror_callback(param: &mut Parameter<FlipperMirror>, handle: i32) {
        param.connect_to_hardware_write(move |position: FlipperMirror| {
            Box::pin(async move {
                let pos = position as i32;
                tokio::task::spawn_blocking(move || {
                    use crate::error::sdk_result;
                    // SAFETY: handle is valid from initialization.
                    // Bound to default port 1.
                    // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
                    // It does not serialize concurrent calls.
                    unsafe {
                        let ret = ShamrockSetFlipperMirror(handle, 1, pos);
                        sdk_result(ret)?;
                        Ok::<(), anyhow::Error>(())
                    }
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }
}

// Implement capability traits

#[async_trait]
impl WavelengthTunable for AndorSpectrograph {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.inner.wavelength_nm.set(wavelength_nm).await?;
        tracing::info!("Wavelength set to {} nm", wavelength_nm);
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        #[cfg(feature = "spectrograph")]
        self.inner.wavelength_nm.read_from_hardware().await?;

        Ok(self.inner.wavelength_nm.get())
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
                // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
                // It does not serialize concurrent calls.
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
                // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
                // It does not serialize concurrent calls.
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

#[async_trait]
impl common::capabilities::SpectrometerControl for AndorSpectrograph {
    async fn set_grating(&self, grating_num: i32) -> anyhow::Result<()> {
        AndorSpectrograph::set_grating(self, grating_num).await
    }

    async fn get_grating(&self) -> anyhow::Result<i32> {
        AndorSpectrograph::get_grating(self).await
    }

    async fn set_wavelength(&self, nm: f64) -> anyhow::Result<()> {
        use common::capabilities::WavelengthTunable;
        WavelengthTunable::set_wavelength(self, nm).await
    }

    async fn get_wavelength(&self) -> anyhow::Result<f64> {
        use common::capabilities::WavelengthTunable;
        WavelengthTunable::get_wavelength(self).await
    }

    async fn set_slit_width(&self, slit_id: i32, width_um: f64) -> anyhow::Result<()> {
        AndorSpectrograph::set_slit_width(self, slit_id, width_um).await
    }

    async fn get_calibration(&self, num_pixels: usize) -> anyhow::Result<Vec<f64>> {
        #[allow(clippy::cast_possible_truncation)]
        let cal = self.get_wavelength_calibration(num_pixels as u32).await?;
        Ok(cal.wavelengths_nm)
    }

    async fn is_at_zero_order(&self) -> anyhow::Result<bool> {
        // Zero-order: wavelength set to 0 nm
        use common::capabilities::WavelengthTunable;
        let wl = WavelengthTunable::get_wavelength(self).await?;
        Ok(wl.abs() < f64::EPSILON)
    }

    async fn set_shutter(&self, _open: bool) -> anyhow::Result<()> {
        anyhow::bail!("Shutter control not supported on Shamrock spectrographs")
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
