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
//! ```no_run
//! use rust_daq::hardware::ell14::Ell14Driver;
//! use rust_daq::hardware::capabilities::Movable;
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

use crate::hardware::capabilities::{Movable, Parameterized};
use crate::observable::ParameterSet;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tokio::time::sleep;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

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
pub struct Ell14Driver {
    /// Serial port protected by Arc<Mutex> for shared access across multiple drivers
    port: Arc<Mutex<SerialStream>>,
    /// Device address (0-9, A-F)
    address: String,
    /// Calibration factor: Pulses per Degree
    /// Default: 398.22 (143360 pulses / 360 degrees for ELL14)
    pulses_per_degree: f64,
    /// Parameter registry
    params: ParameterSet,
}

impl Ell14Driver {
    /// Create a new ELL14 driver instance (opens dedicated serial port)
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0" on Linux, "COM3" on Windows)
    /// * `address` - Device address (usually "0")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    ///
    /// # Note
    /// For multidrop bus configurations with multiple devices on the same port,
    /// use [`open_shared_port`] + [`with_shared_port`] instead.
    pub fn new(port_path: &str, address: &str) -> Result<Self> {
        let port = Self::open_port(port_path)?;
        Ok(Self {
            port: Arc::new(Mutex::new(port)),
            address: address.to_string(),
            pulses_per_degree: 398.2222, // 143360 pulses / 360 degrees
            params: ParameterSet::new(),
        })
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
    pub fn open_shared_port(port_path: &str) -> Result<Arc<Mutex<SerialStream>>> {
        let port = Self::open_port(port_path)?;
        Ok(Arc::new(Mutex::new(port)))
    }

    /// Create an ELL14 driver using a shared serial port
    ///
    /// This is the preferred method for multidrop bus configurations where
    /// multiple devices share the same physical serial port.
    ///
    /// # Arguments
    /// * `shared_port` - Arc<Mutex<SerialStream>> from [`open_shared_port`]
    /// * `address` - Device address on the bus (0-9, A-F)
    pub fn with_shared_port(shared_port: Arc<Mutex<SerialStream>>, address: &str) -> Self {
        Self {
            port: shared_port,
            address: address.to_string(),
            pulses_per_degree: 398.2222,
            params: ParameterSet::new(),
        }
    }

    /// Internal helper to open a serial port with ELL14 settings
    fn open_port(port_path: &str) -> Result<SerialStream> {
        tokio_serial::new(port_path, 9600)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!("Failed to open ELL14 serial port: {}", port_path))
    }

    /// Create a new ELL14 driver instance asynchronously
    ///
    /// This is the preferred constructor as it uses `spawn_blocking` to avoid
    /// blocking the async runtime during serial port opening.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0" on Linux, "COM3" on Windows)
    /// * `address` - Device address (usually "0")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    pub async fn new_async(port_path: &str, address: &str) -> Result<Self> {
        let port_path = port_path.to_string();
        let address = address.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        let port = spawn_blocking(move || Self::open_port(&port_path))
            .await
            .context("spawn_blocking for ELL14 port opening failed")??;

