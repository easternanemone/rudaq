//! 3D EOM + Rotator + Wavelength Power Sweep Test
//!
//! Performs a 3D parameter scan:
//! - Outer loop: MaiTai wavelength (with stabilization delay)
//! - Middle loop: ELL14 rotator angle
//! - Inner loop: EOM voltage via Comedi DAC0
//!
//! SAFETY: This test controls laser power via EOM. The laser shutter is opened
//! during the sweep. Ensure proper laser safety protocols are followed.
//!
//! Run with: EOM_3D_TEST=1 cargo test --features hardware -p daq-driver-comedi \
//!           --test eom_rotator_wavelength_3d_sweep -- --nocapture
//!
//! Environment variables for customization:
//!   WAVELENGTH_MIN=750       # nm (default 780)
//!   WAVELENGTH_MAX=850       # nm (default 820)
//!   WAVELENGTH_STEP=20       # nm (default 20)
//!   WAVELENGTH_SETTLE_SECS=60 # seconds (default 60)
//!   ROTATOR_ANGLE_MIN=0.0    # degrees
//!   ROTATOR_ANGLE_MAX=90.0   # degrees
//!   ROTATOR_ANGLE_STEP=15.0  # degrees
//!   EOM_VOLTAGE_MIN=-5.0     # volts
//!   EOM_VOLTAGE_MAX=5.0      # volts
//!   EOM_VOLTAGE_STEP=1.0     # volts

use chrono::Local;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::thread;
use std::time::Duration;

// Comedi constants
const COMEDI_DEVICE: &str = "/dev/comedi0";
const DAC_SUBDEVICE: u32 = 1;
const DAC_CHANNEL: u32 = 0; // AO0 - EOM amplifier
const DAC_RANGE: u32 = 0; // ±10V

// Serial port paths
const MAITAI_PORT: &str = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const NEWPORT_PORT: &str = "/dev/ttyS0";
const ELLIPTEC_PORT: &str =
    "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";
const ROTATOR_ADDRESS: &str = "2"; // HWP rotator

// Timing
const ROTATOR_SETTLING_MS: u64 = 500;
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

