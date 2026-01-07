// Demo Scan Script for rust-daq
//
// This script demonstrates a basic linear scan with position-based measurements.
// It uses the mock stage to sweep through positions and simulates power readings.
//
// Usage:
//   cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai
//
// NOTE: Power meter integration requires ReadableHandle (planned for v0.6.0).
// This demo simulates power readings based on position for demonstration purposes.

print("╔════════════════════════════════════════╗");
print("║  rust-daq Demo: Automated Scan        ║");
print("╚════════════════════════════════════════╝");
print("");

// Configuration
let scan_start = 0.0;       // mm
let scan_end = 10.0;        // mm
let num_points = 11;
let step_size = (scan_end - scan_start) / (num_points - 1);

print(`Scan Parameters:`);
print(`  Range: ${scan_start} to ${scan_end} mm`);
print(`  Points: ${num_points}`);
print(`  Step: ${step_size} mm`);
print("");

// Perform scan
print("Starting scan...");
print("─────────────────────────────────────────");

// Simulate a Gaussian beam profile centered at 5mm
let beam_center = 5.0;
let beam_width = 2.0;

for i in 0..num_points {
    let position = scan_start + (i * step_size);

    // Move stage to position
    stage.move_abs(position);

    // Wait for stage to settle
    sleep(0.05);

    // Simulate power reading (Gaussian profile + noise)
    // In v0.6.0, this will be: let power = power_meter.read();
    let dist = position - beam_center;
    let power = 1.0e-3 * (1.0 - (dist * dist) / (beam_width * beam_width));
    if power < 0.0 { power = 0.0; }

    print(`[${i+1}/${num_points}] Position: ${position} mm -> Power: ${power} W`);

    sleep(0.1);
}

print("─────────────────────────────────────────");
print("Scan complete!");
print("");
print("NOTE: Power readings are simulated (Gaussian profile).");
print("Real power meter integration coming in v0.6.0.");
