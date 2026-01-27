//! Calibration support for Comedi DAQ devices.
//!
//! This module provides calibration management for NI DAQ boards that store
//! factory calibration data in onboard EEPROM. The calibration system supports:
//!
//! - Loading calibration from calibration files (software calibration)
//! - Applying polynomial calibration to ADC/DAC conversions
//! - User calibration storage and management
//! - Calibration status reporting
//!
//! # Calibration Types
//!
//! NI DAQ boards support two calibration approaches:
//!
//! ## Hardware Calibration (Legacy E-Series)
//!
//! Older E-series boards (like PCI-MIO-16XE-10) use hardware calibration where
//! the calibration is applied in hardware via on-board DACs. The comedilib
//! `comedi_calibrate` utility configures these DACs.
//!
//! ## Software Calibration (M-Series and newer)
//!
//! M-series and newer boards use software calibration where correction
//! polynomials are stored in EEPROM and applied in software during data
//! conversion. The `comedi_soft_calibrate` utility generates calibration files.
//!
//! # Polynomial Calibration
//!
//! Calibration is applied using polynomials of the form:
//!
//! ```text
//! physical_value = sum(coefficient[i] * (raw_value - expansion_origin)^i)
//! ```
//!
//! This allows correction for:
//! - Offset errors (constant term)
//! - Gain errors (linear term)
//! - Nonlinearity (higher order terms)
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_driver_comedi::calibration::{CalibrationManager, CalibrationPolynomial};
//!
//! // Load calibration from default file
//! let manager = CalibrationManager::load(&device)?;
//!
//! // Get converter for a specific channel/range
//! let converter = manager.get_converter(subdevice, channel, range, Direction::ToPhysical)?;
//!
//! // Apply calibration to raw value
//! let raw = 32768_u32;
//! let voltage = converter.apply(raw);
//! ```
//!
//! # Calibration Status
//!
//! The calibration system provides status information:
//!
//! - `CalibrationStatus::Uncalibrated` - No calibration data loaded
//! - `CalibrationStatus::FactoryCalibration` - Using factory calibration
//! - `CalibrationStatus::UserCalibration` - Using user-performed calibration
//! - `CalibrationStatus::Expired` - Calibration may need refresh
//!
//! # GUI Wizard Requirements
//!
//! A GUI calibration wizard would need:
//!
//! 1. **Reference voltage source** - Precision voltage source or known reference
//! 2. **Multi-point measurement** - Measure at several points across range
//! 3. **Polynomial fitting** - Fit correction polynomial to measured errors
//! 4. **Validation** - Verify calibration accuracy with test measurements
//! 5. **Persistence** - Save calibration to file for future sessions

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::device::ComediDevice;
use crate::error::{ComediError, Result};

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of polynomial coefficients supported by comedilib.
pub const MAX_POLYNOMIAL_COEFFICIENTS: usize = 4;

/// Comedi subdevice type for calibration EEPROM.
pub const SUBDEVICE_TYPE_CALIB: u32 = 9;

// =============================================================================
// Polynomial Calibration
// =============================================================================

/// Direction for calibration conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConversionDirection {
    /// Convert raw ADC value to physical units (voltage)
    ToPhysical,
    /// Convert physical units to raw DAC value
    FromPhysical,
}

impl fmt::Display for ConversionDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToPhysical => write!(f, "to_physical"),
            Self::FromPhysical => write!(f, "from_physical"),
        }
    }
}

/// A calibration polynomial for converting between raw and physical values.
///
/// The polynomial is of the form:
/// ```text
/// y = sum(coefficients[i] * (x - expansion_origin)^i) for i in 0..=order
/// ```
///
/// This representation is equivalent to comedilib's `comedi_polynomial_t`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationPolynomial {
    /// Polynomial coefficients, from order 0 (constant) to order N.
    /// Unused coefficients should be 0.0.
    pub coefficients: [f64; MAX_POLYNOMIAL_COEFFICIENTS],

    /// The expansion origin (x value around which polynomial is expanded).
    /// For ADC, this is typically half of maxdata (midpoint).
    pub expansion_origin: f64,

    /// Order of the polynomial (0 = constant, 1 = linear, etc.)
    pub order: u32,
}

impl Default for CalibrationPolynomial {
    fn default() -> Self {
        Self::identity()
    }
}