#[test]
fn test_eom_rotator_wavelength_3d_sweep() {
    if env::var("EOM_3D_TEST").is_err() {
        eprintln!("Skipping 3D sweep test (set EOM_3D_TEST=1 to enable)");
        return;
    }

    // Parse configuration from environment
    let wavelength_min = get_env_f64("WAVELENGTH_MIN", 780.0);
    let wavelength_max = get_env_f64("WAVELENGTH_MAX", 820.0);
    let wavelength_step = get_env_f64("WAVELENGTH_STEP", 20.0);
    let wavelength_settle_secs = get_env_u64("WAVELENGTH_SETTLE_SECS", 60);

    let angle_min = get_env_f64("ROTATOR_ANGLE_MIN", 0.0);
    let angle_max = get_env_f64("ROTATOR_ANGLE_MAX", 90.0);
    let angle_step = get_env_f64("ROTATOR_ANGLE_STEP", 15.0);

    let voltage_min = get_env_f64("EOM_VOLTAGE_MIN", -5.0);
    let voltage_max = get_env_f64("EOM_VOLTAGE_MAX", 5.0);
    let voltage_step = get_env_f64("EOM_VOLTAGE_STEP", 1.0);

    // Build coordinate arrays
    let wavelengths: Vec<f64> = {
        let mut v = Vec::new();
        let mut wl = wavelength_min;
        while wl <= wavelength_max + 1e-9 {
            v.push(wl);
            wl += wavelength_step;
        }
        v
    };

    let angles: Vec<f64> = {
        let mut v = Vec::new();
        let mut a = angle_min;
        while a <= angle_max + 1e-9 {
            v.push(a);
            a += angle_step;
        }
        v
    };

    let voltages: Vec<f64> = {
        let mut v = Vec::new();
        let mut volt = voltage_min;
        while volt <= voltage_max + 1e-9 {
            v.push(volt);
            volt += voltage_step;
        }
        v
    };

    let n_wavelengths = wavelengths.len();
    let n_angles = angles.len();
    let n_voltages = voltages.len();
    let total_points = n_wavelengths * n_angles * n_voltages;

    let output_dir = shellexpand::tilde("~/rust-daq/data").to_string();

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║  EOM + ROTATOR + WAVELENGTH 3D POWER SWEEP TEST              ║");
    println!("║  ⚠️  LASER SAFETY: Opening shutter and controlling power     ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    println!("Configuration:");
    println!("  Comedi device:  {}", COMEDI_DEVICE);
    println!("  MaiTai port:    {}", MAITAI_PORT);
    println!("  Newport port:   {}", NEWPORT_PORT);
    println!("  Elliptec port:  {}", ELLIPTEC_PORT);
    println!("  Rotator addr:   {}", ROTATOR_ADDRESS);
    println!();
    println!(
        "  Wavelength range: {:.0}nm to {:.0}nm (step: {:.0}nm) -> {} points",
        wavelength_min, wavelength_max, wavelength_step, n_wavelengths
    );
    println!(
        "  Stabilization:    {} seconds after wavelength change",
        wavelength_settle_secs
    );
    println!(
        "  Angle range:      {:.1}° to {:.1}° (step: {:.1}°) -> {} points",
        angle_min, angle_max, angle_step, n_angles
    );
    println!(
        "  Voltage range:    {:.1}V to {:.1}V (step: {:.2}V) -> {} points",
        voltage_min, voltage_max, voltage_step, n_voltages
    );
    println!("  Total points:     {}", total_points);
    println!("  Output dir:       {}", output_dir);

    // Estimate time
    let time_per_2d = (n_angles * n_voltages) as u64 * (POWER_SETTLE_MS + 50) / 1000;
    let total_time_secs = n_wavelengths as u64 * (wavelength_settle_secs + time_per_2d);
    println!(
        "  Estimated time:   {} min {} sec",
        total_time_secs / 60,
        total_time_secs % 60
    );
    println!();

    // Initialize hardware
    println!("[1/8] Opening Comedi DAQ...");
    let comedi = ComediSimple::open(COMEDI_DEVICE).expect("Failed to open Comedi device");
    let dac_maxdata = comedi.get_maxdata(DAC_SUBDEVICE, DAC_CHANNEL);
    let (dac_min, dac_max) = comedi.get_range(DAC_SUBDEVICE, DAC_CHANNEL, DAC_RANGE);
    println!("  DAC0 range: {:.1}V to {:.1}V", dac_min, dac_max);

    println!("[2/8] Opening ELL14 rotator...");
    let mut rotator =
        Ell14Simple::open(ELLIPTEC_PORT, ROTATOR_ADDRESS).expect("Failed to open ELL14");
    let initial_pos = rotator.get_position().unwrap_or(f64::NAN);
    println!(
        "  ELL14 (addr {}) pulses/degree: {:.2}",
        ROTATOR_ADDRESS, rotator.pulses_per_degree
    );
    println!("  Current position: {:.2}°", initial_pos);

    println!("[3/8] Setting EOM to 0V (safe state)...");
    let zero_data = voltage_to_data(0.0, dac_min, dac_max, dac_maxdata);
    comedi
        .write_single(DAC_SUBDEVICE, DAC_CHANNEL, DAC_RANGE, zero_data)
        .expect("Failed to set DAC");

    println!("[4/8] Opening MaiTai laser...");
    let mut maitai = MaiTaiSimple::open(MAITAI_PORT).expect("Failed to open MaiTai");
    let initial_wavelength = maitai.get_wavelength().unwrap_or(0.0);
    println!("  Initial wavelength: {:.0} nm", initial_wavelength);

    println!("[5/8] Opening MaiTai shutter...");
    maitai.open_shutter().expect("Failed to open shutter");
    println!("  Shutter opened");

    println!("[6/8] Opening Newport power meter...");
    let mut power_meter = Newport1830Simple::open(NEWPORT_PORT).expect("Failed to open Newport");
    let initial_power = power_meter.read_power().unwrap_or(0.0);
    println!("  Initial power: {:.3} mW", initial_power * 1000.0);

    // 3D data storage: [wavelength][angle][voltage]
    let mut power_3d: Vec<Vec<Vec<f64>>> =
        vec![vec![vec![0.0; n_voltages]; n_angles]; n_wavelengths];

    println!("[7/8] Performing 3D sweep...\n");

    for (wl_idx, &wavelength) in wavelengths.iter().enumerate() {
        println!(
            "━━━ Wavelength {:.0}nm ({}/{}) ━━━",
            wavelength,
            wl_idx + 1,
            n_wavelengths
        );

        // Set MaiTai wavelength
        print!("  Setting MaiTai to {:.0}nm... ", wavelength);
        std::io::stdout().flush().unwrap();
        maitai
            .set_wavelength(wavelength)
            .expect("Failed to set MaiTai wavelength");
        println!("done");

        // Set Newport power meter calibration wavelength
        print!("  Setting Newport calibration to {:.0}nm... ", wavelength);
        std::io::stdout().flush().unwrap();
        power_meter
            .set_wavelength(wavelength)
            .expect("Failed to set Newport wavelength");
        println!("done");

        // Wait for laser stabilization
        print!(
            "  Waiting {} seconds for stabilization",
            wavelength_settle_secs
        );
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

        // 2D sweep at this wavelength
        for (angle_idx, &angle) in angles.iter().enumerate() {
            // Move rotator
            rotator.move_abs(angle).expect("Failed to move rotator");

            let mut row_powers = Vec::with_capacity(n_voltages);
            let mut row_min = f64::MAX;
            let mut row_max = f64::MIN;

            // Sweep voltage
            for (v_idx, &voltage) in voltages.iter().enumerate() {
                let data = voltage_to_data(voltage, dac_min, dac_max, dac_maxdata);
                comedi
                    .write_single(DAC_SUBDEVICE, DAC_CHANNEL, DAC_RANGE, data)
                    .expect("Failed to set DAC");

                thread::sleep(Duration::from_millis(POWER_SETTLE_MS));

                let power_w = power_meter.read_power().unwrap_or(f64::NAN);
                let power_mw = power_w * 1000.0;

                power_3d[wl_idx][angle_idx][v_idx] = power_w;
                row_powers.push(power_mw);

                if power_mw < row_min {
                    row_min = power_mw;
                }
                if power_mw > row_max {
                    row_max = power_mw;
                }
            }

            // Print row visualization
            let bar = make_power_bar(&row_powers, 0.0, 50.0);
            println!(
                "  Angle {:3.0}° ({:2}/{:2}): [{}] min={:.2}mW max={:.2}mW",
                angle,
                angle_idx + 1,
                n_angles,
                bar,
                row_min,
                row_max
            );
        }
        println!();
    }

    // Reset to safe state
    println!("Resetting to safe state...");
    maitai.close_shutter().expect("Failed to close shutter");
    comedi
        .write_single(DAC_SUBDEVICE, DAC_CHANNEL, DAC_RANGE, zero_data)
        .expect("Failed to set DAC to 0V");
    rotator.move_abs(0.0).expect("Failed to home rotator");
    println!("  Shutter closed, EOM at 0V, rotator at 0°");

    // Save 3D data
    println!("[8/8] Saving 3D data to HDF5...");
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("{}/eom_rotator_wl_3d_{}.h5", output_dir, timestamp);

    match save_3d_hdf5(
        &filename,
        &wavelengths,
        &angles,
        &voltages,
        &power_3d,
        wavelength_settle_secs,
    ) {
        Ok(()) => {
            println!("  Data saved to: {}", filename);
            println!("\n  Python analysis:");
            println!("    import xarray as xr");
            println!(
                "    ds = xr.open_dataset('{}', engine='h5netcdf')",
                filename
            );
            println!("    ds.power.sel(wavelength=800.0, method='nearest').plot()  # 2D slice at 800nm");
            println!("    ds.power.sel(angle=45.0, method='nearest').plot()  # 2D slice at 45°");
            println!(
                "    ds.power.sel(wavelength=800.0, angle=45.0, method='nearest').plot()  # 1D slice"
            );
        }
        Err(e) => {
            println!("  WARNING: Failed to save HDF5: {}", e);
        }
    }

    // Statistics
    let mut all_powers: Vec<f64> = power_3d
        .iter()
        .flat_map(|wl| wl.iter().flat_map(|a| a.iter().copied()))
        .filter(|p| !p.is_nan())
        .collect();

    let (min_power, max_power) = if all_powers.is_empty() {
        (0.0, 0.0)
    } else {
        all_powers.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (all_powers[0], all_powers[all_powers.len() - 1])
    };

    let dynamic_range = if min_power > 0.0 {
        max_power / min_power
    } else {
        0.0
    };
    let dynamic_range_db = if dynamic_range > 0.0 {
        10.0 * dynamic_range.log10()
    } else {
        0.0
    };

    println!("\n═══════════════════════════════════════════════════════════════");
    println!("  3D SWEEP COMPLETE");
    println!("═══════════════════════════════════════════════════════════════");
    println!(
        "  Grid size:       {} wavelengths × {} angles × {} voltages = {} points",
        n_wavelengths, n_angles, n_voltages, total_points
    );
    println!("  Min power:       {:.3} mW", min_power * 1000.0);
    println!("  Max power:       {:.3} mW", max_power * 1000.0);
    println!(
        "  Dynamic range:   {:.1}:1 ({:.1} dB)",
        dynamic_range, dynamic_range_db
    );
    println!();
}

fn voltage_to_data(voltage: f64, range_min: f64, range_max: f64, maxdata: u32) -> u32 {
    let fraction = (voltage - range_min) / (range_max - range_min);
    let clamped = fraction.clamp(0.0, 1.0);
    (clamped * maxdata as f64).round() as u32
}

fn make_power_bar(powers: &[f64], min_scale: f64, max_scale: f64) -> String {
    powers
        .iter()
        .map(|&p| {
            if p.is_nan() {
                '?'
            } else {
                let normalized = (p - min_scale) / (max_scale - min_scale);
                if normalized < 0.1 {
                    '.'
                } else if normalized < 0.3 {
                    'o'
                } else if normalized < 0.6 {
                    'O'
                } else {
                    '#'
                }
            }
        })
        .collect()
}

fn save_3d_hdf5(
    filename: &str,
    wavelengths: &[f64],
    angles: &[f64],
    voltages: &[f64],
    power_3d: &[Vec<Vec<f64>>],
    settle_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use hdf5::File;
    use ndarray::Array3;

    println!("Saving 3D data to: {}", filename);

    let file = File::create(filename)?;

    // Wavelength coordinate (1D)
    let wl_ds = file
        .new_dataset::<f64>()
        .shape([wavelengths.len()])
        .create("wavelength")?;
    wl_ds.write(wavelengths)?;
    wl_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("units")?
        .write_scalar(&"nm".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    wl_ds
        .new_attr::<hdf5::types::VarLenUnicode>()
        .create("long_name")?
        .write_scalar(
            &"Laser Wavelength"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;

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
        .write_scalar(
            &"Rotator Angle"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;

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
        .write_scalar(
            &"EOM Control Voltage"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;

    // Power data (3D: [wavelength, angle, voltage])
    let n_wl = wavelengths.len();
    let n_angles = angles.len();
    let n_voltages = voltages.len();

    // Flatten 3D array
    let mut power_flat: Vec<f64> = Vec::with_capacity(n_wl * n_angles * n_voltages);
    for wl_data in power_3d {
        for angle_data in wl_data {
            power_flat.extend(angle_data);
        }
    }

    let power_ds = file
        .new_dataset::<f64>()
        .shape([n_wl, n_angles, n_voltages])
        .create("power")?;

    let power_array = Array3::from_shape_vec((n_wl, n_angles, n_voltages), power_flat.clone())
        .map_err(|e| format!("Failed to create 3D array: {}", e))?;
    power_ds.write(power_array.view())?;

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

    // _ARRAY_DIMENSIONS for xarray
    let _dims = power_ds
        .new_attr_builder()
        .with_data(&[
            "wavelength".parse::<hdf5::types::VarLenUnicode>().unwrap(),
            "angle".parse::<hdf5::types::VarLenUnicode>().unwrap(),
            "voltage".parse::<hdf5::types::VarLenUnicode>().unwrap(),
        ])
        .create("_ARRAY_DIMENSIONS")?;

    // Global attributes
    let timestamp_str = Local::now().to_rfc3339();
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("experiment")?
        .write_scalar(
            &"EOM + Rotator + Wavelength 3D Power Sweep"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("timestamp")?
        .write_scalar(&timestamp_str.parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    file.new_attr::<hdf5::types::VarLenUnicode>()
        .create("instrument")?
        .write_scalar(
            &"Newport 1830-C + MaiTai + ELL14 + Comedi"
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;

    // Sweep parameters
    file.new_attr::<u64>()
        .create("n_wavelengths")?
        .write_scalar(&(n_wl as u64))?;
    file.new_attr::<u64>()
        .create("n_angles")?
        .write_scalar(&(n_angles as u64))?;
    file.new_attr::<u64>()
        .create("n_voltages")?
        .write_scalar(&(n_voltages as u64))?;
    file.new_attr::<u64>()
        .create("n_total_points")?
        .write_scalar(&((n_wl * n_angles * n_voltages) as u64))?;

    file.new_attr::<f64>()
        .create("wavelength_min")?
        .write_scalar(&wavelengths[0])?;
    file.new_attr::<f64>()
        .create("wavelength_max")?
        .write_scalar(&wavelengths[wavelengths.len() - 1])?;
    file.new_attr::<u64>()
        .create("wavelength_settle_secs")?
        .write_scalar(&settle_secs)?;

    // Min/max power
    let valid_powers: Vec<f64> = power_flat.iter().filter(|p| !p.is_nan()).copied().collect();
    if !valid_powers.is_empty() {
        let min_p = valid_powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_p = valid_powers
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        file.new_attr::<f64>()
            .create("min_power_W")?
            .write_scalar(&min_p)?;
        file.new_attr::<f64>()
            .create("max_power_W")?
            .write_scalar(&max_p)?;
    }

    Ok(())
}

// Simple Comedi wrapper for this test
struct ComediSimple {
    handle: *mut comedi_sys::comedi_t,
}

impl ComediSimple {
    fn open(device: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let c_device = std::ffi::CString::new(device)?;
        let handle = unsafe { comedi_sys::comedi_open(c_device.as_ptr()) };
        if handle.is_null() {
            return Err("Failed to open Comedi device".into());
        }
        Ok(Self { handle })
    }

    fn get_maxdata(&self, subdevice: u32, channel: u32) -> u32 {
        unsafe { comedi_sys::comedi_get_maxdata(self.handle, subdevice, channel) }
    }

    fn get_range(&self, subdevice: u32, channel: u32, range: u32) -> (f64, f64) {
        let range_ptr =
            unsafe { comedi_sys::comedi_get_range(self.handle, subdevice, channel, range) };
        if range_ptr.is_null() {
            return (-10.0, 10.0);
        }
        unsafe { ((*range_ptr).min, (*range_ptr).max) }
    }

    fn write_single(
        &self,
        subdevice: u32,
        channel: u32,
        range: u32,
        data: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let result = unsafe {
            comedi_sys::comedi_data_write(
                self.handle,
                subdevice,
                channel,
                range,
                0, // aref
                data,
            )
        };
        if result < 0 {
            return Err("Comedi write failed".into());
        }
        Ok(())
    }
}

impl Drop for ComediSimple {
    fn drop(&mut self) {
        unsafe {
            comedi_sys::comedi_close(self.handle);
        }
    }
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
        // Clear buffer
        let mut discard = [0u8; 256];
        let _ = self.port.read(&mut discard);

        // Send command with LF terminator
        let cmd_with_lf = format!("{}\n", cmd);
        self.port.write_all(cmd_with_lf.as_bytes())?;
        self.port.flush()?;

        thread::sleep(Duration::from_millis(100));

        // Read response
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
        // Response format: "820nm" or similar
        let wl_str = response.trim_end_matches("nm").trim();
        wl_str
            .parse::<f64>()
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
            .timeout(Duration::from_millis(500))
            .open()?;
        Ok(Self { port })
    }

    fn send_command(&mut self, cmd: &str) -> Result<String, Box<dyn std::error::Error>> {
        // Clear buffer
        let mut discard = [0u8; 256];
        let _ = self.port.read(&mut discard);

        // Send command (no terminator needed for 1830-C)
        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()?;

        thread::sleep(Duration::from_millis(100));

        // Read response
        let mut response = [0u8; 64];
        let n = self.port.read(&mut response).unwrap_or(0);
        Ok(String::from_utf8_lossy(&response[..n]).to_string())
    }

    fn read_power(&mut self) -> Result<f64, Box<dyn std::error::Error>> {
        let response = self.send_command("D?")?;
        // Response format: floating point in Watts
        response
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("Failed to parse power: {}", response).into())
    }

    fn set_wavelength(&mut self, wavelength_nm: f64) -> Result<(), Box<dyn std::error::Error>> {
        // Newport 1830-C uses W command with 4-digit integer nm
        let wl_int = wavelength_nm.round() as u32;
        let cmd = format!("W{:04}", wl_int);
        self.send_command(&cmd)?;
        thread::sleep(Duration::from_millis(200));
        Ok(())
    }
}

// Simple ELL14 wrapper (copied from 2D test)
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

        // Query device info to get pulses per degree
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
                    if response.len() >= 32 {
                        break;
                    }
                }
                Err(_) => break,
            }
            thread::sleep(Duration::from_millis(20));
        }

        let response_str = String::from_utf8_lossy(&response);

        // Parse pulses per degree from IN response
        // Format: {addr}IN{type}{serial}{year}{fwrel}{hwrel}{travel}{pulses}
        // pulses is 8 hex chars at position 23-30
        let expected_prefix = format!("{}IN", address);
        let pulses_per_degree = if let Some(idx) = response_str.find(&expected_prefix) {
            let data_start = idx + 3;
            if response_str.len() >= data_start + 28 {
                let pulses_hex = &response_str[data_start + 20..data_start + 28];
                u32::from_str_radix(pulses_hex, 16).unwrap_or(143360) as f64 / 100.0
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
        for _ in 0..5 {
            match self.port.read(&mut discard) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
        thread::sleep(Duration::from_millis(50));

        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()?;

        // Wait based on move distance
        let move_time = ((degrees.abs() / 360.0) * 2000.0).max(ROTATOR_SETTLING_MS as f64);
        thread::sleep(Duration::from_millis(move_time as u64));

        // Read and discard response
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

        // Clear buffer
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

        Ok(f64::NAN)
    }
}
