# Integration Guide: driver-nidaqmx

This guide explains how to integrate the NI-DAQmx driver into a rust-daq deployment.

## System Requirements

### Development (Mock Mode)

- Rust toolchain 1.70+
- PyO3 compatible Python environment (Python 3.7+)

### Production (Hardware Mode)

- **Hardware**: National Instruments DAQ device (e.g., PCIe-6321, USB-6009)
- **NI-DAQmx Runtime**: System drivers from National Instruments
  - Linux: `/usr/local/natinst/nidaqmx`
  - Windows: `C:\Program Files\National Instruments\NI-DAQ\DAQmx`
- **Python 3.7+** with `nidaqmx` package: `pip install nidaqmx`

## Registry Integration

### 1. Add Factory Registration

In `crates/bin/src/main.rs` (or wherever the device registry is initialized):

```rust
use driver_nidaqmx::NiDaqTriggerFactory;

// In your registry initialization function
pub fn register_drivers(registry: &mut DeviceRegistry) {
    // ... existing drivers ...

    registry.register_factory(Box::new(NiDaqTriggerFactory::new()));

    info!("Registered NI-DAQmx trigger factory");
}
```

### 2. Add to Hardware Configuration

In `config/hardware.toml` (or your machine-specific config):

```toml
[[devices]]
id = "camera_trigger"
type = "nidaqmx_trigger"
enabled = true

[devices.config]
high_time = 0.1          # 100ms exposure
low_time = 0.001         # 1ms between frames
samps_per_chan = 1
trigger_mode = "digital"
device_name = "Dev1"     # Check your NI device name
counter = "ctr0"         # Use ctr0, ctr1, etc.
```

### 3. Verify Device Discovery

After starting the daemon with the updated config:

```bash
# Start daemon
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/hardware.toml

# Check logs for:
# "Registered NI-DAQmx trigger factory"
# "Registered 1 device(s): camera_trigger (Triggerable)"
```

## Feature Flags

### In bin/Cargo.toml

Add optional dependency:

```toml
[dependencies]
driver-nidaqmx = { path = "../driver-nidaqmx", optional = true }

[features]
# Add to your hardware feature group
nidaqmx_hardware = ["driver-nidaqmx/hardware"]

# Or include in existing hardware profiles:
maitai = [
    "pvcam_hardware",
    "thorlabs",
    "newport",
    "spectra_physics",
    "serial",
    "nidaqmx_hardware",  # <-- Add here
]
```

### Build Commands

```bash
# Development (mock mode)
cargo build -p bin

# Production (with hardware)
cargo build -p bin --features nidaqmx_hardware

# Or as part of existing hardware profile
cargo build -p bin --features maitai
```

## Device Configuration Reference

### Digital Trigger (Software Triggered)

```toml
[[devices]]
id = "my_trigger"
type = "nidaqmx_trigger"
enabled = true

[devices.config]
high_time = 0.1          # Pulse high duration (seconds)
low_time = 0.001         # Pulse low duration (seconds)
samps_per_chan = 1       # Number of pulses per trigger
trigger_mode = "digital" # Software trigger via Triggerable::trigger()
device_name = "Dev1"     # NI device identifier
counter = "ctr0"         # Counter channel (ctr0-ctr3 typically)
```

### Trigger On Position (External Edge)

```toml
[[devices]]
id = "external_trigger"
type = "nidaqmx_trigger"
enabled = true

[devices.config]
high_time = 0.001
low_time = 0.001
samps_per_chan = 1
trigger_mode = "trigger_on_position"
trigger_source = "/Dev1/PFI0"  # Physical input terminal
rising_edge = true             # true=rising, false=falling
retriggerable = false          # Allow multiple triggers
device_name = "Dev1"
counter = "ctr0"
```

## Usage in Rust Code

### Via Triggerable Trait

```rust
use common::capabilities::Triggerable;

// Get device from registry
let trigger = registry.get_triggerable("camera_trigger")?;

// Arm -> Trigger -> Disarm cycle
trigger.arm().await?;
trigger.trigger().await?;
trigger.disarm().await?;
```

### Via gRPC (from GUI/Client)