impl CalibrationPolynomial {
    /// Create an identity polynomial (output = input).
    ///
    /// This is a linear polynomial with coefficient[1] = 1.0, which
    /// maps input directly to output without transformation.
    pub fn identity() -> Self {
        let mut coefficients = [0.0; MAX_POLYNOMIAL_COEFFICIENTS];
        coefficients[0] = 0.0; // Offset
        coefficients[1] = 1.0; // Unity gain
        Self {
            coefficients,
            expansion_origin: 0.0,
            order: 1,
        }
    }

    /// Create a linear calibration polynomial.
    ///
    /// # Arguments
    ///
    /// * `offset` - Constant offset to add
    /// * `gain` - Multiplicative gain factor
    /// * `origin` - Expansion origin (typically 0 for linear)
    ///
    /// # Example
    ///
    /// ```
    /// use daq_driver_comedi::calibration::CalibrationPolynomial;
    ///
    /// // Correct for 0.1V offset and 1.02 gain error
    /// let poly = CalibrationPolynomial::linear(0.1, 1.0 / 1.02, 0.0);
    /// ```
    pub fn linear(offset: f64, gain: f64, origin: f64) -> Self {
        let mut coefficients = [0.0; MAX_POLYNOMIAL_COEFFICIENTS];
        coefficients[0] = offset;
        coefficients[1] = gain;
        Self {
            coefficients,
            expansion_origin: origin,
            order: 1,
        }
    }

    /// Create a polynomial from comedilib range parameters.
    ///
    /// This creates a linear conversion polynomial for basic ADC/DAC
    /// conversion based on the voltage range and resolution.
    ///
    /// # Arguments
    ///
    /// * `min_voltage` - Minimum voltage of the range
    /// * `max_voltage` - Maximum voltage of the range
    /// * `maxdata` - Maximum raw value (2^bits - 1)
    pub fn from_range(min_voltage: f64, max_voltage: f64, maxdata: u32) -> Self {
        let voltage_span = max_voltage - min_voltage;
        let gain = voltage_span / maxdata as f64;

        let mut coefficients = [0.0; MAX_POLYNOMIAL_COEFFICIENTS];
        coefficients[0] = min_voltage;
        coefficients[1] = gain;

        Self {
            coefficients,
            expansion_origin: 0.0,
            order: 1,
        }
    }

    /// Create a polynomial for DAC output conversion.
    ///
    /// This is the inverse of `from_range`, converting voltage to raw values.
    pub fn for_dac(min_voltage: f64, max_voltage: f64, maxdata: u32) -> Self {
        let voltage_span = max_voltage - min_voltage;
        let gain = maxdata as f64 / voltage_span;

        let mut coefficients = [0.0; MAX_POLYNOMIAL_COEFFICIENTS];
        coefficients[0] = -min_voltage * gain;
        coefficients[1] = gain;

        Self {
            coefficients,
            expansion_origin: 0.0,
            order: 1,
        }
    }

    /// Apply the polynomial to convert a raw value.
    ///
    /// Computes: `sum(coefficients[i] * (value - origin)^i)`
    ///
    /// # Arguments
    ///
    /// * `raw_value` - Raw ADC/DAC value to convert
    ///
    /// # Returns
    ///
    /// The converted value (physical for ADC, raw for DAC inverse)
    pub fn apply(&self, raw_value: u32) -> f64 {
        self.apply_f64(raw_value as f64)
    }

    /// Apply the polynomial to a floating-point value.
    ///
    /// This is useful for inverse polynomials or when raw values
    /// need to be represented with fractional precision.
    pub fn apply_f64(&self, value: f64) -> f64 {
        let x = value - self.expansion_origin;

        // Horner's method for efficient polynomial evaluation
        let mut result = 0.0;
        for i in (0..=self.order as usize).rev() {
            result = result * x + self.coefficients[i];
        }

        result
    }

