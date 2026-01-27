// 4D Waveplate Calibration Sweep
// ===============================
// Performs comprehensive polarization calibration:
// - Wavelength: 780nm to 900nm (10nm steps) -> 13 points
// - Linear Polarizer (addr 3): 0° to 180° (10° steps) -> 19 points  
// - Half-Wave Plate (addr 2): 0° to 90° (10° steps) -> 10 points
// - Quarter-Wave Plate (addr 8): 0° to 90° (10° steps) -> 10 points
//
// Total: 13 × 19 × 10 × 10 = 24,700 points
// Estimated time: ~2-3 hours
//
// SAFETY: Uses with_shutter_open() to GUARANTEE shutter closure on exit/error

// Configuration - use stable by-id paths
let ELLIPTEC_PORT = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";
let NEWPORT_PORT = "/dev/ttyS0";
let MAITAI_PORT = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";

// Sweep parameters
let WAVELENGTH_MIN = 780.0;
let WAVELENGTH_MAX = 900.0;
let WAVELENGTH_STEP = 10.0;
let WAVELENGTH_SETTLE_SECS = 60;  // 1 minute stabilization after wavelength change

let LP_MIN = 0.0;
let LP_MAX = 180.0;
let LP_STEP = 10.0;

let HWP_MIN = 0.0;
let HWP_MAX = 90.0;
let HWP_STEP = 10.0;

let QWP_MIN = 0.0;
let QWP_MAX = 90.0;
let QWP_STEP = 10.0;

let SAMPLES_PER_POINT = 3;
let SETTLE_TIME = 0.1;

// Build coordinate arrays
fn build_array(min, max, step) {
    let arr = [];
    let val = min;
    while val <= max + 0.001 {
        arr.push(val);
        val += step;
    }
    arr
}

let wavelengths = build_array(WAVELENGTH_MIN, WAVELENGTH_MAX, WAVELENGTH_STEP);
let lp_angles = build_array(LP_MIN, LP_MAX, LP_STEP);
let hwp_angles = build_array(HWP_MIN, HWP_MAX, HWP_STEP);
let qwp_angles = build_array(QWP_MIN, QWP_MAX, QWP_STEP);

let n_wl = wavelengths.len();
let n_lp = lp_angles.len();
let n_hwp = hwp_angles.len();
let n_qwp = qwp_angles.len();
let total_points = n_wl * n_lp * n_hwp * n_qwp;

// Estimate time
let points_per_wl = n_lp * n_hwp * n_qwp;
let time_per_wl_secs = (points_per_wl * (SETTLE_TIME * 1000 + 50) / 1000).to_int() + WAVELENGTH_SETTLE_SECS;
let total_time_secs = n_wl * time_per_wl_secs;

print("═══════════════════════════════════════════════════════════════");
print("  4D WAVEPLATE CALIBRATION SWEEP");
print("  Time: " + timestamp());
print("═══════════════════════════════════════════════════════════════");
print("");
print("Configuration:");
print("  Wavelength:  " + WAVELENGTH_MIN + "nm to " + WAVELENGTH_MAX + "nm (step " + WAVELENGTH_STEP + "nm) -> " + n_wl + " points");
print("  LP (addr 3): " + LP_MIN + "° to " + LP_MAX + "° (step " + LP_STEP + "°) -> " + n_lp + " points");
print("  HWP (addr 2): " + HWP_MIN + "° to " + HWP_MAX + "° (step " + HWP_STEP + "°) -> " + n_hwp + " points");
print("  QWP (addr 8): " + QWP_MIN + "° to " + QWP_MAX + "° (step " + QWP_STEP + "°) -> " + n_qwp + " points");
print("");
print("  Total points: " + total_points);
print("  Estimated time: " + (total_time_secs / 3600) + " hr " + ((total_time_secs % 3600) / 60) + " min");
print("");

// Initialize hardware
print("[1/7] Initializing hardware...");
let power_meter = create_newport_1830c(NEWPORT_PORT);
let laser = create_maitai_tunable(MAITAI_PORT);
let rotator_lp = create_elliptec(ELLIPTEC_PORT, "3");   // Linear Polarizer
let rotator_hwp = create_elliptec(ELLIPTEC_PORT, "2");  // Half-Wave Plate  
let rotator_qwp = create_elliptec(ELLIPTEC_PORT, "8");  // Quarter-Wave Plate
print("  [OK] All devices initialized");
print("");

