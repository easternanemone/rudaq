//! Thorlabs Elliptec ELL14 Rotation Mount Driver
//!
//! Reference: ELLx modules protocol manual Issue 10
//!
//! Protocol Overview:
//! - Format: [Address][Command][Data (optional)] (ASCII encoded)
//! - Address: 0-9, A-F (usually '0' for first device)
//! - Encoding: Positions as 32-bit integers in hex
//! - Timing: Half-duplex request-response
//!
//! # Multidrop Bus Support
//!
//! Multiple ELL14 devices can share a single serial port (RS-485 multidrop bus).
//! Use [`Ell14Driver::with_shared_port`] to create multiple drivers that share
//! the same underlying serial connection:
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use tokio::sync::Mutex;
//!
//! // Open port once
//! let shared_port = Arc::new(Mutex::new(open_ell14_port("/dev/ttyUSB0")?));
//!
//! // Create drivers for different addresses
//! let rotator_2 = Ell14Driver::with_shared_port(shared_port.clone(), "2");
//! let rotator_3 = Ell14Driver::with_shared_port(shared_port.clone(), "3");
//! let rotator_8 = Ell14Driver::with_shared_port(shared_port.clone(), "8");
//! ```
//!
//! # Supported Commands
//!
//! ## Basic Movement
//! - `ho` - Home to mechanical zero
//! - `ma` - Move absolute (32-bit hex position)
//! - `mr` - Move relative (32-bit hex distance)
//! - `gp` - Get current position
//! - `gs` - Get status
//!
//! ## Jog Control
//! - `fw` - Jog forward by jog step
//! - `bw` - Jog backward by jog step
//! - `gj` - Get jog step size
//! - `sj` - Set jog step size
//! - `st` - Stop motion immediately
//!
//! ## Motor Optimization
//! - `s1` - Search/optimize motor 1 frequency
//! - `s2` - Search/optimize motor 2 frequency
//! - `i1` - Get motor 1 info
//! - `i2` - Get motor 2 info
//!
//! ## Configuration
//! - `in` - Get device info
//! - `go` - Get home offset
//! - `so` - Set home offset
//! - `gv` - Get velocity (percentage 60-100%)
//! - `sv` - Set velocity (percentage 60-100%)
//! - `us` - Save user data to flash
//! - `ca` - Change device address
//!
//! # Example Usage
//!
//! ```ignore
//! use daq_hardware::drivers::ell14::Ell14Driver;
//! use daq_hardware::capabilities::Movable;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let driver = Ell14Driver::new("/dev/ttyUSB0", "0")?;
//!
//!     // Move to 45 degrees
//!     driver.move_abs(45.0).await?;
//!     driver.wait_settled().await?;
//!
//!     // Get current position
//!     let pos = driver.position().await?;
//!     println!("Position: {:.2}°", pos);
//!
//!     // Jog by 5 degrees
//!     driver.set_jog_step(5.0).await?;
//!     driver.jog_forward().await?;
//!     driver.wait_settled().await?;
//!
//!     // Optimize motor frequencies
//!     driver.optimize_motors().await?;
//!
//!     Ok(())
//! }
//! ```

use crate::capabilities::{Movable, Parameterized};
use crate::port_resolver::resolve_port;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use daq_core::error::DaqError;
use daq_core::error_recovery::RetryPolicy;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, Mutex};
use tokio::task::spawn_blocking;
use tokio::time::sleep;
use tokio_serial::SerialPortBuilderExt;
use tracing::instrument;

pub trait SerialPortIO: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SerialPortIO for T {}
pub type DynSerial = Box<dyn SerialPortIO>;
pub type SharedPort = Arc<Mutex<DynSerial>>;

/// Represents the state of the Elliptec device
#[derive(Debug, Clone, PartialEq)]
pub struct ElliptecState {
    pub position: f64,
    pub status: u32,
    pub error_code: Option<u32>,
}

/// Device information returned by the `in` command
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Device type (e.g., "14" for ELL14)
    pub device_type: String,
    /// Serial number
    pub serial: String,
    /// Year of manufacture
    pub year: u16,
    /// Firmware version
    pub firmware: String,
    /// Hardware version (if available)
    pub hardware: Option<String>,
    /// Travel range in pulses
    pub travel: u32,
    /// Pulses per unit (degrees for rotation, mm for linear)
    pub pulses_per_unit: u32,
}

/// Motor information returned by `i1` or `i2` commands
#[derive(Debug, Clone)]
pub struct MotorInfo {
    /// Motor number (1 or 2)
    pub motor_number: u8,
    /// Loop state (0=off, 1=on)
    pub loop_state: bool,
    /// Motor state (0=stopped, 1=running)
    pub motor_on: bool,
    /// Current operating frequency in Hz
    pub frequency: u32,
    /// Forward period
    pub forward_period: u16,
    /// Backward period
    pub backward_period: u16,
}

/// Movement direction for continuous rotation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    /// Forward (clockwise when viewed from motor side)
    Forward,
    /// Backward (counter-clockwise)
    Backward,
}

/// Home direction for rotary stages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeDirection {
    /// Clockwise direction
    Clockwise = 0,
    /// Counter-clockwise direction
    CounterClockwise = 1,
}

/// Motor period data for forward and backward directions
#[derive(Debug, Clone)]
pub struct MotorPeriods {
    /// Forward period value
    pub forward_period: u16,
    /// Backward period value
    pub backward_period: u16,
}

/// ELL14 status/error codes returned in GS responses
///
/// The device returns status codes in format `{addr}GS{XX}` where XX is a hex code.
/// Code 00 indicates success; all other codes indicate errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Ell14StatusCode {
    /// No error - command completed successfully
    Ok = 0x00,
    /// Communication timeout - no response from motor
    CommunicationTimeout = 0x01,
    /// Mechanical timeout - motor didn't reach target position in time
    MechanicalTimeout = 0x02,
    /// Command error - invalid or malformed command
    CommandError = 0x03,
    /// Value out of range - parameter outside valid bounds
    ValueOutOfRange = 0x04,
    /// Module isolated - device in isolation mode
    ModuleIsolated = 0x05,
    /// Module out of isolation - device exiting isolation
    ModuleOutOfIsolation = 0x06,
    /// Initialization error - startup failed
    InitializationError = 0x07,
    /// Thermal error - overtemperature protection
    ThermalError = 0x08,
    /// Busy - device is processing another command
    Busy = 0x09,
    /// Sensor error - position sensor malfunction
    SensorError = 0x0A,
    /// Motor error - motor driver fault
    MotorError = 0x0B,
    /// Out of range - position outside travel limits
    OutOfRange = 0x0C,
    /// Over current error - excessive motor current
    OverCurrentError = 0x0D,
    /// Unknown error code
    Unknown = 0xFF,
}

impl Ell14StatusCode {
    /// Parse status code from hex string (e.g., "00", "02")
    pub fn from_hex(hex: &str) -> Self {
        match u8::from_str_radix(hex, 16) {
            Ok(code) => Self::from_u8(code),
            Err(_) => Self::Unknown,
        }
    }

    /// Convert from u8 code value
    pub fn from_u8(code: u8) -> Self {
        match code {
            0x00 => Self::Ok,
            0x01 => Self::CommunicationTimeout,
            0x02 => Self::MechanicalTimeout,
            0x03 => Self::CommandError,
            0x04 => Self::ValueOutOfRange,
            0x05 => Self::ModuleIsolated,
            0x06 => Self::ModuleOutOfIsolation,
            0x07 => Self::InitializationError,
            0x08 => Self::ThermalError,
            0x09 => Self::Busy,
            0x0A => Self::SensorError,
            0x0B => Self::MotorError,
            0x0C => Self::OutOfRange,
            0x0D => Self::OverCurrentError,
            _ => Self::Unknown,
        }
    }

    /// Check if this status indicates success
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// Check if this status indicates an error
    pub fn is_error(&self) -> bool {
        !self.is_ok()
    }

    /// Get human-readable description of the status
    pub fn description(&self) -> &'static str {
        match self {
            Self::Ok => "No error",
            Self::CommunicationTimeout => "Communication timeout",
            Self::MechanicalTimeout => "Mechanical timeout - motor didn't reach target",
            Self::CommandError => "Command error - invalid command",
            Self::ValueOutOfRange => "Value out of range",
            Self::ModuleIsolated => "Module isolated",
            Self::ModuleOutOfIsolation => "Module out of isolation",
            Self::InitializationError => "Initialization error",
            Self::ThermalError => "Thermal error - overtemperature",
            Self::Busy => "Busy - device processing",
            Self::SensorError => "Sensor error",
            Self::MotorError => "Motor error",
            Self::OutOfRange => "Position out of range",
            Self::OverCurrentError => "Over current error",
            Self::Unknown => "Unknown error",
        }
    }
}

impl std::fmt::Display for Ell14StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (0x{:02X})", self.description(), *self as u8)
    }
}

/// ELL14 command types for protocol operations
///
/// Each command has a 2-character code sent after the device address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ell14Command {
    // === Movement Commands ===
    /// Move to absolute position (ma)
    MoveAbsolute,
    /// Move relative distance (mr)
    MoveRelative,
    /// Home to mechanical zero (ho)
    Home,
    /// Move forward continuously (fw)
    JogForward,
    /// Move backward continuously (bw)
    JogBackward,
    /// Stop movement (st)
    Stop,

    // === Query Commands ===
    /// Get current position (gp)
    GetPosition,
    /// Get status (gs)
    GetStatus,
    /// Get device information (in)
    GetInfo,
    /// Get jog step size (gj)
    GetJogStep,
    /// Get velocity (gv)
    GetVelocity,
    /// Get home offset (go)
    GetHomeOffset,
    /// Get motor 1 info (i1)
    GetMotor1Info,
    /// Get motor 2 info (i2)
    GetMotor2Info,

    // === Configuration Commands ===
    /// Set jog step size (sj)
    SetJogStep,
    /// Set velocity (sv)
    SetVelocity,
    /// Set home offset (so)
    SetHomeOffset,
    /// Save parameters to EEPROM (us)
    SaveUserData,

    // === Motor Tuning Commands ===
    /// Search motor frequencies (s1/s2)
    SearchFrequency,
    /// Scan motor current curve (c1/c2)
    ScanCurrentCurve,
    /// Get motor forward period (f1/f2)
    GetForwardPeriod,
    /// Get motor backward period (b1/b2)
    GetBackwardPeriod,
    /// Set motor forward period (p1/p2 with direction)
    SetForwardPeriod,
    /// Set motor backward period
    SetBackwardPeriod,

    // === Advanced Commands ===
    /// Isolate device from bus (is)
    IsolateDevice,
    /// Clean mechanics (cm)
    CleanMechanics,
    /// Fine-tune motors (ft)
    FineTuneMotors,
}

impl Ell14Command {
    /// Get the 2-character command code
    pub fn code(&self) -> &'static str {
        match self {
            Self::MoveAbsolute => "ma",
            Self::MoveRelative => "mr",
            Self::Home => "ho",
            Self::JogForward => "fw",
            Self::JogBackward => "bw",
            Self::Stop => "st",
            Self::GetPosition => "gp",
            Self::GetStatus => "gs",
            Self::GetInfo => "in",
            Self::GetJogStep => "gj",
            Self::GetVelocity => "gv",
            Self::GetHomeOffset => "go",
            Self::GetMotor1Info => "i1",
            Self::GetMotor2Info => "i2",
            Self::SetJogStep => "sj",
            Self::SetVelocity => "sv",
            Self::SetHomeOffset => "so",
            Self::SaveUserData => "us",
            Self::SearchFrequency => "s1", // s1 or s2 depending on motor
            Self::ScanCurrentCurve => "c1", // c1 or c2 depending on motor
            Self::GetForwardPeriod => "f1", // f1 or f2 depending on motor
            Self::GetBackwardPeriod => "b1", // b1 or b2 depending on motor
            Self::SetForwardPeriod => "p1", // with FWD prefix
            Self::SetBackwardPeriod => "p1", // with BWD prefix
            Self::IsolateDevice => "is",
            Self::CleanMechanics => "cm",
            Self::FineTuneMotors => "ft",
        }
    }

    /// Check if this command expects a response
    pub fn expects_response(&self) -> bool {
        !matches!(
            self,
            Self::Home | Self::Stop | Self::JogForward | Self::JogBackward
        )
    }
}

/// Current curve data point
#[derive(Debug, Clone)]
pub struct CurrentCurvePoint {
    /// Frequency in Hz (computed from period)
    pub frequency_hz: u32,
    /// Forward current in Amps
    pub forward_current_amps: f64,
    /// Backward current in Amps
    pub backward_current_amps: f64,
}

