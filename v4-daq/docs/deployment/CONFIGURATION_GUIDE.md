# V4 Configuration System Guide

**Last Updated**: 2025-11-17
**Status**: Production Ready

This guide explains how to configure the V4 DAQ system using TOML files and environment variables.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Configuration Sources](#configuration-sources)
3. [File Format](#file-format)
4. [Instrument Types](#instrument-types)
5. [Environment Variables](#environment-variables)
6. [Examples](#examples)
7. [Validation](#validation)
8. [Troubleshooting](#troubleshooting)

---

## Quick Start

### Default Configuration

The system looks for configuration at `config/config.v4.toml`:

```bash
# Copy example configuration
cp config/config.v4.toml.example config/config.v4.toml

# Edit for your environment
nano config/config.v4.toml
```

### Load Configuration in Code

```rust
use v4_daq::config::V4Config;

#[tokio::main]
async fn main() -> Result<()> {
    // Load from default location (config/config.v4.toml)
    let config = V4Config::load()?;

    // Or load from custom path
    let config = V4Config::load_from("my/custom/config.toml")?;

    // Access configuration
    println!("App: {}", config.application.name);
    println!("Log Level: {}", config.application.log_level);
    println!("Instruments: {}", config.instruments.len());

    Ok(())
}
```

---

## Configuration Sources

Configuration is loaded with the following precedence (highest to lowest):

1. **Environment Variables** (with `RUSTDAQ_` prefix)
2. **TOML Configuration File** (default: `config/config.v4.toml`)
3. **Default Values** (built into the schema)

### Example Precedence

If you have:

```toml
# config.v4.toml
[application]
log_level = "info"
```

And run with:

```bash
RUSTDAQ_APPLICATION_LOG_LEVEL=debug ./app
```

The log level will be `debug` (environment variable wins).

---

## File Format

The configuration file uses TOML (Tom's Obvious, Minimal Language) format.

### Complete Example

```toml
# Application settings
[application]
name = "rust-daq-v4"
log_level = "info"
# data_dir = "/var/lib/rust-daq"  # Optional

# Actor system settings
[actors]
default_mailbox_capacity = 100
spawn_timeout_ms = 5000
shutdown_timeout_ms = 5000

# Storage backend settings
[storage]
default_backend = "hdf5"  # or "arrow" or "both"
output_dir = "/var/lib/rust-daq/data"
compression_level = 6     # 0-9
auto_flush_interval_secs = 30

# Define instruments (can have multiple of each type)
[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
enabled = true

[instruments.config.scpi]
resource = "TCPIP0::192.168.1.100::INSTR"
timeout_ms = 5000
enable_caching = false
```

---

## Instrument Types

V4 supports exactly 5 instrument types, each with its own configuration block.

### 1. SCPI Instrument

Generic SCPI devices via VISA (TCP/IP or Serial).

```toml
[[instruments]]
id = "power_meter"
type = "ScpiInstrument"
enabled = true

[instruments.config.scpi]
resource = "TCPIP0::192.168.1.100::INSTR"
timeout_ms = 5000
enable_caching = false
```

**Configuration Fields**:
- `resource` (required): VISA resource string
  - TCP/IP: `TCPIP0::<IP>::INSTR`
  - Serial: `ASRL<port>::INSTR` (e.g., `ASRL2::INSTR`)
- `timeout_ms` (default: 5000): Query timeout in milliseconds
- `enable_caching` (default: false): Cache repeated queries

---

### 2. ESP300 Motion Controller

3-axis motion control stage via serial port.

```toml
[[instruments]]
id = "xyz_stage"
type = "ESP300"
enabled = true

[instruments.config.esp300]
serial_port = "/dev/ttyUSB1"
axes = 3
baud_rate = 9600
```

**Configuration Fields**:
- `serial_port` (required): Serial port path
  - Linux: `/dev/ttyUSB0`, `/dev/ttyUSB1`, etc.
  - macOS: `/dev/tty.usbserial-*`
  - Windows: `COM1`, `COM2`, etc.
- `axes` (default: 3): Number of controlled axes
- `baud_rate` (default: 9600): Serial communication speed

---

### 3. PVCAM Camera

Princeton Instruments camera (e.g., PrimeBSI).

```toml
[[instruments]]
id = "imaging_camera"
type = "PVCAMInstrument"
enabled = true

[instruments.config.pvcam]
camera_name = "PrimeBSI"
# frame_width = 1024      # Optional
# frame_height = 1024     # Optional
# exposure_ms = 100.0     # Optional
```

**Configuration Fields**:
- `camera_name` (required): Camera identifier
- `frame_width` (optional): Frame width in pixels
- `frame_height` (optional): Frame height in pixels
- `exposure_ms` (optional): Exposure time in milliseconds

---

### 4. Newport 1830-C Power Meter

High-precision power meter via VISA.

```toml
[[instruments]]
id = "power_sensor"
type = "Newport1830C"
enabled = true

[instruments.config.newport]
resource = "ASRL2::INSTR"
timeout_ms = 5000
# wavelength_nm = 800.0   # Optional
```

**Configuration Fields**:
- `resource` (required): VISA resource string
- `timeout_ms` (default: 5000): Query timeout in milliseconds
- `wavelength_nm` (optional): Wavelength for power correction in nanometers

---

### 5. MaiTai Laser

Coherent MaiTai laser via serial port.

```toml
[[instruments]]
id = "maitai_laser"
type = "MaiTai"
enabled = true

[instruments.config.maitai]
serial_port = "/dev/ttyUSB2"
baud_rate = 19200
timeout_ms = 5000
auto_control = false
```

**Configuration Fields**:
- `serial_port` (required): Serial port path
- `baud_rate` (default: 19200): Serial communication speed
- `timeout_ms` (default: 5000): Query timeout in milliseconds
- `auto_control` (default: false): Enable automatic wavelength control

**Safety Note**: MaiTai laser operation requires approval from your laser safety officer.

---

## Environment Variables

Any configuration value can be overridden via environment variables using:
- Prefix: `RUSTDAQ_`
- Separator: `_` (underscores between nested keys)
- Case: Uppercase

### Format

```
RUSTDAQ_<SECTION>_<KEY>=value
```

### Examples

```bash
# Set application name
export RUSTDAQ_APPLICATION_NAME="My DAQ System"

# Set log level
export RUSTDAQ_APPLICATION_LOG_LEVEL=debug

# Set storage settings
export RUSTDAQ_STORAGE_OUTPUT_DIR=/custom/path
export RUSTDAQ_STORAGE_COMPRESSION_LEVEL=9

# Run application
./rust-daq-v4
```

### Common Environment Variables

| Variable | Default | Example |
|----------|---------|---------|
| `RUSTDAQ_APPLICATION_NAME` | "rust-daq-v4" | `RUSTDAQ_APPLICATION_NAME="Lab DAQ"` |
| `RUSTDAQ_APPLICATION_LOG_LEVEL` | "info" | `RUSTDAQ_APPLICATION_LOG_LEVEL=debug` |
| `RUSTDAQ_ACTORS_DEFAULT_MAILBOX_CAPACITY` | 100 | `RUSTDAQ_ACTORS_DEFAULT_MAILBOX_CAPACITY=200` |
| `RUSTDAQ_ACTORS_SPAWN_TIMEOUT_MS` | 5000 | `RUSTDAQ_ACTORS_SPAWN_TIMEOUT_MS=10000` |
| `RUSTDAQ_STORAGE_DEFAULT_BACKEND` | "hdf5" | `RUSTDAQ_STORAGE_DEFAULT_BACKEND=arrow` |
| `RUSTDAQ_STORAGE_OUTPUT_DIR` | Required | `RUSTDAQ_STORAGE_OUTPUT_DIR=/var/lib/daq` |
| `RUSTDAQ_STORAGE_COMPRESSION_LEVEL` | 6 | `RUSTDAQ_STORAGE_COMPRESSION_LEVEL=9` |

---

## Examples

### Example 1: Simple Single-Instrument Setup

```toml
[application]
name = "Single Power Meter"
log_level = "info"

[actors]
default_mailbox_capacity = 100
spawn_timeout_ms = 5000
shutdown_timeout_ms = 5000

[storage]
default_backend = "hdf5"
output_dir = "/tmp/daq_data"
compression_level = 6
auto_flush_interval_secs = 30

[[instruments]]
id = "pm1"
type = "ScpiInstrument"
enabled = true

[instruments.config.scpi]
resource = "TCPIP0::192.168.1.100::INSTR"
timeout_ms = 5000
enable_caching = false
```

---

### Example 2: Complete Multi-Instrument Lab Setup

```toml
[application]
name = "Complete Lab Setup"
log_level = "debug"
data_dir = "/lab/daq_data"

[actors]
default_mailbox_capacity = 200
spawn_timeout_ms = 10000
shutdown_timeout_ms = 10000

[storage]
default_backend = "both"          # Both HDF5 and Arrow
output_dir = "/lab/measurements"
compression_level = 9
auto_flush_interval_secs = 30

# SCPI Power Meters
[[instruments]]
id = "scpi_pm1"
type = "ScpiInstrument"
enabled = true
[instruments.config.scpi]
resource = "TCPIP0::192.168.1.100::INSTR"
timeout_ms = 5000

[[instruments]]
id = "scpi_pm2"
type = "ScpiInstrument"
enabled = true
[instruments.config.scpi]
resource = "TCPIP0::192.168.1.101::INSTR"
timeout_ms = 5000

# Motion Control
[[instruments]]
id = "stage"
type = "ESP300"
enabled = true
[instruments.config.esp300]
serial_port = "/dev/ttyUSB0"
axes = 3
baud_rate = 9600

# Imaging
[[instruments]]
id = "camera"
type = "PVCAMInstrument"
enabled = true
[instruments.config.pvcam]
camera_name = "PrimeBSI"
frame_width = 2560
frame_height = 2160
exposure_ms = 50.0

# Power Measurement
[[instruments]]
id = "newport_pm"
type = "Newport1830C"
enabled = true
[instruments.config.newport]
resource = "ASRL1::INSTR"
wavelength_nm = 800.0

# Laser
[[instruments]]
id = "laser"
type = "MaiTai"
enabled = true
[instruments.config.maitai]
serial_port = "/dev/ttyUSB1"
baud_rate = 19200
auto_control = true
```

---

### Example 3: Staging Configuration

```toml
# Development/Testing: All instruments disabled except one
[application]
name = "Development Setup"
log_level = "debug"

[actors]
default_mailbox_capacity = 100

[storage]
default_backend = "arrow"
output_dir = "./test_data"
compression_level = 0    # No compression for speed

# Only enable SCPI meter for testing
[[instruments]]
id = "scpi_pm"
type = "ScpiInstrument"
enabled = true
[instruments.config.scpi]
resource = "TCPIP0::127.0.0.1::INSTR"

# Disable others during development
[[instruments]]
id = "stage"
type = "ESP300"
enabled = false
[instruments.config.esp300]
serial_port = "/dev/ttyUSB0"
axes = 3
```

---

## Validation

Configuration is automatically validated when loaded:

```rust
let config = V4Config::load()?;  // Validation happens here
```

### Validation Checks

1. **Log Level**: Must be one of: `trace`, `debug`, `info`, `warn`, `error`
2. **Storage Backend**: Must be one of: `arrow`, `hdf5`, `both`
3. **Compression Level**: Must be 0-9
4. **Instrument IDs**: Must be unique
5. **Instrument Type**: Must be one of the 5 supported types
6. **Required Fields**: Each instrument type has required configuration fields

### Validation Errors

If validation fails, you'll get clear error messages:

```
Configuration validation error: Invalid log_level 'debug2'.
Must be one of: trace, debug, info, warn, error
```

---

## Troubleshooting

### Problem: "Cannot find config file"

**Solution**: Ensure `config/config.v4.toml` exists in your project root:

```bash
# Create directory
mkdir -p config

# Copy example configuration
cp config/config.v4.toml.example config/config.v4.toml
```

Or specify custom path in code:

```rust
let config = V4Config::load_from("/etc/rust-daq/config.toml")?;
```

---

### Problem: "Invalid VISA resource string"

**Solution**: Verify the resource string format:

- **TCP/IP**: `TCPIP0::<IP>::INSTR`
  - Example: `TCPIP0::192.168.1.100::INSTR`
  - Test: `visainfo` command (NI VISA tools)

- **Serial**: `ASRL<port>::INSTR`
  - Example: `ASRL2::INSTR` (Serial port COM2 on Windows, or /dev/ttyS1 on Linux)
  - Test: Check VISA device list

---

### Problem: "Serial port not found"

**Solution**: Verify the correct serial port:

```bash
# Linux/macOS
ls -la /dev/tty*

# Windows
# Check Device Manager for COM ports

# macOS with USB devices
ls -la /dev/tty.usbserial*
```

Update `config.v4.toml` with correct port.

---

### Problem: "Instrument validation failed"

**Solution**: Check that all required fields are present:

- **ScpiInstrument**: Requires `scpi` block with `resource`
- **ESP300**: Requires `esp300` block with `serial_port`
- **PVCAM**: Requires `pvcam` block with `camera_name`
- **Newport1830C**: Requires `newport` block with `resource`
- **MaiTai**: Requires `maitai` block with `serial_port`

Each instrument must have all required fields filled in.

---

### Problem: "Duplicate instrument ID"

**Solution**: Instrument IDs must be unique:

```toml
# WRONG: Two instruments with same ID
[[instruments]]
id = "meter"
type = "ScpiInstrument"

[[instruments]]
id = "meter"  # ERROR: Duplicate!
type = "Newport1830C"

# RIGHT: Use unique IDs
[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"

[[instruments]]
id = "newport_meter"
type = "Newport1830C"
```

---

### Problem: "Environment variable not working"

**Solution**: Ensure correct format with `RUSTDAQ_` prefix and underscores:

```bash
# WRONG
export RUSTDAQ_LOG_LEVEL=debug        # Missing section
export RUSTDAQ_application.log_level  # Wrong separator

# RIGHT
export RUSTDAQ_APPLICATION_LOG_LEVEL=debug
export RUSTDAQ_STORAGE_OUTPUT_DIR=/path
```

---

## API Reference

### Loading Configuration

```rust
// Load from default location (config/config.v4.toml)
let config = V4Config::load()?;

// Load from custom path
let config = V4Config::load_from("custom/path.toml")?;
```

### Querying Configuration

```rust
// Get all instruments
let all = &config.instruments;

// Get enabled instruments only
let enabled = config.enabled_instruments();

// Get instruments by type
let scpi = config.instruments_by_type("ScpiInstrument");
```

### Configuration Structs

```rust
pub struct V4Config {
    pub application: ApplicationConfig,
    pub actors: ActorConfig,
    pub storage: StorageConfig,
    pub instruments: Vec<InstrumentDefinition>,
}

pub struct InstrumentDefinition {
    pub id: String,
    pub r#type: String,
    pub enabled: bool,
    pub config: InstrumentSpecificConfig,
}
```

---

## Production Deployment

For production, consider:

1. **External configuration files**: Store in `/etc/rust-daq/`
2. **Environment variables**: Override sensitive settings
3. **Validation**: Always validate configuration at startup
4. **Logging**: Set appropriate log level for production (usually `info`)
5. **Backups**: Keep backup of working configurations

Example production setup:

```bash
# Install configuration
sudo mkdir -p /etc/rust-daq
sudo cp config/config.v4.toml /etc/rust-daq/config.toml
sudo chown rustdaq:rustdaq /etc/rust-daq/config.toml
sudo chmod 600 /etc/rust-daq/config.toml

# Run with custom config path
systemctl set-environment DAQ_CONFIG=/etc/rust-daq/config.toml
```

---

## See Also

- [V4 Architecture Plan](../V4_ONLY_ARCHITECTURE_PLAN.md)
- [Deployment Guide](./PRODUCTION_DEPLOYMENT_GUIDE.md)
- [Troubleshooting](./TROUBLESHOOTING.md)
