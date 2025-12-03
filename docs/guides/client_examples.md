# CLI Client Usage Examples

## Quick Start

### 1. Start the Daemon

In terminal 1:
```bash
cargo run --features networking -- daemon --port 50051
```

Expected output:
```
🚀 rust-daq - Headless DAQ System
Architecture: Headless-First + Scriptable (v5)

🌐 Starting Headless DAQ Daemon
   Architecture: V5 (Headless-First + Scriptable)
   gRPC Port: 50051

✅ gRPC server ready
   Listening on: 0.0.0.0:50051
   Features:
     - Script upload & execution
     - Remote hardware control
     - Real-time status streaming

📡 Daemon running - Press Ctrl+C to stop
```

### 2. Upload and Run a Script

In terminal 2:

```bash
# Upload script
cargo run --features networking -- client upload examples/simple_scan.rhai

# Output:
# 📤 Uploading script to daemon at http://localhost:50051
# ✅ Script uploaded successfully
#    Script ID: 3f8d9e2a-4c1b-4e8f-9a3d-7b5c2e1f8d6a
#
#    Next: Start the script with:
#    rust-daq client start 3f8d9e2a-4c1b-4e8f-9a3d-7b5c2e1f8d6a

# Start script (use your actual script ID)
cargo run --features networking -- client start 3f8d9e2a-4c1b-4e8f-9a3d-7b5c2e1f8d6a

# Output:
# ▶️  Starting script 3f8d9e2a-4c1b-4e8f-9a3d-7b5c2e1f8d6a on daemon at http://localhost:50051
# ✅ Script started successfully
#    Execution ID: exec-7a8b9c0d-1e2f-3a4b-5c6d-7e8f9a0b1c2d
#
#    Monitor with:
#    rust-daq client status exec-7a8b9c0d-1e2f-3a4b-5c6d-7e8f9a0b1c2d

# Check status
cargo run --features networking -- client status exec-7a8b9c0d-1e2f-3a4b-5c6d-7e8f9a0b1c2d

# Output:
# 📊 Checking status of execution exec-7a8b9c0d-1e2f-3a4b-5c6d-7e8f9a0b1c2d on daemon at http://localhost:50051
#
# Status: COMPLETED
# Started: 1234567890000000000 ns
# Ended: 1234567891000000000 ns
```

## Advanced Examples

### Remote Daemon

Connect to a daemon running on another machine:

```bash
# Upload to remote daemon
rust-daq client upload examples/scan.rhai \
  --name "Production Scan" \
  --addr http://192.168.1.100:50051

# Start script on remote daemon
rust-daq client start <script-id> --addr http://192.168.1.100:50051

# Monitor remote execution
rust-daq client status <exec-id> --addr http://192.168.1.100:50051
```

### Custom Script Name

Give uploaded scripts meaningful names:

```bash
rust-daq client upload examples/calibration.rhai --name "Daily Calibration"
rust-daq client upload experiments/test_001.rhai --name "Experiment 001 - Baseline"
```

### Stream Real-Time Data

Subscribe to measurement channels (requires hardware drivers):

```bash
# Stream single channel
rust-daq client stream --channels camera_frame

# Stream multiple channels
rust-daq client stream \
  --channels camera_frame \
  --channels stage_position \
  --channels temperature
```

Example output:
```
📡 Streaming data from daemon at http://localhost:50051
   Channels: ["camera_frame", "stage_position"]
   Press Ctrl+C to stop

[1701234567890000000] camera_frame = 42.5
[1701234567900000000] stage_position = 10.2
[1701234568000000000] camera_frame = 43.1
[1701234568100000000] stage_position = 10.3
^C
```

### Stop Running Script

```bash
# Graceful stop (try to complete cleanly)
rust-daq client stop <exec-id>

# Note: Force stop not yet implemented in daemon
```

## Workflow Examples

### Development Workflow

```bash
# 1. Test script locally first
cargo run -- run examples/new_experiment.rhai

# 2. If successful, deploy to daemon
rust-daq client upload examples/new_experiment.rhai --name "Experiment V2"

# 3. Start execution
rust-daq client start <script-id>

# 4. Monitor progress
rust-daq client status <exec-id>
```

### Production Workflow

```bash
# Terminal 1: Long-running daemon
rust-daq daemon --port 50051 > daemon.log 2>&1

# Terminal 2: Upload daily routines
rust-daq client upload routines/morning_calibration.rhai --name "Morning Calibration"
rust-daq client upload routines/data_collection.rhai --name "Data Collection"
rust-daq client upload routines/evening_maintenance.rhai --name "Evening Maintenance"

# Terminal 3: Execute routines on schedule (use cron or systemd timers)
# Morning (8:00 AM)
rust-daq client start <calibration-script-id>

# Afternoon (2:00 PM)
rust-daq client start <data-collection-script-id>

# Evening (6:00 PM)
rust-daq client start <maintenance-script-id>
```