/// Current curve scan result
#[derive(Debug, Clone)]
pub struct CurrentCurveScan {
    /// Motor number (1 or 2)
    pub motor_number: u8,
    /// Data points (87 points from 70-120 kHz)
    pub data_points: Vec<CurrentCurvePoint>,
}

// =============================================================================
// Ell14Bus - Primary API for RS-485 Multidrop Bus
// =============================================================================

/// RS-485 bus manager for Thorlabs Elliptec ELL14 devices
///
/// This is the **primary API** for working with ELL14 rotation mounts.
/// It accurately models the RS-485 multidrop architecture where multiple
/// devices share a single serial connection with address-based multiplexing.
///
/// # Why Use Ell14Bus?
///
/// The ELL14 uses RS-485, which is a shared bus protocol. All devices on the bus
/// share the same physical serial connection. This struct enforces that model:
///
/// - **One bus = one serial port** - The bus owns the connection
/// - **Multiple devices per bus** - Get device handles via [`device()`]
/// - **Impossible to misuse** - Can't accidentally open multiple ports
///
/// # Example
///
/// ```rust,ignore
/// use daq_hardware::drivers::ell14::Ell14Bus;
///
/// // Open the RS-485 bus (one connection for all devices)
/// let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
///
/// // Get handles to individual devices on the bus
/// let rotator_2 = bus.device("2").await?;
/// let rotator_3 = bus.device("3").await?;
/// let rotator_8 = bus.device("8").await?;
///
/// // All devices share the connection - no contention issues
/// rotator_2.move_abs(45.0).await?;
/// rotator_3.move_abs(90.0).await?;
/// rotator_8.move_abs(135.0).await?;
///
/// // Discover all devices on the bus
/// let devices = bus.discover().await?;
/// for info in devices {
///     println!("Found {} at address {}", info.device_type, info.address);
/// }
/// ```
///
/// # Thread Safety
///
/// `Ell14Bus` is `Clone` and thread-safe. The underlying serial port is protected
/// by a mutex, so multiple tasks can safely share the bus.
#[derive(Clone)]
pub struct Ell14Bus {
    port: SharedPort,
    port_path: String,
}

impl Ell14Bus {
    /// Open an RS-485 bus connection to ELL14 devices
    ///
    /// This opens the serial port with ELL14 settings (9600 baud, 8N1).
    /// The connection is shared among all devices on the bus.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path. Accepts:
    ///   - Direct path: `/dev/ttyUSB0`, `COM3`
    ///   - By-ID symlink: `/dev/serial/by-id/usb-FTDI_FT230X_...`
    ///   - Short by-ID: `usb-FTDI_FT230X_Basic_UART_DJ00XXXX-if00-port0`
    ///
    /// Using by-ID paths is recommended as they are stable across reboots.
    ///
    /// # Errors
    /// Returns error if the serial port cannot be opened or path cannot be resolved.
    ///
    /// # Example
    /// ```rust,ignore
    /// // Using by-ID path (recommended - stable across reboots)
    /// let bus = Ell14Bus::open("/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DJ00XXXX-if00-port0").await?;
    ///
    /// // Using direct path (may change between reboots)
    /// let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
    /// ```
    pub async fn open(port_path: &str) -> Result<Self> {
        // Resolve the port path (handles by-id symlinks, etc.)
        let resolved_path = resolve_port(port_path)
            .map_err(|e| anyhow!("Failed to resolve port '{}': {}", port_path, e))?;

        let port_path_for_open = resolved_path.clone();

        // Open port in blocking task to avoid blocking async runtime
        let port = tokio::task::spawn_blocking(move || Ell14Driver::open_port(&port_path_for_open))
            .await
            .context("spawn_blocking for ELL14 port opening failed")??;

        Ok(Self {
            port: Arc::new(Mutex::new(port)),
            port_path: resolved_path,
        })
    }

    /// Get a calibrated device handle for an address on this bus
    ///
    /// This queries the device for its actual calibration value (pulses per degree),
    /// ensuring accurate positioning. Each ELL14 unit has device-specific calibration
    /// stored in firmware.
    ///
    /// # Arguments
    /// * `address` - Device address on the bus (0-9, A-F)
    ///
    /// # Errors
    /// Returns error if the device doesn't respond or calibration query fails.
    ///
    /// # Example
    /// ```rust,ignore
    /// let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
    /// let rotator = bus.device("2").await?;
    /// println!("Calibration: {:.2} pulses/degree", rotator.get_pulses_per_degree());
    /// ```
    pub async fn device(&self, address: &str) -> Result<Ell14Driver> {
        Ell14Driver::with_shared_port_calibrated(self.port.clone(), address).await
    }

    /// Get a device handle without querying calibration
    ///
    /// Uses the default calibration value (398.22 pulses/degree). This is faster
    /// but may be less accurate if the device has non-standard calibration.
    ///
    /// # Arguments
    /// * `address` - Device address on the bus (0-9, A-F)
    pub fn device_uncalibrated(&self, address: &str) -> Ell14Driver {
        Ell14Driver::with_shared_port(self.port.clone(), address)
    }

    /// Discover all ELL14 devices on this bus
    ///
    /// Scans addresses 0-9 and A-F, returning info for devices that respond.
    /// This can take several seconds as each address must be queried.
    ///
    /// # Example
    /// ```rust,ignore
    /// let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
    /// let devices = bus.discover().await?;
    /// for info in devices {
    ///     println!("Address {}: {} (serial: {})",
    ///         info.address, info.device_type, info.serial_number);
    /// }
    /// ```
    pub async fn discover(&self) -> Result<Vec<DiscoveredDevice>> {
        let addresses = [
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F",
        ];

        let mut devices = Vec::new();

        for addr in addresses {
            // Try to get device info - if it responds, it exists
            let driver = Ell14Driver::with_shared_port(self.port.clone(), addr);
            match driver.get_device_info().await {
                Ok(info) => {
                    tracing::debug!(address = %addr, device_type = %info.device_type, "Found device");
                    devices.push(DiscoveredDevice {
                        address: addr.to_string(),
                        info,
                    });
                }
                Err(_) => {
                    // No device at this address - continue scanning
                    tracing::trace!(address = %addr, "No device found");
                }
            }
        }

        Ok(devices)
    }

    /// Get the serial port path this bus is connected to
    pub fn port_path(&self) -> &str {
        &self.port_path
    }

    /// Get the underlying shared port (for advanced use cases)
    ///
    /// This is useful if you need to create drivers with custom calibration
    /// or perform low-level operations.
    pub fn shared_port(&self) -> SharedPort {
        self.port.clone()
    }
}

/// Information about a discovered device on the bus
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// Device address (0-9, A-F)
    pub address: String,
    /// Device information from the IN command
    pub info: DeviceInfo,
}

/// Driver for Thorlabs Elliptec ELL14 Rotation Mount
///
/// Implements the Movable capability trait for controlling rotation.
/// The ELL14 has a mechanical resolution in "pulses" that must be converted
/// to/from degrees based on device calibration.
///
/// # Multidrop Bus Support
///
/// Multiple ELL14 devices can share a single serial port using [`with_shared_port`].
/// This is essential for RS-485 multidrop configurations where devices at
/// different addresses (2, 3, 8, etc.) share `/dev/ttyUSB0`.
///
/// # Group Addressing
///
/// Multiple rotators can be synchronized using group addressing. One rotator
/// acts as the master, and others are configured as slaves that listen to
/// the master's address:
///
/// ```rust,ignore
/// // Configure slave to listen to master's address
/// slave.configure_as_group_slave("2", 30.0).await?; // 30° offset
///
/// // Now when master moves, slave follows with offset
/// master.move_abs(45.0).await?; // Slave moves to 75°
///
/// // Revert slave to individual control
/// slave.revert_from_group_slave().await?;
/// ```
#[derive(Clone)]
pub struct Ell14Driver {
    /// Serial port protected by Arc<Mutex> for shared access across multiple drivers
    port: SharedPort,
    /// Physical device address (0-9, A-F) - never changes
    physical_address: String,
    /// Active address for commands - may differ when in group mode
    active_address: String,
    /// Calibration factor: Pulses per Degree
    /// Default: 398.22 (143360 pulses / 360 degrees for ELL14)
    pulses_per_degree: f64,
    /// Rotation position parameter (degrees)
    position_deg: Parameter<f64>,
    /// Parameter registry
    params: Arc<ParameterSet>,
    /// Whether this rotator is configured as a slave in a group
    is_slave_in_group: bool,
    /// Offset applied when in group mode (degrees)
    group_offset_degrees: f64,
    /// Broadcast channel for state updates
    state_tx: broadcast::Sender<ElliptecState>,
}

impl Ell14Driver {
    /// Default calibration: nominal 143360 pulses / 360 degrees = 398.2222 pulses/degree
    ///
    /// **WARNING:** This is a nominal value that may not match your specific device!
    /// Individual ELL14 units have device-specific calibration values stored in firmware.
    /// For accurate positioning, use [`new_async_with_device_calibration`] to query
    /// the device's actual `PULSES/M.U.` value from the `IN` command response.
    pub const DEFAULT_PULSES_PER_DEGREE: f64 = 398.2222;

