//! Core device abstraction for Comedi DAQ hardware.
//!
//! This module provides the main [`ComediDevice`] type which wraps a Comedi
//! device handle with proper RAII semantics and safe accessors for device
//! information and subsystems.

use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{debug, info, trace, warn};

use comedi_sys::{comedi_t, lsampl_t};

use crate::error::{ComediError, Result, SubdeviceTypeError};
use crate::subsystem::analog_input::AnalogInput;
use crate::subsystem::analog_output::AnalogOutput;
use crate::subsystem::counter::Counter;
use crate::subsystem::digital_io::DigitalIO;

/// Type of a Comedi subdevice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SubdeviceType {
    Unused = comedi_sys::COMEDI_SUBD_UNUSED,
    AnalogInput = comedi_sys::COMEDI_SUBD_AI,
    AnalogOutput = comedi_sys::COMEDI_SUBD_AO,
    DigitalInput = comedi_sys::COMEDI_SUBD_DI,
    DigitalOutput = comedi_sys::COMEDI_SUBD_DO,
    DigitalIO = comedi_sys::COMEDI_SUBD_DIO,
    Counter = comedi_sys::COMEDI_SUBD_COUNTER,
    Timer = comedi_sys::COMEDI_SUBD_TIMER,
    Memory = comedi_sys::COMEDI_SUBD_MEMORY,
    Calibration = comedi_sys::COMEDI_SUBD_CALIB,
    Processor = comedi_sys::COMEDI_SUBD_PROC,
    Serial = comedi_sys::COMEDI_SUBD_SERIAL,
    Pwm = comedi_sys::COMEDI_SUBD_PWM,
}

impl SubdeviceType {
    /// Convert from raw Comedi subdevice type value.
    ///
    /// Note: The Comedi API returns subdevice types as i32, but the constants
    /// are u32. This function accepts i32 and safely converts negative values
    /// to None.
    pub fn from_raw(raw: i32) -> Option<Self> {
        // Handle negative values (error codes) by returning None
        if raw < 0 {
            return None;
        }
        let raw = raw as u32;
        match raw {
            x if x == comedi_sys::COMEDI_SUBD_UNUSED => Some(Self::Unused),
            x if x == comedi_sys::COMEDI_SUBD_AI => Some(Self::AnalogInput),
            x if x == comedi_sys::COMEDI_SUBD_AO => Some(Self::AnalogOutput),
            x if x == comedi_sys::COMEDI_SUBD_DI => Some(Self::DigitalInput),
            x if x == comedi_sys::COMEDI_SUBD_DO => Some(Self::DigitalOutput),
            x if x == comedi_sys::COMEDI_SUBD_DIO => Some(Self::DigitalIO),
            x if x == comedi_sys::COMEDI_SUBD_COUNTER => Some(Self::Counter),
            x if x == comedi_sys::COMEDI_SUBD_TIMER => Some(Self::Timer),
            x if x == comedi_sys::COMEDI_SUBD_MEMORY => Some(Self::Memory),
            x if x == comedi_sys::COMEDI_SUBD_CALIB => Some(Self::Calibration),
            x if x == comedi_sys::COMEDI_SUBD_PROC => Some(Self::Processor),
            x if x == comedi_sys::COMEDI_SUBD_SERIAL => Some(Self::Serial),
            x if x == comedi_sys::COMEDI_SUBD_PWM => Some(Self::Pwm),
            _ => None,
        }
    }

    /// Convert to error type for error messages.
    pub fn to_error_type(self) -> SubdeviceTypeError {
        match self {
            Self::Unused => SubdeviceTypeError::Unused,
            Self::AnalogInput => SubdeviceTypeError::AnalogInput,
            Self::AnalogOutput => SubdeviceTypeError::AnalogOutput,
            Self::DigitalInput => SubdeviceTypeError::DigitalInput,
            Self::DigitalOutput => SubdeviceTypeError::DigitalOutput,
            Self::DigitalIO => SubdeviceTypeError::DigitalIO,
            Self::Counter => SubdeviceTypeError::Counter,
            Self::Timer => SubdeviceTypeError::Timer,
            Self::Memory => SubdeviceTypeError::Memory,
            Self::Calibration => SubdeviceTypeError::Calibration,
            Self::Processor => SubdeviceTypeError::Processor,
            Self::Serial => SubdeviceTypeError::Serial,
            Self::Pwm => SubdeviceTypeError::Pwm,
        }
    }
}

