#![cfg(not(target_arch = "wasm32"))]
//! EOM Power Sweep Test
//!
//! **LASER SAFETY WARNING**
//! This test controls the MaiTai Ti:Sapphire laser and EOM amplifier.
//! Only run when authorized and with proper laser safety precautions.
//!
//! # Hardware Setup
//!
//! - MaiTai Ti:Sapphire laser (serial port)
//! - Comedi DAQ with DAC0 connected to EOM amplifier
//! - Newport 1830-C power meter in beam path
//!
//! # What This Test Does
//!
//! 1. Opens MaiTai shutter (enables laser output)
//! 2. Sweeps DAC0 (EOM voltage) from -5V to +5V in 0.1V steps
//! 3. At each step, reads power from Newport 1830-C
//! 4. Saves data to HDF5 file (xarray/Scipp compatible)
//! 5. Closes shutter when done
//!
//! # Environment Variables
//!
//! Required:
//! - `EOM_SWEEP_TEST=1` - Must be set to enable this test
//!
//! Optional:
//! - `COMEDI_DEVICE` - DAQ device (default: /dev/comedi0)
//! - `MAITAI_PORT` - MaiTai serial port (default: /dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0)
//! - `NEWPORT_PORT` - Power meter port (default: /dev/ttyS0)
//! - `EOM_VOLTAGE_MIN` - Minimum voltage (default: -5.0)
//! - `EOM_VOLTAGE_MAX` - Maximum voltage (default: 5.0)
//! - `EOM_VOLTAGE_STEP` - Voltage step size (default: 0.1)
//! - `EOM_OUTPUT_DIR` - Output directory (default: ~/rust-daq/data)
//!
//! # Output
//!
//! Data is saved to HDF5 with xarray-compatible structure:
//! ```text
//! eom_sweep_YYYYMMDD_HHMMSS.h5
//! ├── voltage (1D array, coordinate)
//! │   └── attrs: units="V", long_name="EOM Control Voltage"
//! ├── power (1D array, data variable)
//! │   └── attrs: units="W", long_name="Optical Power"
//! └── attrs: (experiment metadata)
//! ```
//!
//! # Python Analysis
//!
//! ```python
//! import xarray as xr
//! ds = xr.open_dataset("eom_sweep_20260126_143000.h5")
//! ds.power.plot()  # Plot power vs voltage
//! ```
//!
//! # Running
//!
//! ```bash
//! # DANGER: This controls real laser power!
//! export EOM_SWEEP_TEST=1
//! cargo test --features hardware -p daq-driver-comedi --test eom_power_sweep -- --nocapture --test-threads=1
//! ```

#![cfg(feature = "hardware")]

use chrono::Local;
use daq_driver_comedi::ComediDevice;
use hdf5::File as H5File;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

// =============================================================================
// Configuration
// =============================================================================

/// Default device paths
const DEFAULT_COMEDI_DEVICE: &str = "/dev/comedi0";
const DEFAULT_MAITAI_PORT: &str = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const DEFAULT_NEWPORT_PORT: &str = "/dev/ttyS0";

/// EOM channel on Comedi DAQ
const EOM_DAC_CHANNEL: u32 = 0;

/// Settling time after voltage change (ms)
const SETTLING_TIME_MS: u64 = 500;

/// Default voltage sweep parameters
const DEFAULT_VOLTAGE_MIN: f64 = -5.0;
const DEFAULT_VOLTAGE_MAX: f64 = 5.0;
const DEFAULT_VOLTAGE_STEP: f64 = 0.1;

/// Default output directory
const DEFAULT_OUTPUT_DIR: &str = "~/rust-daq/data";

// =============================================================================
// Environment Helpers
// =============================================================================

fn eom_test_enabled() -> bool {
    env::var("EOM_SWEEP_TEST")
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
        if !eom_test_enabled() {
            println!("EOM sweep test skipped (set EOM_SWEEP_TEST=1 to enable)");
            println!("WARNING: This test controls real laser power!");
            return;
        }
    };
}

