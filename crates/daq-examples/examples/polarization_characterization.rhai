// Polarization Optical Element Characterization Script
// =====================================================
// Purpose: Identify which rotator is HWP, QWP, or Linear Polarizer
// by analyzing the power response vs rotation angle.
//
// Theory:
// - Linear Polarizer: cos^2(theta) -> 2 peaks per 360 degrees (180° period)
// - Half-Wave Plate: cos^2(2*theta) -> 4 peaks per 360 degrees (90° period)
// - Quarter-Wave Plate: Complex elliptical pattern -> 2 peaks, phase shifted
//
// Safety: Uses with_shutter_open() to GUARANTEE shutter closure on exit/error
//
// User confirmed: Linear polarizer is directly in front of power meter

// Configuration - use stable by-id paths that don't change on reboot
let ELLIPTEC_PORT = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";
let NEWPORT_PORT = "/dev/ttyS0";  // Built-in RS-232, always stable
let MAITAI_PORT = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";

let START_ANGLE = 0.0;
let END_ANGLE = 360.0;
let STEP_SIZE = 5.0;
let NUM_STEPS = ((END_ANGLE - START_ANGLE) / STEP_SIZE).to_int() + 1;  // 73 steps (0 to 360 inclusive)
let SAMPLES_PER_POINT = 3;
let SETTLE_TIME = 0.5;

print("=====================================================");
print("  POLARIZATION ELEMENT CHARACTERIZATION");
print("  Time: " + timestamp());
print("=====================================================");
print("");
print("This experiment scans each rotator independently to");
print("identify HWP, QWP, or Linear Polarizer by analyzing");
print("the angular frequency of the power response.");
print("");
print("Configuration:");
print("  Step size: " + STEP_SIZE + " degrees");
print("  Total steps per rotator: " + NUM_STEPS);
print("  Samples per point: " + SAMPLES_PER_POINT);
print("");

// Step 1: Initialize hardware
print("[1/7] Initializing hardware...");
let power_meter = create_newport_1830c(NEWPORT_PORT);
let laser = create_maitai(MAITAI_PORT);
let rotator_2 = create_elliptec(ELLIPTEC_PORT, "2");
let rotator_3 = create_elliptec(ELLIPTEC_PORT, "3");
let rotator_8 = create_elliptec(ELLIPTEC_PORT, "8");
print("  [OK] All devices initialized");
print("");

// Step 2: Zero power meter with shutter closed
print("[2/7] Zeroing power meter (shutter must be closed)...");
laser.close();
sleep(0.5);
if laser.is_open() {
    print("  WARNING: Shutter still open, waiting...");
    sleep(1.0);
}
power_meter.zero();
sleep(0.5);
let zero_check = power_meter.read();
print("  Zero check reading: " + zero_check + " W");
print("  [OK] Power meter zeroed");
print("");

// Step 3: Home all rotators
print("[3/7] Homing all rotators...");
rotator_2.home();
sleep(1.5);
rotator_3.home();
sleep(1.5);
rotator_8.home();
sleep(1.5);
print("  Positions: R2=" + rotator_2.position() + " deg");
print("             R3=" + rotator_3.position() + " deg");
print("             R8=" + rotator_8.position() + " deg");
print("  [OK] All rotators homed at 0 degrees");
print("");

// Step 4: Prepare HDF5 file for data storage
let output_file = "polarization_characterization_" + timestamp() + ".h5";
print("[4/7] Creating HDF5 output file: " + output_file);
let hdf5 = create_hdf5(output_file);
hdf5.write_attr("experiment", "polarization_element_characterization");
hdf5.write_attr("timestamp", timestamp_iso());
hdf5.write_attr_f64("step_size_deg", STEP_SIZE);
hdf5.write_attr_i64("samples_per_point", SAMPLES_PER_POINT);
hdf5.write_attr_f64("settle_time_s", SETTLE_TIME);
hdf5.create_group("rotator_2");
hdf5.create_group("rotator_3");
hdf5.create_group("rotator_8");
print("  [OK] HDF5 file created");
print("");

// Generate angle list
let angles = [];
for i in 0..NUM_STEPS {
    angles.push(i * STEP_SIZE);
}