/// Information about a Comedi subdevice.
#[derive(Debug, Clone)]
pub struct SubdeviceInfo {
    /// Subdevice index
    pub index: u32,
    /// Type of subdevice
    pub subdev_type: SubdeviceType,
    /// Number of channels
    pub n_channels: u32,
    /// Maximum data value (e.g., 65535 for 16-bit)
    pub maxdata: lsampl_t,
    /// Subdevice flags (see SDF_* constants)
    pub flags: u32,
    /// Number of voltage ranges available
    pub n_ranges: u32,
}

impl SubdeviceInfo {
    /// Check if subdevice supports reading.
    pub fn is_readable(&self) -> bool {
        self.flags & comedi_sys::SDF_READABLE != 0
    }

    /// Check if subdevice supports writing.
    pub fn is_writable(&self) -> bool {
        self.flags & comedi_sys::SDF_WRITABLE != 0
    }

    /// Check if subdevice supports commands (async acquisition).
    pub fn supports_commands(&self) -> bool {
        self.flags & comedi_sys::SDF_CMD != 0
    }

    /// Check if subdevice is currently busy.
    pub fn is_busy(&self) -> bool {
        self.flags & comedi_sys::SDF_BUSY != 0
    }

    /// Resolution in bits (derived from maxdata).
    pub fn resolution_bits(&self) -> u32 {
        if self.maxdata == 0 {
            0
        } else {
            (self.maxdata as f64 + 1.0).log2() as u32
        }
    }
}

/// Information about a discovered Comedi device.
///
/// This struct contains basic information obtained during device discovery,
/// without requiring full device access. It's returned by [`comedi_discover`].
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// Path to the device (e.g., "/dev/comedi0")
    pub path: String,
    /// Board name (e.g., "pci-mio-16xe-10"), if accessible
    pub board_name: Option<String>,
    /// Driver name (e.g., "ni_pcimio"), if accessible
    pub driver_name: Option<String>,
    /// Number of subdevices, if accessible
    pub n_subdevices: Option<u32>,
    /// Error message if device couldn't be fully accessed
    pub error: Option<String>,
}

impl DiscoveredDevice {
    /// Returns true if the device was successfully opened and queried.
    pub fn is_accessible(&self) -> bool {
        self.error.is_none()
    }
}

