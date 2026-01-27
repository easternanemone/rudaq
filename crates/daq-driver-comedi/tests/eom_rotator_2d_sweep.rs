#![cfg(not(target_arch = "wasm32"))]
//! EOM + Rotator 2D Power Sweep Test
//!
//! **LASER SAFETY WARNING**
//! This test controls the MaiTai Ti:Sapphire laser, EOM amplifier, and rotator.
//! Only run when authorized and with proper laser safety precautions.
//!
//! # Hardware Setup
//!
//! - MaiTai Ti:Sapphire laser (serial port)
//! - Comedi DAQ with DAC0 connected to EOM amplifier
//! - Newport 1830-C power meter in beam path
//! - ELL14 rotator (address 2 = HWP, or address 3 = polarizer)
//!
//! # What This Test Does
//!
//! 1. Opens MaiTai shutter (enables laser output)
//! 2. For each rotator angle:
//!    a. Move rotator to angle
//!    b. Sweep EOM voltage from min to max
//!    c. Record power at each voltage
//! 3. Saves 2D data to HDF5 file (xarray/Scipp compatible)
//! 4. Closes shutter when done
//!
//! # Environment Variables
//!
//! Required:
//! - `EOM_2D_TEST=1` - Must be set to enable this test
//!
//! Optional:
//! - `COMEDI_DEVICE` - DAQ device (default: /dev/comedi0)
//! - `MAITAI_PORT` - MaiTai serial port
//! - `NEWPORT_PORT` - Power meter port (default: /dev/ttyS0)
//! - `ELLIPTEC_PORT` - Rotator serial port
//! - `ELLIPTEC_ADDR` - Rotator address (default: 2)
//! - `EOM_VOLTAGE_MIN` - Minimum voltage (default: -5.0)
//! - `EOM_VOLTAGE_MAX` - Maximum voltage (default: 5.0)
//! - `EOM_VOLTAGE_STEP` - Voltage step size (default: 0.5)
//! - `ROTATOR_ANGLE_MIN` - Minimum angle (default: 0.0)
//! - `ROTATOR_ANGLE_MAX` - Maximum angle (default: 180.0)
//! - `ROTATOR_ANGLE_STEP` - Angle step size (default: 10.0)
//! - `EOM_OUTPUT_DIR` - Output directory (default: ~/rust-daq/data)
//!
//! # Output
//!
//! Data is saved to HDF5 with xarray-compatible structure:
//! ```text
//! eom_rotator_2d_YYYYMMDD_HHMMSS.h5
//! ├── angle (1D array, coordinate)
//! │   └── attrs: units="deg", long_name="Rotator Angle"
//! ├── voltage (1D array, coordinate)
//! │   └── attrs: units="V", long_name="EOM Control Voltage"
//! ├── power (2D array [angle, voltage], data variable)
//! │   └── attrs: units="W", long_name="Optical Power", _ARRAY_DIMENSIONS=["angle", "voltage"]
//! └── attrs: (experiment metadata)
//! ```
//!
//! # Python Analysis
//!
//! ```python
//! import xarray as xr
//! ds = xr.open_dataset("eom_rotator_2d_20260126_210000.h5", engine='h5netcdf')
//! ds.power.plot()  # 2D heatmap
//! ds.power.sel(angle=45.0, method='nearest').plot()  # 1D slice at 45 degrees
//! ```
//!
//! # Running
//!
//! ```bash
//! export EOM_2D_TEST=1
//! cargo test --features hardware -p daq-driver-comedi --test eom_rotator_2d_sweep -- --nocapture --test-threads=1
//! ```

#![cfg(feature = "hardware")]

use chrono::Local;
use daq_driver_comedi::ComediDevice;
use hdf5::File as H5File;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

// =============================================================================
// Configuration
// =============================================================================

const DEFAULT_COMEDI_DEVICE: &str = "/dev/comedi0";
const DEFAULT_MAITAI_PORT: &str = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const DEFAULT_NEWPORT_PORT: &str = "/dev/ttyS0";
const DEFAULT_ELLIPTEC_PORT: &str = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";
const DEFAULT_ELLIPTEC_ADDR: &str = "2";

