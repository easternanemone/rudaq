# Scripting Examples and Common Patterns

Collection of example scripts and common patterns for rust-daq experiment control.

## Table of Contents
- [Basic Examples](#basic-examples)
- [Stage Control Patterns](#stage-control-patterns)
- [Camera Acquisition](#camera-acquisition)
- [Combined Stage + Camera](#combined-stage--camera)
- [Advanced Patterns](#advanced-patterns)
- [Troubleshooting](#troubleshooting)

---

## Basic Examples

### Hello World
```rhai
// hello_world.rhai
print("Hello from rust-daq!");
```

Run with:
```bash
cargo run -- run examples/hello_world.rhai
```

### Simple Math
```rhai
// Calculate positions
let start = 0.0;
let end = 10.0;
let steps = 5;

let step_size = (end - start) / steps;
print(`Step size: ${step_size}mm`);

for i in 0..steps {
    let pos = start + (i * step_size);
    print(`Position ${i}: ${pos}mm`);
}
```

Output:
```
Step size: 2mm
Position 0: 0mm
Position 1: 2mm
Position 2: 4mm
Position 3: 6mm
Position 4: 8mm
```

---

## Stage Control Patterns

### Linear Scan
```rhai
// linear_scan.rhai
// Scan stage from start to end position

let start = 0.0;
let end = 20.0;
let step = 1.0;

print("Starting linear scan...");

let pos = start;
while pos <= end {
    stage.move_abs(pos);
    stage.wait_settled();

    let actual = stage.position();
    print(`Position: ${actual}mm`);

    sleep(0.1);  // Measurement time

    pos += step;
}

print("Scan complete!");
```

### Bidirectional Scan
```rhai
// bidirectional_scan.rhai
// Scan forward then backward

let positions = [0.0, 5.0, 10.0, 15.0, 20.0];

// Forward scan
print("Forward scan...");
for pos in positions {
    stage.move_abs(pos);
    stage.wait_settled();
    print(`→ ${pos}mm`);
}

// Reverse array for backward scan
print("Backward scan...");
let i = positions.len() - 1;
while i >= 0 {
    let pos = positions[i];
    stage.move_abs(pos);
    stage.wait_settled();
    print(`← ${pos}mm`);
    i -= 1;
}
```

### Stepped Positioning
```rhai
// stepped_positioning.rhai
// Move to predefined positions

fn visit_positions(stage, positions) {
    for i in 0..positions.len() {
        let target = positions[i];
        print(`Moving to position ${i+1}/${positions.len()}: ${target}mm`);

        stage.move_abs(target);
        stage.wait_settled();

        // Verify position
        let actual = stage.position();
        let error = target - actual;

        if error > 0.01 {
            print(`  Warning: Position error ${error}mm`);
        } else {
            print(`  ✓ Reached target`);
        }

        sleep(0.5);  // Dwell time
    }
}

// Define positions
let calibration_points = [0.0, 10.0, 20.0, 10.0, 0.0];
visit_positions(stage, calibration_points);
```

### Spiral Pattern
```rhai
// spiral_pattern.rhai
// Generate spiral positions (requires 2D stage - future)

fn calculate_spiral_point(angle, radius) {
    let x = radius * angle.cos();
    let y = radius * angle.sin();
    return [x, y];
}

let max_radius = 10.0;
let turns = 3.0;
let points = 50;

for i in 0..points {
    let progress = i / points;
    let angle = progress * turns * 2.0 * 3.14159;
    let radius = progress * max_radius;

    let point = calculate_spiral_point(angle, radius);
    print(`Point ${i}: (${point[0]}, ${point[1]})`);

    // For 1D stage, just use X
    // stage.move_abs(point[0]);
    // stage.wait_settled();
}
```

---

## Camera Acquisition

### Single Frame Capture
```rhai
// single_frame.rhai
// Capture one frame

print("Preparing camera...");
camera.arm();

print("Capturing frame...");
camera.trigger();

let res = camera.resolution();
print(`Captured ${res[0]}x${res[1]} frame`);
```

### Time Series
```rhai
// time_series.rhai
// Capture frames at regular intervals

let num_frames = 10;
let interval = 0.5;  // seconds

print(`Capturing ${num_frames} frames at ${interval}s intervals`);

camera.arm();

for i in 0..num_frames {
    print(`Frame ${i+1}/${num_frames}...`);
    camera.trigger();
    sleep(interval);
}

print("Time series complete!");
```

### Burst Acquisition
```rhai
// burst_acquisition.rhai
// Capture frames as fast as possible

let burst_size = 20;

print(`Capturing burst of ${burst_size} frames...`);

let start_time = 0.0;  // Would need actual timestamp
camera.arm();

for i in 0..burst_size {
    camera.trigger();
    // No sleep - acquire as fast as hardware allows
}

print(`Burst complete: ${burst_size} frames`);
```

---

## Combined Stage + Camera

### Triggered Acquisition
```rhai
// triggered_acquisition.rhai
// Move to position, wait, trigger camera

let positions = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0];

print("Setting up triggered acquisition...");
camera.arm();

for i in 0..positions.len() {
    let target = positions[i];
    print(`Position ${i+1}: ${target}mm`);

    // Move to position
    stage.move_abs(target);
    stage.wait_settled();

    // Wait for vibrations to settle
    sleep(0.1);

    // Capture frame
    camera.trigger();
    print(`  ✓ Frame captured`);
}

print("Acquisition sequence complete!");
```

### Z-Stack Acquisition
```rhai
// z_stack.rhai
// Acquire 3D volume by scanning in Z

fn acquire_z_stack(start_z, end_z, step_z) {
    let num_slices = ((end_z - start_z) / step_z) + 1;
    print(`Acquiring ${num_slices} slices from ${start_z} to ${end_z}mm`);

    camera.arm();

    let z = start_z;
    let slice = 0;

    while z <= end_z {
        print(`  Slice ${slice}: z=${z}mm`);

        stage.move_abs(z);
        stage.wait_settled();
        sleep(0.05);  // Settle time

        camera.trigger();

        z += step_z;
        slice += 1;
    }

    print(`Z-stack complete: ${slice} slices`);
}

// Acquire 20µm stack with 1µm steps
acquire_z_stack(0.0, 0.020, 0.001);
```

### Grid Scan
```rhai
// grid_scan.rhai
// 2D grid acquisition (conceptual - needs XY stage)

fn grid_scan(x_start, x_end, x_step, y_start, y_end, y_step) {
    let x_points = ((x_end - x_start) / x_step) + 1;
    let y_points = ((y_end - y_start) / y_step) + 1;
    let total = x_points * y_points;

    print(`Grid scan: ${x_points}x${y_points} = ${total} points`);

    camera.arm();
    let count = 0;

    let y = y_start;
    while y <= y_end {
        let x = x_start;
        while x <= x_end {
            print(`  Point ${count+1}/${total}: (${x}, ${y})`);

            // For 1D stage, just use X
            stage.move_abs(x);
            stage.wait_settled();

            camera.trigger();

            x += x_step;
            count += 1;
        }
        y += y_step;
    }

    print("Grid scan complete!");
}

grid_scan(0.0, 10.0, 1.0, 0.0, 10.0, 1.0);
```

### Drift Correction
```rhai
// drift_correction.rhai
// Return to reference position periodically

let measurement_positions = [0.0, 5.0, 10.0, 15.0, 20.0];
let reference_position = 0.0;
let drift_check_interval = 3;  // Check every 3 measurements

camera.arm();

for i in 0..measurement_positions.len() {
    let pos = measurement_positions[i];

    // Regular measurement
    stage.move_abs(pos);
    stage.wait_settled();
    camera.trigger();
    print(`Measurement ${i+1}: ${pos}mm`);

    // Periodic drift check
    if (i + 1) % drift_check_interval == 0 {
        print("  → Drift check...");
        stage.move_abs(reference_position);
        stage.wait_settled();
        camera.trigger();
        print("  → Returning to sequence");
    }
}
```

---

## Advanced Patterns

### Adaptive Scanning
```rhai
// adaptive_scan.rhai
// Adjust step size based on conditions (simulated)

fn needs_fine_scan(position) {
    // Placeholder - would check actual measurement data
    return position > 5.0 && position < 15.0;
}

let start = 0.0;
let end = 20.0;
let coarse_step = 2.0;
let fine_step = 0.5;

camera.arm();

let pos = start;
while pos <= end {
    stage.move_abs(pos);
    stage.wait_settled();
    camera.trigger();

    // Adaptive step size
    if needs_fine_scan(pos) {
        print(`Fine scan at ${pos}mm (step: ${fine_step}mm)`);
        pos += fine_step;
    } else {
        print(`Coarse scan at ${pos}mm (step: ${coarse_step}mm)`);
        pos += coarse_step;
    }
}
```

### Multi-Pass Acquisition
```rhai
// multi_pass.rhai
// Multiple passes over same positions

fn run_pass(positions, pass_number) {
    print(`Starting pass ${pass_number}...`);

    camera.arm();

    for i in 0..positions.len() {
        stage.move_abs(positions[i]);
        stage.wait_settled();
        camera.trigger();
    }

    print(`Pass ${pass_number} complete`);
}

let positions = [0.0, 5.0, 10.0, 15.0, 20.0];
let num_passes = 3;

for pass in 1..=num_passes {
    run_pass(positions, pass);

    if pass < num_passes {
        print("Waiting before next pass...");
        sleep(5.0);  // Inter-pass delay
    }
}

print("Multi-pass acquisition complete!");
```

### Error Recovery
```rhai
// error_recovery.rhai
// Robust acquisition with position verification

fn safe_move_and_verify(stage, target, tolerance) {
    stage.move_abs(target);
    stage.wait_settled();

    let actual = stage.position();
    let error = (target - actual).abs();

    if error > tolerance {
        print(`  Warning: Position error ${error}mm (tolerance: ${tolerance}mm)`);
        // Could retry or abort here
        return false;
    }

    return true;
}

let positions = [0.0, 5.0, 10.0, 15.0, 20.0];
let tolerance = 0.01;  // 10µm

camera.arm();

for i in 0..positions.len() {
    let target = positions[i];
    print(`Position ${i+1}: ${target}mm`);

    let success = safe_move_and_verify(stage, target, tolerance);

    if success {
        camera.trigger();
        print("  ✓ Acquired");
    } else {
        print("  ✗ Skipped due to position error");
    }
}
```

---

## Troubleshooting

### Common Issues

#### Script doesn't start
```rhai
// Check syntax first
let x = ;  // ERROR: Unexpected end of expression
```

Fix: Validate syntax with `--validate` flag (future feature)

#### Infinite loop protection
```rhai
// This will hit 10,000 operation limit
while true {
    stage.move_abs(0.0);
}
```

Fix: Add proper loop termination condition

#### Position errors
```rhai
// Always wait for motion to complete
stage.move_abs(10.0);
camera.trigger();  // WRONG - stage still moving!
```

Fix: Add `wait_settled()`:
```rhai
stage.move_abs(10.0);
stage.wait_settled();  // CORRECT
camera.trigger();
```

### Best Practices

**1. Always wait for completion:**
```rhai
stage.move_abs(pos);
stage.wait_settled();  // Critical for precise positioning
```

**2. Add settle times for vibrations:**
```rhai
stage.wait_settled();
sleep(0.1);  // Extra settle time
camera.trigger();
```

**3. Verify positions for critical work:**
```rhai
stage.move_abs(target);
stage.wait_settled();

let actual = stage.position();
if (target - actual).abs() > 0.01 {
    print("Position error detected!");
}
```

**4. Use functions for reusable code:**
```rhai
fn acquire_at_position(stage, camera, pos) {
    stage.move_abs(pos);
    stage.wait_settled();
    sleep(0.1);
    camera.trigger();
}

// Reuse
for pos in [0.0, 5.0, 10.0] {
    acquire_at_position(stage, camera, pos);
}
```

**5. Add progress indicators:**
```rhai
let total = positions.len();
for i in 0..total {
    let progress = (i + 1) * 100 / total;
    print(`Progress: ${progress}% (${i+1}/${total})`);
    // ... acquisition code ...
}
```

---

## Performance Tips

### Minimize Sleep Calls
```rhai
// SLOW - unnecessary sleeps
for i in 0..100 {
    stage.move_abs(i * 0.1);
    sleep(0.01);  // Unnecessary
    stage.wait_settled();  // Already waits
}

// FAST - let hardware pace itself
for i in 0..100 {
    stage.move_abs(i * 0.1);
    stage.wait_settled();  // Sufficient
}
```

### Batch Operations
```rhai
// SLOW - individual movements
for i in 0..100 {
    stage.move_abs(i * 0.1);
    stage.wait_settled();
}

// FAST - calculate all positions first
let positions = [];
for i in 0..100 {
    positions.push(i * 0.1);
}

// Then execute
for pos in positions {
    stage.move_abs(pos);
    stage.wait_settled();
}
```

### Avoid Redundant Calls
```rhai
// SLOW - repeated resolution queries
for i in 0..100 {
    let res = camera.resolution();  // Same every time
    camera.trigger();
}

// FAST - query once
let res = camera.resolution();
for i in 0..100 {
    camera.trigger();
}
```

---

## See Also

- [Rhai API Reference](./RHAI_API_REFERENCE.md) - Complete function documentation
- [Scripting Overview](./SCRIPTING_OVERVIEW.md) - Architecture and concepts
- [Async Bridge Guide](./ASYNC_BRIDGE_GUIDE.md) - How async→sync conversion works