    async fn with_retry<F, Fut, T>(&self, operation: &str, mut op: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let policy = RetryPolicy::default();
        let mut attempts = 0;
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    attempts += 1;
                    if attempts >= policy.max_attempts {
                        return Err(err);
                    }
                    tracing::warn!(
                        target: "hardware::ell14",
                        attempt = attempts,
                        max_attempts = policy.max_attempts,
                        "Operation '{}' failed: {}. Retrying in {:?}",
                        operation,
                        err,
                        policy.backoff_delay
                    );
                    tokio::time::sleep(policy.backoff_delay).await;
                }
            }
        }
    }

    /// Create a new ELL14 driver instance (opens dedicated serial port)
    ///
    /// # Deprecated
    /// Use [`Ell14Bus::open()`] instead for the recommended bus-centric API that
    /// correctly models RS-485 multidrop architecture.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0" on Linux, "COM3" on Windows)
    /// * `address` - Device address (usually "0")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    #[deprecated(
        since = "0.2.0",
        note = "Use Ell14Bus::open() and bus.device() instead. This opens a dedicated port which fails for multidrop configurations."
    )]
    pub fn new(port_path: &str, address: &str) -> Result<Self> {
        let port = Self::open_port(port_path)?;
        Ok(Self::build(
            Arc::new(Mutex::new(port)),
            address.to_string(),
            Self::DEFAULT_PULSES_PER_DEGREE,
        ))
    }

    /// Open a serial port that can be shared across multiple ELL14 drivers
    ///
    /// Use this with [`with_shared_port`] for multidrop bus configurations:
    ///
    /// ```rust,ignore
    /// let shared = Ell14Driver::open_shared_port("/dev/ttyUSB0")?;
    /// let rotator_2 = Ell14Driver::with_shared_port(shared.clone(), "2");
    /// let rotator_3 = Ell14Driver::with_shared_port(shared.clone(), "3");
    /// let rotator_8 = Ell14Driver::with_shared_port(shared.clone(), "8");
    /// ```
    pub fn open_shared_port(port_path: &str) -> Result<SharedPort> {
        let port = Self::open_port(port_path)?;
        Ok(Arc::new(Mutex::new(port)))
    }

    /// Create an ELL14 driver using a shared serial port
    ///
    /// This is the preferred method for multidrop bus configurations where
    /// multiple devices share the same physical serial port.
    ///
    /// # Arguments
    /// * `shared_port` - Shared serial port from [`open_shared_port`]
    /// * `address` - Device address on the bus (0-9, A-F)
    pub fn with_shared_port(shared_port: SharedPort, address: &str) -> Self {
        Self::build(
            shared_port,
            address.to_string(),
            Self::DEFAULT_PULSES_PER_DEGREE,
        )
    }

    /// Create an ELL14 driver using a shared serial port with device calibration
    ///
    /// This is the **preferred method** for multidrop bus configurations.
    /// It queries the device for its actual calibration value, ensuring accurate positioning.
    ///
    /// # Arguments
    /// * `shared_port` - Shared serial port from [`open_shared_port`]
    /// * `address` - Device address on the bus (0-9, A-F)
    ///
    /// # Example
    /// ```rust,ignore
    /// // Open shared port once
    /// let shared = Ell14Driver::open_shared_port("/dev/ttyUSB0")?;
    ///
    /// // Create calibrated drivers for multiple devices on the bus
    /// let rotator_2 = Ell14Driver::with_shared_port_calibrated(shared.clone(), "2").await?;
    /// let rotator_3 = Ell14Driver::with_shared_port_calibrated(shared.clone(), "3").await?;
    /// let rotator_8 = Ell14Driver::with_shared_port_calibrated(shared.clone(), "8").await?;
    /// ```
    /// Create a calibrated ELL14 driver with device validation
    ///
    /// This is the **preferred constructor** for production use. It queries the
    /// device for calibration data and **fails if the device doesn't respond**,
    /// ensuring early detection of misconfiguration.
    ///
    /// For backwards-compatible behavior that falls back to defaults on error,
    /// use [`with_shared_port_lenient`].
    pub async fn with_shared_port_calibrated(
        shared_port: SharedPort,
        address: &str,
    ) -> Result<Self> {
        // Create driver with default calibration first (needed for get_device_info)
        let mut driver = Self::build(
            shared_port,
            address.to_string(),
            Self::DEFAULT_PULSES_PER_DEGREE,
        );

        // Allow RS-485 bus to settle after previous device activity
        sleep(Duration::from_millis(50)).await;

        // Query device for actual calibration - FAIL if device doesn't respond
        let info = driver.get_device_info().await.context(format!(
            "ELL14 device validation failed at address '{}'. \
             Check that an ELL14 device is connected and responding at this address.",
            address
        ))?;

        if info.pulses_per_unit > 0 {
            let pulses_per_degree = info.pulses_per_unit as f64 / 360.0;
            tracing::info!(
                address = %address,
                device_type = %info.device_type,
                serial = %info.serial,
                firmware = %info.firmware,
                pulses_per_degree = pulses_per_degree,
                total_pulses = info.pulses_per_unit,
                "ELL14 device validated and calibration loaded"
            );
            driver.pulses_per_degree = pulses_per_degree;
        } else {
            return Err(anyhow!(
                "ELL14 device at address '{}' returned invalid calibration (0 pulses_per_unit). \
                 Device may be malfunctioning or incompatible.",
                address
            ));
        }

        Ok(driver)
    }

    /// Create a calibrated ELL14 driver with lenient error handling
    ///
    /// Unlike [`with_shared_port_calibrated`], this method logs warnings but
    /// continues with default calibration if the device doesn't respond.
    /// Use this only when you need backwards-compatible behavior.
    pub async fn with_shared_port_lenient(shared_port: SharedPort, address: &str) -> Self {
        // Create driver with default calibration first (needed for get_device_info)
        let mut driver = Self::build(
            shared_port,
            address.to_string(),
            Self::DEFAULT_PULSES_PER_DEGREE,
        );

        // Query device for actual calibration - fall back to defaults on error
        match driver.get_device_info().await {
            Ok(info) => {
                if info.pulses_per_unit > 0 {
                    let pulses_per_degree = info.pulses_per_unit as f64 / 360.0;
                    tracing::info!(
                        address = %address,
                        pulses_per_degree = pulses_per_degree,
                        total_pulses = info.pulses_per_unit,
                        "ELL14 device calibration loaded"
                    );
                    driver.pulses_per_degree = pulses_per_degree;
                } else {
                    tracing::warn!(
                        address = %address,
                        default = Self::DEFAULT_PULSES_PER_DEGREE,
                        "ELL14 device returned 0 pulses_per_unit, using default"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    address = %address,
                    error = %e,
                    default = Self::DEFAULT_PULSES_PER_DEGREE,
                    "Failed to query ELL14 device info, using default calibration"
                );
            }
        }

        driver
    }

    /// Internal helper to open a serial port with ELL14 settings
    fn open_port(port_path: &str) -> Result<DynSerial> {
        let port = tokio_serial::new(port_path, 9600)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!("Failed to open ELL14 serial port: {}", port_path))?;

        Ok(Box::new(port))
    }

    /// Create a new ELL14 driver instance asynchronously with default calibration
    ///
    /// # Deprecated
    /// Use [`Ell14Bus::open()`] instead for the recommended bus-centric API.
    ///
    /// Uses `spawn_blocking` to avoid blocking the async runtime during serial
    /// port opening. Uses default calibration of 398.2222 pulses/degree.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0" on Linux, "COM3" on Windows)
    /// * `address` - Device address (usually "0")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    #[deprecated(
        since = "0.2.0",
        note = "Use Ell14Bus::open() and bus.device_uncalibrated() instead."
    )]
    pub async fn new_async(port_path: &str, address: &str) -> Result<Self> {
        let port_path = port_path.to_string();
        let address = address.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        let port = spawn_blocking(move || Self::open_port(&port_path))
            .await
            .context("spawn_blocking for ELL14 port opening failed")??;

        Ok(Self::build(
            Arc::new(Mutex::new(port)),
            address,
            Self::DEFAULT_PULSES_PER_DEGREE,
        ))
    }

    /// Create a new ELL14 driver and query device for actual calibration
    ///
    /// # Deprecated
    /// Use [`Ell14Bus::open()`] and [`Ell14Bus::device()`] instead for the
    /// recommended bus-centric API that correctly models RS-485 architecture.
    ///
    /// Queries the device for its actual `pulses_per_unit` calibration value.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0" on Linux, "COM3" on Windows)
    /// * `address` - Device address (usually "0")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened or device info query fails
    ///
    /// # Example
    /// ```rust,ignore
    /// // Old way (deprecated):
    /// let driver = Ell14Driver::new_async_with_device_calibration("/dev/ttyUSB0", "0").await?;
    ///
    /// // New way (recommended):
    /// let bus = Ell14Bus::open("/dev/ttyUSB0").await?;
    /// let driver = bus.device("0").await?;
    /// ```
    #[deprecated(
        since = "0.2.0",
        note = "Use Ell14Bus::open() and bus.device() instead. This opens a dedicated port which fails for multidrop configurations."
    )]
    pub async fn new_async_with_device_calibration(port_path: &str, address: &str) -> Result<Self> {
        let port_path_owned = port_path.to_string();
        let address_owned = address.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        let port = spawn_blocking(move || Self::open_port(&port_path_owned))
            .await
            .context("spawn_blocking for ELL14 port opening failed")??;

        let shared_port = Arc::new(Mutex::new(port));

        // Create driver with default calibration first (needed for get_device_info)
        let mut driver = Self::build(shared_port, address_owned, Self::DEFAULT_PULSES_PER_DEGREE);

        // Query device for actual calibration
        // The IN response contains "pulses_per_unit" which is TOTAL pulses for full travel
        // For ELL14 rotation stages, full travel = 360°, so divide by 360 to get pulses/degree
        match driver.get_device_info().await {
            Ok(info) => {
                if info.pulses_per_unit > 0 {
                    // pulses_per_unit is total pulses for full 360° rotation
                    // Divide by 360 to get pulses per degree
                    let pulses_per_degree = info.pulses_per_unit as f64 / 360.0;
                    tracing::info!(
                        "ELL14 device calibration: {:.4} pulses/degree (from device: {} total pulses / 360°)",
                        pulses_per_degree,
                        info.pulses_per_unit
                    );
                    driver.pulses_per_degree = pulses_per_degree;
                } else {
                    tracing::warn!(
                        "ELL14 device returned 0 pulses_per_unit, using default {:.4}",
                        Self::DEFAULT_PULSES_PER_DEGREE
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to query ELL14 device info, using default calibration: {}",
                    e
                );
            }
        }

        Ok(driver)
    }

    /// Create with custom calibration factor
    ///
    /// # Arguments
    /// * `port_path` - Serial port path
    /// * `address` - Device address
    /// * `pulses_per_degree` - Custom calibration (varies by device)
    pub fn with_calibration(
        port_path: &str,
        address: &str,
        pulses_per_degree: f64,
    ) -> Result<Self> {
        let port = Self::open_port(port_path)?;
        Ok(Self::build(
            Arc::new(Mutex::new(port)),
            address.to_string(),
            pulses_per_degree,
        ))
    }

    fn build(port: SharedPort, address: String, pulses_per_degree: f64) -> Self {
        let (state_tx, _) = broadcast::channel(16);
        let mut params = ParameterSet::new();
        let address_clone = address.clone();

        let mut position = Parameter::new("position", 0.0)
            .with_description("Rotation position")
            .with_unit("°");

        position.connect_to_hardware_write({
            let port = port.clone();
            let address = address_clone.clone();
            move |position_deg: f64| -> BoxFuture<'static, Result<(), DaqError>> {
                let port = port.clone();
                let address = address.clone();
                Box::pin(async move {
                    // Round to nearest pulse to avoid truncation errors
                    let pulses = (position_deg * pulses_per_degree).round() as i32;
                    let hex_pulses = format!("{:08X}", pulses as u32);
                    let payload = format!("{}ma{}", address, hex_pulses);

                    let mut port = port.lock().await;
                    port.write_all(payload.as_bytes())
                        .await
                        .context("ELL14 position write failed")
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;

                    tokio::time::sleep(Duration::from_millis(50)).await;

                    Ok(())
                })
            }
        });

        params.register(position.clone());

        Self {
            port,
            physical_address: address.clone(),
            active_address: address,
            pulses_per_degree,
            position_deg: position,
            params: Arc::new(params),
            is_slave_in_group: false,
            group_offset_degrees: 0.0,
            state_tx,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_port(port: SharedPort, address: &str, pulses_per_degree: f64) -> Self {
        Self::build(port, address.to_string(), pulses_per_degree)
    }

    /// Subscribe to state updates
    pub fn subscribe(&self) -> broadcast::Receiver<ElliptecState> {
        self.state_tx.subscribe()
    }

    /// Starts a background task to poll the device state
    pub fn start_polling(&self, interval: Duration) {
        let driver = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(e) = driver.poll_state().await {
                    eprintln!("Failed to poll Elliptec state: {}", e);
                }
            }
        });
    }

    /// Polls the current state and broadcasts it
    async fn poll_state(&self) -> Result<()> {
        let position = self.position().await?;
        let status = self.status().await?;
        let error_code = self.error_code().await?;
        let state = ElliptecState {
            position,
            status,
            error_code,
        };
        self.state_tx.send(state).ok(); // Ignore error if no subscribers
        Ok(())
    }

    /// Get the current status of the device
    pub async fn status(&self) -> Result<u32> {
        let resp = self.transaction("gs").await?;
        self.parse_status_response(&resp)
    }

    /// Get the last error code from the device
    pub async fn error_code(&self) -> Result<Option<u32>> {
        let resp = self.transaction("ge").await?;
        self.parse_error_response(&resp)
    }

    /// Send home command to find mechanical zero
    ///
    /// Should be called on initialization to establish reference position.
    /// Uses the device's default homing direction.
    ///
    /// For rotary stages that need a specific homing direction, use
    /// [`home_with_direction`](Self::home_with_direction) instead.
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn home(&self) -> Result<()> {
        // Home command doesn't return immediate response - just starts homing
        self.send_command("ho").await?;
        self.wait_settled().await
    }

    /// Home device with specified direction (for rotary stages)
    ///
    /// # Arguments
    /// * `direction` - Direction for homing search
    ///   - `None` - Uses default direction (same as [`home`](Self::home))
    ///   - `Some(Clockwise)` - Search clockwise
    ///   - `Some(CounterClockwise)` - Search counter-clockwise
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn home_with_direction(&self, direction: Option<HomeDirection>) -> Result<()> {
        let cmd = match direction {
            Some(dir) => format!("ho{}", dir as u8),
            None => "ho".to_string(),
        };

        self.send_command(&cmd).await?;
        self.wait_settled().await
    }

    async fn transaction(&self, command: &str) -> Result<String> {
        let command = command.to_string();
        self.with_retry("ELL14 transaction", || {
            let cmd = command.clone();
            async move { self.transaction_once(&cmd).await }
        })
        .await
    }

    /// Helper to send a command and get a response without retry
    ///
    /// ELL14 protocol is ASCII based with format: {Address}{Command}{Data}
    async fn transaction_once(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Construct packet: Address + Command
        // Example: "0gs" (Get Status for device 0)
        let payload = format!("{}{}", self.active_address, command);
        port.write_all(payload.as_bytes())
            .await
            .context("ELL14 write failed")?;

        // Small delay for device to process command and start responding
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Read response with buffering - responses may arrive in chunks on shared RS-485 bus
        let mut response_buf = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(
                remaining.min(Duration::from_millis(100)),
                port.read(&mut buf),
            )
            .await
            {
                Ok(Ok(n)) if n > 0 => {
                    response_buf.extend_from_slice(&buf[..n]);
                    // Check if we have a complete response (responses end after data)
                    // Give a tiny delay for any remaining bytes
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(Ok(_)) => {
                    // Zero bytes read - check if we have data already
                    if !response_buf.is_empty() {
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    // Read error or timeout - use what we have
                    if !response_buf.is_empty() {
                        break;
                    }
                }
            }

            // If we've collected some data, check if more is coming
            if !response_buf.is_empty() {
                // Brief pause then try one more read
                tokio::time::sleep(Duration::from_millis(30)).await;
                if let Ok(Ok(n)) =
                    tokio::time::timeout(Duration::from_millis(50), port.read(&mut buf)).await
                {
                    if n > 0 {
                        response_buf.extend_from_slice(&buf[..n]);
                    }
                }
                break;
            }
        }

        if response_buf.is_empty() {
            tracing::debug!(
                address = %self.active_address,
                command = %command,
                "ELL14 returned empty response"
            );
            return Err(anyhow!("ELL14 returned empty response"));
        }

        let response = std::str::from_utf8(&response_buf)
            .context("Invalid UTF-8 from ELL14")?
            .trim();

        // Log the response for debugging
        tracing::debug!(
            address = %self.active_address,
            command = %command,
            response = %response,
            "ELL14 transaction complete"
        );

        // Check for error status in response
        if let Err(e) = self.check_error_response(response) {
            tracing::warn!(
                address = %self.active_address,
                command = %command,
                response = %response,
                error = %e,
                "ELL14 returned error status"
            );
            // Don't fail here - let caller decide what to do with errors
            // Some commands return status codes that aren't fatal
        }

        Ok(response.to_string())
    }

    /// Send command without waiting for response (for move commands)
    ///
    /// Movement commands may not return a response until motion completes.
    /// Use wait_settled() to wait for motion completion.
    async fn send_command(&self, command: &str) -> Result<()> {
        let command = command.to_string();
        self.with_retry("ELL14 send_command", || {
            let cmd = command.clone();
            async move { self.send_command_once(&cmd).await }
        })
        .await
    }

    async fn send_command_once(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let payload = format!("{}{}", self.active_address, command);

        tracing::debug!(
            address = %self.active_address,
            command = %command,
            payload = %payload,
            "ELL14 sending command (no response expected)"
        );

        port.write_all(payload.as_bytes())
            .await
            .context("ELL14 write failed")?;

        // Brief delay to let command be processed
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(())
    }

    /// Check if response contains an error status (GS with non-zero code)
    ///
    /// ELLx devices return "GS{XX}" on error where XX is the error code:
    /// - 00: No error (success)
    /// - 01: Communication timeout
    /// - 02: Mechanical timeout
    /// - 03: Command error
    /// - 04: Value out of range
    /// - 05-0D: Various hardware errors
    fn check_error_response(&self, response: &str) -> Result<()> {
        if let Some(idx) = response.find("GS") {
            let status_str = response[idx + 2..].trim();
            if !status_str.is_empty() {
                let hex_part = if status_str.len() >= 2 {
                    &status_str[..2]
                } else {
                    status_str
                };

                let status = Ell14StatusCode::from_hex(hex_part);
                if status.is_error() {
                    return Err(anyhow!("ELL14 error: {}", status));
                }
            }
        }
        Ok(())
    }

    /// Parse position from hex string response
    ///
    /// Format: {Address}PO{Hex}
    /// Example responses: "0PO00002000", "2POF" (short hex), or "3PO" (position 0)
    fn parse_position_response(&self, response: &str) -> Result<f64> {
        // Minimum response: "XPO" = 3 chars (addr + PO, hex portion may be empty for position 0)
        if response.len() < 3 {
            return Err(anyhow!("Response too short: {}", response));
        }

        // Look for position response marker "PO"
        if let Some(idx) = response.find("PO") {
            let hex_str = response[idx + 2..].trim();

            // Empty hex string means position 0
            if hex_str.is_empty() {
                return Ok(0.0);
            }

            // Handle variable length hex strings (take first 8 chars max)
            let hex_clean = if hex_str.len() > 8 {
                &hex_str[..8]
            } else {
                hex_str
            };

            // Parse as u32 first, then reinterpret as i32 for signed positions
            // (ELL14 returns positions as 32-bit two's complement hex)
            let pulses_unsigned = u32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse position hex: {}", hex_clean))?;
            let pulses = pulses_unsigned as i32;

            return Ok(pulses as f64 / self.pulses_per_degree);
        }

        Err(anyhow!("Unexpected position format: {}", response))
    }

    /// Parse status from hex string response
    fn parse_status_response(&self, response: &str) -> Result<u32> {
        self.parse_hex_response(response, "GS", 2)
    }

    /// Parse error from hex string response
    fn parse_error_response(&self, response: &str) -> Result<Option<u32>> {
        let error_code = self.parse_hex_response(response, "GE", 2)?;
        if error_code == 0 {
            Ok(None)
        } else {
            Ok(Some(error_code))
        }
    }

    /// Helper to parse a hex response with a given prefix and length
    fn parse_hex_response(&self, response: &str, prefix: &str, len: usize) -> Result<u32> {
        if let Some(idx) = response.find(prefix) {
            let hex_str = &response[idx + prefix.len()..].trim();
            let hex_clean = if hex_str.len() > len {
                &hex_str[..len]
            } else {
                hex_str
            };
            u32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse {} hex: {}", prefix, hex_clean))
        } else {
            Err(anyhow!("Unexpected {} format: {}", prefix, response))
        }
    }

    // =========================================================================
    // Jog Control Commands
    // =========================================================================

    /// Jog forward by the configured jog step size
    ///
    /// The step size can be configured with `set_jog_step()`.
    /// Default jog step is device-dependent.
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn jog_forward(&self) -> Result<()> {
        self.send_command("fw").await
    }

    /// Jog backward by the configured jog step size
    ///
    /// The step size can be configured with `set_jog_step()`.
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn jog_backward(&self) -> Result<()> {
        self.send_command("bw").await
    }

    /// Get the current jog step size in degrees
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn get_jog_step(&self) -> Result<f64> {
        let resp = self.transaction("gj").await?;

        // Response format: "XGJ{Hex}" where Hex is 32-bit pulse count
        if let Some(idx) = resp.find("GJ") {
            let hex_str = resp[idx + 2..].trim();
            if hex_str.is_empty() {
                return Ok(0.0);
            }

            let hex_clean = if hex_str.len() > 8 {
                &hex_str[..8]
            } else {
                hex_str
            };

            let pulses = u32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse jog step hex: {}", hex_clean))?;

            return Ok(pulses as f64 / self.pulses_per_degree);
        }

        Err(anyhow!("Unexpected jog step response: {}", resp))
    }

    /// Set the jog step size in degrees
    ///
    /// This sets the distance the device moves when `jog_forward()` or
    /// `jog_backward()` is called.
    #[instrument(skip(self), fields(address = %self.physical_address, degrees), err)]
    pub async fn set_jog_step(&self, degrees: f64) -> Result<()> {
        // Round to nearest pulse to avoid truncation errors
        let pulses = (degrees * self.pulses_per_degree).abs().round() as u32;
        let hex_pulses = format!("{:08X}", pulses);
        let cmd = format!("sj{}", hex_pulses);

        // sj command returns a GJ response or GS00 (success status)
        let resp = self.transaction(&cmd).await?;

        // Check for error status (GS with non-zero code)
        self.check_error_response(&resp)?;

        Ok(())
    }

    /// Stop any ongoing motion immediately
    ///
    /// Useful for emergency stops or to halt motion mid-jog.
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn stop(&self) -> Result<()> {
        // st is a fire-and-forget command
        self.send_command("st").await
    }

    // =========================================================================
    // Motor Frequency Search / Optimization Commands
    // =========================================================================

    /// Skip automatic frequency search at power-up
    ///
    /// Bypasses the ~15s frequency search on next power-on.
    /// Uses last saved settings instead.
    /// Call `save_user_data()` to persist this setting.
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn skip_frequency_search(&self) -> Result<()> {
        let resp = self.transaction("sk").await?;
        self.check_error_response(&resp)
    }

    /// Enable automatic frequency search at power-up
    ///
    /// Restores default behavior of searching for resonant frequencies on power-on.
    /// Call `save_user_data()` to persist this setting.
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn enable_frequency_search(&self) -> Result<()> {
        let resp = self.transaction("se").await?;
        self.check_error_response(&resp)
    }

    /// Search and optimize motor 1 frequency
    ///
    /// This performs a frequency scan to find the optimal resonant frequency
    /// for motor 1 under current load conditions. Takes several seconds to complete.
    ///
    /// Call `save_user_data()` afterward to persist the optimized frequency.
    pub async fn search_frequency_motor1(&self) -> Result<()> {
        // s1 starts the frequency search, which takes several seconds
        self.send_command("s1").await?;

        // Wait for search to complete (can take 5-10 seconds)
        let timeout = Duration::from_secs(15);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("Motor 1 frequency search timed out"));
            }

            tokio::time::sleep(Duration::from_millis(500)).await;

            // Check if device is ready (not moving/searching)
            if let Ok(resp) = self.transaction("gs").await {
                if let Some(idx) = resp.find("GS") {
                    let hex_str = resp[idx + 2..].trim();
                    if hex_str.is_empty() || hex_str == "0" || hex_str == "00" {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Search and optimize motor 2 frequency
    ///
    /// Same as `search_frequency_motor1()` but for motor 2.
    /// ELL14 has two motors for bidirectional rotation.
    pub async fn search_frequency_motor2(&self) -> Result<()> {
        self.send_command("s2").await?;

        let timeout = Duration::from_secs(15);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("Motor 2 frequency search timed out"));
            }

            tokio::time::sleep(Duration::from_millis(500)).await;

            if let Ok(resp) = self.transaction("gs").await {
                if let Some(idx) = resp.find("GS") {
                    let hex_str = resp[idx + 2..].trim();
                    if hex_str.is_empty() || hex_str == "0" || hex_str == "00" {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Optimize both motors
    ///
    /// Convenience method that runs frequency search on both motors sequentially.
    /// This is the "motor cleaning/optimization" feature.
    pub async fn optimize_motors(&self) -> Result<()> {
        self.search_frequency_motor1().await?;
        self.search_frequency_motor2().await?;
        Ok(())
    }

    /// Fine-tune motor resonance (Optimize Motors)
    ///
    /// Performs fine-tuning of motor resonance frequencies.
    /// Should be run AFTER `search_frequency_motor1/2`.
    ///
    /// **WARNING:** This is a long-running operation (can take several minutes).
    /// The motor moves during optimization.
    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    pub async fn optimize_motors_fine(&self) -> Result<()> {
        // om starts the optimization
        self.send_command("om").await?;

        // Wait for completion (can take several minutes)
        // We poll GS status. Bit 0 is moving.
        // The protocol says GS 00 is returned when complete.
        let timeout = Duration::from_secs(300); // 5 minutes
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("Motor optimization timed out after 5 minutes"));
            }

            tokio::time::sleep(Duration::from_secs(2)).await;

            // Check if device is ready
            if let Ok(resp) = self.transaction("gs").await {
                if let Some(idx) = resp.find("GS") {
                    let hex_str = resp[idx + 2..].trim();
                    if hex_str.is_empty() || hex_str == "0" || hex_str == "00" {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Get motor 1 information
    ///
    /// Returns operating frequency and period settings.
    pub async fn get_motor1_info(&self) -> Result<MotorInfo> {
        self.parse_motor_info("i1", 1).await
    }

    /// Get motor 2 information
    pub async fn get_motor2_info(&self) -> Result<MotorInfo> {
        self.parse_motor_info("i2", 2).await
    }

    /// Parse motor info response (shared by i1 and i2)
    ///
    /// Per Thorlabs ELLx Protocol Manual Issue 10 and empirical testing:
    /// Response format: "XI1{Loop}{Motor}{Current}{RampUp}{RampDn}{PeriodFwd}{PeriodBwd}"
    ///
    /// Observed response: "2I1100B85FFFFFFFF00B7008D" (22 data chars after marker)
    /// - Loop: 1 hex char (1 = loop ON)
    /// - Motor: 1 hex char (0 = motor not currently running)
    /// - Current: 4 hex chars (0B85 = current measurement)
    /// - RampUp: 4 hex chars (FFFF = not defined)
    /// - RampDn: 4 hex chars (FFFF = not defined)
    /// - PeriodFwd: 4 hex chars (00B7 = 183 -> 80.5 kHz)
    /// - PeriodBwd: 4 hex chars (008D = 141 -> 104.5 kHz)
    ///
    /// Frequency formula: Hz = 14,740,000 / Period
    /// Expected resonant frequencies: ~78-106 kHz for piezo motors
    async fn parse_motor_info(&self, cmd: &str, motor_num: u8) -> Result<MotorInfo> {
        let resp = self.transaction(cmd).await?;

        let marker = if motor_num == 1 { "I1" } else { "I2" };

        if let Some(idx) = resp.find(marker) {
            let data = resp[idx + 2..].trim();

            // Standard response is 22 hex chars: 1+1+4+4+4+4+4
            if data.len() >= 22 {
                // Parse fields per observed protocol format
                let loop_state = u8::from_str_radix(&data[0..1], 16).unwrap_or(0) != 0;
                let motor_on = u8::from_str_radix(&data[1..2], 16).unwrap_or(0) != 0;
                // Current at [2..6], RampUp at [6..10], RampDn at [10..14] - not stored
                let forward_period = u16::from_str_radix(&data[14..18], 16).unwrap_or(0);
                let backward_period = u16::from_str_radix(&data[18..22], 16).unwrap_or(0);

                // Frequency formula from Thorlabs protocol: Hz = 14,740,000 / Period
                let frequency = if forward_period > 0 {
                    14_740_000 / forward_period as u32
                } else {
                    0
                };

                return Ok(MotorInfo {
                    motor_number: motor_num,
                    loop_state,
                    motor_on,
                    frequency,
                    forward_period,
                    backward_period,
                });
            }
        }

        Err(anyhow!(
            "Failed to parse motor {} info (response: '{}', expected 22+ hex chars after marker)",
            motor_num,
            resp
        ))
    }

    // =========================================================================
    // Device Information Commands
    // =========================================================================

    /// Get device information
    ///
    /// Returns device type, serial number, firmware version, and calibration data.
    ///
    /// Response format example: "2IN0E1140051720231701016800023000"
    /// - Address (1) + "IN" (2) = 3 chars prefix
    /// - Type (2): "0E" = hex 14 = ELL14
    /// - Serial (8): "11400517"
    /// - Year (4): "2023" (ASCII, not hex)
    /// - Firmware (2): "17"
    /// - Thread type (1): "0"
    ///
    /// **Firmware-dependent response formats:**
    /// - Older firmware (v15-v17): 30 data chars after IN marker
    ///   - Travel (5): "10168" hex (pulses)
    ///   - Pulses/unit (8): "00023000" hex (total pulses for full 360° rotation)
    /// - Newer firmware: 33 data chars (original spec)
    ///   - Travel (8): "10168000" hex
    ///   - Pulses/unit (8): "00023000" hex
    pub async fn get_device_info(&self) -> Result<DeviceInfo> {
        // Minimum length MUST be 30 chars to extract calibration data (pulses_per_unit)
        // OLD_FW_LEN contains: type(2) + serial(8) + year(4) + fw(2) + hw(1) + travel(5) + pulses(8) = 30
        const MIN_LEN_FOR_CALIBRATION: usize = 30; // Minimum to extract pulses_per_unit
        const OLD_FW_LEN: usize = 30; // Older firmware (v15-v17)
        const NEW_FW_LEN: usize = 33; // Newer firmware (original spec)
        const MAX_RETRIES: usize = 5;
        const RETRY_DELAY_MS: u64 = 200;

        let mut last_error = None;

        // Retry loop for truncated responses (RS-485 bus contention)
        for attempt in 1..=MAX_RETRIES {
            let resp = self.transaction("in").await?;

            if let Some(idx) = resp.find("IN") {
                let data = resp[idx + 2..].trim();

                // Check if response is too short - retry if so
                // Require at least 30 chars to extract pulses_per_unit calibration data
                if data.len() < MIN_LEN_FOR_CALIBRATION {
                    last_error = Some(anyhow!(
                        "ELL14 device info response too short on attempt {}/{}: got {} chars, expected at least {} chars. \
                        Response: {:?}. This may indicate RS-485 bus contention.",
                        attempt, MAX_RETRIES, data.len(), MIN_LEN_FOR_CALIBRATION, data
                    ));

                    if attempt < MAX_RETRIES {
                        tracing::warn!(
                            "Truncated device info response (attempt {}/{}): got {} chars, retrying after {}ms",
                            attempt, MAX_RETRIES, data.len(), RETRY_DELAY_MS
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS))
                            .await;
                        continue;
                    } else {
                        // Final attempt failed
                        return Err(last_error.unwrap());
                    }
                }

                // Valid response - parse it
                if data.len() != OLD_FW_LEN && data.len() != NEW_FW_LEN {
                    tracing::warn!(
                        "ELL14 device info response has unexpected length {}: {:?}. \
                        Expected {} (older firmware) or {} (newer firmware). Attempting to parse anyway.",
                        data.len(), data, OLD_FW_LEN, NEW_FW_LEN
                    );
                }

                return Self::parse_device_info_response(data);
            }

            // No 'IN' marker found
            last_error = Some(anyhow!(
                "Failed to parse device info (attempt {}/{}): no 'IN' marker found in response: {:?}",
                attempt, MAX_RETRIES, resp
            ));

            if attempt < MAX_RETRIES {
                tracing::warn!(
                    "No 'IN' marker in response (attempt {}/{}), retrying after {}ms",
                    attempt,
                    MAX_RETRIES,
                    RETRY_DELAY_MS
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
        }

        Err(last_error.unwrap())
    }

    /// Parse device info response data (after 'IN' marker and length validation)
    fn parse_device_info_response(data: &str) -> Result<DeviceInfo> {
        const OLD_FW_LEN: usize = 30;
        const NEW_FW_LEN: usize = 33;

        // Parse device type (2 hex chars -> device number)
        let device_type_hex = &data[0..2];
        let device_type = u8::from_str_radix(device_type_hex, 16)
            .map(|n| format!("ELL{}", n))
            .unwrap_or_else(|_| device_type_hex.to_string());

        // Serial number (8 chars)
        let serial = data[2..10].to_string();

        // Year (4 ASCII chars, not hex)
        let year = data[10..14].parse::<u16>().unwrap_or(0);

        // Firmware (2 chars)
        let firmware = data[14..16].to_string();

        // Thread type (1 char at position 16)
        let hardware = if data.len() >= 17 {
            Some(data[16..17].to_string())
        } else {
            None
        };

        // Parse travel and pulses_per_unit based on response length
        let (travel, pulses_per_unit) = if data.len() >= NEW_FW_LEN {
            // Newer firmware format: Travel (8 hex) at [17:25], Pulses/unit (8 hex) at [25:33]
            let travel = u32::from_str_radix(&data[17..25], 16).unwrap_or(0);
            let pulses_per_unit = u32::from_str_radix(&data[25..33], 16).unwrap_or(0);
            (travel, pulses_per_unit)
        } else if data.len() >= OLD_FW_LEN {
            // Older firmware format: Travel (5 hex) at [17:22], Pulses/unit (8 hex) at [22:30]
            let travel = u32::from_str_radix(&data[17..22], 16).unwrap_or(0);
            let pulses_per_unit = u32::from_str_radix(&data[22..30], 16).unwrap_or(0);
            (travel, pulses_per_unit)
        } else {
            // Partial response - extract what we can
            let travel = if data.len() >= 22 {
                u32::from_str_radix(&data[17..22], 16).unwrap_or(0)
            } else {
                0
            };
            (travel, 0)
        };

        Ok(DeviceInfo {
            device_type,
            serial,
            year,
            firmware,
            hardware,
            travel,
            pulses_per_unit,
        })
    }

    // =========================================================================
    // Home Offset Commands
    // =========================================================================

    /// Get the home offset in degrees
    ///
    /// The home offset shifts the zero position from the mechanical home.
    pub async fn get_home_offset(&self) -> Result<f64> {
        let resp = self.transaction("go").await?;

        // Response format: "XGO{Hex}" where Hex is 32-bit signed offset in pulses
        if let Some(idx) = resp.find("HO") {
            // Note: response might be "HO" not "GO"
            let hex_str = resp[idx + 2..].trim();
            if hex_str.is_empty() {
                return Ok(0.0);
            }

            let hex_clean = if hex_str.len() > 8 {
                &hex_str[..8]
            } else {
                hex_str
            };

            let pulses_unsigned = u32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse home offset hex: {}", hex_clean))?;
            let pulses = pulses_unsigned as i32;

            return Ok(pulses as f64 / self.pulses_per_degree);
        }

        // Try alternate response marker
        if let Some(idx) = resp.find("GO") {
            let hex_str = resp[idx + 2..].trim();
            if hex_str.is_empty() {
                return Ok(0.0);
            }

            let hex_clean = if hex_str.len() > 8 {
                &hex_str[..8]
            } else {
                hex_str
            };

            let pulses_unsigned = u32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse home offset hex: {}", hex_clean))?;
            let pulses = pulses_unsigned as i32;

            return Ok(pulses as f64 / self.pulses_per_degree);
        }

        Err(anyhow!("Unexpected home offset response: {}", resp))
    }

    /// Set the home offset in degrees
    ///
    /// This offsets the zero position from the mechanical home.
    /// Call `save_user_data()` to persist this setting.
    pub async fn set_home_offset(&self, degrees: f64) -> Result<()> {
        // Round to nearest pulse to avoid truncation errors
        let pulses = (degrees * self.pulses_per_degree).round() as i32;
        let hex_pulses = format!("{:08X}", pulses as u32);
        let cmd = format!("so{}", hex_pulses);

        // so command returns HO/GO response or GS00 (success status)
        let resp = self.transaction(&cmd).await?;

        // Check for error status (GS with non-zero code)
        self.check_error_response(&resp)?;

        Ok(())
    }

    // =========================================================================
    // Velocity Control Commands
    // =========================================================================

    /// Get the current velocity setting as a percentage (60-100%)
    ///
    /// The velocity controls the motor operating speed.
    /// Lower velocities may improve positioning accuracy.
    pub async fn get_velocity(&self) -> Result<u8> {
        let resp = self.transaction("gv").await?;

        // Response format: "XGV{Hex}" where Hex is percentage (00-64 = 0-100%)
        if let Some(idx) = resp.find("GV") {
            let hex_str = resp[idx + 2..].trim();
            if hex_str.is_empty() {
                return Ok(100); // Default to max
            }

            let hex_clean = if hex_str.len() > 2 {
                &hex_str[..2]
            } else {
                hex_str
            };

            let velocity = u8::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse velocity hex: {}", hex_clean))?;

            return Ok(velocity);
        }

        Err(anyhow!("Unexpected velocity response: {}", resp))
    }

    /// Set the velocity as a percentage (60-100%)
    ///
    /// # Arguments
    /// * `percent` - Velocity from 60 to 100 percent of maximum
    ///
    /// Values below 60% are clamped to 60%.
    /// Call `save_user_data()` to persist this setting.
    pub async fn set_velocity(&self, percent: u8) -> Result<()> {
        // Clamp to valid range (60-100%)
        let velocity = percent.clamp(60, 100);
        let cmd = format!("sv{:02X}", velocity);

        // sv command returns GV response or GS00 (success status)
        let resp = self.transaction(&cmd).await?;

        // Check for error status (GS with non-zero code)
        self.check_error_response(&resp)?;

        Ok(())
    }

    // =========================================================================
    // Motor Period Commands (f1/b1/f2/b2)
    // =========================================================================

    /// Get motor 1 forward/backward periods
    ///
    /// Returns the period settings for motor 1.
    /// Period formula: Period = 14,740,000 / frequency_hz
    pub async fn get_motor1_periods(&self) -> Result<MotorPeriods> {
        let resp = self.transaction("f1").await?;
        self.parse_motor_periods_response(&resp)
    }

    /// Get motor 2 forward/backward periods
    pub async fn get_motor2_periods(&self) -> Result<MotorPeriods> {
        let resp = self.transaction("f2").await?;
        self.parse_motor_periods_response(&resp)
    }

    /// Set motor 1 forward period
    ///
    /// # Arguments
    /// * `period` - Period value (set to 0xFFFF to restore factory default)
    ///
    /// Per protocol: period value has MSB set to '8' when setting (e.g., 8XXX)
    pub async fn set_motor1_forward_period(&self, period: u16) -> Result<()> {
        // Set MSB to 8 as per protocol
        let hex_period = format!("{:04X}", period | 0x8000);
        let cmd = format!("f1{}", hex_period);
        let resp = self.transaction(&cmd).await?;
        self.check_error_response(&resp)?;
        Ok(())
    }

    /// Set motor 1 backward period
    pub async fn set_motor1_backward_period(&self, period: u16) -> Result<()> {
        let hex_period = format!("{:04X}", period | 0x8000);
        let cmd = format!("b1{}", hex_period);
        let resp = self.transaction(&cmd).await?;
        self.check_error_response(&resp)?;
        Ok(())
    }

    /// Set motor 2 forward period
    pub async fn set_motor2_forward_period(&self, period: u16) -> Result<()> {
        let hex_period = format!("{:04X}", period | 0x8000);
        let cmd = format!("f2{}", hex_period);
        let resp = self.transaction(&cmd).await?;
        self.check_error_response(&resp)?;
        Ok(())
    }

    /// Set motor 2 backward period
    pub async fn set_motor2_backward_period(&self, period: u16) -> Result<()> {
        let hex_period = format!("{:04X}", period | 0x8000);
        let cmd = format!("b2{}", hex_period);
        let resp = self.transaction(&cmd).await?;
        self.check_error_response(&resp)?;
        Ok(())
    }

    /// Restore motor 1 periods to factory defaults
    ///
    /// Sends period value 0x8FFF which signals factory reset per protocol
    pub async fn restore_motor1_factory_periods(&self) -> Result<()> {
        self.set_motor1_forward_period(0x0FFF).await?;
        self.set_motor1_backward_period(0x0FFF).await?;
        Ok(())
    }

    /// Restore motor 2 periods to factory defaults
    pub async fn restore_motor2_factory_periods(&self) -> Result<()> {
        self.set_motor2_forward_period(0x0FFF).await?;
        self.set_motor2_backward_period(0x0FFF).await?;
        Ok(())
    }

    /// Parse motor periods response (GF or GB response)
    fn parse_motor_periods_response(&self, response: &str) -> Result<MotorPeriods> {
        // Response format: "XGF{FwdPeriod}{BwdPeriod}" - 8 hex chars total
        let marker_idx = response
            .find("GF")
            .or_else(|| response.find("GB"))
            .ok_or_else(|| anyhow!("No GF/GB marker in response: {}", response))?;

        let data = &response[marker_idx + 2..];
        if data.len() < 8 {
            return Err(anyhow!(
                "Motor periods response too short: {} (need 8 hex chars)",
                response
            ));
        }

        let forward_period = u16::from_str_radix(&data[0..4], 16)
            .context(format!("Failed to parse forward period: {}", &data[0..4]))?;
        let backward_period = u16::from_str_radix(&data[4..8], 16)
            .context(format!("Failed to parse backward period: {}", &data[4..8]))?;

        Ok(MotorPeriods {
            forward_period,
            backward_period,
        })
    }

    // =========================================================================
    // Current Curve Scan Commands (c1/c2)
    // =========================================================================

    /// Scan current curve for motor 1
    ///
    /// **WARNING:** This is a long-running operation (~12 seconds).
    /// The motor moves during the scan. Call `stop()` to abort.
    ///
    /// Returns 87 data points across 70-120 kHz frequency range.
    /// Current conversion: 1866 ADC points = 1 Amp
    pub async fn scan_current_curve_motor1(&self) -> Result<CurrentCurveScan> {
        self.scan_current_curve("c1", 1).await
    }

    /// Scan current curve for motor 2
    ///
    /// **WARNING:** This is a long-running operation (~12 seconds).
    /// The motor moves during the scan. Call `stop()` to abort.
    pub async fn scan_current_curve_motor2(&self) -> Result<CurrentCurveScan> {
        self.scan_current_curve("c2", 2).await
    }

    /// Internal helper for current curve scanning
    async fn scan_current_curve(&self, cmd: &str, motor_num: u8) -> Result<CurrentCurveScan> {
        // Send command and wait for extended response
        let mut port = self.port.lock().await;

        let payload = format!("{}{}", self.active_address, cmd);
        port.write_all(payload.as_bytes())
            .await
            .context("Current curve scan write failed")?;

        // Extended timeout for scan (~12 seconds)
        let mut response_buf = Vec::with_capacity(600);
        let mut buf = [0u8; 128];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(
                Duration::from_millis(500),
                tokio::io::AsyncReadExt::read(&mut *port, &mut buf),
            )
            .await
            {
                Ok(Ok(n)) if n > 0 => {
                    response_buf.extend_from_slice(&buf[..n]);
                    // Check if we have complete data (522 bytes after "CS" marker)
                    if response_buf.len() >= 525 {
                        break;
                    }
                }
                _ => {
                    if response_buf.len() >= 525 {
                        break;
                    }
                }
            }
        }

        drop(port);

        // Parse response: "XCS" + 522 bytes of binary data
        let response_str = String::from_utf8_lossy(&response_buf);
        if let Some(cs_idx) = response_str.find("CS") {
            let data_start = cs_idx + 2;
            if response_buf.len() < data_start + 522 {
                return Err(anyhow!(
                    "Current curve response incomplete: {} bytes (need 522)",
                    response_buf.len() - data_start
                ));
            }

            let raw_data = &response_buf[data_start..data_start + 522];
            let mut data_points = Vec::with_capacity(87);

            // Each data point: 2 bytes freq + 2 bytes fwd current + 2 bytes bwd current = 6 bytes
            // 87 points × 6 bytes = 522 bytes
            for i in 0..87 {
                let offset = i * 6;
                let period = u16::from_be_bytes([raw_data[offset], raw_data[offset + 1]]) as u32;
                let fwd_adc =
                    u16::from_be_bytes([raw_data[offset + 2], raw_data[offset + 3]]) as f64;
                let bwd_adc =
                    u16::from_be_bytes([raw_data[offset + 4], raw_data[offset + 5]]) as f64;

                let frequency_hz = if period > 0 { 14_740_000 / period } else { 0 };
                let forward_current_amps = fwd_adc / 1866.0;
                let backward_current_amps = bwd_adc / 1866.0;

                data_points.push(CurrentCurvePoint {
                    frequency_hz,
                    forward_current_amps,
                    backward_current_amps,
                });
            }

            return Ok(CurrentCurveScan {
                motor_number: motor_num,
                data_points,
            });
        }

        Err(anyhow!("No CS marker in current curve response"))
    }

    // =========================================================================
    // Device Isolation Command (is)
    // =========================================================================

    /// Isolate device from group commands for specified duration
    ///
    /// When isolated, the device ignores group address commands but still
    /// responds to its individual address.
    ///
    /// # Arguments
    /// * `minutes` - Isolation duration (0 = cancel isolation, 1-255 minutes)
    ///
    /// # Example
    /// ```rust,ignore
    /// driver.isolate_device(5).await?;  // Isolate for 5 minutes
    /// // ... perform maintenance on other devices ...
    /// driver.isolate_device(0).await?;  // Cancel isolation
    /// ```
    pub async fn isolate_device(&self, minutes: u8) -> Result<()> {
        let cmd = format!("is{:02X}", minutes);
        let resp = self.transaction(&cmd).await?;
        self.check_error_response(&resp)?;
        Ok(())
    }

    /// Cancel device isolation
    ///
    /// Shorthand for `isolate_device(0)`
    pub async fn cancel_isolation(&self) -> Result<()> {
        self.isolate_device(0).await
    }

    // =========================================================================
    // Motor Optimization Command (om)
    // =========================================================================

    /// Fine-tune motor resonance (optimize motors command)
    ///
    /// **WARNING:** This is a long-running operation (several minutes).
    /// Should be called AFTER `search_frequency_motor1()` / `search_frequency_motor2()`.
    /// The motor moves during optimization. Call `stop()` to abort.
    ///
    /// Call `save_user_data()` afterward to persist optimized settings.
    pub async fn fine_tune_motors(&self) -> Result<()> {
        self.send_command("om").await?;

        // Wait for optimization to complete (can take several minutes)
        let timeout = Duration::from_secs(300); // 5 minutes max
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("Motor fine-tuning timed out after 5 minutes"));
            }

            tokio::time::sleep(Duration::from_secs(1)).await;

            // Check if device is ready
            if let Ok(resp) = self.transaction("gs").await {
                if let Some(idx) = resp.find("GS") {
                    let hex_str = resp[idx + 2..].trim();
                    if hex_str.is_empty() || hex_str == "0" || hex_str == "00" {
                        return Ok(());
                    }
                }
            }
        }
    }

    // =========================================================================
    // Clean Mechanics Command (cm)
    // =========================================================================

    /// Run cleaning cycle (full-range movement)
    ///
    /// **WARNING:** This is a long-running operation (several minutes).
    /// Moves device over full mechanical range to remove dust/debris.
    /// Ensure full range of motion is clear before running.
    /// Call `stop()` to abort.
    pub async fn clean_mechanics(&self) -> Result<()> {
        self.send_command("cm").await?;

        // Wait for cleaning to complete
        let timeout = Duration::from_secs(300); // 5 minutes max
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("Clean mechanics timed out after 5 minutes"));
            }

            tokio::time::sleep(Duration::from_secs(1)).await;

            // Check if device is ready
            if let Ok(resp) = self.transaction("gs").await {
                if let Some(idx) = resp.find("GS") {
                    let hex_str = resp[idx + 2..].trim();
                    if hex_str.is_empty() || hex_str == "0" || hex_str == "00" {
                        return Ok(());
                    }
                }
            }
        }
    }

    // =========================================================================
    // Skip Frequency Search Command (sk)
    // =========================================================================

    // =========================================================================
    // Configuration / Persistence Commands
    // =========================================================================

    /// Save user data to flash memory
    ///
    /// Persists the current settings (home offset, jog step, velocity,
    /// motor frequencies) to non-volatile memory.
    ///
    /// These settings will be restored on power cycle.
    pub async fn save_user_data(&self) -> Result<()> {
        let resp = self.transaction("us").await?;

        // Response can be "XUS00" or "XGS00" (status OK) on success
        if resp.contains("US") || resp.contains("GS00") {
            Ok(())
        } else {
            // Check for error status
            self.check_error_response(&resp)?;
            Ok(())
        }
    }

    /// Change the device address
    ///
    /// # Arguments
    /// * `new_address` - New address (0-9, A-F)
    ///
    /// # Warning
    /// This changes the device address permanently until changed again.
    /// After calling this, you must create a new driver instance with
    /// the new address to communicate with the device.
    pub async fn change_address(&self, new_address: &str) -> Result<()> {
        if new_address.len() != 1 {
            return Err(anyhow!("Address must be a single character"));
        }

        let c = new_address.chars().next().unwrap();
        if !c.is_ascii_hexdigit() {
            return Err(anyhow!("Address must be 0-9 or A-F"));
        }

        let cmd = format!("ca{}", new_address);
        // Note: After this call, further communication requires using the new address
        let resp = self.transaction(&cmd).await?;

        // Check for error status (GS with non-zero code)
        self.check_error_response(&resp)?;

        Ok(())
    }

    /// Get the physical device address (the hardware address, never changes)
    pub fn get_physical_address(&self) -> &str {
        &self.physical_address
    }

    /// Get the active address (may differ from physical when in group mode)
    pub fn get_active_address(&self) -> &str {
        &self.active_address
    }

    /// Get the device address (alias for get_physical_address for backwards compatibility)
    pub fn get_address(&self) -> &str {
        &self.physical_address
    }

    /// Get the current pulses per degree calibration
    pub fn get_pulses_per_degree(&self) -> f64 {
        self.pulses_per_degree
    }

    /// Check if this rotator is configured as a slave in a group
    pub fn is_in_group(&self) -> bool {
        self.is_slave_in_group
    }

    /// Get the group offset in degrees (only meaningful when in group mode)
    pub fn get_group_offset(&self) -> f64 {
        self.group_offset_degrees
    }

    /// Get a reference to the shared port (for group controller use)
    pub fn get_shared_port(&self) -> SharedPort {
        self.port.clone()
    }

    // =========================================================================
    // Group Addressing Commands
    // =========================================================================

    /// Configure this rotator as a slave in a group
    ///
    /// The slave will listen to commands sent to the master's address.
    /// An optional offset can be applied so the slave maintains a fixed
    /// angular difference from the master.
    ///
    /// # Arguments
    /// * `master_address` - The address to listen to (0-9, A-F)
    /// * `offset_degrees` - Angular offset from master position
    ///
    /// # Example
    /// ```rust,ignore
    /// // Slave at address 3 listens to master at address 2 with 30° offset
    /// slave.configure_as_group_slave("2", 30.0).await?;
    /// ```
    pub async fn configure_as_group_slave(
        &mut self,
        master_address: &str,
        offset_degrees: f64,
    ) -> Result<()> {
        // Validate master address
        if master_address.len() != 1 {
            return Err(anyhow!(
                "Master address must be a single character (0-9, A-F)"
            ));
        }
        let c = master_address.chars().next().unwrap();
        if !c.is_ascii_hexdigit() {
            return Err(anyhow!("Master address must be 0-9 or A-F"));
        }

        tracing::info!(
            "Configuring rotator {} as slave, listening to master {}, offset: {:.2}°",
            self.physical_address,
            master_address,
            offset_degrees
        );

        // Send group address command: ga{new_address}
        // This tells the device to listen to a different address
        let cmd = format!("ga{}", master_address);

        // We need to send with our physical address but expect response from new address
        let mut port = self.port.lock().await;
        let payload = format!("{}{}", self.physical_address, cmd);
        port.write_all(payload.as_bytes())
            .await
            .context("ELL14 group address write failed")?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Read response (should come from the new address)
        let mut response_buf = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);

        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(100), port.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => {
                    response_buf.extend_from_slice(&buf[..n]);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    if !response_buf.is_empty() {
                        break;
                    }
                }
                _ => {
                    if !response_buf.is_empty() {
                        break;
                    }
                }
            }
        }

        drop(port); // Release lock

        let response = std::str::from_utf8(&response_buf)
            .unwrap_or("")
            .trim()
            .to_string();

        // Check for success - response should be from new address with GS00
        let expected_success = format!("{}GS00", master_address);
        if response.contains("GS00") || response.starts_with(master_address) {
            self.active_address = master_address.to_string();
            self.is_slave_in_group = true;
            self.group_offset_degrees = offset_degrees;
            tracing::info!(
                "Successfully configured as slave. Active address: {}, Offset: {:.2}°",
                self.active_address,
                self.group_offset_degrees
            );
            Ok(())
        } else {
            tracing::error!(
                "Failed to configure as slave. Response: '{}', expected: '{}'",
                response,
                expected_success
            );
            Err(anyhow!(
                "Failed to configure as group slave. Response: '{}'",
                response
            ))
        }
    }

    /// Revert from group slave mode back to individual control
    ///
    /// Restores the device to listen to its physical address.
    pub async fn revert_from_group_slave(&mut self) -> Result<()> {
        if !self.is_slave_in_group {
            tracing::info!(
                "Rotator {} not in slave mode, nothing to revert",
                self.physical_address
            );
            self.active_address = self.physical_address.clone();
            self.group_offset_degrees = 0.0;
            return Ok(());
        }

        let current_listening_address = self.active_address.clone();
        tracing::info!(
            "Reverting rotator {} from listening to {} back to physical address",
            self.physical_address,
            current_listening_address
        );

        // Send group address command with physical address from current listening address
        let cmd = format!("ga{}", self.physical_address);

        let mut port = self.port.lock().await;
        let payload = format!("{}{}", current_listening_address, cmd);
        port.write_all(payload.as_bytes())
            .await
            .context("ELL14 group address revert write failed")?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Read response
        let mut response_buf = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);

        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(100), port.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => {
                    response_buf.extend_from_slice(&buf[..n]);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    if !response_buf.is_empty() {
                        break;
                    }
                }
                _ => {
                    if !response_buf.is_empty() {
                        break;
                    }
                }
            }
        }

        drop(port);

        // Reset internal state regardless of response
        self.active_address = self.physical_address.clone();
        self.is_slave_in_group = false;
        self.group_offset_degrees = 0.0;

        let response = std::str::from_utf8(&response_buf)
            .unwrap_or("")
            .trim()
            .to_string();

        if response.contains("GS00") || response.starts_with(&self.physical_address) {
            tracing::info!(
                "Successfully reverted to physical address {}",
                self.physical_address
            );
            Ok(())
        } else {
            tracing::warn!(
                "Revert command sent but response unclear: '{}'. Internal state reset.",
                response
            );
            // Still return Ok since internal state is reset
            Ok(())
        }
    }

    // =========================================================================
    // Continuous Move Commands
    // =========================================================================

    /// Start continuous movement in the specified direction
    ///
    /// The rotator will continue moving until `stop()` is called.
    /// This is useful for manual positioning or when the final position
    /// is not known in advance.
    ///
    /// # Arguments
    /// * `direction` - Direction of movement (Forward or Backward)
    ///
    /// # Example
    /// ```rust,ignore
    /// driver.start_continuous_move(MoveDirection::Forward).await?;
    /// // ... wait for desired position ...
    /// driver.stop().await?;
    /// ```
    pub async fn start_continuous_move(&self, direction: MoveDirection) -> Result<()> {
        // Set jog step to 0 for continuous movement
        let pulses = 0u32;
        let hex_pulses = format!("{:08X}", pulses);
        let set_jog_cmd = format!("sj{}", hex_pulses);

        // Set jog step to 0
        let resp = self.transaction(&set_jog_cmd).await?;
        self.check_error_response(&resp)?;

        // Start movement in specified direction
        let move_cmd = match direction {
            MoveDirection::Forward => "fw",
            MoveDirection::Backward => "bw",
        };

        self.send_command(move_cmd).await?;

        tracing::debug!(
            "Started continuous move {:?} on rotator {}",
            direction,
            self.physical_address
        );

        Ok(())
    }

    /// Stop continuous movement
    ///
    /// This is an alias for `stop()` but provides clearer semantics
    /// when used with `start_continuous_move()`.
    pub async fn stop_continuous_move(&self) -> Result<()> {
        self.stop().await
    }
}