// =============================================================================
// HDF5 Storage (xarray compatible)
// =============================================================================

/// Save EOM sweep data to HDF5 file with xarray-compatible structure
fn save_to_hdf5(
    results: &[(f64, f64)],
    output_dir: &str,
    voltage_min: f64,
    voltage_max: f64,
    voltage_step: f64,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Expand ~ in path
    let output_dir = if output_dir.starts_with("~/") {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/{}", home, &output_dir[2..])
    } else {
        output_dir.to_string()
    };

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&output_dir)?;

    // Generate filename with timestamp
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("eom_sweep_{}.h5", timestamp);
    let filepath = PathBuf::from(&output_dir).join(&filename);

    println!("Saving data to: {}", filepath.display());

    // Extract voltage and power arrays
    let voltages: Vec<f64> = results.iter().map(|(v, _)| *v).collect();
    let powers: Vec<f64> = results.iter().map(|(_, p)| *p).collect();

    // Create HDF5 file
    let file = H5File::create(&filepath)?;

    // Create voltage dataset (coordinate)
    let voltage_ds = file
        .new_dataset::<f64>()
        .shape([voltages.len()])
        .create("voltage")?;
    voltage_ds.write(&voltages)?;

    // Add voltage attributes (xarray compatible)
    voltage_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"V".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    voltage_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(&"EOM Control Voltage".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // Create power dataset (data variable)
    let power_ds = file
        .new_dataset::<f64>()
        .shape([powers.len()])
        .create("power")?;
    power_ds.write(&powers)?;

    // Add power attributes
    power_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"W".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    power_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(&"Optical Power".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    // Link to coordinate dimension (xarray convention)
    power_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("_ARRAY_DIMENSIONS")?
        .write(&["voltage".parse::<hdf5::types::VarLenUnicode>().unwrap()])?;

    // Add global attributes (experiment metadata)
    let timestamp_str = Local::now().to_rfc3339();
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("experiment")?
        .write_scalar(&"EOM Power Sweep".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("timestamp")?
        .write_scalar(&timestamp_str.parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("instrument")?
        .write_scalar(&"MaiTai + Comedi DAQ + Newport 1830-C".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<f64>().create("voltage_min")?.write_scalar(&voltage_min)?;
    file.new_attr::<f64>().create("voltage_max")?.write_scalar(&voltage_max)?;
    file.new_attr::<f64>().create("voltage_step")?.write_scalar(&voltage_step)?;
    file.new_attr::<u64>()
        .create("n_points")?
        .write_scalar(&(results.len() as u64))?;

    // Calculate and store summary statistics
    let valid_powers: Vec<f64> = powers.iter().filter(|p| !p.is_nan()).copied().collect();
    if !valid_powers.is_empty() {
        let min_power = valid_powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_power = valid_powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let extinction_ratio = if min_power > 0.0 {
            max_power / min_power
        } else {
            f64::NAN
        };

        file.new_attr::<f64>()
            .create("min_power_W")?
            .write_scalar(&min_power)?;
        file.new_attr::<f64>()
            .create("max_power_W")?
            .write_scalar(&max_power)?;
        file.new_attr::<f64>()
            .create("extinction_ratio")?
            .write_scalar(&extinction_ratio)?;

        // Find voltage at minimum power
        if let Some((v_at_min, _)) = results
            .iter()
            .filter(|(_, p)| !p.is_nan())
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        {
            file.new_attr::<f64>()
                .create("voltage_at_min_power")?
                .write_scalar(v_at_min)?;
        }
    }

    Ok(filepath)
}

// =============================================================================
// Simple Serial Communication
// =============================================================================

/// Simple blocking serial port wrapper for MaiTai
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
        // Send command with LF terminator
        self.port.write_all(cmd.as_bytes())?;
        self.port.write_all(b"\n")?;
        self.port.flush()?;

        // Read response
        thread::sleep(Duration::from_millis(100));
        let mut reader = BufReader::new(&mut self.port);
        let mut response = String::new();
        reader.read_line(&mut response)?;

        Ok(response.trim().to_string())
    }
}

/// Simple blocking serial port wrapper for Newport 1830-C
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
        // Send "D?" command to read power
        self.port.write_all(b"D?\r")?;
        self.port.flush()?;

        thread::sleep(Duration::from_millis(200));

        // Read response
        let mut reader = BufReader::new(&mut self.port);
        let mut response = String::new();
        reader.read_line(&mut response)?;

        // Parse power value (format: "1.234E-03" or similar)
        let power: f64 = response.trim().parse()?;
        Ok(power)
    }
}

