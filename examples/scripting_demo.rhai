// Example Rhai script demonstrating hardware control
// This script controls a stage and camera to perform a simple scan

print("=== Starting Hardware Scan Demo ===");

// Setup camera
print("Arming camera...");
camera.arm();

let res = camera.resolution();
print("Camera resolution: " + res[0] + "x" + res[1]);

// Perform scan
print("Starting scan from 0 to 10mm in 1mm steps...");

for i in 0..11 {
    let target_pos = i * 1.0;

    print("Step " + i + ": Moving to " + target_pos + "mm");
    stage.move_abs(target_pos);
    stage.wait_settled();

    let actual_pos = stage.position();
    print("  Position confirmed: " + actual_pos + "mm");

    print("  Triggering camera...");
    camera.trigger();
}

// Return home
print("Returning to home position...");
stage.move_abs(0.0);
stage.wait_settled();

let final_pos = stage.position();
print("=== Scan Complete ===");
print("Final position: " + final_pos + "mm");
print("Total frames captured: 11");

final_pos
