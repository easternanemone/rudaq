# Rhai Scripting Guide

This guide covers writing experiment scripts using Rhai, rust-daq's embedded scripting language.

## Getting Started

### Your First Script

Create a file `my_experiment.rhai`:

```rhai
// Simple wavelength scan with power measurement
let start_wavelength = 800;
let end_wavelength = 900;
let step = 10;

for wavelength in range(start_wavelength, end_wavelength + 1, step) {
    laser.set_wavelength(wavelength);
    sleep(500);  // Wait 500ms for stabilization

    let power = power_meter.read();
    print(`${wavelength} nm: ${power} mW`);
}

print("Scan complete!");
```

### Running Scripts

**Via GUI:**
1. Open Script Editor panel
2. Load or paste your script
3. Click "Run"

**Via CLI:**
```bash
./rust-daq-daemon run-script my_experiment.rhai
```

**Via gRPC (Python):**
```python
from rust_daq import DaqClient

client = DaqClient("localhost:50051")
result = client.run_script("my_experiment.rhai")
```

## Available Hardware Bindings

### Stage Control

```rhai
// Absolute movement
stage.move_abs(10.0);       // Move to position 10
stage.wait_settled();       // Wait until motion complete

// Relative movement
stage.move_rel(5.0);        // Move +5 from current position

// Query position
let pos = stage.position(); // Get current position
print(`Position: ${pos}`);

// Velocity control
stage.set_velocity(10.0);   // Set speed
```

### Camera/Frame Acquisition

```rhai
// Single frame
let frame = camera.snap();

// Continuous acquisition
camera.start_acquisition();
for i in 0..100 {
    let frame = camera.get_frame();
    // Process frame...
}
camera.stop_acquisition();

// Triggered acquisition
camera.set_trigger_mode("external");
camera.arm();
// Wait for external trigger...
let frame = camera.get_frame();
```

### Power Meter

```rhai
// Simple reading
let power = power_meter.read();
print(`Power: ${power} W`);

// Set wavelength for calibration
power_meter.set_wavelength(800);

// Averaged reading
let sum = 0.0;
for i in 0..10 {
    sum += power_meter.read();
    sleep(100);
}
let avg = sum / 10.0;
```

### Laser Control

```rhai
// Wavelength
laser.set_wavelength(850);
let wl = laser.wavelength();

// Shutter
laser.open_shutter();
// ... do measurement ...
laser.close_shutter();

// Emission
laser.emission_on();
laser.emission_off();
```

### Rotator (ELL14)

```rhai
// Create rotator handle
let rotator = create_elliptec("/dev/serial/by-id/...", "2");

// Move to angle
rotator.move_abs(45.0);
rotator.wait_settled();

// Query
let angle = rotator.position();
let velocity = rotator.velocity();

// Speed control (0-100%)
rotator.set_velocity(100);  // Maximum speed
```

### Comedi DAQ

```rhai
// Read analog input
let voltage = comedi.read_ai(0);  // Channel 0

// Write analog output
comedi.write_ao(0, 2.5);  // Set channel 0 to 2.5V

// Digital I/O
comedi.write_dio(0, true);   // Set high
let state = comedi.read_dio(0);
```

## Common Experiment Patterns

### 1D Scan (Wavelength Sweep)

```rhai
let results = [];

for wavelength in range(800, 901, 5) {
    laser.set_wavelength(wavelength);
    sleep(200);

    let power = power_meter.read();
    results.push([wavelength, power]);

    print(`${wavelength} nm: ${power} W`);
}

// Results available after script
results
```

### 2D Scan (XY Stage)

```rhai
let x_start = 0.0;
let x_end = 10.0;
let y_start = 0.0;
let y_end = 10.0;
let step = 1.0;

for y in range(y_start, y_end + step, step) {
    stage_y.move_abs(y);
    stage_y.wait_settled();

    for x in range(x_start, x_end + step, step) {
        stage_x.move_abs(x);
        stage_x.wait_settled();

        let frame = camera.snap();
        // Frame is automatically saved

        print(`Captured at (${x}, ${y})`);
    }
}
```

### Time Series Acquisition

```rhai
let interval_ms = 100;
let duration_s = 10;
let num_points = (duration_s * 1000) / interval_ms;

for i in 0..num_points {
    let power = power_meter.read();
    let timestamp = i * interval_ms;

    print(`${timestamp} ms: ${power} W`);
    sleep(interval_ms);
}
```

### Triggered Frame Capture

```rhai
camera.set_trigger_mode("external");
camera.set_exposure(50.0);  // 50 ms

let num_frames = 100;
camera.arm();

for i in 0..num_frames {
    // External trigger fires...
    let frame = camera.get_frame();
    print(`Frame ${i}: ${frame.timestamp}`);
}

camera.disarm();
```

## Error Handling

```rhai
// Try-catch for error handling
try {
    stage.move_abs(1000.0);  // May exceed limits
} catch(err) {
    print(`Error: ${err}`);
    stage.stop();  // Emergency stop
}

// Check device availability
if stage.is_connected() {
    stage.move_abs(10.0);
} else {
    print("Stage not available!");
}
```

## Resource Limits

Scripts have built-in safety limits:

| Limit | Value | Purpose |
|-------|-------|---------|
| Max iterations | 1,000,000 | Prevent infinite loops |
| Max script size | 1 MB | Memory protection |
| Operation timeout | 30s | Prevent hanging |

These limits protect against runaway scripts but allow long experiments.

## Tips and Best Practices

1. **Always wait for motion to complete**
   ```rhai
   stage.move_abs(10.0);
   stage.wait_settled();  // Don't skip this!
   ```

2. **Close shutters when done**
   ```rhai
   laser.open_shutter();
   // ... experiment ...
   laser.close_shutter();  // Always close!
   ```

3. **Use sleep() for stabilization**
   ```rhai
   laser.set_wavelength(850);
   sleep(500);  // Wait for wavelength to stabilize
   ```

4. **Return results as last expression**
   ```rhai
   let data = [];
   // ... collect data ...
   data  // This becomes the script result
   ```

5. **Print progress for long experiments**
   ```rhai
   for i in 0..1000 {
       if i % 100 == 0 {
           print(`Progress: ${i}/1000`);
       }
       // ... work ...
   }
   ```

## Related Documentation

- [scripting README](../../crates/scripting/README.md) - Full API reference
- [Testing Guide](testing.md) - Running script tests
- [Hardware Drivers](hardware-drivers.md) - Driver implementation

## Troubleshooting

**Script won't start:**
- Check device connections in GUI
- Verify daemon is running
- Check script syntax with "Validate" button
- Review daemon logs for device initialization errors

**Device not responding:**
- Use `device.is_connected()` to check status
- Check physical connections (USB cables, power)
- Restart daemon if needed
- Check hardware permissions

**Script runs too slow:**
- Reduce sleep() durations if not needed for stabilization
- Use triggered acquisition instead of polling
- Check for unnecessary wait_settled() calls
- Profile with print() statements to identify bottlenecks

**Incomplete results:**
If your script returns partial results:
```rhai
try {
    // ... experiment ...
    results
} catch(err) {
    print(`Error: ${err}`);
    results  // Return partial results on error
}
```

**Memory or iteration limits:**
If you hit resource limits:
- Break the experiment into smaller scripts
- Use higher step values in loops
- Move data processing to post-analysis
