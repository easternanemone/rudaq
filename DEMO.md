# rust-daq Demo Mode 🚀

**Try rust-daq without any hardware!** This guide shows you how to run a complete data acquisition system using mock devices in under 2 minutes.

## Quick Start (30 seconds)

```bash
# Terminal 1: Start daemon with demo hardware
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Terminal 2: Run demo scan script
cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai
```

**That's it!** You just ran an automated scan with mock devices.

---

## What You Get

The demo configuration (`config/demo.toml`) provides three mock devices:

| Device | Type | Capabilities | Demo Purpose |
|--------|------|--------------|--------------|
| **mock_stage** | Linear Stage | `Movable` (move, read position) | Simulates motion control |
| **mock_power_meter** | Optical Sensor | `Readable` (measure values) | Simulates data acquisition |
| **mock_camera** | Scientific Camera | `FrameProducer`, `Triggerable`, `ExposureControl` | Simulates imaging (640x480) |

All devices implement the same **capability traits** as real hardware, so scripts work identically when you switch to physical equipment.

---

## Demo Workflows

### 1. Command-Line Scripting

Run the included demo scan:
```bash
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml &
cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai
```

**Output:**
```
╔════════════════════════════════════════╗
║  rust-daq Demo: Automated Scan        ║
╚════════════════════════════════════════╝

Scan Parameters:
  Range: 0 to 10 mm
  Points: 11
  Step: 1 mm

Starting scan...
[1/11] Position: 0.00 mm → Power: 1.000e-6 W
[2/11] Position: 1.00 mm → Power: 1.000e-6 W
...
Scan complete!
```

### 2. Interactive GUI

Launch the desktop GUI to control devices visually:

```bash
# Terminal 1: Daemon
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Terminal 2: GUI
cargo run --bin rust-daq-gui --features networking
```

**In the GUI:**
1. Connect to `http://127.0.0.1:50051`
2. Navigate to **Devices** panel
3. See all three mock devices
4. Click on **mock_stage** to control position
5. Watch **mock_power_meter** readings update
6. Trigger frames from **mock_camera**

### 3. gRPC API (Python, Go, etc.)

The daemon exposes a gRPC API for remote control:

```python
# Python client example (requires grpcio + generated stubs)
import grpc
from daq_proto import hardware_service_pb2, hardware_service_pb2_grpc

channel = grpc.insecure_channel('localhost:50051')
hw = hardware_service_pb2_grpc.HardwareServiceStub(channel)

# List devices
devices = hw.ListDevices(hardware_service_pb2.ListDevicesRequest())
for device in devices.devices:
    print(f"Found: {device.id} - {device.name}")

# Move stage
hw.MoveAbsolute(hardware_service_pb2.MoveRequest(
    device_id="mock_stage",
    value=5.0,
    wait_for_completion=False
))
```

---

## Customize the Demo

### Modify Scan Parameters

Edit `examples/demo_scan.rhai`:
```rust
let scan_start = 0.0;       // Change start position
let scan_end = 20.0;        // Change end position
let num_points = 21;        // Change number of points
```

### Change Mock Device Settings

Edit `config/demo.toml`:
```toml
[[devices]]
id = "mock_camera"
name = "Demo Camera (Mock)"
[devices.driver]
type = "mock_camera"
width = 1920   # Higher resolution (slower)
height = 1080
```

### Add More Mock Devices

```toml
[[devices]]
id = "another_stage"
name = "Second Mock Stage"
[devices.driver]
type = "mock_stage"
initial_position = 100.0
```

---

## Next Steps

### 1. Try Other Example Scripts

```bash
# Explore available scripts
ls examples/*.rhai

# Run any example (daemon must be running)
cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai
```

### 2. Write Your Own Script

Create `my_experiment.rhai`:
```rust
print("My first experiment!");

// Move stage through positions
for pos in [0.0, 2.5, 5.0, 7.5, 10.0] {
    stage.move_abs(pos);
    sleep(0.1);
    let reading = power_meter.read();
    print(`Position ${pos} mm: ${reading} W`);
}
```

Run it:
```bash
cargo run --bin rust-daq-daemon -- run my_experiment.rhai
```

### 3. Connect Real Hardware

**When ready for real experiments:**

1. Copy `config/hardware.example.toml` to `config/hardware.toml`
2. Edit device configurations (serial ports, addresses, etc.)
3. Start daemon with real hardware:
   ```bash
   cargo run --bin rust-daq-daemon -- daemon --hardware-config config/hardware.toml
   ```
4. **Your scripts work without modification!** Mock→Real is just a config change.

See [Hardware Drivers Guide](docs/guides/hardware-drivers.md) for supported devices.

### 4. Enable Data Storage

Add recording to your scripts:
```rust
// Save data to HDF5 (requires storage_hdf5 feature)
storage.start_recording("my_experiment_001");

// ... run experiment ...

storage.stop_recording();
```

See [Storage Formats Guide](docs/guides/storage-formats.md) for storage backends (CSV, HDF5, Arrow).

---

## Troubleshooting

### "Failed to connect"
- Ensure daemon is running in Terminal 1
- Check address is `http://127.0.0.1:50051`
- Look for errors in daemon terminal

### "Device not found"
- Verify `config/demo.toml` is loaded (check daemon startup logs)
- Device IDs must match exactly: `mock_stage`, `mock_power_meter`, `mock_camera`

### "Version mismatch warning"
- This is OK if minor versions differ (e.g., 0.5.1 daemon, 0.5.2 GUI)
- Rebuild both binaries to sync versions:
  ```bash
  cargo build --workspace
  ```

---

## What's Next?

Explore the full capabilities:

- **[Architecture Guide](docs/architecture/ARCHITECTURE.md)** - System design
- **[Scripting Guide](docs/guides/scripting.md)** - Rhai scripting
- **[Hardware Drivers Guide](docs/guides/hardware-drivers.md)** - Add new hardware

**Ready for production?** See [V6_USABILITY_ROADMAP](./docs/project_management/V6_USABILITY_ROADMAP.md) for upcoming features.

---

**Questions?** Check [CLAUDE.md](./CLAUDE.md) for developer documentation or open an issue.
