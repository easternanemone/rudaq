//! EOM Power Sweep Example
//!
//! This example demonstrates using the rust-daq driver ecosystem to characterize
//! an electro-optic modulator (EOM) power control system.
//!
//! # Hardware Setup
//!
//! - MaiTai Ti:Sapphire Laser (shutter control)
//! - Comedi DAQ with DAC0 connected to EOM amplifier
//! - Newport 1830-C power meter in beam path
//!
//! # Safety
//!
//! **LASER SAFETY WARNING**: This program controls a high-power laser.
//! - Ensure proper laser safety enclosure is in place
//! - Wear appropriate laser safety glasses
//! - Verify interlock system is functional
//!
//! # Usage
//!
//! ```bash
//! # Build with hardware features
//! cargo build --release -p daq-examples --features "spectra_physics,newport,comedi" \
//!     --example eom_power_sweep
//!
//! # Run (requires explicit confirmation)
//! ./target/release/examples/eom_power_sweep --confirm-laser-safety
//! ```

use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use common::capabilities::{Readable, ShutterControl};
use daq_driver_comedi::ComediDevice;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

#[cfg(feature = "spectra_physics")]
use daq_driver_spectra_physics::MaiTaiDriver;

#[cfg(feature = "newport")]
use daq_driver_newport::Newport1830CDriver;

/// EOM Power Sweep - Characterize electro-optic modulator transfer function
#[derive(Parser, Debug)]
#[command(name = "eom_power_sweep")]
#[command(about = "Sweep EOM voltage and measure optical power")]
struct Args {
    /// MaiTai laser serial port
    #[arg(long, default_value = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0")]
    maitai_port: String,

    /// Newport 1830-C power meter serial port
    #[arg(long, default_value = "/dev/ttyS0")]
    newport_port: String,

    /// Comedi device path
    #[arg(long, default_value = "/dev/comedi0")]
    comedi_device: String,

    /// Minimum voltage (V)
    #[arg(long, default_value = "-5.0")]
    voltage_min: f64,

    /// Maximum voltage (V)
    #[arg(long, default_value = "5.0")]
    voltage_max: f64,

    /// Voltage step size (V)
    #[arg(long, default_value = "0.1")]
    voltage_step: f64,

    /// Settling time after voltage change (ms)
    #[arg(long, default_value = "500")]
    settling_ms: u64,

    /// Output directory for HDF5 data
    #[arg(long, default_value = "data")]
    output_dir: PathBuf,

    /// Confirm laser safety requirements have been met
    #[arg(long)]
    confirm_laser_safety: bool,

    /// Dry run - don't actually open shutter or control EOM
    #[arg(long)]
    dry_run: bool,
}

/// Sweep results
struct SweepResults {
    voltages: Vec<f64>,
    powers: Vec<f64>,
    voltage_min: f64,
    voltage_max: f64,
    voltage_step: f64,
}

