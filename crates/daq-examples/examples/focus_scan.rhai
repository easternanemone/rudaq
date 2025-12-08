// Focus Scan Script
// ===========================================
// Performs a focus scan by moving the stage in Z
// and measuring the signal at each position.
// Useful for finding optimal focal plane.
//
// Equipment:
// - Motion stage (Z-axis)
// - Power meter or camera for signal
//
// ===========================================

// Configuration
let CENTER_Z = 0.0;        // Starting Z position (mm)
let SCAN_RANGE = 1.0;      // +/- range from center (mm)
let STEP_SIZE = 0.05;      // Step size (mm)
let SAMPLES_PER_POINT = 5; // Readings to average

print("==========================================");
print("  FOCUS SCAN");
print("  Center: " + CENTER_Z + " mm");
print("  Range: +/- " + SCAN_RANGE + " mm");
print("  Step: " + STEP_SIZE + " mm");
print("==========================================");
print("");

// Initialize mock hardware for demonstration
print("[1/3] Initializing hardware...");
let stage = create_mock_stage();
let meter = create_mock_power_meter(1.0);
print("  MockStage: Created");
print("  MockPowerMeter: Created");

// Calculate scan positions
let start_z = CENTER_Z - SCAN_RANGE;
let end_z = CENTER_Z + SCAN_RANGE;
let num_points = ((end_z - start_z) / STEP_SIZE).floor() + 1;

print("");
print("[2/3] Scanning " + num_points + " positions...");
print("");
print("  Z (mm)   | Signal (W) | Std Dev");
print("  ---------|------------|--------");

let results = [];
let z = start_z;

// Simulated Gaussian focus curve for demonstration
fn simulated_signal(z, center) {
    let sigma = 0.3;  // Focus depth
    let amplitude = 5.0;
    let offset = 0.5;
    amplitude * (-((z - center) * (z - center)) / (2.0 * sigma * sigma)).exp() + offset
}

while z <= end_z {
    // Move to Z position
    stage.move_abs(z);
    sleep(0.1);

    // Take multiple readings
    let readings = [];
    for i in 0..SAMPLES_PER_POINT {
        // In real usage: let signal = meter.read();
        // For demo, simulate a Gaussian focus curve
        let signal = simulated_signal(z, CENTER_Z);
        readings.push(signal);
        sleep(0.02);
    }

    // Calculate statistics
    let sum = 0.0;
    for r in readings { sum += r; }
    let mean = sum / SAMPLES_PER_POINT;

    let var_sum = 0.0;
    for r in readings {
        let diff = r - mean;
        var_sum += diff * diff;
    }
    let std_dev = (var_sum / SAMPLES_PER_POINT).sqrt();

    // Format and print
    let z_str = if z >= 0.0 { " " + z } else { z };
    print("  " + z_str + "    | " + mean + " | " + std_dev);

    results.push([z, mean, std_dev]);
    z += STEP_SIZE;
}

// Find peak (best focus)
print("");
print("[3/3] Analyzing results...");

let max_signal = 0.0;
let best_z = CENTER_Z;

for point in results {
    if point[1] > max_signal {
        max_signal = point[1];
        best_z = point[0];
    }
}

print("  Peak signal: " + max_signal + " W");
print("  Best focus Z: " + best_z + " mm");

// Move to best focus
print("");
print("[RESULT] Moving to best focus position...");
stage.move_abs(best_z);
sleep(0.2);

print("");
print("==========================================");
print("  FOCUS SCAN COMPLETE");
print("  Optimal Z: " + best_z + " mm");
print("  Peak Signal: " + max_signal + " W");
print("==========================================");

// Return results including best focus
#{
    scan_data: results,
    best_focus_z: best_z,
    peak_signal: max_signal
}