// Zero power meter with shutter closed
print("[2/7] Zeroing power meter (shutter closed)...");
laser.close();
sleep(0.5);
power_meter.set_attenuator(true);
power_meter.zero_with_attenuator();
sleep(0.5);
print("  [OK] Power meter zeroed");
print("");

// Home all rotators
print("[3/7] Homing all rotators...");
rotator_lp.home();
sleep(1.0);
rotator_hwp.home();
sleep(1.0);
rotator_qwp.home();
sleep(1.0);
print("  [OK] All rotators homed at 0°");
print("");

// Create HDF5 file
let output_file = "waveplate_cal_4d_" + timestamp() + ".h5";
print("[4/7] Creating HDF5 output file: " + output_file);
let hdf5 = create_hdf5(output_file);

// Write metadata
hdf5.write_attr("experiment", "4D_waveplate_calibration");
hdf5.write_attr("timestamp", timestamp_iso());
hdf5.write_attr_f64("wavelength_min_nm", WAVELENGTH_MIN);
hdf5.write_attr_f64("wavelength_max_nm", WAVELENGTH_MAX);
hdf5.write_attr_f64("wavelength_step_nm", WAVELENGTH_STEP);
hdf5.write_attr_i64("wavelength_settle_secs", WAVELENGTH_SETTLE_SECS);
hdf5.write_attr_f64("lp_min_deg", LP_MIN);
hdf5.write_attr_f64("lp_max_deg", LP_MAX);
hdf5.write_attr_f64("lp_step_deg", LP_STEP);
hdf5.write_attr_f64("hwp_min_deg", HWP_MIN);
hdf5.write_attr_f64("hwp_max_deg", HWP_MAX);
hdf5.write_attr_f64("hwp_step_deg", HWP_STEP);
hdf5.write_attr_f64("qwp_min_deg", QWP_MIN);
hdf5.write_attr_f64("qwp_max_deg", QWP_MAX);
hdf5.write_attr_f64("qwp_step_deg", QWP_STEP);
hdf5.write_attr_i64("samples_per_point", SAMPLES_PER_POINT);
hdf5.write_attr_i64("total_points", total_points);

// Write coordinate arrays
hdf5.write_array_1d("wavelength", wavelengths);
hdf5.write_array_1d("lp_angle", lp_angles);
hdf5.write_array_1d("hwp_angle", hwp_angles);
hdf5.write_array_1d("qwp_angle", qwp_angles);

print("  [OK] HDF5 file created");
print("");

// Create groups for each wavelength
for wl in wavelengths {
    hdf5.create_group("wl_" + wl.to_int());
}

// Run calibration with shutter safety
print("[5/7] Starting calibration sweep (shutter will open)...");
print("  SAFETY: Using with_shutter_open() - shutter will close on any error");
print("");

let shutter_handle = laser.as_shutter();