```rust
// In GUI or client code
use protocol::daq_service_client::DaqServiceClient;

let mut client = DaqServiceClient::connect("http://localhost:50051").await?;

// Arm device
client.arm_device(ArmRequest {
    device_id: "camera_trigger".to_string(),
}).await?;

// Trigger
client.trigger_device(TriggerRequest {
    device_id: "camera_trigger".to_string(),
}).await?;
```

## Troubleshooting

### "Failed to import nidaqmx" Error

**Symptom**: Python import error when creating device

**Solutions**:
1. Install Python package: `pip install nidaqmx`
2. Verify Python environment: `python -c "import nidaqmx; print(nidaqmx.__version__)"`
3. Check PyO3 finds correct Python: Set `PYTHONHOME` or use virtual environment

### "DAQmx Error -200220" (Device Not Found)

**Symptom**: Error when arming/triggering

**Solutions**:
1. Check device name: Run NI MAX (Windows) or `lsdaq` (Linux) to list devices
2. Update `device_name` in config to match actual device (e.g., "Dev2" instead of "Dev1")
3. Verify NI-DAQmx drivers are installed: Check for `/usr/local/natinst` (Linux) or NI MAX (Windows)

### "Counter ctr0 is reserved" Error

**Symptom**: Can't create task on counter

**Solutions**:
1. Use different counter: Change `counter = "ctr1"` in config
2. Close other applications using DAQ device
3. Reset device in NI MAX

### Python GIL Deadlocks

**Symptom**: Application hangs when triggering

**Solutions**:
1. Ensure all Python calls are in `spawn_blocking` (already implemented)
2. Don't hold Rust mutexes across `.await` points (already handled)
3. Check for circular async-to-sync dependencies

## Testing

### Unit Tests (Mock Mode)

```bash
# Run driver tests without hardware
cargo test -p driver-nidaqmx
```

### Integration Tests (Hardware Mode)

```bash
# Set environment variable to enable hardware tests
export NIDAQMX_HARDWARE_TEST=1

# Run with hardware feature
cargo test -p driver-nidaqmx --features hardware -- --nocapture --test-threads=1
```

### Manual Testing

```bash
# Run examples
cargo run -p driver-nidaqmx --example basic_trigger --features hardware
cargo run -p driver-nidaqmx --example trigger_on_position --features hardware
```

## Migration Path to Native Comedi (Linux)

For production Linux deployments, consider migrating to native Comedi driver:

1. **Hardware Check**: Verify NI device has Comedi support (`comedi_boards` kernel module)
2. **Install Comedilib**: `sudo apt-get install libcomedi-dev`
3. **Use driver-comedi**: Replace `nidaqmx_trigger` with `comedi_analog_output` in config
4. **Benefits**: No Python dependency, better real-time performance, native Linux integration

Example Comedi migration:

```toml
# Before (NI-DAQmx via PyO3)
[[devices]]
type = "nidaqmx_trigger"
# ...

# After (Native Comedi)
[[devices]]
type = "comedi_analog_output"
[devices.config]
device = "/dev/comedi0"
channel = 0
# ...
```

## Performance Considerations

- **Latency**: ~1-5ms overhead from Python bridge (acceptable for most triggering applications)
- **Throughput**: Limited by Python GIL - not suitable for high-frequency continuous generation
- **CPU Usage**: Minimal (Python calls are blocking, not spinning)

For sub-millisecond timing or >1kHz triggering, use native C API driver or Comedi.

## Security Notes

- **Python Dependency**: Ensure `nidaqmx` package is from trusted source (PyPI)
- **System Access**: NI-DAQmx drivers run with elevated permissions - validate all config inputs
- **Network Exposure**: If daemon is network-accessible, enable authentication in gRPC config

## License Compliance

- **driver-nidaqmx**: MIT OR Apache-2.0 (same as rust-daq)
- **PyO3**: Apache-2.0 OR MIT
- **nidaqmx (Python)**: MIT (https://github.com/ni/nidaqmx-python)
- **NI-DAQmx Runtime**: Proprietary (National Instruments EULA)

Ensure your deployment complies with NI-DAQmx license terms.