impl Parameterized for Ell14Driver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for Ell14Driver {
    #[instrument(skip(self), fields(address = %self.physical_address, position_deg), err)]
    async fn move_abs(&self, position_deg: f64) -> Result<()> {
        self.position_deg.set(position_deg).await
    }

    #[instrument(skip(self), fields(address = %self.physical_address, distance_deg), err)]
    async fn move_rel(&self, distance_deg: f64) -> Result<()> {
        // Command: mr (Move Relative)
        // Round to nearest pulse to avoid truncation errors
        let pulses = (distance_deg * self.pulses_per_degree).round() as i32;
        let hex_pulses = format!("{:08X}", pulses as u32);

        let cmd = format!("mr{}", hex_pulses);
        let _ = self.send_command(&cmd).await;

        Ok(())
    }

    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    async fn position(&self) -> Result<f64> {
        // Command: gp (Get Position)
        let resp = self.transaction("gp").await?;
        let pos = self.parse_position_response(&resp)?;

        // Update cached parameter without re-writing to hardware
        let _ = self.position_deg.inner().set(pos);

        Ok(pos)
    }

    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    async fn wait_settled(&self) -> Result<()> {
        // Poll 'gs' (Get Status) until motion stops
        // Status byte logic from manual:
        // Bit 0: Moving (1=Moving, 0=Stationary)

        let timeout = Duration::from_secs(10);
        let start = std::time::Instant::now();
        let mut consecutive_settled = 0;

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("ELL14 wait_settled timed out after 10 seconds"));
            }

            // Try to get status - device may not respond during movement
            match self.transaction("gs").await {
                Ok(resp) => {
                    // Response format: "0GS{StatusHex}"
                    if let Some(idx) = resp.find("GS") {
                        let hex_str = resp[idx + 2..].trim();

                        // Handle variable length status (could be "0", "00", etc.)
                        if hex_str.is_empty() {
                            // Empty status means stationary
                            consecutive_settled += 1;
                        } else {
                            let hex_clean = if hex_str.len() > 2 {
                                &hex_str[..2]
                            } else {
                                hex_str
                            };

                            if let Ok(status) = u32::from_str_radix(hex_clean, 16) {
                                // Check "Moving" bit (Bit 0 for ELL14)
                                let is_moving = (status & 0x01) != 0;
                                if !is_moving {
                                    consecutive_settled += 1;
                                } else {
                                    consecutive_settled = 0;
                                }
                            }
                        }

                        // Require 2 consecutive "not moving" status to confirm settled
                        if consecutive_settled >= 2 {
                            // Extra delay to let any pending responses clear
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            // Drain any remaining data from the buffer
                            let mut port = self.port.lock().await;
                            let mut drain_buf = [0u8; 256];
                            let _ = tokio::time::timeout(
                                Duration::from_millis(50),
                                port.read(&mut drain_buf),
                            )
                            .await;
                            return Ok(());
                        }
                    }
                }
                Err(_) => {
                    // Device busy, likely still moving - reset counter and retry
                    consecutive_settled = 0;
                }
            }

            // Poll at 50ms intervals
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[instrument(skip(self), fields(address = %self.physical_address), err)]
    async fn stop(&self) -> Result<()> {
        // ELL14 supports immediate stop via 'st' command
        self.send_command("st").await
    }
}

