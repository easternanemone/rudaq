# driver-comedi

Safe Rust bindings for Comedi, the Linux Control and Measurement Device Interface. Provides high-level abstractions for DAQ hardware with compile-time safety and async support.

## Hardware Supported

- **National Instruments PCI-MIO-16XE-10** - 16-channel analog input, 2-channel analog output, digital I/O, counters
- **Other Comedi-compatible devices** (Measurement Computing, Advantech, etc.)

This driver includes HAL trait implementations for integration with rust-daq's device model.

## Quick Start

### Hardware Setup

The Comedi driver communicates with DAQ hardware via the Linux kernel Comedi subsystem:

- **Device Node:** `/dev/comedi0` (first DAQ device)
- **Kernel Driver:** Comedi (must be installed)
- **Hardware:** NI PCI-MIO-16XE-10 installed in maitai machine
- **Breakout Box:** BNC-2110 for signal access

### Installation

```bash
# Install kernel comedi driver (Ubuntu/Debian)
sudo apt-get install comedi-modules

# Verify device is accessible
ls -la /dev/comedi0
```

### Configuration Example

```toml
[[devices]]
id = "photodiode"
type = "comedi_analog_input"
enabled = true

[devices.config]
device = "/dev/comedi0"
channel = 0
range_index = 0
input_mode = "rse"  # Referenced Single-Ended
units = "V"
```

### Usage in Rust

```rust
use driver_comedi::{ComediDevice, Range};

// Open the device
let device = ComediDevice::open("/dev/comedi0")?;

println!("Board: {}", device.board_name());
println!("Driver: {}", device.driver_name());

// Single-sample reading
let ai = device.analog_input()?;
let voltage = ai.read_voltage(0, Range::default())?;
println!("Channel 0: {:.3} V", voltage);

// Streaming acquisition
let ao = device.analog_output()?;
ao.write_voltage(0, 5.0, Range::default())?;
println!("Wrote 5V to output 0");
```

### Usage in Rhai Scripts

```rhai
let daq = create_comedi("/dev/comedi0");

// Single-sample read
let voltage = daq.read_channel(0);  // Channel ACH0
print(`Photodiode: ${voltage} V`);

// Write analog output
daq.write_channel(0, 3.3);  // Write 3.3V to DAC0

// Digital I/O
let dio = daq.digital_io(0);
dio.set_direction(0, true);  // Set pin 0 as output
dio.write_bit(0, true);      // Set pin 0 high
```

## Features

### Analog Input (16 channels)

Reading voltage from analog input channels:

```rust
let ai = device.analog_input()?;

// Single-sample read
let voltage = ai.read_voltage(0, Range::default())?;

// Raw reading (before conversion)
let raw = ai.read_raw(1, Range::default())?;

// Query input capabilities
let range = ai.range(0, 0)?;
println!("Range: {} to {} V", range.min, range.max);
```

### Analog Output (2 channels)

Writing voltage to analog output channels:

```rust
let ao = device.analog_output()?;

// Single-sample write
ao.write_voltage(0, 5.0, Range::default())?;

// Raw write
ao.write_raw(1, 32768)?;
```

### Input Reference Modes

Comedi supports three reference modes for differential measurements:

| Mode | Config Value | Description | Use Case |
|------|--------------|-------------|----------|
| **RSE** | `"rse"` | Referenced Single-Ended | Default, measures vs card ground |
| **NRSE** | `"nrse"` | Non-Referenced Single-Ended | Measures vs separate AISENSE pin |
| **DIFF** | `"diff"` | Differential | ACH0+ACH8, ACH1+ACH9, etc. (8 pairs max) |

Configuration example:

```toml
# RSE mode (default)
[devices.config]
device = "/dev/comedi0"
channel = 0
input_mode = "rse"

# Differential mode (use pairs: 0+8, 1+9, ..., 7+15)
[devices.config]
device = "/dev/comedi0"
channel = 0  # Channel 0 paired with channel 8
input_mode = "diff"
```

### Loopback Testing

Test without external equipment by connecting DAC output to ADC input:

```bash
# Hardware: Connect BNC cable from DAC0 to ACH0
# On BNC-2110: Set ACH0 switch to FS (Floating Source)
```