impl SweepResults {
    fn min_power(&self) -> Option<(f64, f64)> {
        self.voltages
            .iter()
            .zip(self.powers.iter())
            .filter(|(_, p)| !p.is_nan())
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(v, p)| (*v, *p))
    }

    fn max_power(&self) -> Option<f64> {
        self.powers
            .iter()
            .filter(|p| !p.is_nan())
            .cloned()
            .fold(None, |acc, p| match acc {
                None => Some(p),
                Some(max) if p > max => Some(p),
                _ => acc,
            })
    }

    fn extinction_ratio(&self) -> Option<f64> {
        let min = self.min_power().map(|(_, p)| p)?;
        let max = self.max_power()?;
        if min > 0.0 {
            Some(max / min)
        } else {
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let args = Args::parse();

    // Safety check
    if !args.confirm_laser_safety && !args.dry_run {
        eprintln!("╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║  LASER SAFETY WARNING                                        ║");
        eprintln!("║                                                              ║");
        eprintln!("║  This program controls a high-power Ti:Sapphire laser.       ║");
        eprintln!("║  Before running, ensure:                                     ║");
        eprintln!("║                                                              ║");
        eprintln!("║  1. Laser safety enclosure is in place                       ║");
        eprintln!("║  2. Interlock system is functional                           ║");
        eprintln!("║  3. Appropriate laser safety glasses are worn                ║");
        eprintln!("║  4. Area is clear of unauthorized personnel                  ║");
        eprintln!("║                                                              ║");
        eprintln!("║  To proceed, add: --confirm-laser-safety                     ║");
        eprintln!("║  For testing without hardware: --dry-run                     ║");
        eprintln!("╚══════════════════════════════════════════════════════════════╝");
        std::process::exit(1);
    }

    info!("EOM Power Sweep starting...");
    info!(
        "Voltage range: {:.1}V to {:.1}V (step: {:.2}V)",
        args.voltage_min, args.voltage_max, args.voltage_step
    );

    if args.dry_run {
        info!("DRY RUN MODE - No hardware will be controlled");
        return run_dry(&args);
    }

    // Run the actual sweep
    run_sweep(&args).await
}

/// Dry run for testing without hardware
fn run_dry(args: &Args) -> Result<()> {
    info!("[DRY RUN] Would connect to:");
    info!("  MaiTai: {}", args.maitai_port);
    info!("  Newport: {}", args.newport_port);
    info!("  Comedi: {}", args.comedi_device);

    let n_points = ((args.voltage_max - args.voltage_min) / args.voltage_step).ceil() as usize + 1;
    info!("[DRY RUN] Would acquire {} data points", n_points);
    info!(
        "[DRY RUN] Estimated time: {:.1} seconds",
        n_points as f64 * (args.settling_ms as f64 / 1000.0 + 0.2)
    );

    Ok(())
}

/// Run the actual power sweep
#[cfg(all(feature = "spectra_physics", feature = "newport", feature = "comedi"))]
async fn run_sweep(args: &Args) -> Result<()> {
    // Initialize devices
    info!("[1/6] Initializing MaiTai laser...");
    let maitai = MaiTaiDriver::new_async(&args.maitai_port, 115200)
        .await
        .context("Failed to connect to MaiTai laser")?;
    info!("  MaiTai connected");

    info!("[2/6] Initializing Newport 1830-C power meter...");
    let power_meter = Newport1830CDriver::new_async(&args.newport_port)
        .await
        .context("Failed to connect to Newport power meter")?;
    info!("  Power meter connected");

    info!("[3/6] Opening Comedi DAQ...");
    let device = ComediDevice::open(&args.comedi_device).context("Failed to open Comedi device")?;
    let ao = device
        .analog_output()
        .context("Failed to get analog output subsystem")?;
    let ao_range = ao
        .range_info(0, 0)
        .context("Failed to get DAC0 range info")?;
    info!("  DAC0 range: {:.1}V to {:.1}V", ao_range.min, ao_range.max);

    // Validate voltage range
    if args.voltage_min < ao_range.min || args.voltage_max > ao_range.max {
        anyhow::bail!(
            "Requested voltage range ({:.1}V to {:.1}V) exceeds DAC range ({:.1}V to {:.1}V)",
            args.voltage_min,
            args.voltage_max,
            ao_range.min,
            ao_range.max
        );
    }

    // Set EOM to safe state
    info!("[4/6] Setting EOM to 0V (safe state)...");
    ao.write_voltage(0, 0.0, ao_range)
        .context("Failed to set initial voltage")?;
    sleep(Duration::from_millis(args.settling_ms)).await;

    // Open shutter
    info!("[5/6] Opening MaiTai shutter...");
    maitai
        .open_shutter()
        .await
        .context("Failed to open shutter")?;
    sleep(Duration::from_secs(1)).await;
    info!("  Shutter opened");

    // Initial power reading
    let initial_power = power_meter.read().await.unwrap_or(0.0);
    info!("  Initial power: {:.3} mW", initial_power * 1000.0);

    // Perform sweep
    info!("[6/6] Performing voltage sweep...");
    let results = perform_sweep(&ao, ao_range, &power_meter, args).await?;

    // Reset to safe state
    info!("Resetting to safe state...");
    ao.write_voltage(0, 0.0, ao_range)
        .context("Failed to reset voltage")?;
    maitai
        .close_shutter()
        .await
        .context("Failed to close shutter")?;
    info!("  Shutter closed, EOM at 0V");

    // Save results
    let filepath = save_results(&results, args)?;
    info!("Data saved to: {}", filepath.display());

    // Print summary
    print_summary(&results);

    Ok(())
}

#[cfg(all(feature = "spectra_physics", feature = "newport", feature = "comedi"))]
async fn perform_sweep(
    ao: &daq_driver_comedi::subsystem::AnalogOutput,
    ao_range: daq_driver_comedi::Range,
    power_meter: &Newport1830CDriver,
    args: &Args,
) -> Result<SweepResults> {
    let mut voltages = Vec::new();
    let mut powers = Vec::new();

    let mut voltage = args.voltage_min;
    let settling = Duration::from_millis(args.settling_ms);

    println!();
    println!("┌──────────────┬──────────────────┬──────────────────┐");
    println!("│ EOM Voltage  │    Power (W)     │    Power (mW)    │");
    println!("├──────────────┼──────────────────┼──────────────────┤");

    while voltage <= args.voltage_max + 0.001 {
        // Set voltage
        ao.write_voltage(0, voltage, ao_range)
            .context("Failed to set voltage")?;
        sleep(settling).await;

        // Read power
        let power = power_meter.read().await.unwrap_or(f64::NAN);

        println!(
            "│ {:+8.3} V   │ {:>14.6e} │ {:>14.6} │",
            voltage,
            power,
            power * 1000.0
        );

        voltages.push(voltage);
        powers.push(power);
        voltage += args.voltage_step;
    }

    println!("└──────────────┴──────────────────┴──────────────────┘");
    println!();

    Ok(SweepResults {
        voltages,
        powers,
        voltage_min: args.voltage_min,
        voltage_max: args.voltage_max,
        voltage_step: args.voltage_step,
    })
}

fn save_results(results: &SweepResults, args: &Args) -> Result<PathBuf> {
    use hdf5::File as H5File;

    std::fs::create_dir_all(&args.output_dir)?;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("eom_sweep_{}.h5", timestamp);
    let filepath = args.output_dir.join(&filename);

    let file = H5File::create(&filepath)?;

    // Voltage dataset
    let voltage_ds = file
        .new_dataset::<f64>()
        .shape([results.voltages.len()])
        .create("voltage")?;
    voltage_ds.write(&results.voltages)?;
    voltage_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"V".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    voltage_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(
            &"EOM Control Voltage"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;

    // Power dataset
    let power_ds = file
        .new_dataset::<f64>()
        .shape([results.powers.len()])
        .create("power")?;
    power_ds.write(&results.powers)?;
    power_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"W".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    power_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(
            &"Optical Power"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;
    let _dims = power_ds
        .new_attr_builder()
        .with_data(&["voltage".parse::<hdf5::types::VarLenUnicode>().unwrap()])
        .create("_ARRAY_DIMENSIONS")?;

    // Metadata
    let timestamp_str = Local::now().to_rfc3339();
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("experiment")?
        .write_scalar(
            &"EOM Power Sweep"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("timestamp")?
        .write_scalar(&timestamp_str.parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<f64>()
        .create("voltage_min")?
        .write_scalar(&results.voltage_min)?;
    file.new_attr::<f64>()
        .create("voltage_max")?
        .write_scalar(&results.voltage_max)?;
    file.new_attr::<f64>()
        .create("voltage_step")?
        .write_scalar(&results.voltage_step)?;
    file.new_attr::<u64>()
        .create("n_points")?
        .write_scalar(&(results.voltages.len() as u64))?;

    // Summary statistics
    if let Some((v_min, p_min)) = results.min_power() {
        file.new_attr::<f64>()
            .create("min_power_W")?
            .write_scalar(&p_min)?;
        file.new_attr::<f64>()
            .create("voltage_at_min_power")?
            .write_scalar(&v_min)?;
    }
    if let Some(p_max) = results.max_power() {
        file.new_attr::<f64>()
            .create("max_power_W")?
            .write_scalar(&p_max)?;
    }
    if let Some(er) = results.extinction_ratio() {
        file.new_attr::<f64>()
            .create("extinction_ratio")?
            .write_scalar(&er)?;
    }

    Ok(filepath)
}

fn print_summary(results: &SweepResults) {
    println!("═══════════════════════════════════════════════════════════════");
    println!("  SWEEP COMPLETE");
    println!("═══════════════════════════════════════════════════════════════");

    if let Some((v_min, p_min)) = results.min_power() {
        println!(
            "  Min power:        {:.6e} W ({:.3} mW) at {:.2} V",
            p_min,
            p_min * 1000.0,
            v_min
        );
    }
    if let Some(p_max) = results.max_power() {
        println!(
            "  Max power:        {:.6e} W ({:.3} mW)",
            p_max,
            p_max * 1000.0
        );
    }
    if let Some(er) = results.extinction_ratio() {
        println!(
            "  Extinction ratio: {:.1}:1 ({:.1} dB)",
            er,
            10.0 * er.log10()
        );
    }
    println!();
}

// Fallback when features not enabled
#[cfg(not(all(feature = "spectra_physics", feature = "newport", feature = "comedi")))]
async fn run_sweep(_args: &Args) -> Result<()> {
    eprintln!("ERROR: This example requires features: spectra_physics, newport, comedi");
    eprintln!("Build with: cargo build --features \"spectra_physics,newport,comedi\"");
    std::process::exit(1);
}