    /// Apply inverse conversion (physical to raw).
    ///
    /// For linear polynomials, this computes the inverse mapping.
    /// For higher-order polynomials, this uses Newton-Raphson iteration.
    ///
    /// # Arguments
    ///
    /// * `physical_value` - Physical value (voltage) to convert
    /// * `maxdata` - Maximum raw value (for clamping)
    ///
    /// # Returns
    ///
    /// Raw DAC value, clamped to [0, maxdata]
    pub fn apply_inverse(&self, physical_value: f64, maxdata: u32) -> u32 {
        let raw = if self.order == 1 && self.coefficients[1].abs() > 1e-10 {
            // Linear case: direct inversion
            let offset = self.coefficients[0];
            let gain = self.coefficients[1];
            (physical_value - offset) / gain + self.expansion_origin
        } else if self.order == 0 {
            // Constant case: no inversion possible
            warn!("Cannot invert constant polynomial");
            physical_value
        } else {
            // Higher order: Newton-Raphson
            self.newton_raphson_inverse(physical_value)
        };

        // Clamp to valid range
        raw.clamp(0.0, maxdata as f64).round() as u32
    }

    /// Newton-Raphson iteration for polynomial inversion.
    fn newton_raphson_inverse(&self, target: f64) -> f64 {
        // Initial guess: assume linear approximation
        let mut x = if self.coefficients[1].abs() > 1e-10 {
            (target - self.coefficients[0]) / self.coefficients[1] + self.expansion_origin
        } else {
            self.expansion_origin
        };

        // Iterate to find x where f(x) = target
        for _ in 0..20 {
            let fx = self.apply_f64(x);
            let dfx = self.derivative_at(x);

            if dfx.abs() < 1e-15 {
                break;
            }

            let new_x = x - (fx - target) / dfx;

            if (new_x - x).abs() < 1e-10 {
                break;
            }

            x = new_x;
        }

        x
    }

    /// Compute the derivative of the polynomial at a point.
    fn derivative_at(&self, value: f64) -> f64 {
        let x = value - self.expansion_origin;

        let mut result = 0.0;
        for i in (1..=self.order as usize).rev() {
            result = result * x + (i as f64) * self.coefficients[i];
        }

        result
    }

    /// Get the order of the polynomial.
    pub fn order(&self) -> u32 {
        self.order
    }

    /// Check if this is an identity (no-op) polynomial.
    pub fn is_identity(&self) -> bool {
        (self.coefficients[0].abs() < 1e-10)
            && ((self.coefficients[1] - 1.0).abs() < 1e-10)
            && (self.order <= 1 || self.coefficients[2..].iter().all(|&c| c.abs() < 1e-10))
    }
}

// =============================================================================
// Calibration Settings
// =============================================================================

/// A single calibration setting for a specific subdevice/channel/range combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationSetting {
    /// Subdevice index this setting applies to
    pub subdevice: u32,

    /// Channels this setting applies to (empty = all channels)
    pub channels: Vec<u32>,

    /// Ranges this setting applies to (empty = all ranges)
    pub ranges: Vec<u32>,

    /// Polynomial for converting raw to physical
    pub to_physical: CalibrationPolynomial,

    /// Polynomial for converting physical to raw
    pub from_physical: CalibrationPolynomial,
}

impl CalibrationSetting {
    /// Check if this setting applies to the given subdevice/channel/range.
    pub fn applies_to(&self, subdevice: u32, channel: u32, range: u32) -> bool {
        if self.subdevice != subdevice {
            return false;
        }

        let channel_matches = self.channels.is_empty() || self.channels.contains(&channel);
        let range_matches = self.ranges.is_empty() || self.ranges.contains(&range);

        channel_matches && range_matches
    }

    /// Get the converter polynomial for the specified direction.
    pub fn get_polynomial(&self, direction: ConversionDirection) -> &CalibrationPolynomial {
        match direction {
            ConversionDirection::ToPhysical => &self.to_physical,
            ConversionDirection::FromPhysical => &self.from_physical,
        }
    }
}

// =============================================================================
// Calibration Data
// =============================================================================

/// Status of the calibration data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalibrationStatus {
    /// No calibration data loaded
    Uncalibrated,

    /// Using factory calibration from EEPROM
    FactoryCalibration,

    /// Using user-performed calibration
    UserCalibration,

    /// Calibration data may be expired (older than recommended)
    Expired,

    /// Calibration failed or is invalid
    Invalid,
}

impl fmt::Display for CalibrationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uncalibrated => write!(f, "Uncalibrated"),
            Self::FactoryCalibration => write!(f, "Factory Calibration"),
            Self::UserCalibration => write!(f, "User Calibration"),
            Self::Expired => write!(f, "Calibration Expired"),
            Self::Invalid => write!(f, "Invalid Calibration"),
        }
    }
}