Configuration:

```toml
# Loopback test - write value to DAC0, read from ACH0
[[devices]]
id = "dac_loopback_test"
type = "comedi_analog_output"
[devices.config]
device = "/dev/comedi0"
channel = 0
range_index = 0

[[devices]]
id = "adc_loopback_test"
type = "comedi_analog_input"
[devices.config]
device = "/dev/comedi0"
channel = 0
range_index = 0
input_mode = "rse"
```

Test sequence:

```rust
let device = ComediDevice::open("/dev/comedi0")?;
let ao = device.analog_output()?;
let ai = device.analog_input()?;

// Write 3.3V to DAC0
ao.write_voltage(0, 3.3, Range::default())?;

// Read back from ADC0
std::thread::sleep(Duration::from_millis(100));
let voltage = ai.read_voltage(0, Range::default())?;
println!("Loopback test: Wrote 3.3V, read {} V", voltage);
assert!((voltage - 3.3).abs() < 0.1);  // Within 0.1V
```

### Digital I/O

Control digital I/O pins:

```rust
let dio = device.digital_io()?;

// Set direction
dio.set_direction(0, true)?;   // Pin 0 as output

// Read pin
let bit = dio.read_bit(1)?;    // Read pin 1 (input)

// Write pin
dio.write_bit(0, true)?;       // Set pin 0 high
```

### Streaming Acquisition

Hardware-timed multi-channel data acquisition:

```rust
use driver_comedi::{ComediDevice, StreamConfig, StreamAcquisition};

let device = ComediDevice::open("/dev/comedi0")?;

let config = StreamConfig::builder()
    .channels(&[0, 1, 2, 3])
    .sample_rate(50000.0)  // 50 kS/s per channel
    .build()?;

let stream = StreamAcquisition::new(&device, config)?;
stream.start()?;

// Read 1000 scans (4 channels × 1000 = 4000 samples)
let data = stream.read_n_scans(1000)?;
println!("Acquired {} samples", data.len());

stream.stop()?;
```

### Device Introspection

Query device capabilities:

```rust
let device = ComediDevice::open("/dev/comedi0")?;

println!("Board: {}", device.board_name());    // "PCI-MIO-16XE-10"
println!("Driver: {}", device.driver_name());  // "ni_pcimio"

let info = device.info();
println!("Number of subdevices: {}", info.n_subdevices);

// Query analog input subdevice
let ai_info = device.subdevice_info(0)?;
println!("AI channels: {}", ai_info.n_channels);
println!("AI ranges: {}", ai_info.n_ranges);
```

## Protocol Reference

### Comedi Subsystem Model

```
/dev/comedi0 (Device)
├── Subdevice 0: Analog Input (16 channels)
├── Subdevice 1: Analog Output (2 channels)
├── Subdevice 2: Digital I/O (8 lines)
├── Subdevice 3: Counters
└── ...
```

### Range Configuration

Voltage ranges are indexed. Common ranges for NI cards:

- **Range 0:** ±10V
- **Range 1:** ±5V
- **Range 2:** ±2.5V
- **Range 3:** ±1.25V
- **Range 4:** 0-10V
- **Range 5:** 0-5V

Query available ranges:

```rust
let ai = device.analog_input()?;
let range = ai.range(0, 0)?;  // Query channel 0, range index 0
println!("Range 0: {} to {} V", range.min, range.max);
```

### Capabilities Implemented

- **ReadableAnalogInput:** `read_voltage()`, `read_raw()`
- **SettableAnalogOutput:** `write_voltage()`, `write_raw()`
- **SwitchableDigitalIO:** `set_direction()`, `read_bit()`, `write_bit()`
- **ReadableCounter:** Counter operations

## Hardware Inventory

### maitai Machine (Verified Working)

| Device | Node | Type | Channels | Notes |
|--------|------|------|----------|-------|
| NI PCI-MIO-16XE-10 | `/dev/comedi0` | DAQ Card | AI: 16, AO: 2, DIO: 8 | BNC-2110 breakout |

### BNC-2110 Breakout Configuration

The BNC-2110 terminal block provides access to the NI card:

**Analog Input (ACH0-ACH7 on BNC):**
- ACH0-ACH7: Primary analog inputs (available on BNC connectors)
- ACH8-ACH15: Secondary inputs (spring terminal block only)
- AISENSE: Reference return (configurable)

**Analog Output (DAC0-DAC1 on BNC):**
- DAC0, DAC1: Analog output channels

**Digital I/O:**
- DIO0-DIO7: Digital lines (configurable input/output)

**Input Mode Switches (on BNC-2110):**
- **RSE:** Referenced (normal mode, measures vs ground)
- **FS:** Floating Source (for differential measurements)

## Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `device` | string | Required | Device node (e.g., `/dev/comedi0`) |
| `channel` | integer | Required | Channel number (0-15 for AI, 0-1 for AO) |
| `range_index` | integer | 0 | Voltage range index (0-5 typical) |
| `input_mode` | string | `"rse"` | Input mode: `"rse"`, `"nrse"`, or `"diff"` |
| `units` | string | `"V"` | Measurement units |

## Troubleshooting

### Device Not Found

```
Error: "Cannot open /dev/comedi0"
```

**Solution:** Install Comedi driver:

```bash
sudo apt-get install comedi-modules
sudo modprobe ni_pcimio
ls /dev/comedi*
```

### Permission Denied

```
Error: "Permission denied (os error 13)"
```

**Solution:** Run with elevated privileges or configure udev:

```bash
# Option 1: Run as root
sudo cargo run

# Option 2: Add user to comedi group
sudo usermod -a -G comedi $USER
newgrp comedi  # Apply group membership
```

### Voltage Out of Range

```
Error: "Voltage 15V out of range"
```

**Solution:** Check configured range. Example ranges for NI-MIO:

```rust
// Common ranges (check your hardware)
// Range 0: ±10V (allows -10 to +10V)
// Range 4: 0-10V (allows 0 to +10V only)

// Use correct range based on your configuration
let voltage = if voltage < 0.0 {
    // Need bipolar range (e.g., range 0: ±10V)
    ai.read_voltage(channel, Range::bipolar_10v())?
} else {
    // Can use unipolar range (e.g., range 4: 0-10V)
    ai.read_voltage(channel, Range::unipolar_10v())?
};
```

### Differential Mode Not Working

```
Error: "Invalid differential channel pair"
```

**Solution:** Differential mode requires valid channel pairs. For NI-MIO-16XE-10:

```
Valid differential pairs:
- ACH0 + ACH8
- ACH1 + ACH9
- ACH2 + ACH10
- ... (8 pairs total)
```

Configuration:

```toml
[devices.config]
channel = 0  # Primary channel of pair (0+8, 1+9, etc.)
input_mode = "diff"
```

### Streaming Acquisition Slow

```
Acquisition rate much slower than configured
```

**Solution:** Check:

1. Sample rate is achievable: `sample_rate * n_channels < hardware_limit`
2. No other processes using the device
3. Ring buffer size is adequate

```rust
// Check available sample rates
let caps = device.timing_capabilities()?;
println!("Min rate: {} S/s", caps.min_rate);
println!("Max rate: {} S/s", caps.max_rate);

// Use a conservative sample rate
let rate = 10000.0 * n_channels as f64;  // 10 kS/s per channel
```

## Dependencies

- `tokio` - Async runtime
- `anyhow` - Error handling
- `comedi-sys` - Low-level FFI bindings to libcomedi

## Hardware References

- [NI PCI-MIO-16XE-10 Manual](https://www.ni.com/) - Full specifications
- [Comedi Documentation](https://comedi.github.io/) - Kernel driver reference
- [BNC-2110 Manual](https://www.ni.com/) - Breakout box specifications

## See Also

- [CLAUDE.md - Comedi DAQ](../../CLAUDE.md#comedi-daq-ni-pci-mio-16xe-10) - Hardware specifics and channel mapping
- [CLAUDE.md - Hardware Inventory](../../CLAUDE.md#hardware-inventory-maitai) - maitai device configuration
- [Hardware Drivers Guide](../../docs/how-to/hardware-drivers.md) - General driver development