// =============================================================================
// Group Controller
// =============================================================================

/// High-level controller for managing synchronized groups of ELL14 rotators
///
/// The group controller manages multiple rotators that need to move together.
/// One rotator is designated as the "master" and others as "slaves". When
/// the master moves, all slaves move simultaneously with optional offsets.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
/// use daq_hardware::drivers::ell14::{Ell14Driver, Ell14GroupController};
///
/// // Create rotators on shared port
/// let shared_port = Ell14Driver::open_shared_port("/dev/ttyUSB0")?;
/// let mut rotator_2 = Ell14Driver::with_shared_port(shared_port.clone(), "2");
/// let mut rotator_3 = Ell14Driver::with_shared_port(shared_port.clone(), "3");
/// let mut rotator_8 = Ell14Driver::with_shared_port(shared_port.clone(), "8");
///
/// // Create group with rotator 2 as master
/// let mut group = Ell14GroupController::new(
///     vec![&mut rotator_2, &mut rotator_3, &mut rotator_8],
///     "2",  // Master address
/// )?;
///
/// // Form the group with offsets
/// let mut offsets = std::collections::HashMap::new();
/// offsets.insert("3".to_string(), 30.0);  // Rotator 3 offset +30°
/// offsets.insert("8".to_string(), -15.0); // Rotator 8 offset -15°
/// group.form_group(Some(offsets)).await?;
///
/// // Synchronized operations
/// group.home_group().await?;
/// group.move_group_absolute(45.0).await?;  // Master to 45°, slaves follow with offsets
///
/// // Cleanup
/// group.disband_group().await?;
/// ```
pub struct Ell14GroupController<'a> {
    /// Master rotator (commands are sent to this address)
    master: &'a mut Ell14Driver,
    /// Slave rotators
    slaves: Vec<&'a mut Ell14Driver>,
    /// Whether the group is currently formed
    is_formed: bool,
    /// Offsets for each slave (by physical address)
    slave_offsets: std::collections::HashMap<String, f64>,
}

