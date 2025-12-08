// Angular Power Measurement Script
// ===========================================
// Measures optical power at multiple HWP rotation angles
// to characterize polarization-dependent transmission.
//
// Equipment:
// - Newport 1830-C power meter (or compatible)
// - Elliptec ELL14 half-wave plate rotator
//
// No laser safety concerns - passive measurement only.
// ===========================================

// Configuration
let NEWPORT_PORT = "/dev/ttyS0";
let ELLIPTEC_PORT = "/dev/ttyUSB0";
let HWP_ADDRESS = "2";

// Scan parameters
let START_ANGLE = 0.0;
let END_ANGLE = 180.0;
let STEP_SIZE = 5.0;  // degrees
let SAMPLES_PER_ANGLE = 3;  // averaging

print("==========================================");
print("  ANGULAR POWER MEASUREMENT");
print("  Scan: " + START_ANGLE + " to " + END_ANGLE + " deg");
print("  Step: " + STEP_SIZE + " deg");
print("==========================================");
print("");

// Initialize instruments
print("[1/3] Initializing instruments...");

let power_meter = create_newport_1830c(NEWPORT_PORT);
print("  Newport 1830-C: Connected");

let hwp = create_elliptec(ELLIPTEC_PORT, HWP_ADDRESS);
print("  ELL14 HWP (addr " + HWP_ADDRESS + "): Connected");

// Home the rotator
print("");
print("[2/3] Homing HWP...");
hwp.home();
sleep(2.0);
let home_pos = hwp.position();
print("  Home position: " + home_pos + " deg");

// Perform scan
print("");
print("[3/3] Running angular scan...");
print("");
print("  Angle (deg) | Power (W)     | Std Dev");
print("  ------------|---------------|----------");

let results = [];
let angle = START_ANGLE;

while angle <= END_ANGLE {
    // Move to angle
    hwp.move_abs(angle);
    sleep(0.3);  // settling time

    // Take multiple readings
    let readings = [];
    for i in 0..SAMPLES_PER_ANGLE {
        let p = power_meter.read();
        readings.push(p);
        sleep(0.05);
    }

    // Calculate mean and std dev
    let sum = 0.0;
    for r in readings {
        sum += r;
    }
    let mean = sum / SAMPLES_PER_ANGLE;

    let var_sum = 0.0;
    for r in readings {
        let diff = r - mean;
        var_sum += diff * diff;
    }
    let std_dev = (var_sum / SAMPLES_PER_ANGLE).sqrt();

    // Format output
    let angle_str = angle;
    if angle < 100.0 { angle_str = " " + angle; }
    if angle < 10.0 { angle_str = "  " + angle; }

    print("  " + angle_str + "         | " + mean + " | " + std_dev);

    results.push([angle, mean, std_dev]);
    angle += STEP_SIZE;
}

// Return to home
print("");
print("[CLEANUP] Returning to home position...");
hwp.home();
sleep(1.0);

print("");
print("==========================================");
print("  SCAN COMPLETE");
print("  " + results.len() + " angles measured");
print("==========================================");

// Return results for post-processing
results
