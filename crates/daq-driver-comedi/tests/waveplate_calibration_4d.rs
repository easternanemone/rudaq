//! 4D Waveplate Calibration Sweep
//!
//! Performs a comprehensive polarization calibration:
//! - Outer loop: MaiTai wavelength (with stabilization delay)
//! - Loop 2: Linear polarizer angle (addr 3)
//! - Loop 3: Half-wave plate angle (addr 2)
//! - Inner loop: Quarter-wave plate angle (addr 8)
//!
//! Records optical power at each (wavelength, LP, HWP, QWP) point.
//!
//! SAFETY: This test opens the laser shutter during the sweep.
//! Ensure proper laser safety protocols are followed.
//!
//! Run with: WAVEPLATE_CAL=1 cargo test --features hardware -p daq-driver-comedi \
//!           --test waveplate_calibration_4d -- --nocapture
//!
//! Environment variables for customization:
//!   WAVELENGTH_MIN=780        # nm (default 780)
//!   WAVELENGTH_MAX=900        # nm (default 900)
//!   WAVELENGTH_STEP=10        # nm (default 10)
//!   WAVELENGTH_SETTLE_SECS=60 # seconds (default 60)
//!   LP_ANGLE_MIN=0.0          # degrees (default 0)
//!   LP_ANGLE_MAX=180.0        # degrees (default 180)
//!   LP_ANGLE_STEP=10.0        # degrees (default 10)
//!   HWP_ANGLE_MIN=0.0         # degrees (default 0)
//!   HWP_ANGLE_MAX=90.0        # degrees (default 90)
//!   HWP_ANGLE_STEP=10.0       # degrees (default 10)
//!   QWP_ANGLE_MIN=0.0         # degrees (default 0)
//!   QWP_ANGLE_MAX=90.0        # degrees (default 90)
//!   QWP_ANGLE_STEP=10.0       # degrees (default 10)

use chrono::Local;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::thread;
use std::time::{Duration, Instant};

// Serial port paths
const MAITAI_PORT: &str = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const NEWPORT_PORT: &str = "/dev/ttyS0";
const ELLIPTEC_PORT: &str =
    "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";

// Rotator addresses
const LP_ADDRESS: &str = "3";   // Linear Polarizer
const HWP_ADDRESS: &str = "2";  // Half-Wave Plate
const QWP_ADDRESS: &str = "8";  // Quarter-Wave Plate

// Timing
const ROTATOR_SETTLING_MS: u64 = 300;
const POWER_SETTLE_MS: u64 = 100;

fn get_env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn get_env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn build_coordinate_array(min: f64, max: f64, step: f64) -> Vec<f64> {
    let mut v = Vec::new();
    let mut val = min;
    while val <= max + 1e-9 {
        v.push(val);
        val += step;
    }
    v
}

