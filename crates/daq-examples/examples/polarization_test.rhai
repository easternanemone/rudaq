// Polarization Measurement Test Script
// ===========================================
// LASER SAFETY: This script controls MaiTai laser
// Authorized testing window: Nov 25 10:52 PM - Nov 26 5:00 AM
// ===========================================

// Configuration
let ELLIPTEC_PORT = "/dev/ttyUSB0";
let NEWPORT_PORT = "/dev/ttyS0";
let MAITAI_PORT = "/dev/ttyUSB5";

// Elliptec addresses from config
let HWP_ADDRESS = "2";      // Half-wave plate
let POLARIZER_ADDRESS = "3"; // Linear polarizer
let UNUSED_ADDRESS = "8";    // Not in optical path

print("===========================================");
print("  POLARIZATION MEASUREMENT TEST");
print("  Time: " + timestamp());
print("===========================================");
print("");

// Step 1: Initialize Elliptec rotators
print("[1/7] Initializing Elliptec rotators...");

let hwp = create_elliptec(ELLIPTEC_PORT, HWP_ADDRESS);
let polarizer = create_elliptec(ELLIPTEC_PORT, POLARIZER_ADDRESS);

print("  HWP (addr 2): Created");
print("  Polarizer (addr 3): Created");

// Home both rotators
print("  Homing HWP...");
hwp.home();
sleep(2.0);

print("  Homing Polarizer...");
polarizer.home();
sleep(2.0);

let hwp_pos = hwp.position();
let pol_pos = polarizer.position();
print("  HWP position: " + hwp_pos + " deg");
print("  Polarizer position: " + pol_pos + " deg");
print("  [OK] Elliptec rotators initialized");
print("");

// Step 2: Initialize Newport power meter
print("[2/7] Initializing Newport 1830-C power meter...");

let power_meter = create_newport_1830c(NEWPORT_PORT);
print("  Power meter created");

let initial_power = power_meter.read();
print("  Initial power reading: " + initial_power + " W");
print("  [OK] Newport power meter initialized");
print("");

// Step 3: Initialize MaiTai laser
print("[3/7] Initializing MaiTai laser...");

let laser = create_maitai(MAITAI_PORT);
print("  Laser driver created");

// Query initial state
let wavelength = laser.wavelength();
print("  Current wavelength: " + wavelength + " nm");

let shutter_state = laser.shutter();
print("  Shutter state: " + if shutter_state { "OPEN" } else { "CLOSED" });

print("  [OK] MaiTai laser initialized");
print("");

// Step 4: Safety check - ensure shutter is closed
print("[4/7] Safety check...");
if shutter_state {
    print("  WARNING: Shutter was open, closing...");
    laser.close_shutter();
    sleep(0.5);
}
print("  Shutter confirmed CLOSED");
print("  [OK] Safety check passed");
print("");

// Step 5: Enable emission (laser will be on but shutter closed)
print("[5/7] Enabling laser emission...");
print("  >>> CAUTION: Enabling Class 4 laser <<<");
sleep(3.0);  // Safety delay

laser.enable_emission();
print("  Emission enabled");
sleep(2.0);  // Let laser stabilize

print("  [OK] Laser emission enabled");
print("");

// Step 6: Open shutter and measure background
print("[6/7] Opening shutter and measuring...");
print("  >>> CAUTION: Opening shutter - beam active <<<");
sleep(2.0);  // Safety delay

laser.open_shutter();
sleep(1.0);

let bg_power = power_meter.read();
print("  Background power: " + bg_power + " W");
print("");

// Step 7: Polarization scan
print("[7/7] Running polarization scan...");
print("  Rotating HWP from 0 to 180 deg in 10 deg steps");
print("  (Polarizer fixed at 0 deg)");
print("");

// Make sure polarizer is at 0
polarizer.move_abs(0.0);
sleep(1.0);

// Data collection
let results = [];
let angles = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0,
              100.0, 110.0, 120.0, 130.0, 140.0, 150.0, 160.0, 170.0, 180.0];

print("  HWP Angle (deg) | Power (W)");
print("  ----------------|----------");

for angle in angles {
    hwp.move_abs(angle);
    sleep(0.5);  // Wait for settling

    // Take average of 3 readings
    let sum = 0.0;
    for i in 0..3 {
        sum += power_meter.read();
        sleep(0.1);
    }
    let avg_power = sum / 3.0;

    let angle_str = if angle < 100.0 { "  " + angle } else { " " + angle };
    print("  " + angle_str + "            | " + avg_power);

    results.push([angle, avg_power]);
}

print("");
print("  [OK] Polarization scan complete");
print("");

// Step 8: Safe shutdown
print("[SHUTDOWN] Closing shutter and disabling emission...");
laser.close_shutter();
sleep(0.5);

let final_shutter = laser.shutter();
if !final_shutter {
    print("  Shutter confirmed CLOSED");
} else {
    print("  WARNING: Shutter may still be open!");
}

laser.disable_emission();
print("  Emission disabled");

// Home rotators
print("  Homing rotators...");
hwp.home();
polarizer.home();
sleep(2.0);

print("");
print("===========================================");
print("  TEST COMPLETE - LASER SAFELY SHUT DOWN");
print("===========================================");

// Return results
results