const EOM_DAC_CHANNEL: u32 = 0;
const EOM_SETTLING_MS: u64 = 200;
const ROTATOR_SETTLING_MS: u64 = 500;

// Default sweep parameters (coarser for 2D to keep test time reasonable)
const DEFAULT_VOLTAGE_MIN: f64 = -5.0;
const DEFAULT_VOLTAGE_MAX: f64 = 5.0;
const DEFAULT_VOLTAGE_STEP: f64 = 0.5;

const DEFAULT_ANGLE_MIN: f64 = 0.0;
const DEFAULT_ANGLE_MAX: f64 = 180.0;
const DEFAULT_ANGLE_STEP: f64 = 10.0;

const DEFAULT_OUTPUT_DIR: &str = "~/rust-daq/data";

// =============================================================================
// Environment Helpers
// =============================================================================

fn test_enabled() -> bool {
    env::var("EOM_2D_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn get_env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn get_env_f64_or(key: &str, default: f64) -> f64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

macro_rules! skip_if_disabled {
    () => {
        if !test_enabled() {
            println!("EOM 2D sweep test skipped (set EOM_2D_TEST=1 to enable)");
            println!("WARNING: This test controls real laser power and rotator!");
            return;
        }
    };
}

// =============================================================================
// HDF5 Storage (xarray compatible, 2D)
// =============================================================================

fn save_to_hdf5_2d(
    angles: &[f64],
    voltages: &[f64],
    power_2d: &[Vec<f64>], // power_2d[angle_idx][voltage_idx]
    output_dir: &str,
    config: &SweepConfig,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let output_dir = if output_dir.starts_with("~/") {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/{}", home, &output_dir[2..])
    } else {
        output_dir.to_string()
    };

    std::fs::create_dir_all(&output_dir)?;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("eom_rotator_2d_{}.h5", timestamp);
    let filepath = PathBuf::from(&output_dir).join(&filename);

    println!("Saving 2D data to: {}", filepath.display());

    let file = H5File::create(&filepath)?;

    // Angle coordinate (1D)
    let angle_ds = file
        .new_dataset::<f64>()
        .shape([angles.len()])
        .create("angle")?;
    angle_ds.write(angles)?;
    angle_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"deg".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    angle_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(&"Rotator Angle".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // Voltage coordinate (1D)
    let voltage_ds = file
        .new_dataset::<f64>()
        .shape([voltages.len()])
        .create("voltage")?;
    voltage_ds.write(voltages)?;
    voltage_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"V".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    voltage_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(&"EOM Control Voltage".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // Power data (2D: [angle, voltage])
    let n_angles = angles.len();
    let n_voltages = voltages.len();

    // Flatten 2D array for HDF5 (row-major: angle is outer dimension)
    let mut power_flat: Vec<f64> = Vec::with_capacity(n_angles * n_voltages);
    for angle_row in power_2d {
        power_flat.extend(angle_row);
    }

    // Create dataset and write using raw slice with explicit shape
    let power_ds = file
        .new_dataset::<f64>()
        .shape([n_angles, n_voltages])
        .create("power")?;
    
    // Write using ndarray for proper 2D shape handling
    use ndarray::Array2;
    let power_array = Array2::from_shape_vec((n_angles, n_voltages), power_flat.clone())
        .map_err(|e| format!("Failed to create 2D array: {}", e))?;
    power_ds.write(power_array.view())?;

    power_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"W".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    power_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(&"Optical Power".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // _ARRAY_DIMENSIONS for xarray (order matches shape: [angle, voltage])
    let _dims = power_ds
        .new_attr_builder()
        .with_data(&[
            "angle".parse::<hdf5::types::VarLenUnicode>().unwrap(),
            "voltage".parse::<hdf5::types::VarLenUnicode>().unwrap(),
        ])
        .create("_ARRAY_DIMENSIONS")?;

    // Global attributes
    let timestamp_str = Local::now().to_rfc3339();
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("experiment")?
        .write_scalar(&"EOM + Rotator 2D Power Sweep".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("timestamp")?
        .write_scalar(&timestamp_str.parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("instrument")?
        .write_scalar(&"MaiTai + Comedi DAQ + Newport 1830-C + ELL14 Rotator".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // Sweep parameters
    file.new_attr::<f64>().create("voltage_min")?.write_scalar(&config.voltage_min)?;
    file.new_attr::<f64>().create("voltage_max")?.write_scalar(&config.voltage_max)?;
    file.new_attr::<f64>().create("voltage_step")?.write_scalar(&config.voltage_step)?;
    file.new_attr::<f64>().create("angle_min")?.write_scalar(&config.angle_min)?;
    file.new_attr::<f64>().create("angle_max")?.write_scalar(&config.angle_max)?;
    file.new_attr::<f64>().create("angle_step")?.write_scalar(&config.angle_step)?;
    file.new_attr::<u64>().create("n_angles")?.write_scalar(&(n_angles as u64))?;
    file.new_attr::<u64>().create("n_voltages")?.write_scalar(&(n_voltages as u64))?;
    file.new_attr::<u64>().create("n_total_points")?.write_scalar(&((n_angles * n_voltages) as u64))?;

    // Summary statistics
    let valid_powers: Vec<f64> = power_flat.iter().filter(|p| !p.is_nan()).copied().collect();
    if !valid_powers.is_empty() {
        let min_power = valid_powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_power = valid_powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        file.new_attr::<f64>().create("min_power_W")?.write_scalar(&min_power)?;
        file.new_attr::<f64>().create("max_power_W")?.write_scalar(&max_power)?;
    }

    Ok(filepath)
}

// =============================================================================
// Simple Serial Communication
// =============================================================================

struct SimpleSerial {
    port: Box<dyn serialport::SerialPort>,
}

impl SimpleSerial {
    fn open(path: &str, baud: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_secs(2))
            .open()?;
        Ok(Self { port })
    }

    fn send_command(&mut self, cmd: &str) -> Result<String, Box<dyn std::error::Error>> {
        self.port.write_all(cmd.as_bytes())?;
        self.port.write_all(b"\n")?;
        self.port.flush()?;

        thread::sleep(Duration::from_millis(100));
        let mut reader = BufReader::new(&mut self.port);
        let mut response = String::new();
        reader.read_line(&mut response)?;

        Ok(response.trim().to_string())
    }
}

struct Newport1830C {
    port: Box<dyn serialport::SerialPort>,
}

impl Newport1830C {
    fn open(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let port = serialport::new(path, 9600)
            .timeout(Duration::from_secs(2))
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .open()?;
        Ok(Self { port })
    }

    fn read_power(&mut self) -> Result<f64, Box<dyn std::error::Error>> {
        self.port.write_all(b"D?\r")?;
        self.port.flush()?;
        thread::sleep(Duration::from_millis(200));

        let mut reader = BufReader::new(&mut self.port);
        let mut response = String::new();
        reader.read_line(&mut response)?;

        let power: f64 = response.trim().parse()?;
        Ok(power)
    }
}

// =============================================================================
// ELL14 Rotator (Simple blocking driver)
// =============================================================================

struct Ell14Simple {
    port: Box<dyn serialport::SerialPort>,
    address: String,
    pulses_per_degree: f64,
}

impl Ell14Simple {
    fn open(path: &str, address: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut port = serialport::new(path, 9600)
            .timeout(Duration::from_secs(2))
            .open()?;

        // Query device info to get pulses per degree
        let cmd = format!("{}in", address);
        port.write_all(cmd.as_bytes())?;
        port.flush()?;

        thread::sleep(Duration::from_millis(200));

        let mut response = [0u8; 64];
        let n = port.read(&mut response)?;
        let response_str = String::from_utf8_lossy(&response[..n]);

        // Parse pulses per degree from IN response
        // Format: {addr}IN{type}{serial}{year}{firmware}{travel}{pulses_per_unit}
        // We need the last 8 hex chars for pulses/unit
        let pulses_per_degree = if response_str.len() >= 32 {
            // Extract last 8 hex chars (pulses per degree * 100)
            let hex_start = response_str.len().saturating_sub(10);
            let hex_str = &response_str[hex_start..hex_start + 8];
            if let Ok(pulses_x100) = u32::from_str_radix(hex_str.trim(), 16) {
                pulses_x100 as f64 / 100.0
            } else {
                // Default calibration for ELL14
                143.0
            }
        } else {
            143.0
        };

        println!("  ELL14 (addr {}) pulses/degree: {:.2}", address, pulses_per_degree);

        Ok(Self {
            port,
            address: address.to_string(),
            pulses_per_degree,
        })
    }

    fn move_abs(&mut self, degrees: f64) -> Result<(), Box<dyn std::error::Error>> {
        let pulses = (degrees * self.pulses_per_degree).round() as u32;
        let cmd = format!("{}ma{:08X}", self.address, pulses);

        // Clear any pending data aggressively
        let mut discard = [0u8; 256];
        for _ in 0..5 {
            match self.port.read(&mut discard) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
        thread::sleep(Duration::from_millis(50));

        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()?;

        // Wait for move to complete (longer for larger moves)
        let move_time = ((degrees.abs() / 360.0) * 2000.0).max(ROTATOR_SETTLING_MS as f64);
        thread::sleep(Duration::from_millis(move_time as u64));

        // Read and discard response (PO)
        let mut response = [0u8; 64];
        for _ in 0..3 {
            match self.port.read(&mut response) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }

        Ok(())
    }

    fn get_position(&mut self) -> Result<f64, Box<dyn std::error::Error>> {
        let cmd = format!("{}gp", self.address);

        // Clear buffer aggressively
        let mut discard = [0u8; 256];
        for _ in 0..5 {
            match self.port.read(&mut discard) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
        thread::sleep(Duration::from_millis(50));

        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()?;

        thread::sleep(Duration::from_millis(150));

        // Read response with multiple attempts
        let mut response = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        for _ in 0..5 {
            match self.port.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    response.extend_from_slice(&buf[..n]);
                    if response.len() >= 12 {
                        break;
                    }
                }
                Err(_) => break,
            }
            thread::sleep(Duration::from_millis(20));
        }

        let response_str = String::from_utf8_lossy(&response);

        // Parse position from PO response: {addr}PO{8 hex}
        let expected_prefix = format!("{}PO", self.address);
        if let Some(idx) = response_str.find(&expected_prefix) {
            let hex_start = idx + 3;
            if response_str.len() >= hex_start + 8 {
                let hex_str = &response_str[hex_start..hex_start + 8];
                if let Ok(pulses) = u32::from_str_radix(hex_str.trim(), 16) {
                    return Ok(pulses as f64 / self.pulses_per_degree);
                }
            }
        }

        // If position query failed, return NaN rather than error
        // (better to continue sweep than abort)
        Ok(f64::NAN)
    }

    fn home(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let cmd = format!("{}ho", self.address);
        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()?;
        thread::sleep(Duration::from_millis(2000)); // Homing takes longer
        Ok(())
    }
}

// =============================================================================
// Configuration Struct
// =============================================================================

struct SweepConfig {
    voltage_min: f64,
    voltage_max: f64,
    voltage_step: f64,
    angle_min: f64,
    angle_max: f64,
    angle_step: f64,
}

// =============================================================================
// Test: EOM + Rotator 2D Sweep
// =============================================================================

#[test]
fn test_eom_rotator_2d_sweep() {
    skip_if_disabled!();

    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  EOM + ROTATOR 2D POWER SWEEP TEST                           ║");
    println!("║  ⚠️  LASER SAFETY: Opening shutter and controlling power     ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Configuration
    let comedi_device = get_env_or("COMEDI_DEVICE", DEFAULT_COMEDI_DEVICE);
    let maitai_port = get_env_or("MAITAI_PORT", DEFAULT_MAITAI_PORT);
    let newport_port = get_env_or("NEWPORT_PORT", DEFAULT_NEWPORT_PORT);
    let elliptec_port = get_env_or("ELLIPTEC_PORT", DEFAULT_ELLIPTEC_PORT);
    let elliptec_addr = get_env_or("ELLIPTEC_ADDR", DEFAULT_ELLIPTEC_ADDR);

    let config = SweepConfig {
        voltage_min: get_env_f64_or("EOM_VOLTAGE_MIN", DEFAULT_VOLTAGE_MIN),
        voltage_max: get_env_f64_or("EOM_VOLTAGE_MAX", DEFAULT_VOLTAGE_MAX),
        voltage_step: get_env_f64_or("EOM_VOLTAGE_STEP", DEFAULT_VOLTAGE_STEP),
        angle_min: get_env_f64_or("ROTATOR_ANGLE_MIN", DEFAULT_ANGLE_MIN),
        angle_max: get_env_f64_or("ROTATOR_ANGLE_MAX", DEFAULT_ANGLE_MAX),
        angle_step: get_env_f64_or("ROTATOR_ANGLE_STEP", DEFAULT_ANGLE_STEP),
    };

    let output_dir = get_env_or("EOM_OUTPUT_DIR", DEFAULT_OUTPUT_DIR);

    // Calculate grid dimensions
    let n_voltages = ((config.voltage_max - config.voltage_min) / config.voltage_step).ceil() as usize + 1;
    let n_angles = ((config.angle_max - config.angle_min) / config.angle_step).ceil() as usize + 1;

    println!("Configuration:");
    println!("  Comedi device:  {}", comedi_device);
    println!("  MaiTai port:    {}", maitai_port);
    println!("  Newport port:   {}", newport_port);
    println!("  Elliptec port:  {}", elliptec_port);
    println!("  Rotator addr:   {}", elliptec_addr);
    println!();
    println!("  Voltage range:  {:.1}V to {:.1}V (step: {:.2}V) -> {} points",
        config.voltage_min, config.voltage_max, config.voltage_step, n_voltages);
    println!("  Angle range:    {:.1}° to {:.1}° (step: {:.1}°) -> {} points",
        config.angle_min, config.angle_max, config.angle_step, n_angles);
    println!("  Total points:   {}", n_angles * n_voltages);
    println!("  Output dir:     {}", output_dir);
    println!();

    // Initialize hardware
    println!("[1/7] Opening Comedi DAQ...");
    let device = ComediDevice::open(&comedi_device).expect("Failed to open Comedi device");
    let ao = device.analog_output().expect("Failed to get analog output");
    let ao_range = ao.range_info(EOM_DAC_CHANNEL, 0).expect("Failed to get AO range");
    println!("  DAC0 range: {:.1}V to {:.1}V", ao_range.min, ao_range.max);

    println!("[2/7] Opening ELL14 rotator...");
    let mut rotator = Ell14Simple::open(&elliptec_port, &elliptec_addr)
        .expect("Failed to open Elliptec rotator");
    let initial_pos = rotator.get_position().unwrap_or(0.0);
    println!("  Current position: {:.2}°", initial_pos);

    println!("[3/7] Setting EOM to 0V (safe state)...");
    ao.write_voltage(EOM_DAC_CHANNEL, 0.0, ao_range)
        .expect("Failed to set initial voltage");
    thread::sleep(Duration::from_millis(EOM_SETTLING_MS));

    println!("[4/7] Opening MaiTai shutter...");
    let mut maitai = SimpleSerial::open(&maitai_port, 115200)
        .expect("Failed to open MaiTai");
    let _ = maitai.send_command("SHUTTER 1");
    thread::sleep(Duration::from_secs(1));
    println!("  Shutter opened");

    println!("[5/7] Opening Newport power meter...");
    let mut power_meter = Newport1830C::open(&newport_port)
        .expect("Failed to open power meter");
    let initial_power = power_meter.read_power().unwrap_or(0.0);
    println!("  Initial power: {:.3} mW", initial_power * 1000.0);

    // Build coordinate arrays
    let mut angles: Vec<f64> = Vec::new();
    let mut angle = config.angle_min;
    while angle <= config.angle_max + 0.001 {
        angles.push(angle);
        angle += config.angle_step;
    }

    let mut voltages: Vec<f64> = Vec::new();
    let mut voltage = config.voltage_min;
    while voltage <= config.voltage_max + 0.001 {
        voltages.push(voltage);
        voltage += config.voltage_step;
    }

    // 2D data array: power_2d[angle_idx][voltage_idx]
    let mut power_2d: Vec<Vec<f64>> = Vec::with_capacity(angles.len());

    println!("[6/7] Performing 2D sweep...");
    println!();

    for (angle_idx, &target_angle) in angles.iter().enumerate() {
        // Move rotator
        rotator.move_abs(target_angle).expect("Failed to move rotator");
        let actual_angle = rotator.get_position().unwrap_or(target_angle);

        print!("Angle {:3.0}° ({:2}/{:2}): [", actual_angle, angle_idx + 1, angles.len());

        let mut voltage_powers: Vec<f64> = Vec::with_capacity(voltages.len());

        for &target_voltage in &voltages {
            // Set EOM voltage
            ao.write_voltage(EOM_DAC_CHANNEL, target_voltage, ao_range)
                .expect("Failed to set voltage");
            thread::sleep(Duration::from_millis(EOM_SETTLING_MS));

            // Read power
            let power = power_meter.read_power().unwrap_or(f64::NAN);
            voltage_powers.push(power);

            // Progress indicator
            if power.is_nan() {
                print!("X");
            } else if power < 0.003 {
                print!(".");
            } else if power < 0.008 {
                print!("o");
            } else {
                print!("O");
            }
        }

        println!("] min={:.2}mW max={:.2}mW",
            voltage_powers.iter().filter(|p| !p.is_nan()).cloned().fold(f64::INFINITY, f64::min) * 1000.0,
            voltage_powers.iter().filter(|p| !p.is_nan()).cloned().fold(0.0_f64, f64::max) * 1000.0);

        power_2d.push(voltage_powers);
    }

    println!();

    // Reset to safe state
    println!("Resetting to safe state...");
    ao.write_voltage(EOM_DAC_CHANNEL, 0.0, ao_range).expect("Failed to reset voltage");
    let _ = maitai.send_command("SHUTTER 0");
    rotator.move_abs(0.0).ok();
    println!("  Shutter closed, EOM at 0V, rotator at 0°");

    // Save to HDF5
    println!("[7/7] Saving 2D data to HDF5...");
    match save_to_hdf5_2d(&angles, &voltages, &power_2d, &output_dir, &config) {
        Ok(filepath) => {
            println!("  Data saved to: {}", filepath.display());
            println!();
            println!("  Python analysis:");
            println!("    import xarray as xr");
            println!("    ds = xr.open_dataset('{}', engine='h5netcdf')", filepath.display());
            println!("    ds.power.plot()  # 2D heatmap");
            println!("    ds.power.sel(angle=45.0, method='nearest').plot()  # 1D slice");
        }
        Err(e) => {
            println!("  WARNING: Failed to save HDF5: {}", e);
        }
    }

    // Summary statistics
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("  2D SWEEP COMPLETE");
    println!("═══════════════════════════════════════════════════════════════");

    let all_powers: Vec<f64> = power_2d.iter().flatten().filter(|p| !p.is_nan()).copied().collect();
    if !all_powers.is_empty() {
        let min_power = all_powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_power = all_powers.iter().cloned().fold(0.0_f64, f64::max);

        println!("  Grid size:       {} angles × {} voltages = {} points",
            angles.len(), voltages.len(), angles.len() * voltages.len());
        println!("  Min power:       {:.3} mW", min_power * 1000.0);
        println!("  Max power:       {:.3} mW", max_power * 1000.0);
        println!("  Dynamic range:   {:.1}:1 ({:.1} dB)",
            max_power / min_power, 10.0 * (max_power / min_power).log10());
    }

    println!();
}

#[test]
fn eom_2d_test_skip_check() {
    if !test_enabled() {
        println!("EOM 2D sweep test correctly disabled (EOM_2D_TEST not set)");
        println!("To enable: export EOM_2D_TEST=1");
        println!("WARNING: This test controls real laser power and rotator!");
    } else {
        println!("EOM 2D sweep test enabled via EOM_2D_TEST=1");
        println!("⚠️  LASER SAFETY: Test will control MaiTai shutter, EOM, and rotator");
    }
}