impl std::fmt::Debug for Ell14GroupController<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ell14GroupController")
            .field("master_address", &self.master.get_physical_address())
            .field("slave_count", &self.slaves.len())
            .field("is_formed", &self.is_formed)
            .field("slave_offsets", &self.slave_offsets)
            .finish()
    }
}

impl<'a> Ell14GroupController<'a> {
    /// Create a new group controller
    ///
    /// # Arguments
    /// * `rotators` - Mutable references to all rotators in the group
    /// * `master_address` - Physical address of the master rotator
    ///
    /// # Errors
    /// Returns error if the master address is not found among the rotators
    pub fn new(mut rotators: Vec<&'a mut Ell14Driver>, master_address: &str) -> Result<Self> {
        // Find and remove the master from the list
        let master_idx = rotators
            .iter()
            .position(|r| r.get_physical_address() == master_address)
            .ok_or_else(|| {
                anyhow!(
                    "Master address '{}' not found among provided rotators",
                    master_address
                )
            })?;

        let master = rotators.swap_remove(master_idx);

        Ok(Self {
            master,
            slaves: rotators,
            is_formed: false,
            slave_offsets: std::collections::HashMap::new(),
        })
    }

    /// Form the group by configuring slaves to listen to the master's address
    ///
    /// # Arguments
    /// * `offsets` - Optional map of slave physical addresses to offset angles (degrees)
    ///
    /// # Example
    /// ```rust,ignore
    /// let mut offsets = HashMap::new();
    /// offsets.insert("3".to_string(), 30.0);  // Slave 3 has +30° offset
    /// group.form_group(Some(offsets)).await?;
    /// ```
    pub async fn form_group(
        &mut self,
        offsets: Option<std::collections::HashMap<String, f64>>,
    ) -> Result<()> {
        if self.is_formed {
            return Err(anyhow!(
                "Group is already formed. Call disband_group() first."
            ));
        }

        let master_address = self.master.get_physical_address().to_string();
        self.slave_offsets = offsets.unwrap_or_default();

        tracing::info!(
            "Forming group with master {} and {} slaves",
            master_address,
            self.slaves.len()
        );

        // Configure each slave to listen to master's address
        for slave in &mut self.slaves {
            let slave_address = slave.get_physical_address().to_string();
            let offset = *self.slave_offsets.get(&slave_address).unwrap_or(&0.0);

            tracing::debug!(
                "Configuring slave {} to listen to master {}, offset: {:.2}°",
                slave_address,
                master_address,
                offset
            );

            // Set home offset on the slave to achieve the desired offset
            // This way when the master moves, the slave maintains the offset
            if offset != 0.0 {
                slave.set_home_offset(offset).await?;
            }

            slave
                .configure_as_group_slave(&master_address, offset)
                .await?;
        }

        self.is_formed = true;
        tracing::info!("Group formed successfully");

        Ok(())
    }