/// Complete calibration data for a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationData {
    /// Driver name (e.g., "ni_pcimio")
    pub driver_name: String,

    /// Board name (e.g., "pci-mio-16xe-10")
    pub board_name: String,

    /// Calibration settings for different subdevice/channel/range combinations
    pub settings: Vec<CalibrationSetting>,

    /// When the calibration was performed
    pub calibration_date: Option<SystemTime>,

    /// Status of the calibration
    pub status: CalibrationStatus,
}

impl Default for CalibrationData {
    fn default() -> Self {
        Self {
            driver_name: String::new(),
            board_name: String::new(),
            settings: Vec::new(),
            calibration_date: None,
            status: CalibrationStatus::Uncalibrated,
        }
    }
}

impl CalibrationData {
    /// Create empty calibration data for a device.
    pub fn new(driver_name: &str, board_name: &str) -> Self {
        Self {
            driver_name: driver_name.to_string(),
            board_name: board_name.to_string(),
            settings: Vec::new(),
            calibration_date: None,
            status: CalibrationStatus::Uncalibrated,
        }
    }

    /// Find a calibration setting that applies to the given parameters.
    pub fn find_setting(
        &self,
        subdevice: u32,
        channel: u32,
        range: u32,
    ) -> Option<&CalibrationSetting> {
        self.settings
            .iter()
            .find(|s| s.applies_to(subdevice, channel, range))
    }

    /// Add or update a calibration setting.
    pub fn add_setting(&mut self, setting: CalibrationSetting) {
        // Remove any existing setting for the same subdevice
        self.settings.retain(|s| {
            s.subdevice != setting.subdevice
                || s.channels != setting.channels
                || s.ranges != setting.ranges
        });
        self.settings.push(setting);
    }

    /// Check if calibration data is considered expired.
    ///
    /// Calibration is recommended to be refreshed annually for best accuracy.
    pub fn is_expired(&self) -> bool {
        if let Some(date) = self.calibration_date {
            if let Ok(elapsed) = date.elapsed() {
                // Consider expired after 1 year
                return elapsed.as_secs() > 365 * 24 * 60 * 60;
            }
        }
        false
    }
}

// =============================================================================
// Calibration Manager
// =============================================================================

/// Key for caching calibration converters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConverterKey {
    subdevice: u32,
    channel: u32,
    range: u32,
    direction: ConversionDirection,
}

/// Manager for device calibration.
///
/// The CalibrationManager handles:
/// - Loading calibration data from files or EEPROM
/// - Caching calibration polynomials for efficient access
/// - Providing converters for ADC/DAC value transformation
/// - Managing user calibration data
pub struct CalibrationManager {
    /// The device being calibrated
    device: ComediDevice,

    /// Current calibration data
    data: RwLock<CalibrationData>,

    /// Cached converter polynomials
    converters: RwLock<HashMap<ConverterKey, Arc<CalibrationPolynomial>>>,

    /// Path to user calibration file
    user_calibration_path: Option<PathBuf>,
}

impl CalibrationManager {
    /// Create a new calibration manager for a device.
    ///
    /// This creates an uncalibrated manager. Use `load_from_file()` or
    /// `load_default()` to load calibration data.
    pub fn new(device: ComediDevice) -> Self {
        let data = CalibrationData::new(&device.driver_name(), &device.board_name());

        Self {
            device,
            data: RwLock::new(data),
            converters: RwLock::new(HashMap::new()),
            user_calibration_path: None,
        }
    }

    /// Load calibration from the default comedilib calibration file.
    ///
    /// Comedilib stores calibration files in `/var/lib/comedi/calibrations/`.
    /// The filename is based on the board and driver name.
    ///
    /// # Returns
    ///
    /// Returns the calibration status after loading.
    pub fn load_default(&self) -> Result<CalibrationStatus> {
        let path = self.get_default_calibration_path()?;
        self.load_from_file(&path)
    }

    /// Get the default calibration file path for this device.
    pub fn get_default_calibration_path(&self) -> Result<PathBuf> {
        // Comedilib stores calibrations in /var/lib/comedi/calibrations/
        // Format: /var/lib/comedi/calibrations/<driver>_<board>
        let driver = self.device.driver_name();
        let board = self.device.board_name();

        let filename = format!("{}_{}", driver, board);
        let path = PathBuf::from("/var/lib/comedi/calibrations").join(filename);

        Ok(path)
    }