// Function to analyze scan results and identify element type
fn analyze_element(results, name) {
    // Find min/max power
    let min_power = 1e10;
    let max_power = -1e10;
    let min_angle = 0.0;
    let max_angle = 0.0;

    for point in results {
        let angle = point[0];
        let power = point[1];
        if power < min_power {
            min_power = power;
            min_angle = angle;
        }
        if power > max_power {
            max_power = power;
            max_angle = angle;
        }
    }

    // Calculate contrast
    let contrast = if max_power + min_power > 0.0 {
        (max_power - min_power) / (max_power + min_power)
    } else {
        0.0
    };

    // Count peaks using threshold crossing (rising edge)
    let threshold = (max_power + min_power) / 2.0;
    let peak_count = 0;
    let was_below = true;  // Start assuming below threshold

    for point in results {
        let is_above = point[1] > threshold;
        if is_above && was_below {
            peak_count += 1;
        }
        was_below = !is_above;
    }

    // Identify element based on peaks and contrast
    let element = if peak_count >= 4 {
        "HWP (Half-Wave Plate)"
    } else if peak_count == 2 && contrast > 0.7 {
        "Linear Polarizer"
    } else if peak_count == 2 {
        "QWP (Quarter-Wave Plate)"
    } else if peak_count == 1 {
        "Possible QWP or misaligned element"
    } else {
        "Unknown (peaks=" + peak_count + ")"
    };

    print("");
    print("  " + name + " Analysis:");
    print("    Min: " + min_power + " W at " + min_angle + " deg");
    print("    Max: " + max_power + " W at " + max_angle + " deg");
    print("    Contrast: " + (contrast * 100.0) + " %");
    print("    Peaks detected: " + peak_count);
    print("    --> Identified as: " + element);

    // Return analysis results as map
    #{
        name: name,
        element: element,
        min_power: min_power,
        max_power: max_power,
        min_angle: min_angle,
        max_angle: max_angle,
        contrast: contrast,
        peak_count: peak_count
    }
}

// Step 5: Run characterization inside shutter safety wrapper
print("[5/7] Opening shutter and running characterization...");
print("  SAFETY: Using with_shutter_open() - shutter will close on any error");
print("");

let all_results = with_shutter_open(laser, || {
    let initial_power = power_meter.read();
    print("  Shutter open, initial power: " + initial_power + " W");
    print("");

    // -----------------------------------------------------------------
    // Scan Rotator 2 (others at 0 degrees)
    // -----------------------------------------------------------------
    print("  --- Scanning Rotator 2 (R3=0, R8=0) ---");
    rotator_3.move_abs(0.0);
    rotator_8.move_abs(0.0);
    sleep(0.5);

    let results_2 = [];
    for i in 0..NUM_STEPS {
        let angle = i * STEP_SIZE;
        rotator_2.move_abs(angle);
        rotator_2.wait_settled();
        sleep(SETTLE_TIME);

        // Average multiple readings
        let sum = 0.0;
        for s in 0..SAMPLES_PER_POINT {
            sum += power_meter.read();
            sleep(0.05);
        }
        let power = sum / SAMPLES_PER_POINT;
        results_2.push([angle, power]);

        if i % 18 == 0 {  // Every 90 degrees
            print("    R2=" + angle + " deg: " + power + " W");
        }
    }
    rotator_2.move_abs(0.0);
    sleep(0.5);

    // -----------------------------------------------------------------
    // Scan Rotator 3 (others at 0 degrees)
    // -----------------------------------------------------------------
    print("");
    print("  --- Scanning Rotator 3 (R2=0, R8=0) ---");
    rotator_2.move_abs(0.0);
    rotator_8.move_abs(0.0);
    sleep(0.5);

    let results_3 = [];
    for i in 0..NUM_STEPS {
        let angle = i * STEP_SIZE;
        rotator_3.move_abs(angle);
        rotator_3.wait_settled();
        sleep(SETTLE_TIME);

        let sum = 0.0;
        for s in 0..SAMPLES_PER_POINT {
            sum += power_meter.read();
            sleep(0.05);
        }
        let power = sum / SAMPLES_PER_POINT;
        results_3.push([angle, power]);

        if i % 18 == 0 {
            print("    R3=" + angle + " deg: " + power + " W");
        }
    }
    rotator_3.move_abs(0.0);
    sleep(0.5);

    // -----------------------------------------------------------------
    // Scan Rotator 8 (others at 0 degrees)
    // -----------------------------------------------------------------
    print("");
    print("  --- Scanning Rotator 8 (R2=0, R3=0) ---");
    rotator_2.move_abs(0.0);
    rotator_3.move_abs(0.0);
    sleep(0.5);

    let results_8 = [];
    for i in 0..NUM_STEPS {
        let angle = i * STEP_SIZE;
        rotator_8.move_abs(angle);
        rotator_8.wait_settled();
        sleep(SETTLE_TIME);

        let sum = 0.0;
        for s in 0..SAMPLES_PER_POINT {
            sum += power_meter.read();
            sleep(0.05);
        }
        let power = sum / SAMPLES_PER_POINT;
        results_8.push([angle, power]);

        if i % 18 == 0 {
            print("    R8=" + angle + " deg: " + power + " W");
        }
    }
    rotator_8.move_abs(0.0);
    sleep(0.5);

    print("");
    print("  [OK] All characterization scans complete");

    // Return results from closure
    [results_2, results_3, results_8]
});