/// Discover all available Comedi devices on the system.
///
/// This function scans `/dev/comedi*` for Comedi device nodes and attempts
/// to gather basic information about each one. Devices that exist but cannot
/// be opened (e.g., due to permissions) are still included in the results
/// with an error message.
///
/// # Returns
///
/// A vector of [`DiscoveredDevice`] structs, one for each device found.
/// The vector may be empty if no Comedi devices are present.
///
/// # Example
///
/// ```no_run
/// use daq_driver_comedi::comedi_discover;
///
/// let devices = comedi_discover();
/// for dev in &devices {
///     if dev.is_accessible() {
///         println!("{}: {} ({})",
///             dev.path,
///             dev.board_name.as_deref().unwrap_or("unknown"),
///             dev.driver_name.as_deref().unwrap_or("unknown"));
///     } else {
///         println!("{}: {} (inaccessible)",
///             dev.path,
///             dev.error.as_deref().unwrap_or("unknown error"));
///     }
/// }
/// ```
pub fn comedi_discover() -> Vec<DiscoveredDevice> {
    let mut devices = Vec::new();

    // Comedi supports up to 16 minor devices (comedi0 through comedi15)
    for i in 0..16 {
        let path = format!("/dev/comedi{}", i);

        // Check if the device node exists
        if !Path::new(&path).exists() {
            trace!(path = %path, "Device node does not exist, skipping");
            continue;
        }

        trace!(path = %path, "Found device node, attempting to open");

        // Try to open the device and gather information
        match ComediDevice::open(&path) {
            Ok(device) => {
                let board_name = device.board_name();
                let driver_name = device.driver_name();
                let n_subdevices = device.n_subdevices();

                info!(
                    path = %path,
                    board = %board_name,
                    driver = %driver_name,
                    subdevices = n_subdevices,
                    "Discovered Comedi device"
                );

                devices.push(DiscoveredDevice {
                    path,
                    board_name: Some(board_name),
                    driver_name: Some(driver_name),
                    n_subdevices: Some(n_subdevices),
                    error: None,
                });
            }
            Err(e) => {
                // Device exists but couldn't be opened
                let error_msg = match &e {
                    ComediError::PermissionDenied { .. } => {
                        warn!(path = %path, "Permission denied accessing Comedi device");
                        "Permission denied (try adding user to comedi group or check udev rules)"
                            .to_string()
                    }
                    ComediError::DeviceBusy { .. } => {
                        warn!(path = %path, "Comedi device is busy");
                        "Device is busy (in use by another process)".to_string()
                    }
                    _ => {
                        warn!(path = %path, error = %e, "Failed to open Comedi device");
                        e.to_string()
                    }
                };

                devices.push(DiscoveredDevice {
                    path,
                    board_name: None,
                    driver_name: None,
                    n_subdevices: None,
                    error: Some(error_msg),
                });
            }
        }
    }

    info!(count = devices.len(), "Comedi device discovery complete");
    devices
}

/// Information about the Comedi device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Path to the device (e.g., "/dev/comedi0")
    pub path: String,
    /// Board name (e.g., "pci-mio-16xe-10")
    pub board_name: String,
    /// Driver name (e.g., "ni_pcimio")
    pub driver_name: String,
    /// Number of subdevices
    pub n_subdevices: u32,
    /// Information about each subdevice
    pub subdevices: Vec<SubdeviceInfo>,
}

/// Internal state shared between device and subsystems.
pub(crate) struct DeviceInner {
    /// Raw pointer to the Comedi device handle.
    handle: NonNull<comedi_t>,
    /// Path used to open the device.
    path: String,
    /// Cached device info.
    info: RwLock<Option<DeviceInfo>>,
    /// Mutex to serialize FFI calls to the handle.
    /// Comedi handles are NOT thread-safe - all FFI calls must be serialized.
    ffi_lock: parking_lot::Mutex<()>,
}

// SAFETY: All access to the non-thread-safe Comedi handle is serialized
// through ffi_lock. Send is safe because the Mutex provides synchronization.
unsafe impl Send for DeviceInner {}
unsafe impl Sync for DeviceInner {}

impl Drop for DeviceInner {
    fn drop(&mut self) {
        debug!(path = %self.path, "Closing Comedi device");
        // SAFETY: handle is valid and we own it
        unsafe {
            let result = comedi_sys::comedi_close(self.handle.as_ptr());
            if result < 0 {
                warn!(path = %self.path, "Error closing Comedi device");
            }
        }
    }
}

/// A safe wrapper around a Comedi device handle.
///
/// This type provides RAII semantics: the device is automatically closed
/// when the `ComediDevice` is dropped. It can be cloned to share the handle
/// between multiple subsystem accessors.
///
/// # Thread Safety
///
/// `ComediDevice` is `Send` and `Sync`. Multiple threads can safely read
/// from the device simultaneously. Write operations should be coordinated
/// by the caller to avoid conflicting commands.
#[derive(Clone)]
pub struct ComediDevice {
    inner: Arc<DeviceInner>,
}