    /// Load calibration data from a file.
    ///
    /// The file format is JSON-serialized `CalibrationData`.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the calibration file
    ///
    /// # Returns
    ///
    /// Returns the calibration status after loading.
    pub fn load_from_file<P: AsRef<Path>>(&self, path: P) -> Result<CalibrationStatus> {
        let path = path.as_ref();

        if !path.exists() {
            info!(
                "Calibration file not found: {}. Using uncalibrated mode.",
                path.display()
            );
            return Ok(CalibrationStatus::Uncalibrated);
        }

        let contents = std::fs::read_to_string(path).map_err(|e| ComediError::IoError {
            message: format!("Failed to read calibration file: {}", e),
        })?;

        let data: CalibrationData =
            serde_json::from_str(&contents).map_err(|e| ComediError::CalibrationError {
                message: format!("Failed to parse calibration file: {}", e),
            })?;

        // Verify calibration matches device
        if data.board_name != self.device.board_name() {
            warn!(
                "Calibration board name '{}' doesn't match device '{}'",
                data.board_name,
                self.device.board_name()
            );
        }

        let mut status = data.status;
        if data.is_expired() {
            warn!("Calibration data is older than recommended refresh period");
            status = CalibrationStatus::Expired;
        }

        info!(
            "Loaded calibration from {}: {} settings, status: {}",
            path.display(),
            data.settings.len(),
            status
        );

        // Clear converter cache
        self.converters.write().clear();

        *self.data.write() = data;
        Ok(status)
    }

    /// Save current calibration data to a file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to save the calibration file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();