// Shutter is now GUARANTEED closed by with_shutter_open()
print("");
print("  [OK] Shutter closed safely");
print("");

// Step 6: Home all rotators
print("[6/7] Homing all rotators...");
rotator_2.home();
sleep(1.5);
rotator_3.home();
sleep(1.5);
rotator_8.home();
sleep(1.5);
print("  [OK] All rotators homed");
print("");

// Step 7: Analyze and save results
print("[7/7] Analyzing results and saving to HDF5...");

let analysis_2 = analyze_element(all_results[0], "Rotator 2");
let analysis_3 = analyze_element(all_results[1], "Rotator 3");
let analysis_8 = analyze_element(all_results[2], "Rotator 8");

// Save raw data to HDF5
print("");
print("  Saving data to HDF5...");
hdf5.write_array_2d("rotator_2/data", all_results[0]);
hdf5.write_array_2d("rotator_3/data", all_results[1]);
hdf5.write_array_2d("rotator_8/data", all_results[2]);

// Save analysis metadata
hdf5.write_attr("rotator_2/element", analysis_2.element);
hdf5.write_attr_f64("rotator_2/contrast", analysis_2.contrast);
hdf5.write_attr_i64("rotator_2/peak_count", analysis_2.peak_count);
hdf5.write_attr_f64("rotator_2/min_power_W", analysis_2.min_power);
hdf5.write_attr_f64("rotator_2/max_power_W", analysis_2.max_power);

hdf5.write_attr("rotator_3/element", analysis_3.element);
hdf5.write_attr_f64("rotator_3/contrast", analysis_3.contrast);
hdf5.write_attr_i64("rotator_3/peak_count", analysis_3.peak_count);
hdf5.write_attr_f64("rotator_3/min_power_W", analysis_3.min_power);
hdf5.write_attr_f64("rotator_3/max_power_W", analysis_3.max_power);

hdf5.write_attr("rotator_8/element", analysis_8.element);
hdf5.write_attr_f64("rotator_8/contrast", analysis_8.contrast);
hdf5.write_attr_i64("rotator_8/peak_count", analysis_8.peak_count);
hdf5.write_attr_f64("rotator_8/min_power_W", analysis_8.min_power);
hdf5.write_attr_f64("rotator_8/max_power_W", analysis_8.max_power);

hdf5.close();
print("  [OK] Data saved to: " + output_file);
print("");

// Summary
print("=====================================================");
print("  ELEMENT IDENTIFICATION SUMMARY");
print("=====================================================");
print("");
print("  Rotator 2: " + analysis_2.element);
print("             Contrast=" + (analysis_2.contrast * 100.0) + "%, Peaks=" + analysis_2.peak_count);
print("");
print("  Rotator 3: " + analysis_3.element);
print("             Contrast=" + (analysis_3.contrast * 100.0) + "%, Peaks=" + analysis_3.peak_count);
print("");
print("  Rotator 8: " + analysis_8.element);
print("             Contrast=" + (analysis_8.contrast * 100.0) + "%, Peaks=" + analysis_8.peak_count);
print("");
print("  Output file: " + output_file);
print("");
print("  Identification guide:");
print("  - 4 peaks (90 deg period): Half-Wave Plate (HWP)");
print("  - 2 peaks + high contrast: Linear Polarizer");
print("  - 2 peaks + lower contrast: Quarter-Wave Plate (QWP)");
print("=====================================================");

// Return analysis for programmatic use
#{
    rotator_2: analysis_2,
    rotator_3: analysis_3,
    rotator_8: analysis_8,
    output_file: output_file
}