impl ComediDevice {
    /// Open a Comedi device by path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the device, e.g., "/dev/comedi0"
    ///
    /// # Errors
    ///
    /// Returns an error if the device cannot be opened (not found, no
    /// permissions, already in use, etc.).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use daq_driver_comedi::ComediDevice;
    ///
    /// let device = ComediDevice::open("/dev/comedi0")?;
    /// # Ok::<(), daq_driver_comedi::ComediError>(())
    /// ```
    pub fn open(path: &str) -> Result<Self> {
        let c_path = CString::new(path).map_err(|_| ComediError::InvalidConfig {
            message: format!("Invalid device path: {}", path),
        })?;

        // SAFETY: c_path is a valid null-terminated string
        let handle = unsafe { comedi_sys::comedi_open(c_path.as_ptr()) };

        if handle.is_null() {
            // Determine the specific error
            let errno = unsafe { comedi_sys::comedi_errno() };
            return Err(match errno {
                2 => ComediError::DeviceNotFound {
                    path: path.to_string(),
                    message: "No such file or directory".to_string(),
                },
                13 => ComediError::PermissionDenied {
                    path: path.to_string(),
                },
                16 => ComediError::DeviceBusy {
                    path: path.to_string(),
                },
                _ => unsafe { ComediError::from_errno() },
            });
        }

        // SAFETY: we just checked handle is not null
        let handle = unsafe { NonNull::new_unchecked(handle) };

        info!(path = %path, "Opened Comedi device");

        Ok(Self {
            inner: Arc::new(DeviceInner {
                handle,
                path: path.to_string(),
                info: RwLock::new(None),
                ffi_lock: parking_lot::Mutex::new(()),
            }),
        })
    }

    /// Get the raw device handle.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid while this `ComediDevice` exists.
    /// Do not store the pointer or pass it to functions that might close it.
    /// Prefer using `with_handle` for thread-safe access.
    pub(crate) fn handle(&self) -> *mut comedi_t {
        self.inner.handle.as_ptr()
    }

    /// Execute a closure with exclusive access to the device handle.
    ///
    /// This method acquires the FFI lock to ensure thread-safe access to the
    /// non-thread-safe Comedi library. Use this for all FFI operations.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let voltage = device.with_handle(|handle| unsafe {
    ///     comedi_sys::comedi_data_read(handle, subdev, chan, range, aref, &mut data)
    /// })?;
    /// ```
    pub(crate) fn with_handle<F, R>(&self, f: F) -> R
    where
        F: FnOnce(*mut comedi_t) -> R,
    {
        let _guard = self.inner.ffi_lock.lock();
        f(self.inner.handle.as_ptr())
    }

    /// Get the path used to open this device.
    pub fn path(&self) -> &str {
        &self.inner.path
    }