        let data = self.data.read();
        let contents =
            serde_json::to_string_pretty(&*data).map_err(|e| ComediError::CalibrationError {
                message: format!("Failed to serialize calibration: {}", e),
            })?;

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ComediError::IoError {
                message: format!("Failed to create calibration directory: {}", e),
            })?;
        }

        std::fs::write(path, contents).map_err(|e| ComediError::IoError {
            message: format!("Failed to write calibration file: {}", e),
        })?;

        info!("Saved calibration to {}", path.display());
        Ok(())
    }

    /// Get the current calibration status.
    pub fn status(&self) -> CalibrationStatus {
        self.data.read().status
    }

    /// Get a converter polynomial for a specific subdevice/channel/range.
    ///
    /// Returns a cached polynomial if available, or creates one based on
    /// the calibration data.
    pub fn get_converter(
        &self,
        subdevice: u32,
        channel: u32,
        range: u32,
        direction: ConversionDirection,
    ) -> Arc<CalibrationPolynomial> {
        let key = ConverterKey {
            subdevice,
            channel,
            range,
            direction,
        };

        // Check cache first
        if let Some(poly) = self.converters.read().get(&key) {
            return poly.clone();
        }

        // Build converter from calibration data
        let poly = self.build_converter(subdevice, channel, range, direction);
        let poly = Arc::new(poly);

        // Cache it
        self.converters.write().insert(key, poly.clone());

        poly
    }

    /// Build a converter polynomial for the given parameters.
    fn build_converter(
        &self,
        subdevice: u32,
        channel: u32,
        range: u32,
        direction: ConversionDirection,
    ) -> CalibrationPolynomial {
        let data = self.data.read();

        // Try to find calibration setting
        if let Some(setting) = data.find_setting(subdevice, channel, range) {
            debug!(
                "Using calibration for subdev={}, ch={}, range={}, direction={}",
                subdevice, channel, range, direction
            );
            return setting.get_polynomial(direction).clone();
        }

        // Fall back to range-based conversion
        debug!(
            "No calibration for subdev={}, ch={}, range={}. Using range-based conversion.",
            subdevice, channel, range
        );

        // Get range info from device
        if let Ok(info) = self.device.subdevice_info(subdevice) {
            // Try to get actual range values
            // For now, use default +-10V range
            let min = -10.0;
            let max = 10.0;
            let maxdata = info.maxdata;

            match direction {
                ConversionDirection::ToPhysical => {
                    CalibrationPolynomial::from_range(min, max, maxdata)
                }
                ConversionDirection::FromPhysical => {
                    CalibrationPolynomial::for_dac(min, max, maxdata)
                }
            }
        } else {
            // Ultimate fallback: identity
            CalibrationPolynomial::identity()
        }
    }

    /// Apply calibration to convert a raw ADC value to physical units.
    ///
    /// # Arguments
    ///
    /// * `raw` - Raw ADC value
    /// * `subdevice` - Subdevice index
    /// * `channel` - Channel number
    /// * `range` - Range index
    ///
    /// # Returns
    ///
    /// Physical value (typically voltage in V)
    pub fn to_physical(&self, raw: u32, subdevice: u32, channel: u32, range: u32) -> f64 {
        let poly = self.get_converter(subdevice, channel, range, ConversionDirection::ToPhysical);
        poly.apply(raw)
    }

    /// Apply calibration to convert physical units to raw DAC value.
    ///
    /// # Arguments
    ///
    /// * `physical` - Physical value (voltage)
    /// * `subdevice` - Subdevice index
    /// * `channel` - Channel number
    /// * `range` - Range index
    /// * `maxdata` - Maximum raw value for clamping
    ///
    /// # Returns
    ///
    /// Raw DAC value, clamped to valid range
    pub fn from_physical(
        &self,
        physical: f64,
        subdevice: u32,
        channel: u32,
        range: u32,
        maxdata: u32,
    ) -> u32 {
        let poly = self.get_converter(subdevice, channel, range, ConversionDirection::FromPhysical);
        poly.apply_inverse(physical, maxdata)
    }

    /// Add a user calibration setting.
    ///
    /// This updates the calibration data with a user-defined setting.
    pub fn add_user_calibration(&self, setting: CalibrationSetting) {
        let mut data = self.data.write();
        data.add_setting(setting);
        data.calibration_date = Some(SystemTime::now());
        data.status = CalibrationStatus::UserCalibration;

        // Clear converter cache
        self.converters.write().clear();
    }

    /// Reset calibration to factory defaults.
    ///
    /// This clears all user calibration and attempts to reload factory calibration.
    pub fn reset_to_factory(&self) -> Result<CalibrationStatus> {
        info!("Resetting calibration to factory defaults");

        // Clear current data
        let mut data = self.data.write();
        *data = CalibrationData::new(&self.device.driver_name(), &self.device.board_name());

        // Clear converter cache
        self.converters.write().clear();

        drop(data);

        // Attempt to load factory calibration
        self.load_default()
    }

    /// Get calibration information for display.
    pub fn info(&self) -> CalibrationInfo {
        let data = self.data.read();
        CalibrationInfo {
            driver_name: data.driver_name.clone(),
            board_name: data.board_name.clone(),
            status: data.status,
            settings_count: data.settings.len(),
            calibration_date: data.calibration_date,
            is_expired: data.is_expired(),
        }
    }

    /// Get the underlying device.
    pub fn device(&self) -> &ComediDevice {
        &self.device
    }

    /// Set the path for user calibration storage.
    pub fn set_user_calibration_path(&mut self, path: PathBuf) {
        self.user_calibration_path = Some(path);
    }

    /// Save user calibration to the configured path.
    pub fn save_user_calibration(&self) -> Result<()> {
        let path =
            self.user_calibration_path
                .as_ref()
                .ok_or_else(|| ComediError::CalibrationError {
                    message: "No user calibration path configured".to_string(),
                })?;

        self.save_to_file(path)
    }
}

/// Calibration information for display.
#[derive(Debug, Clone)]
pub struct CalibrationInfo {
    /// Driver name
    pub driver_name: String,
    /// Board name
    pub board_name: String,
    /// Calibration status
    pub status: CalibrationStatus,
    /// Number of calibration settings
    pub settings_count: usize,
    /// When calibration was performed
    pub calibration_date: Option<SystemTime>,
    /// Whether calibration is considered expired
    pub is_expired: bool,
}

// =============================================================================
// GUI Wizard Requirements (Documentation)
// =============================================================================