        Ok(Self {
            port: Arc::new(Mutex::new(port)),
            address,
            pulses_per_degree: 398.2222, // 143360 pulses / 360 degrees
            params: ParameterSet::new(),
        })
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
        let mut driver = Self::new(port_path, address)?;
        driver.pulses_per_degree = pulses_per_degree;
        Ok(driver)
    }

    /// Send home command to find mechanical zero
    ///
    /// Should be called on initialization to establish reference position
    pub async fn home(&self) -> Result<()> {
        // Home command doesn't return immediate response - just starts homing
        self.send_command("ho").await?;
        self.wait_settled().await
    }

    /// Helper to send a command and get a response
    ///
    /// ELL14 protocol is ASCII based with format: {Address}{Command}{Data}
    async fn transaction(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Construct packet: Address + Command
        // Example: "0gs" (Get Status for device 0)
        let payload = format!("{}{}", self.address, command);
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

            match tokio::time::timeout(remaining.min(Duration::from_millis(100)), port.read(&mut buf)).await {
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
                if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(50), port.read(&mut buf)).await {
                    if n > 0 {
                        response_buf.extend_from_slice(&buf[..n]);
                    }
                }
                break;
            }
        }

        if response_buf.is_empty() {
            return Err(anyhow!("ELL14 returned empty response"));
        }

        let response = std::str::from_utf8(&response_buf)
            .context("Invalid UTF-8 from ELL14")?
            .trim();

        Ok(response.to_string())
    }

    /// Send command without waiting for response (for move commands)
    ///
    /// Movement commands may not return a response until motion completes.
    /// Use wait_settled() to wait for motion completion.
    async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let payload = format!("{}{}", self.address, command);
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

                if let Ok(status_code) = u8::from_str_radix(hex_part, 16) {
                    if status_code != 0 {
                        let error_msg = match status_code {
                            0x01 => "Communication timeout",
                            0x02 => "Mechanical timeout",
                            0x03 => "Command error",
                            0x04 => "Value out of range",
                            0x05 => "Module isolated",
                            0x06 => "Module out of isolation",
                            0x07 => "Initialization error",
                            0x08 => "Thermal error",
                            0x09 => "Busy",
                            0x0A => "Sensor error",
                            0x0B => "Motor error",
                            0x0C => "Out of range",
                            0x0D => "Over current error",
                            _ => "Unknown error",
                        };
                        return Err(anyhow!(
                            "ELL14 error (code 0x{:02X}): {}",
                            status_code,
                            error_msg
                        ));
                    }
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

    // =========================================================================
    // Jog Control Commands
    // =========================================================================

    /// Jog forward by the configured jog step size
    ///
    /// The step size can be configured with `set_jog_step()`.
    /// Default jog step is device-dependent.
    pub async fn jog_forward(&self) -> Result<()> {
        self.send_command("fw").await
    }

    /// Jog backward by the configured jog step size
    ///
    /// The step size can be configured with `set_jog_step()`.
    pub async fn jog_backward(&self) -> Result<()> {
        self.send_command("bw").await
    }

    /// Get the current jog step size in degrees
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
    pub async fn set_jog_step(&self, degrees: f64) -> Result<()> {
        let pulses = (degrees * self.pulses_per_degree).abs() as u32;
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
    pub async fn stop(&self) -> Result<()> {
        // st is a fire-and-forget command
        self.send_command("st").await
    }

    // =========================================================================
    // Motor Frequency Search / Optimization Commands
    // =========================================================================

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
    async fn parse_motor_info(&self, cmd: &str, motor_num: u8) -> Result<MotorInfo> {
        let resp = self.transaction(cmd).await?;

        // Response format: "XI1{Loop}{Motor}{PWM_Fwd}{PWM_Bwd}{Period_Fwd}{Period_Bwd}"
        // All fields are hex encoded
        let marker = if motor_num == 1 { "I1" } else { "I2" };

        if let Some(idx) = resp.find(marker) {
            let data = resp[idx + 2..].trim();
            if data.len() >= 12 {
                // Parse fields (approximate, actual format may vary)
                let loop_state = u8::from_str_radix(&data[0..2], 16).unwrap_or(0) != 0;
                let motor_on = u8::from_str_radix(&data[2..4], 16).unwrap_or(0) != 0;
                let forward_period = u16::from_str_radix(&data[4..8], 16).unwrap_or(0);
                let backward_period = u16::from_str_radix(&data[8..12], 16).unwrap_or(0);

                // Frequency is calculated from period
                let frequency = if forward_period > 0 {
                    1_000_000 / forward_period as u32
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

        Err(anyhow!("Failed to parse motor {} info: {}", motor_num, resp))
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
    /// - Travel (8): "10168000" hex
    /// - Pulses/unit (8): "00023000" hex
    pub async fn get_device_info(&self) -> Result<DeviceInfo> {
        let resp = self.transaction("in").await?;

        if let Some(idx) = resp.find("IN") {
            let data = resp[idx + 2..].trim();
            if data.len() >= 25 {
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

                // Travel in pulses (8 hex chars starting at 17)
                let travel = if data.len() >= 25 {
                    u32::from_str_radix(&data[17..25], 16).unwrap_or(0)
                } else {
                    0
                };

                // Pulses per unit (8 hex chars starting at 25)
                let pulses_per_unit = if data.len() >= 33 {
                    u32::from_str_radix(&data[25..33], 16).unwrap_or(0)
                } else {
                    0
                };

                return Ok(DeviceInfo {
                    device_type,
                    serial,
                    year,
                    firmware,
                    hardware,
                    travel,
                    pulses_per_unit,
                });
            }
        }

        Err(anyhow!("Failed to parse device info: {}", resp))
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
        let pulses = (degrees * self.pulses_per_degree) as i32;
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

        // Should return "XUS00" on success
        if resp.contains("US") {
            Ok(())
        } else {
            Err(anyhow!("Failed to save user data: {}", resp))
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

    /// Get the device address
    pub fn get_address(&self) -> &str {
        &self.address
    }

    /// Get the current pulses per degree calibration
    pub fn get_pulses_per_degree(&self) -> f64 {
        self.pulses_per_degree
    }
}

impl Parameterized for Ell14Driver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for Ell14Driver {
    async fn move_abs(&self, position_deg: f64) -> Result<()> {
        // Convert degrees to pulses (handle negative positions properly)
        let pulses = (position_deg * self.pulses_per_degree) as i32;

        // Format as 8-digit hex (uppercase, zero-padded)
        // For negative values, the u32 cast gives two's complement representation
        let hex_pulses = format!("{:08X}", pulses as u32);

        // Command: ma (Move Absolute)
        // Format: "0ma00002000" for device 0, position 0x00002000
        let cmd = format!("ma{}", hex_pulses);

        // Move commands may return a response after motion starts or completes
        // We try to get a response but don't fail if none comes - use wait_settled for confirmation
        let _ = self.send_command(&cmd).await;

        Ok(())
    }

    async fn move_rel(&self, distance_deg: f64) -> Result<()> {
        // Command: mr (Move Relative)
        let pulses = (distance_deg * self.pulses_per_degree) as i32;
        let hex_pulses = format!("{:08X}", pulses as u32);

        let cmd = format!("mr{}", hex_pulses);
        let _ = self.send_command(&cmd).await;

        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        // Command: gp (Get Position)
        let resp = self.transaction("gp").await?;
        self.parse_position_response(&resp)
    }

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

    async fn stop(&self) -> Result<()> {
        // ELL14 supports immediate stop via 'st' command
        self.send_command("st").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let pulses_per_degree = 398.2222;

        // Test 45 degrees: 398.2222 * 45 = 17919.999 ≈ 17919
        let pulses = (45.0 * pulses_per_degree) as i32;
        assert_eq!(pulses, 17919);

        // Test 90 degrees: 398.2222 * 90 = 35839.998 ≈ 35839
        let pulses = (90.0 * pulses_per_degree) as i32;
        assert_eq!(pulses, 35839);
    }
}