#[test]
fn test_waveplate_calibration_4d() {
    if env::var("WAVEPLATE_CAL").is_err() {
        eprintln!("Skipping waveplate calibration (set WAVEPLATE_CAL=1 to enable)");
        return;
    }

    // Parse configuration
    let wavelength_min = get_env_f64("WAVELENGTH_MIN", 780.0);
    let wavelength_max = get_env_f64("WAVELENGTH_MAX", 900.0);
    let wavelength_step = get_env_f64("WAVELENGTH_STEP", 10.0);
    let wavelength_settle_secs = get_env_u64("WAVELENGTH_SETTLE_SECS", 60);

    let lp_min = get_env_f64("LP_ANGLE_MIN", 0.0);
    let lp_max = get_env_f64("LP_ANGLE_MAX", 180.0);
    let lp_step = get_env_f64("LP_ANGLE_STEP", 10.0);

    let hwp_min = get_env_f64("HWP_ANGLE_MIN", 0.0);
    let hwp_max = get_env_f64("HWP_ANGLE_MAX", 90.0);
    let hwp_step = get_env_f64("HWP_ANGLE_STEP", 10.0);

    let qwp_min = get_env_f64("QWP_ANGLE_MIN", 0.0);
    let qwp_max = get_env_f64("QWP_ANGLE_MAX", 90.0);
    let qwp_step = get_env_f64("QWP_ANGLE_STEP", 10.0);

    // Build coordinate arrays
    let wavelengths = build_coordinate_array(wavelength_min, wavelength_max, wavelength_step);
    let lp_angles = build_coordinate_array(lp_min, lp_max, lp_step);
    let hwp_angles = build_coordinate_array(hwp_min, hwp_max, hwp_step);
    let qwp_angles = build_coordinate_array(qwp_min, qwp_max, qwp_step);

    let n_wl = wavelengths.len();
    let n_lp = lp_angles.len();
    let n_hwp = hwp_angles.len();
    let n_qwp = qwp_angles.len();
    let total_points = n_wl * n_lp * n_hwp * n_qwp;

    let output_dir = env::var("HOME")
        .map(|h| format!("{}/rust-daq/data", h))
        .unwrap_or_else(|_| "/tmp".to_string());

    // Time estimate
    let points_per_wl = n_lp * n_hwp * n_qwp;
    let time_per_point_ms = POWER_SETTLE_MS + 50; // measurement + overhead
    let time_per_wl_secs = (points_per_wl as u64 * time_per_point_ms) / 1000 + wavelength_settle_secs;
    let total_time_secs = n_wl as u64 * time_per_wl_secs;

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║  4D WAVEPLATE CALIBRATION SWEEP                              ║");
    println!("║  ⚠️  LASER SAFETY: Opening shutter during measurement        ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    println!("Configuration:");
    println!("  MaiTai port:    {}", MAITAI_PORT);
    println!("  Newport port:   {}", NEWPORT_PORT);
    println!("  Elliptec port:  {}", ELLIPTEC_PORT);
    println!();
    println!("  Wavelength:     {:.0}nm to {:.0}nm (step {:.0}nm) -> {} points", 
        wavelength_min, wavelength_max, wavelength_step, n_wl);
    println!("  Stabilization:  {} seconds after wavelength change", wavelength_settle_secs);
    println!("  LP (addr {}):    {:.0}° to {:.0}° (step {:.0}°) -> {} points",
        LP_ADDRESS, lp_min, lp_max, lp_step, n_lp);
    println!("  HWP (addr {}):   {:.0}° to {:.0}° (step {:.0}°) -> {} points",
        HWP_ADDRESS, hwp_min, hwp_max, hwp_step, n_hwp);
    println!("  QWP (addr {}):   {:.0}° to {:.0}° (step {:.0}°) -> {} points",
        QWP_ADDRESS, qwp_min, qwp_max, qwp_step, n_qwp);
    println!();
    println!("  Total points:   {} ({} × {} × {} × {})", 
        total_points, n_wl, n_lp, n_hwp, n_qwp);
    println!("  Estimated time: {} hr {} min", total_time_secs / 3600, (total_time_secs % 3600) / 60);
    println!("  Output dir:     {}", output_dir);
    println!();

    // Initialize hardware
    let start_time = Instant::now();

    println!("[1/6] Opening ELL14 rotators...");
    let mut rotator_lp = Ell14Simple::open(ELLIPTEC_PORT, LP_ADDRESS)
        .expect("Failed to open LP rotator");
    println!("  LP (addr {}): pulses/deg = {:.2}", LP_ADDRESS, rotator_lp.pulses_per_degree);
    
    let mut rotator_hwp = Ell14Simple::open(ELLIPTEC_PORT, HWP_ADDRESS)
        .expect("Failed to open HWP rotator");
    println!("  HWP (addr {}): pulses/deg = {:.2}", HWP_ADDRESS, rotator_hwp.pulses_per_degree);
    
    let mut rotator_qwp = Ell14Simple::open(ELLIPTEC_PORT, QWP_ADDRESS)
        .expect("Failed to open QWP rotator");
    println!("  QWP (addr {}): pulses/deg = {:.2}", QWP_ADDRESS, rotator_qwp.pulses_per_degree);

    println!("[2/6] Opening MaiTai laser...");
    let mut maitai = MaiTaiSimple::open(MAITAI_PORT).expect("Failed to open MaiTai");
    let initial_wavelength = maitai.get_wavelength().unwrap_or(0.0);
    println!("  Initial wavelength: {:.0} nm", initial_wavelength);

    println!("[3/6] Opening MaiTai shutter...");
    maitai.open_shutter().expect("Failed to open shutter");
    println!("  Shutter opened");

    println!("[4/6] Opening Newport power meter...");
    let mut power_meter = Newport1830Simple::open(NEWPORT_PORT).expect("Failed to open Newport");
    let initial_power = power_meter.read_power().unwrap_or(0.0);
    println!("  Initial power: {:.3} mW", initial_power * 1000.0);

    // 4D data storage: [wavelength][lp][hwp][qwp]
    let mut power_4d: Vec<Vec<Vec<Vec<f64>>>> = 
        vec![vec![vec![vec![0.0; n_qwp]; n_hwp]; n_lp]; n_wl];

    println!("[5/6] Performing 4D calibration sweep...\n");

    let mut total_measured = 0usize;

    for (wl_idx, &wavelength) in wavelengths.iter().enumerate() {
        let wl_start = Instant::now();
        
        println!("═══════════════════════════════════════════════════════════════");
        println!("  Wavelength {:.0}nm ({}/{})", wavelength, wl_idx + 1, n_wl);
        println!("═══════════════════════════════════════════════════════════════");

        // Set wavelengths
        print!("  Setting MaiTai to {:.0}nm... ", wavelength);
        std::io::stdout().flush().unwrap();
        maitai.set_wavelength(wavelength).expect("Failed to set MaiTai wavelength");
        println!("done");

        print!("  Setting Newport calibration to {:.0}nm... ", wavelength);
        std::io::stdout().flush().unwrap();
        power_meter.set_wavelength(wavelength).expect("Failed to set Newport wavelength");
        println!("done");

        // Stabilization
        print!("  Stabilizing ({} sec)", wavelength_settle_secs);
        std::io::stdout().flush().unwrap();
        for i in 0..wavelength_settle_secs {
            thread::sleep(Duration::from_secs(1));
            if i % 10 == 9 {
                print!(".");
                std::io::stdout().flush().unwrap();
            }
        }
        println!(" done");

        // Verify wavelength
        let actual_wl = maitai.get_wavelength().unwrap_or(0.0);
        println!("  Actual wavelength: {:.0}nm", actual_wl);
        println!();

        // Sweep LP, HWP, QWP
        for (lp_idx, &lp_angle) in lp_angles.iter().enumerate() {
            // Move LP
            rotator_lp.move_abs(lp_angle).expect("Failed to move LP");
            
            print!("  LP {:3.0}° ({:2}/{:2}): ", lp_angle, lp_idx + 1, n_lp);
            std::io::stdout().flush().unwrap();

            let mut lp_min_power = f64::MAX;
            let mut lp_max_power = f64::MIN;

            for (hwp_idx, &hwp_angle) in hwp_angles.iter().enumerate() {
                // Move HWP
                rotator_hwp.move_abs(hwp_angle).expect("Failed to move HWP");

                for (qwp_idx, &qwp_angle) in qwp_angles.iter().enumerate() {
                    // Move QWP
                    rotator_qwp.move_abs(qwp_angle).expect("Failed to move QWP");
                    
                    thread::sleep(Duration::from_millis(POWER_SETTLE_MS));

                    // Read power
                    let power_w = power_meter.read_power().unwrap_or(f64::NAN);
                    power_4d[wl_idx][lp_idx][hwp_idx][qwp_idx] = power_w;

                    let power_mw = power_w * 1000.0;
                    if power_mw < lp_min_power { lp_min_power = power_mw; }
                    if power_mw > lp_max_power { lp_max_power = power_mw; }

                    total_measured += 1;
                }
            }

            // Progress indicator for this LP angle
            let points_at_lp = n_hwp * n_qwp;
            print!("{} pts, ", points_at_lp);
            if lp_min_power.is_finite() && lp_max_power.is_finite() {
                println!("range: {:.2} - {:.2} mW", lp_min_power, lp_max_power);
            } else {
                println!("range: ERROR");
            }
        }

        let wl_elapsed = wl_start.elapsed();
        let remaining_wl = n_wl - wl_idx - 1;
        let eta_secs = remaining_wl as u64 * wl_elapsed.as_secs();
        println!();
        println!("  Wavelength complete in {:.1} min, ETA: {} min {} sec remaining",
            wl_elapsed.as_secs_f64() / 60.0,
            eta_secs / 60,
            eta_secs % 60);
        println!();
    }

    // Reset to safe state
    println!("Resetting to safe state...");
    maitai.close_shutter().expect("Failed to close shutter");
    rotator_lp.move_abs(0.0).expect("Failed to home LP");
    rotator_hwp.move_abs(0.0).expect("Failed to home HWP");
    rotator_qwp.move_abs(0.0).expect("Failed to home QWP");
    println!("  Shutter closed, all rotators homed to 0°");

    // Save 4D data
    println!("[6/6] Saving 4D calibration data to HDF5...");
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("{}/waveplate_cal_4d_{}.h5", output_dir, timestamp);

    match save_4d_hdf5(
        &filename,
        &wavelengths,
        &lp_angles,
        &hwp_angles,
        &qwp_angles,
        &power_4d,
        wavelength_settle_secs,
    ) {
        Ok(()) => {
            println!("  Data saved to: {}", filename);
            println!("\n  Python analysis:");
            println!("    import xarray as xr");
            println!("    ds = xr.open_dataset('{}', engine='h5netcdf')", filename);
            println!("    ds.power.sel(wavelength=800.0, method='nearest').max(dim='qwp_angle').plot()");
            println!("    ds.power.sel(lp_angle=90.0, hwp_angle=45.0, method='nearest').plot()");
        }
        Err(e) => {
            println!("  WARNING: Failed to save HDF5: {}", e);
        }
    }

    // Statistics
    let all_powers: Vec<f64> = power_4d
        .iter()
        .flat_map(|wl| wl.iter().flat_map(|lp| lp.iter().flat_map(|hwp| hwp.iter().copied())))
        .filter(|p| p.is_finite())
        .collect();

    let (min_power, max_power) = if all_powers.is_empty() {
        (0.0, 0.0)
    } else {
        let min = all_powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = all_powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        (min, max)
    };

    let dynamic_range = if min_power > 0.0 { max_power / min_power } else { 0.0 };
    let dynamic_range_db = if dynamic_range > 0.0 { 10.0 * dynamic_range.log10() } else { 0.0 };

    let total_elapsed = start_time.elapsed();

    println!("\n═══════════════════════════════════════════════════════════════");
    println!("  4D CALIBRATION COMPLETE");
    println!("═══════════════════════════════════════════════════════════════");
    println!("  Grid size:       {} wl × {} LP × {} HWP × {} QWP = {} points",
        n_wl, n_lp, n_hwp, n_qwp, total_points);
    println!("  Points measured: {}", total_measured);
    println!("  Min power:       {:.3} mW", min_power * 1000.0);
    println!("  Max power:       {:.3} mW", max_power * 1000.0);
    println!("  Dynamic range:   {:.1}:1 ({:.1} dB)", dynamic_range, dynamic_range_db);
    println!("  Total time:      {} hr {} min {} sec",
        total_elapsed.as_secs() / 3600,
        (total_elapsed.as_secs() % 3600) / 60,
        total_elapsed.as_secs() % 60);
    println!();
}

fn save_4d_hdf5(
    filename: &str,
    wavelengths: &[f64],
    lp_angles: &[f64],
    hwp_angles: &[f64],
    qwp_angles: &[f64],
    power_4d: &[Vec<Vec<Vec<f64>>>],
    settle_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use hdf5::File;
    use ndarray::Array4;

    println!("Saving 4D data to: {}", filename);

    let file = File::create(filename)?;

    // Coordinate arrays
    let wl_ds = file.new_dataset::<f64>().shape([wavelengths.len()]).create("wavelength")?;
    wl_ds.write(wavelengths)?;
    wl_ds.new_attr::<hdf5::types::VarLenUnicode>().create("units")?
        .write_scalar(&"nm".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    wl_ds.new_attr::<hdf5::types::VarLenUnicode>().create("long_name")?
        .write_scalar(&"Laser Wavelength".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    let lp_ds = file.new_dataset::<f64>().shape([lp_angles.len()]).create("lp_angle")?;
    lp_ds.write(lp_angles)?;
    lp_ds.new_attr::<hdf5::types::VarLenUnicode>().create("units")?
        .write_scalar(&"deg".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    lp_ds.new_attr::<hdf5::types::VarLenUnicode>().create("long_name")?
        .write_scalar(&"Linear Polarizer Angle".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    let hwp_ds = file.new_dataset::<f64>().shape([hwp_angles.len()]).create("hwp_angle")?;
    hwp_ds.write(hwp_angles)?;
    hwp_ds.new_attr::<hdf5::types::VarLenUnicode>().create("units")?
        .write_scalar(&"deg".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    hwp_ds.new_attr::<hdf5::types::VarLenUnicode>().create("long_name")?
        .write_scalar(&"Half-Wave Plate Angle".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    let qwp_ds = file.new_dataset::<f64>().shape([qwp_angles.len()]).create("qwp_angle")?;
    qwp_ds.write(qwp_angles)?;
    qwp_ds.new_attr::<hdf5::types::VarLenUnicode>().create("units")?
        .write_scalar(&"deg".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    qwp_ds.new_attr::<hdf5::types::VarLenUnicode>().create("long_name")?
        .write_scalar(&"Quarter-Wave Plate Angle".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // Power data (4D)
    let n_wl = wavelengths.len();
    let n_lp = lp_angles.len();
    let n_hwp = hwp_angles.len();
    let n_qwp = qwp_angles.len();

    let mut power_flat: Vec<f64> = Vec::with_capacity(n_wl * n_lp * n_hwp * n_qwp);
    for wl_data in power_4d {
        for lp_data in wl_data {
            for hwp_data in lp_data {
                power_flat.extend(hwp_data);
            }
        }
    }

    let power_ds = file.new_dataset::<f64>()
        .shape([n_wl, n_lp, n_hwp, n_qwp])
        .create("power")?;

    let power_array = Array4::from_shape_vec((n_wl, n_lp, n_hwp, n_qwp), power_flat.clone())
        .map_err(|e| format!("Failed to create 4D array: {}", e))?;
    power_ds.write(power_array.view())?;

    power_ds.new_attr::<hdf5::types::VarLenUnicode>().create("units")?
        .write_scalar(&"W".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    power_ds.new_attr::<hdf5::types::VarLenUnicode>().create("long_name")?
        .write_scalar(&"Optical Power".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // _ARRAY_DIMENSIONS for xarray
    let _dims = power_ds.new_attr_builder()
        .with_data(&[
            "wavelength".parse::<hdf5::types::VarLenUnicode>().unwrap(),
            "lp_angle".parse::<hdf5::types::VarLenUnicode>().unwrap(),
            "hwp_angle".parse::<hdf5::types::VarLenUnicode>().unwrap(),
            "qwp_angle".parse::<hdf5::types::VarLenUnicode>().unwrap(),
        ])
        .create("_ARRAY_DIMENSIONS")?;

    // Global attributes
    let timestamp_str = Local::now().to_rfc3339();
    file.new_attr::<hdf5::types::VarLenUnicode>().create("experiment")?
        .write_scalar(&"4D Waveplate Calibration".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<hdf5::types::VarLenUnicode>().create("timestamp")?
        .write_scalar(&timestamp_str.parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<hdf5::types::VarLenUnicode>().create("instrument")?
        .write_scalar(&"MaiTai + Newport 1830-C + 3x ELL14".parse::<hdf5::types::VarLenUnicode>().unwrap())?;

    // Dimensions
    file.new_attr::<u64>().create("n_wavelengths")?.write_scalar(&(n_wl as u64))?;
    file.new_attr::<u64>().create("n_lp_angles")?.write_scalar(&(n_lp as u64))?;
    file.new_attr::<u64>().create("n_hwp_angles")?.write_scalar(&(n_hwp as u64))?;
    file.new_attr::<u64>().create("n_qwp_angles")?.write_scalar(&(n_qwp as u64))?;
    file.new_attr::<u64>().create("n_total_points")?.write_scalar(&((n_wl * n_lp * n_hwp * n_qwp) as u64))?;
    file.new_attr::<u64>().create("wavelength_settle_secs")?.write_scalar(&settle_secs)?;

    // Min/max
    let valid_powers: Vec<f64> = power_flat.iter().filter(|p| p.is_finite()).copied().collect();
    if !valid_powers.is_empty() {
        let min_p = valid_powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_p = valid_powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        file.new_attr::<f64>().create("min_power_W")?.write_scalar(&min_p)?;
        file.new_attr::<f64>().create("max_power_W")?.write_scalar(&max_p)?;
    }

    Ok(())
}

// Simple MaiTai wrapper
struct MaiTaiSimple {
    port: Box<dyn serialport::SerialPort>,
}

impl MaiTaiSimple {
    fn open(port_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let port = serialport::new(port_path, 115_200)
            .timeout(Duration::from_millis(500))
            .open()?;
        Ok(Self { port })
    }

    fn send_command(&mut self, cmd: &str) -> Result<String, Box<dyn std::error::Error>> {
        let mut discard = [0u8; 256];
        let _ = self.port.read(&mut discard);

        let cmd_with_lf = format!("{}\n", cmd);
        self.port.write_all(cmd_with_lf.as_bytes())?;
        self.port.flush()?;

        thread::sleep(Duration::from_millis(100));

        let mut reader = BufReader::new(&mut self.port);
        let mut response = String::new();
        let _ = reader.read_line(&mut response);

        Ok(response.trim().to_string())
    }

    fn set_wavelength(&mut self, wavelength_nm: f64) -> Result<(), Box<dyn std::error::Error>> {
        let cmd = format!("wav {:.0}", wavelength_nm);
        self.send_command(&cmd)?;
        thread::sleep(Duration::from_millis(500));
        Ok(())
    }

    fn get_wavelength(&mut self) -> Result<f64, Box<dyn std::error::Error>> {
        let response = self.send_command("wav?")?;
        let wl_str = response.trim_end_matches("nm").trim();
        wl_str.parse::<f64>()
            .map_err(|_| format!("Failed to parse wavelength: {}", response).into())
    }

    fn open_shutter(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.send_command("shut 1")?;
        thread::sleep(Duration::from_millis(500));
        Ok(())
    }

    fn close_shutter(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.send_command("shut 0")?;
        thread::sleep(Duration::from_millis(200));
        Ok(())
    }
}

// Simple Newport 1830-C wrapper
struct Newport1830Simple {
    port: Box<dyn serialport::SerialPort>,
}

impl Newport1830Simple {
    fn open(port_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let port = serialport::new(port_path, 9600)
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

    fn set_wavelength(&mut self, wavelength_nm: f64) -> Result<(), Box<dyn std::error::Error>> {
        let wl_int = wavelength_nm.round() as u32;
        let cmd = format!("W{:04}\r", wl_int);
        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()?;
        thread::sleep(Duration::from_millis(200));
        Ok(())
    }
}

// Simple ELL14 wrapper with RS-485 multidrop support
struct Ell14Simple {
    port: Box<dyn serialport::SerialPort>,
    address: String,
    pulses_per_degree: f64,
}

impl Ell14Simple {
    fn open(port_path: &str, address: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut port = serialport::new(port_path, 9600)
            .timeout(Duration::from_millis(500))
            .open()?;

        // Clear buffer
        let mut discard = [0u8; 256];
        for _ in 0..5 {
            match port.read(&mut discard) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
        thread::sleep(Duration::from_millis(100));

        // Query device info
        let cmd = format!("{}in", address);
        port.write_all(cmd.as_bytes())?;
        port.flush()?;
        thread::sleep(Duration::from_millis(200));

        let mut response = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        for _ in 0..5 {
            match port.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    response.extend_from_slice(&buf[..n]);
                    if response.len() >= 32 { break; }
                }
                Err(_) => break,
            }
            thread::sleep(Duration::from_millis(20));
        }

        let response_str = String::from_utf8_lossy(&response);
        let expected_prefix = format!("{}IN", address);
        
        // Default for ELL14: 143360 pulses/revolution = 1433.60 pulses/degree
        let pulses_per_degree = if let Some(idx) = response_str.find(&expected_prefix) {
            let data_start = idx + 3;
            if response_str.len() >= data_start + 30 {
                let pulses_hex = &response_str[data_start + 22..data_start + 30];
                match u32::from_str_radix(pulses_hex.trim(), 16) {
                    Ok(p) if p > 1000 && p < 1000000 => p as f64 / 100.0,
                    _ => 1433.60,
                }
            } else {
                1433.60
            }
        } else {
            1433.60
        };

        Ok(Self {
            port,
            address: address.to_string(),
            pulses_per_degree,
        })
    }

    fn move_abs(&mut self, degrees: f64) -> Result<(), Box<dyn std::error::Error>> {
        let pulses = (degrees * self.pulses_per_degree).round() as u32;
        let cmd = format!("{}ma{:08X}", self.address, pulses);

        // Clear buffer
        let mut discard = [0u8; 256];
        for _ in 0..3 {
            match self.port.read(&mut discard) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }

        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()?;

        // Wait for move
        let move_time = ((degrees.abs() / 360.0) * 1500.0).max(ROTATOR_SETTLING_MS as f64);
        thread::sleep(Duration::from_millis(move_time as u64));

        // Discard response
        let mut response = [0u8; 64];
        for _ in 0..3 {
            match self.port.read(&mut response) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }

        Ok(())
    }
}