/// Requirements for implementing a calibration wizard in the GUI.
///
/// A complete calibration wizard would need the following components:
///
/// ## 1. Reference Voltage Source
///
/// The wizard needs access to known reference voltages. Options:
///
/// - **External precision voltage source**: Most accurate, user provides voltages
/// - **Calibrated signal generator**: Automated sweeping through voltage range
/// - **Known internal reference**: Some boards have onboard references
///
/// ## 2. Measurement Points
///
/// For linear calibration (offset + gain), minimum 2 points needed:
/// - Near minimum voltage (e.g., -9.0V for +-10V range)
/// - Near maximum voltage (e.g., +9.0V for +-10V range)
///
/// For higher-order calibration, additional points at intermediate values.
///
/// ## 3. Data Collection Flow
///
/// ```text
/// 1. User connects reference voltage source
/// 2. For each calibration point:
///    a. Prompt user to set reference to specific voltage
///    b. Wait for user confirmation
///    c. Read multiple samples and average
///    d. Store (expected, measured) pair
/// 3. Fit calibration polynomial to data
/// 4. Validate calibration accuracy
/// 5. Save calibration to file
/// ```
///
/// ## 4. Polynomial Fitting
///
/// Use least-squares fitting to determine polynomial coefficients:
///
/// ```rust,ignore
/// fn fit_polynomial(points: &[(f64, f64)], order: usize) -> CalibrationPolynomial {
///     // Vandermonde matrix approach or use nalgebra for robust fitting
///     // ...
/// }
/// ```
///
/// ## 5. Validation
///
/// After fitting, validate by measuring known voltages and checking error:
///
/// - Maximum error should be < 1 LSB for good calibration
/// - RMS error across range indicates overall quality
///
/// ## 6. GUI Components Needed
///
/// - Subdevice/channel selector
/// - Range selector
/// - Reference voltage input (numeric entry)
/// - "Measure" button
/// - Progress indicator
/// - Results display (measured vs expected)
/// - Error statistics
/// - "Apply" and "Save" buttons
/// - Factory reset option
pub struct CalibrationWizardRequirements;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polynomial_identity() {
        let poly = CalibrationPolynomial::identity();

        // Identity should pass through values unchanged
        assert!((poly.apply(0) - 0.0).abs() < 1e-10);
        assert!((poly.apply(1000) - 1000.0).abs() < 1e-10);
        assert!((poly.apply(65535) - 65535.0).abs() < 1e-10);