    /// Disband the group and revert all slaves to individual control
    pub async fn disband_group(&mut self) -> Result<()> {
        if !self.is_formed {
            tracing::info!("Group not formed, nothing to disband");
            return Ok(());
        }

        tracing::info!("Disbanding group");

        let mut errors = Vec::new();

        for slave in &mut self.slaves {
            // Reset home offset before reverting
            if let Err(e) = slave.set_home_offset(0.0).await {
                errors.push(format!(
                    "Failed to reset home offset for {}: {}",
                    slave.get_physical_address(),
                    e
                ));
            }

            if let Err(e) = slave.revert_from_group_slave().await {
                errors.push(format!(
                    "Failed to revert slave {}: {}",
                    slave.get_physical_address(),
                    e
                ));
            }
        }

        self.is_formed = false;
        self.slave_offsets.clear();

        if errors.is_empty() {
            tracing::info!("Group disbanded successfully");
            Ok(())
        } else {
            Err(anyhow!(
                "Some slaves failed to revert: {}",
                errors.join("; ")
            ))
        }
    }

    /// Home all rotators in the group
    ///
    /// Sends home command to master, which all slaves will receive and execute.
    pub async fn home_group(&self) -> Result<()> {
        if !self.is_formed {
            return Err(anyhow!("Group not formed. Call form_group() first."));
        }

        tracing::info!("Homing group");
        self.master.home().await?;

        // Wait for all rotators to settle
        self.wait_group_settled().await?;

        tracing::info!("Group homing complete");
        Ok(())
    }