### Debugging Failed Scripts

```bash
# Check status of failed execution
rust-daq client status <exec-id>

# Output will show:
# Status: ERROR
# Error: Runtime error at line 15: divide by zero
# Started: ...
# Ended: ...

# Fix script, then re-upload
rust-daq client upload examples/fixed_script.rhai
rust-daq client start <new-script-id>
```

## Script Examples

### Simple Scan Script (examples/simple_scan.rhai)

```rhai
// Move stage and capture images

print("Starting scan...");

// Move to starting position
stage.move_to(0.0, 0.0, 0.0);
print("Moved to origin");

// Scan in X direction
for x in 0..10 {
    stage.move_to(x * 0.1, 0.0, 0.0);
    let frame = camera.capture();
    print(`Captured frame at x=${x * 0.1}: ${frame.width}x${frame.height}`);
}

print("Scan complete!");
```

### Calibration Script (examples/calibration.rhai)

```rhai
// Hardware calibration routine

print("Calibration starting...");

// Home all axes
stage.home();
print("Stage homed");

// Test camera
let test_frame = camera.capture();
if test_frame.width == 1920 && test_frame.height == 1080 {
    print("✅ Camera OK: 1920x1080");
} else {
    print("❌ Camera resolution mismatch");
}

// Test stage movement
stage.move_to(10.0, 10.0, 0.0);
stage.move_to(0.0, 0.0, 0.0);
print("✅ Stage movement OK");

print("Calibration complete!");
```

## Choosing a client (CLI vs GUI vs Python)

`rust-daq` exposes a gRPC API, so you can choose the client style that best matches your workflow.

| Client            | Best For                                 | Stack                            | How to Run (examples)                                                                 |
|-------------------|------------------------------------------|----------------------------------|---------------------------------------------------------------------------------------|
| **CLI (`rust-daq`)** | Automation, CI, scripting, headless use | Rust binary + gRPC client        | `cargo run --features networking -- client upload …`, `client start`, `client stream` |
| **egui GUI**     | Interactive lab use on macOS/Linux/Win   | Native Rust, `eframe` + `egui`   | `cargo run --features networking -- daemon …` + `cargo run --features networking --bin rust_daq_gui_egui` |
| **Tauri GUI**    | Rich web‑style desktop UI (legacy/optional) | React + TypeScript + Tauri      | See `gui-tauri/README.md` (`npm install`, `npm run tauri dev`)                        |
| **Python client**| Analysis notebooks & Python ecosystems   | `clients/python` gRPC wrapper    | See `clients/python/docs` and `clients/python/examples/*.py`                         |

For day‑to‑day interactive control on a single workstation, the **egui GUI** is recommended.  
For automated runs and integration into pipelines, the **CLI** and **Python** clients are usually a better fit.

## Error Handling

### Connection Errors

```bash
$ rust-daq client upload examples/scan.rhai
📤 Uploading script to daemon at http://localhost:50051
Error: transport error

# Solution: Start the daemon first
$ cargo run --features networking -- daemon --port 50051
```

### Invalid Script ID

```bash
$ rust-daq client start invalid-id-12345
▶️  Starting script invalid-id-12345 on daemon at http://localhost:50051
Error: status: NotFound, message: "Script not found"

# Solution: Use the script ID from upload response
```

### Script Validation Errors

```bash
$ rust-daq client upload examples/bad_syntax.rhai
📤 Uploading script to daemon at http://localhost:50051
❌ Upload failed: Validation failed: Parse error: unexpected token at line 5

# Solution: Fix script syntax before uploading
```

## Tips and Best Practices

1. **Always test scripts locally first:**
   ```bash
   cargo run -- run examples/new_script.rhai
   ```

2. **Use meaningful script names:**
   ```bash
   rust-daq client upload script.rhai --name "Descriptive Name Here"
   ```

3. **Save script IDs for reuse:**
   ```bash
   SCRIPT_ID=$(rust-daq client upload examples/scan.rhai | grep "Script ID" | awk '{print $3}')
   rust-daq client start $SCRIPT_ID
   ```

4. **Monitor long-running scripts:**
   ```bash
   # Poll status every 5 seconds
   while true; do
     rust-daq client status <exec-id>
     sleep 5
   done
   ```

5. **Use remote daemons for hardware isolation:**
   ```bash
   # Run daemon on hardware-connected machine
   ssh hardware-server "rust-daq daemon --port 50051"

   # Control from development machine
   rust-daq client upload script.rhai --addr http://hardware-server:50051
   ```

## Building for Production

Build optimized release binary:

```bash
cargo build --release --features networking --bin rust_daq

# Binary location: target/release/rust_daq

# Install system-wide (optional)
sudo cp target/release/rust_daq /usr/local/bin/
```

Then use without cargo:

```bash
rust_daq daemon --port 50051
rust_daq client upload examples/scan.rhai
```