        assert!(poly.is_identity());
    }

    #[test]
    fn test_polynomial_linear() {
        // y = 0.1 + 2.0 * x
        let poly = CalibrationPolynomial::linear(0.1, 2.0, 0.0);

        assert!((poly.apply(0) - 0.1).abs() < 1e-10);
        assert!((poly.apply(1) - 2.1).abs() < 1e-10);
        assert!((poly.apply(100) - 200.1).abs() < 1e-10);

        assert!(!poly.is_identity());
    }

    #[test]
    fn test_polynomial_with_origin() {
        // y = 5.0 + 1.0 * (x - 32768)
        let poly = CalibrationPolynomial::linear(5.0, 1.0, 32768.0);

        assert!((poly.apply(32768) - 5.0).abs() < 1e-10);
        assert!((poly.apply(32769) - 6.0).abs() < 1e-10);
        assert!((poly.apply(32767) - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_polynomial_from_range() {
        // +-10V range with 16-bit resolution
        let poly = CalibrationPolynomial::from_range(-10.0, 10.0, 65535);

        // 0 -> -10V
        assert!((poly.apply(0) - (-10.0)).abs() < 0.001);
        // midpoint -> 0V
        assert!((poly.apply(32767) - 0.0).abs() < 0.001);
        // max -> +10V
        assert!((poly.apply(65535) - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_polynomial_for_dac() {
        // +-10V range with 16-bit resolution
        let poly = CalibrationPolynomial::for_dac(-10.0, 10.0, 65535);

        // -10V -> 0
        let raw_neg = poly.apply_f64(-10.0);
        assert!((raw_neg - 0.0).abs() < 1.0);

        // 0V -> midpoint
        let raw_zero = poly.apply_f64(0.0);
        assert!((raw_zero - 32767.5).abs() < 1.0);

        // +10V -> max
        let raw_pos = poly.apply_f64(10.0);
        assert!((raw_pos - 65535.0).abs() < 1.0);
    }

    #[test]
    fn test_polynomial_inverse() {
        let poly = CalibrationPolynomial::from_range(-10.0, 10.0, 65535);

        // Round-trip: raw -> physical -> raw
        for raw in [0, 10000, 32768, 50000, 65535] {
            let physical = poly.apply(raw);
            let recovered = poly.apply_inverse(physical, 65535);
            assert!(
                (recovered as i32 - raw as i32).abs() <= 1,
                "raw={} physical={} recovered={}",
                raw,
                physical,
                recovered
            );
        }
    }

    #[test]
    fn test_calibration_setting_applies() {
        let setting = CalibrationSetting {
            subdevice: 0,
            channels: vec![0, 1, 2],
            ranges: vec![0],
            to_physical: CalibrationPolynomial::identity(),
            from_physical: CalibrationPolynomial::identity(),
        };

        assert!(setting.applies_to(0, 0, 0));
        assert!(setting.applies_to(0, 1, 0));
        assert!(setting.applies_to(0, 2, 0));
        assert!(!setting.applies_to(0, 3, 0)); // channel not in list
        assert!(!setting.applies_to(0, 0, 1)); // range not in list
        assert!(!setting.applies_to(1, 0, 0)); // wrong subdevice
    }

    #[test]
    fn test_calibration_data() {
        let mut data = CalibrationData::new("ni_pcimio", "pci-mio-16xe-10");

        assert_eq!(data.status, CalibrationStatus::Uncalibrated);
        assert!(data.settings.is_empty());

        // Add a setting
        let setting = CalibrationSetting {
            subdevice: 0,
            channels: vec![],
            ranges: vec![],
            to_physical: CalibrationPolynomial::linear(0.01, 1.001, 0.0),
            from_physical: CalibrationPolynomial::identity(),
        };

        data.add_setting(setting);
        assert_eq!(data.settings.len(), 1);

        // Find setting
        let found = data.find_setting(0, 5, 2);
        assert!(found.is_some());
    }

    #[test]
    fn test_calibration_status_display() {
        assert_eq!(CalibrationStatus::Uncalibrated.to_string(), "Uncalibrated");
        assert_eq!(
            CalibrationStatus::FactoryCalibration.to_string(),
            "Factory Calibration"
        );
        assert_eq!(
            CalibrationStatus::UserCalibration.to_string(),
            "User Calibration"
        );
    }

    #[test]
    fn test_conversion_direction_display() {
        assert_eq!(ConversionDirection::ToPhysical.to_string(), "to_physical");
        assert_eq!(
            ConversionDirection::FromPhysical.to_string(),
            "from_physical"
        );
    }

    #[test]
    fn test_polynomial_quadratic() {
        // y = 1 + 2x + 3x^2
        let mut coefficients = [0.0; MAX_POLYNOMIAL_COEFFICIENTS];
        coefficients[0] = 1.0;
        coefficients[1] = 2.0;
        coefficients[2] = 3.0;

        let poly = CalibrationPolynomial {
            coefficients,
            expansion_origin: 0.0,
            order: 2,
        };

        // At x=0: 1 + 0 + 0 = 1
        assert!((poly.apply(0) - 1.0).abs() < 1e-10);

        // At x=1: 1 + 2 + 3 = 6
        assert!((poly.apply(1) - 6.0).abs() < 1e-10);

        // At x=2: 1 + 4 + 12 = 17
        assert!((poly.apply(2) - 17.0).abs() < 1e-10);
    }

    #[test]
    fn test_polynomial_derivative() {
        // y = 1 + 2x + 3x^2
        // dy/dx = 2 + 6x
        let mut coefficients = [0.0; MAX_POLYNOMIAL_COEFFICIENTS];
        coefficients[0] = 1.0;
        coefficients[1] = 2.0;
        coefficients[2] = 3.0;

        let poly = CalibrationPolynomial {
            coefficients,
            expansion_origin: 0.0,
            order: 2,
        };

        // At x=0: dy/dx = 2
        assert!((poly.derivative_at(0.0) - 2.0).abs() < 1e-10);

        // At x=1: dy/dx = 2 + 6 = 8
        assert!((poly.derivative_at(1.0) - 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_serialization() {
        let poly = CalibrationPolynomial::linear(0.1, 1.001, 32768.0);
        let json = serde_json::to_string(&poly).expect("serialize");
        let recovered: CalibrationPolynomial = serde_json::from_str(&json).expect("deserialize");

        assert!((recovered.coefficients[0] - poly.coefficients[0]).abs() < 1e-10);
        assert!((recovered.coefficients[1] - poly.coefficients[1]).abs() < 1e-10);
        assert!((recovered.expansion_origin - poly.expansion_origin).abs() < 1e-10);
        assert_eq!(recovered.order, poly.order);
    }
}
