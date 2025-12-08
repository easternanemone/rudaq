// Polarization Optical Element Characterization Script
// =====================================================
// Purpose: Identify which rotator is HWP, QWP, or Linear Polarizer
// by analyzing the power response vs rotation angle.
//
// Theory:
// - Linear Polarizer: cos^2(theta) -> 2 peaks per 360 degrees
// - Half-Wave Plate: cos^2(2*theta) -> 4 peaks per 360 degrees
// - Quarter-Wave Plate: Complex elliptical pattern -> 2 peaks, phase shifted
//
// User confirmed: Linear polarizer is directly in front of power meter

print("=====================================================");
print("  POLARIZATION ELEMENT CHARACTERIZATION");
print("  Time: " + timestamp());
print("=====================================================");
print("");
print("This experiment will scan each rotator independently");
print("to identify which element is which based on the");
print("angular frequency of the power response.");
print("");

// Configuration
let ELLIPTEC_PORT = "/dev/ttyUSB0";
let NEWPORT_PORT = "/dev/ttyS0";

// All three Elliptec addresses to characterize
let ADDRESSES = ["2", "3", "8"];
let ROTATOR_NAMES = ["rotator_2", "rotator_3", "rotator_8"];

// Step 1: Initialize power meter
print("[1/4] Initializing Newport 1830-C power meter...");
let power_meter = create_newport_1830c(NEWPORT_PORT);
let initial_power = power_meter.read();
print("  Initial power reading: " + initial_power + " W");
print("  [OK] Power meter initialized");
print("");

// Step 2: Initialize all rotators
print("[2/4] Initializing all Elliptec rotators...");
let rotator_2 = create_elliptec(ELLIPTEC_PORT, "2");
let rotator_3 = create_elliptec(ELLIPTEC_PORT, "3");
let rotator_8 = create_elliptec(ELLIPTEC_PORT, "8");

print("  Homing all rotators...");
rotator_2.home();
sleep(1.5);
rotator_3.home();
sleep(1.5);
rotator_8.home();
sleep(1.5);

print("  Positions: R2=" + rotator_2.position() + " deg, R3=" + rotator_3.position() + " deg, R8=" + rotator_8.position() + " deg");
print("  [OK] All rotators initialized and homed");
print("");

// Step 3: Characterization scans (no laser - using ambient light or pre-existing beam)
print("[3/4] Running characterization scans...");
print("  NOTE: Using existing beam condition");
print("  (Laser control bypassed for initial characterization)");
print("");

// Scan parameters
let STEP_SIZE = 15.0;  // degrees
let NUM_STEPS = 24;    // 0 to 360 degrees
let SETTLE_TIME = 0.5; // seconds after each move
let NUM_SAMPLES = 3;   // readings to average

// Results storage
let all_results = [];

// Function to scan a single rotator
fn scan_rotator(rotator, rotator_name, power_meter, angles, settle_time, num_samples) {
    let results = [];
    print("  Scanning " + rotator_name + "...");

    for angle in angles {
        rotator.move_abs(angle);
        sleep(settle_time);

        // Average multiple readings
        let sum = 0.0;
        for i in 0..num_samples {
            sum += power_meter.read();
            sleep(0.05);
        }
        let avg_power = sum / num_samples;
        results.push([angle, avg_power]);
    }

    results
}

// Generate angle list
let angles = [];
for i in 0..NUM_STEPS + 1 {
    angles.push(i * STEP_SIZE);
}

// Scan rotator 2 (fix others at 0)
print("  --- Scanning Rotator 2 (R3=0, R8=0) ---");
rotator_3.move_abs(0.0);
rotator_8.move_abs(0.0);
sleep(1.0);

let results_2 = [];
for angle in angles {
    rotator_2.move_abs(angle);
    sleep(SETTLE_TIME);
    let sum = 0.0;
    for i in 0..NUM_SAMPLES {
        sum += power_meter.read();
        sleep(0.05);
    }
    let avg = sum / NUM_SAMPLES;
    results_2.push([angle, avg]);
    print("    R2=" + angle + " deg: " + avg + " W");
}
all_results.push(["rotator_2", results_2]);

// Return rotator 2 to 0, scan rotator 3
print("");
print("  --- Scanning Rotator 3 (R2=0, R8=0) ---");
rotator_2.move_abs(0.0);
rotator_8.move_abs(0.0);
sleep(1.0);

let results_3 = [];
for angle in angles {
    rotator_3.move_abs(angle);
    sleep(SETTLE_TIME);
    let sum = 0.0;
    for i in 0..NUM_SAMPLES {
        sum += power_meter.read();
        sleep(0.05);
    }
    let avg = sum / NUM_SAMPLES;
    results_3.push([angle, avg]);
    print("    R3=" + angle + " deg: " + avg + " W");
}
all_results.push(["rotator_3", results_3]);

// Return rotator 3 to 0, scan rotator 8
print("");
print("  --- Scanning Rotator 8 (R2=0, R3=0) ---");
rotator_2.move_abs(0.0);
rotator_3.move_abs(0.0);
sleep(1.0);

let results_8 = [];
for angle in angles {
    rotator_8.move_abs(angle);
    sleep(SETTLE_TIME);
    let sum = 0.0;
    for i in 0..NUM_SAMPLES {
        sum += power_meter.read();
        sleep(0.05);
    }
    let avg = sum / NUM_SAMPLES;
    results_8.push([angle, avg]);
    print("    R8=" + angle + " deg: " + avg + " W");
}
all_results.push(["rotator_8", results_8]);

print("");
print("  [OK] All characterization scans complete");
print("");

// Step 4: Analysis (simplified - look at variations)
print("[4/4] Analyzing results...");
print("");

// Helper to find min/max and count peaks
fn analyze_scan(results, name) {
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

    let contrast = (max_power - min_power) / (max_power + min_power);

    print("  " + name + ":");
    print("    Min: " + min_power + " W at " + min_angle + " deg");
    print("    Max: " + max_power + " W at " + max_angle + " deg");
    print("    Contrast: " + (contrast * 100.0) + " %");
    print("");

    [min_power, max_power, min_angle, max_angle, contrast]
}

let analysis_2 = analyze_scan(results_2, "Rotator 2");
let analysis_3 = analyze_scan(results_3, "Rotator 3");
let analysis_8 = analyze_scan(results_8, "Rotator 8");

// Identification logic
print("  Identification hints:");
print("  - Highest contrast element closest to detector = Linear Polarizer");
print("  - 4 peaks/360deg (45deg period) = Half-Wave Plate");
print("  - 2 peaks/360deg (90deg period) = Linear Polarizer or QWP");
print("");

// Return all rotators to home
print("[CLEANUP] Homing all rotators...");
rotator_2.home();
sleep(1.5);
rotator_3.home();
sleep(1.5);
rotator_8.home();
sleep(1.5);

print("");
print("=====================================================");
print("  CHARACTERIZATION COMPLETE");
print("  Examine the data to identify:");
print("  - Which rotator shows 4 peaks/360deg (HWP)");
print("  - Which rotator shows highest contrast (Polarizer)");
print("  - Which rotator shows intermediate behavior (QWP)");
print("=====================================================");

// Return all results for external analysis
all_results