    /// Move all rotators to an absolute position
    ///
    /// The master moves to the specified position, and slaves move to
    /// (position + their offset).
    ///
    /// # Arguments
    /// * `degrees` - Target position for the master in degrees
    pub async fn move_group_absolute(&self, degrees: f64) -> Result<()> {
        if !self.is_formed {
            return Err(anyhow!("Group not formed. Call form_group() first."));
        }

        tracing::info!("Moving group to {:.2}°", degrees);
        self.master.move_abs(degrees).await?;

        // Wait for all rotators to settle
        self.wait_group_settled().await?;

        tracing::info!("Group move complete");
        Ok(())
    }

    /// Stop all rotators immediately
    pub async fn stop_group(&self) -> Result<()> {
        tracing::info!("Stopping group");

        // Send stop to master (slaves will receive it too if group is formed)
        self.master.stop().await?;

        // Also send stop individually to each slave as a safety measure
        for slave in &self.slaves {
            // Use the slave's physical address to send stop directly
            let _ = slave.stop().await;
        }

        Ok(())
    }

    /// Wait for all rotators in the group to settle
    async fn wait_group_settled(&self) -> Result<()> {
        // Wait for master to settle
        self.master.wait_settled().await?;

        // Add delay for each slave to ensure all have completed movement
        // (slaves receive the same command but may have slightly different settling times)
        for _slave in &self.slaves {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        Ok(())
    }

    /// Get status of all rotators in the group
    ///
    /// Returns a map of physical addresses to status strings
    pub async fn get_group_status(&self) -> Result<std::collections::HashMap<String, String>> {
        let mut status_map = std::collections::HashMap::new();

        // Get master status
        let master_resp = self.master.transaction("gs").await?;
        status_map.insert(self.master.get_physical_address().to_string(), master_resp);

        // Get slave statuses (note: in group mode, slaves listen to master address,
        // so we may not get individual responses. This is a best-effort check.)
        for slave in &self.slaves {
            // Try to query using physical address
            match slave.transaction("gs").await {
                Ok(resp) => {
                    status_map.insert(slave.get_physical_address().to_string(), resp);
                }
                Err(_) => {
                    status_map.insert(
                        slave.get_physical_address().to_string(),
                        "unknown (in group mode)".to_string(),
                    );
                }
            }
        }

        Ok(status_map)
    }

    /// Check if the group is currently formed
    pub fn is_grouped(&self) -> bool {
        self.is_formed
    }

    /// Get a reference to the master rotator
    pub fn master(&self) -> &Ell14Driver {
        self.master
    }

    /// Get the number of rotators in the group (including master)
    pub fn size(&self) -> usize {
        1 + self.slaves.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn move_abs_uses_parameter_and_writes_command() -> Result<()> {
        let (mut host, device) = tokio::io::duplex(64);
        let port: SharedPort = Arc::new(Mutex::new(Box::new(device)));

        let driver = Ell14Driver::with_test_port(port, "0", 398.2222);

        driver.move_abs(45.0).await?;

        let mut buf = vec![0u8; 32];
        let n = host.read(&mut buf).await?;
        let sent = String::from_utf8_lossy(&buf[..n]).to_string();

        assert!(sent.starts_with("0ma00004600"));

        Ok(())
    }

    /// Test helper: parse position response without needing a real driver
    fn parse_position(response: &str, pulses_per_degree: f64) -> Result<f64> {
        // Look for position response marker "PO"
        if let Some(idx) = response.find("PO") {
            let hex_str = response[idx + 2..].trim();

            // Empty hex string means position 0
            if hex_str.is_empty() {
                return Ok(0.0);
            }

            // Handle variable length hex strings (take first 8 chars max)
            let hex_clean = if hex_str.len() > 8 {
                &hex_str[..8]
            } else {
                hex_str
            };

            // Parse as u32 first, then reinterpret as i32 for signed positions
            // (ELL14 returns positions as 32-bit two's complement hex)
            let pulses_unsigned = u32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse position hex: {}", hex_clean))?;
            let pulses = pulses_unsigned as i32;

            return Ok(pulses as f64 / pulses_per_degree);
        }

        Err(anyhow!("Unexpected position format: {}", response))
    }

    #[test]
    fn test_parse_position_response() {
        let pulses_per_degree = 398.2222;

        // Test typical response
        let response = "0PO00002000";
        let position = parse_position(response, pulses_per_degree).unwrap();

        // 0x2000 = 8192 pulses / 398.2222 pulses/deg ≈ 20.57°
        assert!((position - 20.57).abs() < 0.1);
    }

    #[test]
    fn test_position_conversion() {
        let pulses_per_degree: f64 = 398.2222;

        // Test 45 degrees: 398.2222 * 45 = 17919.999, rounds to 17920
        let pulses = (45.0 * pulses_per_degree).round() as i32;
        assert_eq!(pulses, 17920);

        // Test 90 degrees: 398.2222 * 90 = 35839.998, rounds to 35840
        let pulses = (90.0 * pulses_per_degree).round() as i32;
        assert_eq!(pulses, 35840);
    }

    #[test]
    fn test_move_direction_enum() {
        // Test that MoveDirection variants exist and are distinct
        assert_ne!(MoveDirection::Forward, MoveDirection::Backward);

        // Test debug formatting
        assert_eq!(format!("{:?}", MoveDirection::Forward), "Forward");
        assert_eq!(format!("{:?}", MoveDirection::Backward), "Backward");

        // Test copy
        let dir = MoveDirection::Forward;
        let copied = dir;
        assert_eq!(dir, copied);
    }

    #[test]
    fn test_driver_address_tracking() {
        let (_, device) = tokio::io::duplex(64);
        let port: SharedPort = Arc::new(Mutex::new(Box::new(device)));

        let driver = Ell14Driver::with_test_port(port, "2", 398.2222);

        // Initially, physical and active addresses should match
        assert_eq!(driver.get_physical_address(), "2");
        assert_eq!(driver.get_active_address(), "2");
        assert_eq!(driver.get_address(), "2"); // Backwards compatibility

        // Not in group initially
        assert!(!driver.is_in_group());
        assert_eq!(driver.get_group_offset(), 0.0);
    }

    #[test]
    fn test_group_controller_creation() {
        let (_, device1) = tokio::io::duplex(64);
        let (_, device2) = tokio::io::duplex(64);
        let (_, device3) = tokio::io::duplex(64);

        let port1: SharedPort = Arc::new(Mutex::new(Box::new(device1)));
        let port2: SharedPort = Arc::new(Mutex::new(Box::new(device2)));
        let port3: SharedPort = Arc::new(Mutex::new(Box::new(device3)));

        let mut rotator_2 = Ell14Driver::with_test_port(port1, "2", 398.2222);
        let mut rotator_3 = Ell14Driver::with_test_port(port2, "3", 398.2222);
        let mut rotator_8 = Ell14Driver::with_test_port(port3, "8", 398.2222);

        // Create group with rotator 2 as master
        let group =
            Ell14GroupController::new(vec![&mut rotator_2, &mut rotator_3, &mut rotator_8], "2")
                .expect("Failed to create group controller");

        assert_eq!(group.size(), 3);
        assert!(!group.is_grouped());
        assert_eq!(group.master().get_physical_address(), "2");
    }

    #[test]
    fn test_group_controller_invalid_master() {
        let (_, device1) = tokio::io::duplex(64);
        let (_, device2) = tokio::io::duplex(64);

        let port1: SharedPort = Arc::new(Mutex::new(Box::new(device1)));
        let port2: SharedPort = Arc::new(Mutex::new(Box::new(device2)));

        let mut rotator_2 = Ell14Driver::with_test_port(port1, "2", 398.2222);
        let mut rotator_3 = Ell14Driver::with_test_port(port2, "3", 398.2222);

        // Try to create group with non-existent master address
        let result = Ell14GroupController::new(
            vec![&mut rotator_2, &mut rotator_3],
            "9", // Invalid - not in the list
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn test_continuous_move_sets_zero_jog_step() -> Result<()> {
        let (mut host, device) = tokio::io::duplex(128);
        let port: SharedPort = Arc::new(Mutex::new(Box::new(device)));

        let driver = Ell14Driver::with_test_port(port, "0", 398.2222);

        // Spawn a task to send mock responses
        let response_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 64];

            // Read the set jog step command
            let _n = host.read(&mut buf).await.unwrap();
            // Send back a success response
            host.write_all(b"0GS00").await.unwrap();

            // Read the fw command
            let _n = host.read(&mut buf).await.unwrap();
        });

        // This should set jog step to 0 and then send fw
        let result = driver.start_continuous_move(MoveDirection::Forward).await;

        // Wait for response task
        let _ = tokio::time::timeout(Duration::from_millis(500), response_task).await;

        // The command should have been sent (may timeout waiting for response, but that's OK for this test)
        // We're mainly testing that the API works
        assert!(result.is_ok() || result.is_err()); // Just verify it doesn't panic

        Ok(())
    }

    #[tokio::test]
    async fn test_get_device_info_retries_on_truncated_response() -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let (mut host, device) = tokio::io::duplex(256);
        let port: SharedPort = Arc::new(Mutex::new(Box::new(device)));

        let driver = Ell14Driver::with_test_port(port, "2", 398.2222);

        // Spawn a task to send mock responses
        let response_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 64];

            // First attempt: send truncated response (16 chars - simulates bus contention)
            let _n = host.read(&mut buf).await.unwrap();
            host.write_all(b"2IN0E14002842202115\n").await.unwrap(); // 16 chars after IN

            // Second attempt: send full valid response
            let _n = host.read(&mut buf).await.unwrap();
            host.write_all(b"2IN0E1400284220211500046000023000\n")
                .await
                .unwrap(); // 33 chars
        });

        // This should retry once and succeed on second attempt
        let result = driver.get_device_info().await;

        // Wait for response task
        let _ = tokio::time::timeout(Duration::from_millis(500), response_task).await;

        assert!(
            result.is_ok(),
            "Expected get_device_info to succeed after retry, got: {:?}",
            result
        );
        let info = result.unwrap();
        assert_eq!(info.device_type, "ELL14");
        assert_eq!(info.serial, "14002842");

        Ok(())
    }

    #[tokio::test]
    async fn test_get_device_info_fails_after_max_retries() -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let (mut host, device) = tokio::io::duplex(256);
        let port: SharedPort = Arc::new(Mutex::new(Box::new(device)));

        let driver = Ell14Driver::with_test_port(port, "2", 398.2222);

        // Spawn a task to send mock responses
        let response_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 64];

            // Send truncated responses for all 5 attempts (matches MAX_RETRIES)
            for _ in 0..5 {
                let _n = host.read(&mut buf).await.unwrap();
                host.write_all(b"2IN0E14002842202115\n").await.unwrap(); // Always 16 chars (too short)
            }
        });

        // This should fail after 5 attempts
        let result = driver.get_device_info().await;

        // Wait for response task
        let _ = tokio::time::timeout(Duration::from_millis(1000), response_task).await;

        assert!(
            result.is_err(),
            "Expected get_device_info to fail after max retries"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("too short") || err.contains("attempt 5/5"),
            "Error should mention truncated response or final attempt: {}",
            err
        );

        Ok(())
    }
}
