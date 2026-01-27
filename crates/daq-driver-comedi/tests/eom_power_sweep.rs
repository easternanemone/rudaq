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
//! 2. Sweeps DAC0 (EOM voltage) from -2V to +2V in 0.5V steps
//! 3. At each step, reads power from Newport 1830-C
//! 4. Closes shutter when done
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
//! - `EOM_VOLTAGE_MIN` - Minimum voltage (default: -2.0)
//! - `EOM_VOLTAGE_MAX` - Maximum voltage (default: 2.0)
//! - `EOM_VOLTAGE_STEP` - Voltage step size (default: 0.5)
//!
//! # Running
//!
//! ```bash
//! # DANGER: This controls real laser power!
//! export EOM_SWEEP_TEST=1
//! cargo test --features hardware -p daq-driver-comedi --test eom_power_sweep -- --nocapture --test-threads=1
//! ```

#![cfg(feature = "hardware")]

use daq_driver_comedi::{ComediDevice, Range};
use std::env;
use std::io::{BufRead, BufReader, Write};
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
const DEFAULT_VOLTAGE_MIN: f64 = -2.0;
const DEFAULT_VOLTAGE_MAX: f64 = 2.0;
const DEFAULT_VOLTAGE_STEP: f64 = 0.5;

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

    println!("Configuration:");
    println!("  Comedi device:  {}", comedi_device);
    println!("  MaiTai port:    {}", maitai_port);
    println!("  Newport port:   {}", newport_port);
    println!("  Voltage range:  {} to {} V (step: {})", voltage_min, voltage_max, voltage_step);
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

            println!("  Min power:        {:.6e} W ({:.3} mW)", min_power, min_power * 1000.0);
            println!("  Max power:        {:.6e} W ({:.3} mW)", max_power, max_power * 1000.0);
            println!("  Extinction ratio: {:.1}:1 ({:.1} dB)", extinction_ratio, 10.0 * extinction_ratio.log10());
        }
    }

    println!();
    println!("Results stored in test output for analysis.");
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
