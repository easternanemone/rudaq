# Rhai API Reference

Complete reference for all functions and types available in rust-daq Rhai scripts.

## Table of Contents
- [Stage Control (Movable)](#stage-control-movable)
- [Camera Control](#camera-control)
- [Utility Functions](#utility-functions)
- [Built-in Rhai Features](#built-in-rhai-features)
- [Error Handling](#error-handling)

---

## Stage Control (Movable)

All devices implementing the `Movable` capability trait (stages, actuators, goniometers, rotators).

### `stage.move_abs(position: f64)`

Move to absolute position in device-native units (typically mm or degrees).

**Parameters:**
- `position` - Target position (float)

**Returns:** None (blocks until command sent)

**Errors:**
- Position out of range
- Hardware communication failure
- Device not initialized

**Example:**
```rhai
// Move to 10mm
stage.move_abs(10.0);

// Move to position calculated in script
let target = 5.0 * 2.0;
stage.move_abs(target);  // → 10.0mm
```

**Notes:**
- Method returns immediately; motion continues in background
- Use `wait_settled()` to block until motion completes
- Concurrent `move_abs` calls will queue

---

### `stage.move_rel(distance: f64)`

Move relative distance from current position.

**Parameters:**
- `distance` - Distance to move (float, can be negative)

**Returns:** None

**Example:**
```rhai
stage.move_abs(0.0);    // Start at origin
stage.move_rel(5.0);    // → 5.0mm
stage.move_rel(3.0);    // → 8.0mm
stage.move_rel(-2.0);   // → 6.0mm
```

**Notes:**
- Relative moves are referenced to current position, not command position
- If stage is moving, relative position may be intermediate

---

### `stage.position() -> f64`

Get current position of stage.

**Parameters:** None

**Returns:** Current position (float)

**Example:**
```rhai
stage.move_abs(10.0);
let pos = stage.position();
print(`Stage is at ${pos}mm`);
```

**Notes:**
- Returns current position, which may differ from commanded position during motion
- For settled position, call `wait_settled()` first

---

### `stage.wait_settled()`

Block until motion completes and stage is settled.

**Parameters:** None

**Returns:** None

**Example:**
```rhai
// Precise positioning
stage.move_abs(10.0);
stage.wait_settled();     // Wait for motion to complete
let pos = stage.position();
print(`Settled at ${pos}mm`);

// Multiple sequential moves
for i in 0..10 {
    stage.move_abs(i * 1.0);
    stage.wait_settled();  // Ensure each move completes
    camera.trigger();
}
```

**Notes:**
- Blocking call - script execution pauses
- Timeout depends on hardware (typically 30s)
- Essential for triggered acquisition workflows

---

## Camera Control

All devices implementing the `Camera` trait (combines `Triggerable` + `FrameProducer`).

### `camera.arm()`

Prepare camera for triggered acquisition.

**Parameters:** None

**Returns:** None

**Errors:**
- Camera already armed
- Hardware communication failure
- Invalid camera state

**Example:**
```rhai
camera.arm();
// Camera is now ready to capture on trigger
camera.trigger();
```

**Notes:**
- Must call before `trigger()`
- Some cameras require re-arming after each trigger
- Arming may configure exposure, ROI, etc.

---

### `camera.trigger()`

Capture a single frame.

**Parameters:** None

**Returns:** None

**Errors:**
- Camera not armed
- Trigger timeout
- Hardware communication failure

**Example:**
```rhai
// Single frame
camera.arm();
camera.trigger();

// Multi-frame acquisition
camera.arm();
for i in 0..10 {
    camera.trigger();
    sleep(0.1);  // Wait between frames
}
```

**Notes:**
- Blocks until frame capture completes
- Frame data saved to HDF5 automatically
- Some cameras auto-rearm, others require explicit `arm()` per trigger

---

### `camera.resolution() -> [i64, i64]`

Get camera resolution as [width, height] array.

**Parameters:** None

**Returns:** Array `[width, height]` (integers)

**Example:**
```rhai
let res = camera.resolution();
let width = res[0];
let height = res[1];
print(`Camera: ${width}x${height} pixels`);

// Calculate pixel count
let total_pixels = width * height;
print(`Total pixels: ${total_pixels}`);
```

**Notes:**
- Resolution is read-only (set via hardware config)
- Values in pixels

---

## Utility Functions

### `sleep(seconds: f64)`

Pause script execution for specified duration.

**Parameters:**
- `seconds` - Duration in seconds (float)

**Returns:** None

**Example:**
```rhai
print("Starting...");
sleep(1.0);          // Wait 1 second
print("1 second later");

sleep(0.5);          // Wait 500ms
print("Done");
```

**Notes:**
- Uses `std::thread::sleep` - safe for Rhai context
- Does NOT block async runtime
- Precision: ~1ms on most systems

---

### `print(message: string)`

Print message to console (stdout).

**Parameters:**
- `message` - String or value to print

**Returns:** None

**Example:**
```rhai
print("Hello, world!");

let x = 42;
print(x);              // Prints: 42

let pos = stage.position();
print(`Position: ${pos}mm`);  // String interpolation
```

**Notes:**
- Built-in Rhai function
- Useful for debugging scripts
- Output appears in server logs

---

## Built-in Rhai Features

### Variables and Types
```rhai
let x = 10;           // Integer
let y = 3.14;         // Float
let s = "hello";      // String
let arr = [1, 2, 3];  // Array
```

### Arithmetic
```rhai
let a = 10 + 5;       // 15
let b = 10 - 5;       // 5
let c = 10 * 5;       // 50
let d = 10 / 5;       // 2
let e = 10 % 3;       // 1 (modulo)
```

### String Interpolation
```rhai
let name = "Alice";
let age = 30;
print(`${name} is ${age} years old`);
// → "Alice is 30 years old"
```

### Loops
```rhai
// Range loop
for i in 0..10 {
    print(i);  // 0, 1, 2, ..., 9
}

// Array loop
let arr = [1, 2, 3];
for x in arr {
    print(x);
}

// While loop
let i = 0;
while i < 10 {
    print(i);
    i += 1;
}
```

### Conditionals
```rhai
let pos = stage.position();

if pos < 5.0 {
    print("Position is low");
} else if pos > 15.0 {
    print("Position is high");
} else {
    print("Position is in range");
}
```

### Functions
```rhai
fn calculate_target(index) {
    return index * 2.0 + 1.0;
}

for i in 0..10 {
    let target = calculate_target(i);
    stage.move_abs(target);
    stage.wait_settled();
}
```

### Arrays
```rhai
let positions = [0.0, 5.0, 10.0, 15.0, 20.0];

for pos in positions {
    stage.move_abs(pos);
    stage.wait_settled();
    camera.trigger();
}

// Array access
let first = positions[0];
let last = positions[positions.len() - 1];
```

---

## Error Handling

### Script Errors

**Syntax Error (compile time):**
```rhai
let x = ;  // Missing value
// → Error: Unexpected end of expression
```

**Runtime Error (execution time):**
```rhai
stage.move_abs(1000.0);  // Out of range
// → Error: Position 1000.0 exceeds maximum travel
```

**Safety Limit Error:**
```rhai
while true {
    stage.move_abs(0.0);  // Infinite loop
}
// → Error: Safety limit exceeded: maximum 10000 operations
```

### Error Propagation

Errors from hardware operations terminate the script immediately:
```rhai
camera.arm();
camera.trigger();  // If this fails...
print("Done");     // ...this never executes
```

### Best Practices

**Check ranges before moving:**
```rhai
fn safe_move(stage, target) {
    if target < 0.0 || target > 25.0 {
        print(`Invalid target: ${target}`);
        return;
    }
    stage.move_abs(target);
}
```

**Add timeouts for long operations:**
```rhai
// Use sleep() to prevent tight loops
for i in 0..100 {
    camera.trigger();
    sleep(0.01);  // 10ms between frames
}
```

**Validate inputs:**
```rhai
fn run_scan(start, end, step) {
    if step <= 0.0 {
        print("Error: step must be positive");
        return;
    }
    if start >= end {
        print("Error: start must be less than end");
        return;
    }

    let pos = start;
    while pos <= end {
        stage.move_abs(pos);
        stage.wait_settled();
        camera.trigger();
        pos += step;
    }
}
```

---

## Type Reference

### `Stage`
- Type: `StageHandle` (internally `Arc<dyn Movable>`)
- Copyable: No (use by reference)
- Thread-safe: Yes

### `Camera`
- Type: `CameraHandle` (internally `Arc<dyn Camera>`)
- Copyable: No (use by reference)
- Thread-safe: Yes

### Primitive Types
- `i64` - Integer (64-bit signed)
- `f64` - Float (64-bit, double precision)
- `string` - UTF-8 string
- `array` - Dynamic array `[T]`
- `bool` - Boolean (`true`, `false`)

---

## Limitations

### Not Currently Supported

1. **Multiple Devices**
   - Scripts currently support one stage and one camera
   - Multi-device support planned for future

2. **Custom Hardware Config**
   - Hardware configured via Rust, not scripts
   - Scripts control pre-configured devices

3. **Data Access**
   - Scripts trigger acquisition but don't access frame data
   - Use Python client for data analysis

4. **Async/Await**
   - Rhai doesn't support async syntax
   - All async calls bridged to synchronous

5. **External Libraries**
   - No `import` or `use` statements
   - Only built-in Rhai + registered hardware functions

### Performance Notes

| Operation | Overhead | Notes |
|-----------|----------|-------|
| Function call | ~1µs | Rhai function call |
| Hardware command | ~50ms | Includes async bridging |
| Loop iteration | ~0.1µs | Empty loop |
| String interpolation | ~10µs | Per interpolation |

**Recommendation:** For > 1kHz operations, use compiled Rust instead of scripts.

---

## Examples

See [SCRIPTING_EXAMPLES.md](./SCRIPTING_EXAMPLES.md) for complete example scripts and common patterns.

## See Also

- [Scripting Overview](./SCRIPTING_OVERVIEW.md)
- [Async Bridge Guide](./ASYNC_BRIDGE_GUIDE.md)
- [Rhai Language Book](https://rhai.rs/book/)
