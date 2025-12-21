# Rhai Scripting Guide

rust-daq uses [Rhai](https://rhai.rs/) as its embedded scripting language for experiment automation. Scripts run in the headless daemon and can control hardware, perform scans, and orchestrate complex experiments.

## Running Scripts

```bash
# Run a script with the daemon
cargo run -- run examples/simple_scan.rhai

# Or with the script_runner tool
cargo run --bin script_runner -- examples/simple_scan.rhai
```

## Built-in Types

### Stage (StageHandle)

Controls movable devices (stages, rotation mounts).

```rhai
// Pre-injected as `stage` when a stage is configured
stage.move_abs(10.0);      // Move to absolute position
stage.move_rel(5.0);       // Move relative to current position
stage.wait_settled();      // Wait for motion to complete
let pos = stage.position(); // Get current position
```

### Camera (CameraHandle)

Controls triggerable cameras and frame producers.

```rhai
// Pre-injected as `camera` when a camera is configured
camera.arm();              // Arm camera for acquisition
camera.trigger();          // Trigger frame capture
let res = camera.resolution(); // Get resolution as [width, height]
```

### RunEngine (RunEngineHandle)

Bluesky-style plan execution engine. Pre-injected as the global `run_engine` variable when configured in the daemon.

```rhai
// Use the pre-injected run_engine global
run_engine.queue(plan);           // Queue a plan
run_engine.start();               // Start processing queue
run_engine.pause();               // Pause at checkpoint
run_engine.resume();              // Resume execution
run_engine.abort("reason");       // Abort current plan
run_engine.halt();                // Emergency stop

// Query state
let state = run_engine.state();         // "idle", "running", "paused"
let len = run_engine.queue_len();       // Queue length
let uid = run_engine.current_run_uid(); // Current run UUID
let prog = run_engine.current_progress(); // Progress (0-100)
```

### Plan (PlanHandle)

Defines experiment plans for the RunEngine.

```rhai
// Create plans
let plan = count_simple(10);                                  // Simple count plan
let plan = count(10, "detector", 0.5);                        // Count with detector and delay
let plan = line_scan("motor", 0.0, 10.0, 11, "detector");    // 1D linear scan
let plan = grid_scan("x_motor", 0.0, 10.0, 11,               // 2D grid scan
                     "y_motor", 0.0, 5.0, 6, "detector");

// Plan properties
let type_str = plan.plan_type();  // "count", "line_scan", "grid_scan"
let name = plan.plan_name();      // Plan name
let points = plan.num_points();   // Number of points
```

## Global Functions

| Function | Description |
|----------|-------------|
| `print(msg)` | Print message to console |
| `sleep(seconds)` | Pause execution (use `f64`) |
| `create_mock_stage()` | Create a mock stage for testing |
| `count_simple(n)` | Create a simple count plan (n points) |
| `count(n, detector, delay)` | Create count plan with detector and delay |
| `line_scan(motor, start, end, points, detector)` | Create 1D linear scan plan |
| `grid_scan(x_motor, x_start, x_end, x_points, y_motor, y_start, y_end, y_points, detector)` | Create 2D grid scan plan |

## Example Scripts

### Basic Examples

| Script | Description |
|--------|-------------|
| [`simple_scan.rhai`](../../../examples/simple_scan.rhai) | Basic stage movement loop |
| [`triggered_acquisition.rhai`](../../../examples/triggered_acquisition.rhai) | Camera-triggered acquisition |
| [`error_demo.rhai`](../../../examples/error_demo.rhai) | Error handling demonstration |

### Advanced Experiments

| Script | Description |
|--------|-------------|
| [`focus_scan.rhai`](../../../examples/focus_scan.rhai) | Focus optimization scan |
| [`polarization_test.rhai`](../../../examples/polarization_test.rhai) | Polarization measurement |
| [`polarization_characterization.rhai`](../../../examples/polarization_characterization.rhai) | Full polarization characterization |
| [`angular_power_scan.rhai`](../../../examples/angular_power_scan.rhai) | Angular power measurement |
| [`multi_angle_acquisition.rhai`](../../../examples/multi_angle_acquisition.rhai) | Multi-angle data acquisition |
| [`orchestrated_scan.rhai`](../../../examples/orchestrated_scan.rhai) | Complex orchestrated scan |

### Learning Rhai

| Script | Description |
|--------|-------------|
| [`scripts/simple_math.rhai`](../../../examples/scripts/simple_math.rhai) | Basic Rhai syntax |
| [`scripts/loops.rhai`](../../../examples/scripts/loops.rhai) | Loop constructs |
| [`scripts/globals_demo.rhai`](../../../examples/scripts/globals_demo.rhai) | Global variables |
| [`scripts/validation_test.rhai`](../../../examples/scripts/validation_test.rhai) | Script validation |

## Simple Example

```rhai
// simple_scan.rhai - Basic stage scan
print("Starting scan...");

for i in 0..10 {
    let pos = i * 1.0;
    stage.move_abs(pos);
    print(`Moved to ${pos}mm`);
    sleep(0.1);
}

print("Scan complete!");
```

## Triggered Acquisition Example

```rhai
// Camera triggered acquisition at multiple positions
print("Setting up acquisition...");

camera.arm();
print("Camera armed");

for i in 0..5 {
    let pos = i * 2.0;
    stage.move_abs(pos);
    stage.wait_settled();
    camera.trigger();
    print(`Frame ${i+1} captured at ${pos}mm`);
}

print("Acquisition complete!");
```

## RunEngine Example

```rhai
// Queue and execute declarative plans
print("Setting up experiment plans...");

// Create multiple plans
let count_plan = count_simple(10);
let scan_plan = line_scan("motor", 0.0, 10.0, 11, "detector");

// Queue plans for execution
run_engine.queue(count_plan);
run_engine.queue(scan_plan);

// Check queue
let queue_len = run_engine.queue_len();
print(`Queued ${queue_len} plans`);

// Start execution
run_engine.start();

// Monitor progress
while run_engine.state() != "idle" {
    let progress = run_engine.current_progress();
    let uid = run_engine.current_run_uid();
    print(`Run ${uid}: ${progress}% complete`);
    sleep(0.5);
}

print("All plans complete!");
```

## Rhai Language Reference

Rhai is a simple, safe scripting language. Key features:

- **No null/nil** - Variables must be initialized
- **Dynamic typing** - Types determined at runtime
- **Immutable by default** - Use `let` for variables
- **String interpolation** - Use backticks: `` `Value: ${x}` ``

```rhai
// Variables
let x = 42;
let name = "test";
let arr = [1, 2, 3];
let map = #{ key: "value", num: 123 };

// Control flow
if x > 0 {
    print("positive");
} else {
    print("non-positive");
}

// Loops
for i in 0..10 { print(i); }
while x > 0 { x -= 1; }
loop { if done { break; } }

// Functions
fn add(a, b) { a + b }
let result = add(1, 2);
```

For complete Rhai documentation, see [rhai.rs/book](https://rhai.rs/book/).