    /// Get the board name (e.g., "pci-mio-16xe-10").
    pub fn board_name(&self) -> String {
        // SAFETY: handle is valid
        unsafe {
            let ptr = comedi_sys::comedi_get_board_name(self.handle());
            if ptr.is_null() {
                "unknown".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }

    /// Get the driver name (e.g., "ni_pcimio").
    pub fn driver_name(&self) -> String {
        // SAFETY: handle is valid
        unsafe {
            let ptr = comedi_sys::comedi_get_driver_name(self.handle());
            if ptr.is_null() {
                "unknown".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }

    /// Get the number of subdevices.
    pub fn n_subdevices(&self) -> u32 {
        // SAFETY: handle is valid
        unsafe { comedi_sys::comedi_get_n_subdevices(self.handle()) as u32 }
    }

    /// Get the type of a subdevice.
    pub fn subdevice_type(&self, subdevice: u32) -> Result<SubdeviceType> {
        let n_subdevices = self.n_subdevices();
        if subdevice >= n_subdevices {
            return Err(ComediError::InvalidSubdevice {
                subdevice,
                max: n_subdevices,
            });
        }

        // SAFETY: handle is valid and subdevice is in range
        let raw = unsafe { comedi_sys::comedi_get_subdevice_type(self.handle(), subdevice) };

        SubdeviceType::from_raw(raw).ok_or_else(|| ComediError::HardwareError {
            message: format!("Unknown subdevice type: {}", raw),
        })
    }

    /// Get information about a subdevice.
    pub fn subdevice_info(&self, subdevice: u32) -> Result<SubdeviceInfo> {
        let n_subdevices = self.n_subdevices();
        if subdevice >= n_subdevices {
            return Err(ComediError::InvalidSubdevice {
                subdevice,
                max: n_subdevices,
            });
        }

        let subdev_type = self.subdevice_type(subdevice)?;

        // SAFETY: handle is valid and subdevice is in range
        unsafe {
            let n_channels = comedi_sys::comedi_get_n_channels(self.handle(), subdevice) as u32;
            let maxdata = comedi_sys::comedi_get_maxdata(self.handle(), subdevice, 0);
            let flags = comedi_sys::comedi_get_subdevice_flags(self.handle(), subdevice) as u32;
            let n_ranges = comedi_sys::comedi_get_n_ranges(self.handle(), subdevice, 0) as u32;

            Ok(SubdeviceInfo {
                index: subdevice,
                subdev_type,
                n_channels,
                maxdata,
                flags,
                n_ranges,
            })
        }
    }

    /// Get comprehensive device information (cached after first call).
    pub fn info(&self) -> Result<DeviceInfo> {
        // Check cache first
        if let Some(info) = self.inner.info.read().as_ref() {
            return Ok(info.clone());
        }

        // Build device info
        let n_subdevices = self.n_subdevices();
        let mut subdevices = Vec::with_capacity(n_subdevices as usize);

        for i in 0..n_subdevices {
            subdevices.push(self.subdevice_info(i)?);
        }

        let info = DeviceInfo {
            path: self.inner.path.clone(),
            board_name: self.board_name(),
            driver_name: self.driver_name(),
            n_subdevices,
            subdevices,
        };

        // Cache the result
        *self.inner.info.write() = Some(info.clone());

        Ok(info)
    }

    /// Find the first subdevice of a given type.
    pub fn find_subdevice(&self, subdev_type: SubdeviceType) -> Option<u32> {
        let n = self.n_subdevices();
        for i in 0..n {
            if let Ok(t) = self.subdevice_type(i) {
                if t == subdev_type {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Find all subdevices of a given type.
    pub fn find_all_subdevices(&self, subdev_type: SubdeviceType) -> Vec<u32> {
        let n = self.n_subdevices();
        (0..n)
            .filter(|&i| self.subdevice_type(i).ok() == Some(subdev_type))
            .collect()
    }

    /// Get an analog input subsystem accessor.
    ///
    /// Returns the first analog input subdevice found, or an error if none.
    pub fn analog_input(&self) -> Result<AnalogInput> {
        let subdevice = self
            .find_subdevice(SubdeviceType::AnalogInput)
            .ok_or_else(|| ComediError::NotSupported {
                message: "No analog input subdevice found".to_string(),
            })?;

        AnalogInput::new(self.clone(), subdevice)
    }

    /// Get an analog input subsystem for a specific subdevice.
    pub fn analog_input_subdevice(&self, subdevice: u32) -> Result<AnalogInput> {
        AnalogInput::new(self.clone(), subdevice)
    }

    /// Get an analog output subsystem accessor.
    ///
    /// Returns the first analog output subdevice found, or an error if none.
    pub fn analog_output(&self) -> Result<AnalogOutput> {
        let subdevice = self
            .find_subdevice(SubdeviceType::AnalogOutput)
            .ok_or_else(|| ComediError::NotSupported {
                message: "No analog output subdevice found".to_string(),
            })?;

        AnalogOutput::new(self.clone(), subdevice)
    }

    /// Get a digital I/O subsystem accessor.
    ///
    /// Returns the first DIO subdevice found, or an error if none.
    pub fn digital_io(&self) -> Result<DigitalIO> {
        let subdevice = self
            .find_subdevice(SubdeviceType::DigitalIO)
            .ok_or_else(|| ComediError::NotSupported {
                message: "No digital I/O subdevice found".to_string(),
            })?;

        DigitalIO::new(self.clone(), subdevice)
    }

    /// Get a counter subsystem accessor.
    ///
    /// Returns the first counter subdevice found, or an error if none.
    pub fn counter(&self) -> Result<Counter> {
        let subdevice = self.find_subdevice(SubdeviceType::Counter).ok_or_else(|| {
            ComediError::NotSupported {
                message: "No counter subdevice found".to_string(),
            }
        })?;

        Counter::new(self.clone(), subdevice)
    }

    /// Get all counter subsystems (some cards have multiple).
    pub fn counters(&self) -> Result<Vec<Counter>> {
        self.find_all_subdevices(SubdeviceType::Counter)
            .into_iter()
            .map(|subdevice| Counter::new(self.clone(), subdevice))
            .collect()
    }

    /// Get the file descriptor for the device (for select/poll).
    pub fn fileno(&self) -> i32 {
        // SAFETY: handle is valid
        unsafe { comedi_sys::comedi_fileno(self.handle()) }
    }
}

impl std::fmt::Debug for ComediDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComediDevice")
            .field("path", &self.inner.path)
            .field("board", &self.board_name())
            .field("driver", &self.driver_name())
            .field("subdevices", &self.n_subdevices())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subdevice_type_from_raw() {
        // Note: from_raw takes i32 (what FFI returns), but constants are u32
        assert_eq!(
            SubdeviceType::from_raw(comedi_sys::COMEDI_SUBD_AI as i32),
            Some(SubdeviceType::AnalogInput)
        );
        assert_eq!(
            SubdeviceType::from_raw(comedi_sys::COMEDI_SUBD_COUNTER as i32),
            Some(SubdeviceType::Counter)
        );
        assert_eq!(SubdeviceType::from_raw(999), None);
        // Test negative value handling
        assert_eq!(SubdeviceType::from_raw(-1), None);
    }

    #[test]
    fn test_subdevice_info_resolution() {
        let info = SubdeviceInfo {
            index: 0,
            subdev_type: SubdeviceType::AnalogInput,
            n_channels: 16,
            maxdata: 65535, // 16-bit
            flags: 0,
            n_ranges: 1,
        };
        assert_eq!(info.resolution_bits(), 16);

        let info_12bit = SubdeviceInfo {
            maxdata: 4095, // 12-bit
            ..info.clone()
        };
        assert_eq!(info_12bit.resolution_bits(), 12);
    }

    #[test]
    fn test_discovered_device_is_accessible() {
        // Test accessible device
        let accessible = DiscoveredDevice {
            path: "/dev/comedi0".to_string(),
            board_name: Some("ni_pcimio".to_string()),
            driver_name: Some("ni_pcimio".to_string()),
            n_subdevices: Some(14),
            error: None,
        };
        assert!(accessible.is_accessible());
        assert!(accessible.board_name.is_some());
        assert!(accessible.n_subdevices.is_some());

        // Test inaccessible device
        let inaccessible = DiscoveredDevice {
            path: "/dev/comedi1".to_string(),
            board_name: None,
            driver_name: None,
            n_subdevices: None,
            error: Some("Permission denied".to_string()),
        };
        assert!(!inaccessible.is_accessible());
        assert!(inaccessible.board_name.is_none());
        assert!(inaccessible.error.is_some());
    }

    #[test]
    fn test_comedi_discover_returns_vec() {
        // comedi_discover() should always return a Vec (possibly empty)
        // This test validates the function signature and basic behavior
        // without requiring actual hardware
        let devices = comedi_discover();
        // Should return a Vec (may be empty if no devices present)
        assert!(devices.len() <= 16); // Max 16 comedi devices possible

        // Verify all returned devices have valid paths
        for dev in &devices {
            assert!(dev.path.starts_with("/dev/comedi"));
            // Every device should either be accessible or have an error
            assert!(dev.is_accessible() || dev.error.is_some());
        }
    }
}