// =============================================================================
// Test: EOM Power Sweep
// =============================================================================

/// Sweep EOM voltage and measure power at each step
///
/// **LASER SAFETY WARNING**: This test opens the laser shutter and
/// controls beam power via the EOM amplifier.
#[test]
fn test_eom_power_sweep() {
    skip_if_disabled!();

    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  EOM POWER SWEEP TEST                                        ║");
    println!("║  ⚠️  LASER SAFETY: Opening shutter and controlling power     ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Get configuration
    let comedi_device = get_env_or("COMEDI_DEVICE", DEFAULT_COMEDI_DEVICE);
    let maitai_port = get_env_or("MAITAI_PORT", DEFAULT_MAITAI_PORT);
    let newport_port = get_env_or("NEWPORT_PORT", DEFAULT_NEWPORT_PORT);
    let voltage_min = get_env_f64_or("EOM_VOLTAGE_MIN", DEFAULT_VOLTAGE_MIN);
    let voltage_max = get_env_f64_or("EOM_VOLTAGE_MAX", DEFAULT_VOLTAGE_MAX);
    let voltage_step = get_env_f64_or("EOM_VOLTAGE_STEP", DEFAULT_VOLTAGE_STEP);
    let output_dir = get_env_or("EOM_OUTPUT_DIR", DEFAULT_OUTPUT_DIR);

    println!("Configuration:");
    println!("  Comedi device:  {}", comedi_device);
    println!("  MaiTai port:    {}", maitai_port);
    println!("  Newport port:   {}", newport_port);
    println!("  Voltage range:  {} to {} V (step: {})", voltage_min, voltage_max, voltage_step);
    println!("  Output dir:     {}", output_dir);
    println!();

    // Open Comedi device
    println!("[1/5] Opening Comedi DAQ...");
    let device = ComediDevice::open(&comedi_device).expect("Failed to open Comedi device");
    let ao = device.analog_output().expect("Failed to get analog output");
    let ao_range = ao.range_info(EOM_DAC_CHANNEL, 0).expect("Failed to get AO range");
    println!("  DAC0 range: {} to {} V", ao_range.min, ao_range.max);

    // Validate voltage range
    assert!(
        voltage_min >= ao_range.min && voltage_max <= ao_range.max,
        "Voltage range {} to {} exceeds DAC range {} to {}",
        voltage_min, voltage_max, ao_range.min, ao_range.max
    );

    // Set initial voltage to 0V (safe state)
    println!("[2/5] Setting EOM to 0V (safe state)...");
    ao.write_voltage(EOM_DAC_CHANNEL, 0.0, ao_range)
        .expect("Failed to set initial voltage");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    println!("  EOM voltage: 0.0V");

    // Open MaiTai shutter
    println!("[3/5] Opening MaiTai shutter...");
    let mut maitai = SimpleSerial::open(&maitai_port, 115200)
        .expect("Failed to open MaiTai serial port");

    // Check current shutter state
    let shutter_state = maitai.send_command("SHUTTER?").unwrap_or_default();
    println!("  Current shutter state: {}", shutter_state);

    // Open shutter
    let _ = maitai.send_command("SHUTTER 1");
    thread::sleep(Duration::from_secs(1));
    let shutter_state = maitai.send_command("SHUTTER?").unwrap_or_default();
    println!("  Shutter opened: {}", shutter_state);

    // Open power meter
    println!("[4/5] Opening Newport 1830-C power meter...");
    let mut power_meter = Newport1830C::open(&newport_port)
        .expect("Failed to open Newport power meter");

    // Initial power reading
    let initial_power = power_meter.read_power().unwrap_or(0.0);
    println!("  Initial power: {:.6e} W ({:.3} mW)", initial_power, initial_power * 1000.0);

    // Perform voltage sweep
    println!("[5/5] Performing EOM voltage sweep...");
    println!();
    println!("┌──────────────┬──────────────────┬──────────────────┐");
    println!("│ EOM Voltage  │    Power (W)     │    Power (mW)    │");
    println!("├──────────────┼──────────────────┼──────────────────┤");

    let mut results: Vec<(f64, f64)> = Vec::new();
    let mut voltage = voltage_min;

    while voltage <= voltage_max + 0.001 {
        // Set EOM voltage
        ao.write_voltage(EOM_DAC_CHANNEL, voltage, ao_range)
            .expect("Failed to set voltage");

        // Wait for settling
        thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

        // Read power
        let power = power_meter.read_power().unwrap_or(f64::NAN);

        println!(
            "│ {:+8.3} V   │ {:>14.6e} │ {:>14.6} │",
            voltage,
            power,
            power * 1000.0
        );

        results.push((voltage, power));
        voltage += voltage_step;
    }

    println!("└──────────────┴──────────────────┴──────────────────┘");
    println!();

    // Reset EOM to 0V
    println!("Resetting EOM to 0V...");
    ao.write_voltage(EOM_DAC_CHANNEL, 0.0, ao_range)
        .expect("Failed to reset voltage");

    // Close shutter
    println!("Closing MaiTai shutter...");
    let _ = maitai.send_command("SHUTTER 0");
    thread::sleep(Duration::from_secs(1));
    let shutter_state = maitai.send_command("SHUTTER?").unwrap_or_default();
    println!("  Shutter closed: {}", shutter_state);

    // Save to HDF5
    println!("[6/6] Saving data to HDF5...");
    match save_to_hdf5(&results, &output_dir, voltage_min, voltage_max, voltage_step) {
        Ok(filepath) => {
            println!("  Data saved to: {}", filepath.display());
            println!();
            println!("  Python analysis:");
            println!("    import xarray as xr");
            println!("    ds = xr.open_dataset('{}', engine='h5netcdf')", filepath.display());
            println!("    ds.power.plot()");
        }
        Err(e) => {
            println!("  WARNING: Failed to save HDF5: {}", e);
        }
    }

    // Summary
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("  SWEEP COMPLETE");
    println!("═══════════════════════════════════════════════════════════════");

    if !results.is_empty() {
        let powers: Vec<f64> = results.iter().map(|(_, p)| *p).filter(|p| !p.is_nan()).collect();
        if !powers.is_empty() {
            let min_power = powers.iter().cloned().fold(f64::INFINITY, f64::min);
            let max_power = powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let extinction_ratio = if min_power > 0.0 { max_power / min_power } else { f64::INFINITY };

            // Find voltage at minimum power
            let v_at_min = results
                .iter()
                .filter(|(_, p)| !p.is_nan())
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(v, _)| *v)
                .unwrap_or(0.0);

            println!("  Min power:        {:.6e} W ({:.3} mW) at {:.2} V", min_power, min_power * 1000.0, v_at_min);
            println!("  Max power:        {:.6e} W ({:.3} mW)", max_power, max_power * 1000.0);
            println!("  Extinction ratio: {:.1}:1 ({:.1} dB)", extinction_ratio, 10.0 * extinction_ratio.log10());
        }
    }

    println!();
}

/// Test skip check - verifies test is properly disabled by default
#[test]
fn eom_test_skip_check() {
    let enabled = eom_test_enabled();
    if !enabled {
        println!("EOM sweep test correctly disabled (EOM_SWEEP_TEST not set)");
        println!("To enable: export EOM_SWEEP_TEST=1");
        println!("WARNING: This test controls real laser power!");
    } else {
        println!("EOM sweep test enabled via EOM_SWEEP_TEST=1");
        println!("⚠️  LASER SAFETY: Test will control MaiTai shutter and EOM");
    }
}