let all_data = with_shutter_open(shutter_handle, || {
    sleep(1.0);  // Let laser stabilize
    
    let initial_power = power_meter.read();
    print("  Initial power: " + (initial_power * 1000.0) + " mW");
    print("");
    
    let collected = [];
    
    for wl_idx in 0..n_wl {
        let wavelength = wavelengths[wl_idx];
        
        print("═══════════════════════════════════════════════════════════════");
        print("  Wavelength " + wavelength + "nm (" + (wl_idx + 1) + "/" + n_wl + ")");
        print("═══════════════════════════════════════════════════════════════");
        
        // Set wavelengths
        print("  Setting MaiTai to " + wavelength + "nm...");
        laser.set_wavelength(wavelength);
        sleep(0.5);
        
        print("  Setting Newport calibration to " + wavelength + "nm...");
        power_meter.set_wavelength(wavelength);
        sleep(0.2);
        
        // Stabilization
        print("  Stabilizing (" + WAVELENGTH_SETTLE_SECS + " sec)...");
        for i in 0..WAVELENGTH_SETTLE_SECS {
            sleep(1.0);
            if i % 10 == 9 {
                print("    " + (i + 1) + " sec elapsed");
            }
        }
        
        let actual_wl = laser.get_wavelength();
        print("  Actual wavelength: " + actual_wl + "nm");
        print("");
        
        // Sweep LP, HWP, QWP
        let wl_data = [];
        
        for lp_idx in 0..n_lp {
            let lp_angle = lp_angles[lp_idx];
            rotator_lp.move_abs(lp_angle);
            rotator_lp.wait_settled();
            
            let lp_min = 1e10;
            let lp_max = -1e10;
            
            for hwp_idx in 0..n_hwp {
                let hwp_angle = hwp_angles[hwp_idx];
                rotator_hwp.move_abs(hwp_angle);
                rotator_hwp.wait_settled();
                
                for qwp_idx in 0..n_qwp {
                    let qwp_angle = qwp_angles[qwp_idx];
                    rotator_qwp.move_abs(qwp_angle);
                    rotator_qwp.wait_settled();
                    sleep(SETTLE_TIME);
                    
                    // Average multiple readings
                    let sum = 0.0;
                    for s in 0..SAMPLES_PER_POINT {
                        sum += power_meter.read();
                        sleep(0.02);
                    }
                    let power = sum / SAMPLES_PER_POINT;
                    let power_mw = power * 1000.0;
                    
                    wl_data.push([lp_angle, hwp_angle, qwp_angle, power]);
                    
                    if power_mw < lp_min { lp_min = power_mw; }
                    if power_mw > lp_max { lp_max = power_mw; }
                }
            }
            
            print("  LP " + lp_angle + "° (" + (lp_idx + 1) + "/" + n_lp + "): " + 
                  (n_hwp * n_qwp) + " pts, range: " + lp_min + " - " + lp_max + " mW");
        }
        
        collected.push([wavelength, wl_data]);
        print("");
    }
    
    collected
});

print("");
print("  [OK] Shutter closed safely");
print("");

// Home rotators
print("[6/7] Homing all rotators...");
rotator_lp.home();
sleep(1.0);
rotator_hwp.home();
sleep(1.0);
rotator_qwp.home();
sleep(1.0);
print("  [OK] All rotators homed");
print("");

// Save data to HDF5
print("[7/7] Saving data to HDF5...");

let min_power = 1e10;
let max_power = -1e10;

for wl_entry in all_data {
    let wavelength = wl_entry[0];
    let wl_data = wl_entry[1];
    
    // Convert to 2D array format for HDF5
    hdf5.write_array_2d("wl_" + wavelength.to_int() + "/data", wl_data);
    
    // Track min/max
    for point in wl_data {
        let power_mw = point[3] * 1000.0;
        if power_mw < min_power { min_power = power_mw; }
        if power_mw > max_power { max_power = power_mw; }
    }
}

// Write summary stats
hdf5.write_attr_f64("min_power_mW", min_power);
hdf5.write_attr_f64("max_power_mW", max_power);

hdf5.close();
print("  [OK] Data saved to: " + output_file);
print("");

// Summary
let dynamic_range = if min_power > 0.0 { max_power / min_power } else { 0.0 };
let dynamic_range_db = if dynamic_range > 0.0 { 10.0 * log10(dynamic_range) } else { 0.0 };

print("═══════════════════════════════════════════════════════════════");
print("  4D CALIBRATION COMPLETE");
print("═══════════════════════════════════════════════════════════════");
print("  Grid size:     " + n_wl + " wl × " + n_lp + " LP × " + n_hwp + " HWP × " + n_qwp + " QWP = " + total_points + " points");
print("  Min power:     " + min_power + " mW");
print("  Max power:     " + max_power + " mW");
print("  Dynamic range: " + dynamic_range + ":1 (" + dynamic_range_db + " dB)");
print("  Output file:   " + output_file);
print("");
print("Python analysis:");
print("  import h5py");
print("  import numpy as np");
print("  f = h5py.File('" + output_file + "', 'r')");
print("  # Data stored per wavelength: f['wl_800/data']");
print("═══════════════════════════════════════════════════════════════");

// Return summary
#{
    output_file: output_file,
    total_points: total_points,
    min_power_mW: min_power,
    max_power_mW: max_power,
    dynamic_range: dynamic_range
}
