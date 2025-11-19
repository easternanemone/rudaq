# Configuration Reference - Rust DAQ V4

**Version**: 1.0
**Date**: 2025-11-17
**Format**: TOML with environment variable overrides

---

## Table of Contents

1. [Configuration Overview](#configuration-overview)
2. [Application Configuration](#application-configuration)
3. [Actor System Configuration](#actor-system-configuration)
4. [Storage Configuration](#storage-configuration)
5. [Instrument Configuration](#instrument-configuration)
6. [Environment Variable Overrides](#environment-variable-overrides)
7. [Configuration Validation](#configuration-validation)
8. [Example Configurations](#example-configurations)

---

## Configuration Overview

Rust DAQ V4 uses TOML configuration files with optional environment variable overrides. Configuration is loaded in this order (later values override earlier):

1. Base configuration file: `config/config.v4.toml`
2. Environment variables prefixed with `RUST_DAQ_`

### File Location

Default: `/opt/rust-daq/config/config.v4.toml`

Can be overridden:
```bash
/opt/rust-daq/bin/rust-daq-v4 --config /path/to/config.v4.toml
```

### Minimal Valid Configuration

```toml
[application]
name = "rust-daq"
log_level = "info"

[actors]
default_mailbox_capacity = 100
spawn_timeout_ms = 5000
shutdown_timeout_ms = 5000

[storage]
default_backend = "arrow"
output_dir = "/var/lib/rust-daq/data"

[[instruments]]
id = "example"
type = "ScpiInstrument"
enabled = true
config = { resource = "TCPIP0::192.168.1.100::INSTR" }
```

---

## Application Configuration

### Section: `[application]`

Controls application-level behavior.

#### `name` (required)
- **Type**: String
- **Default**: (none)
- **Description**: Application name, used in logs and metrics
- **Example**: `name = "rust-daq-lab"`

#### `log_level` (required)
- **Type**: String (enum)
- **Valid Values**: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`
- **Default**: (none - must be specified)
- **Description**: Global logging level
- **Recommendations**:
  - Production: `"info"`
  - Staging: `"debug"`
  - Development: `"trace"`
- **Example**: `log_level = "info"`

#### `data_dir` (optional)
- **Type**: String (path)
- **Default**: `/var/lib/rust-daq`
- **Description**: Root directory for application data
- **Permissions**: Must be readable/writable by `rustdaq` user
- **Example**: `data_dir = "/var/lib/rust-daq"`

### Example

```toml
[application]
name = "rust-daq-production"
log_level = "info"
data_dir = "/var/lib/rust-daq"
```

---

## Actor System Configuration

### Section: `[actors]`

Controls Kameo actor framework behavior.

#### `default_mailbox_capacity` (optional)
- **Type**: Integer
- **Default**: `100`
- **Range**: `1` - `1000000`
- **Description**: Maximum pending messages per actor
- **Notes**:
  - Higher values = more memory but better throughput under load
  - Set per-instrument config to override
  - Recommended: `100-200` for typical workloads
- **Example**: `default_mailbox_capacity = 200`

#### `spawn_timeout_ms` (optional)
- **Type**: Integer (milliseconds)
- **Default**: `5000`
- **Range**: `100` - `60000`
- **Description**: Timeout for actor spawning
- **Notes**:
  - Increase if hardware initialization is slow
  - Hardware init typically takes 500-2000ms
  - Timeout includes: init, device connect, first command
- **Example**: `spawn_timeout_ms = 5000`

#### `shutdown_timeout_ms` (optional)
- **Type**: Integer (milliseconds)
- **Default**: `5000`
- **Range**: `100` - `60000`
- **Description**: Timeout for graceful shutdown per actor
- **Notes**:
  - Hardware cleanup and final commands must complete
  - MaiTai shutdown: ~1000ms (laser stability check)
  - PVCAM shutdown: ~500ms (frame buffer cleanup)
- **Example**: `shutdown_timeout_ms = 5000`

### Example

```toml
[actors]
default_mailbox_capacity = 200
spawn_timeout_ms = 5000
shutdown_timeout_ms = 10000  # MaiTai may need extra time
```

---

## Storage Configuration

### Section: `[storage]`

Controls data storage backend.

#### `default_backend` (required)
- **Type**: String (enum)
- **Valid Values**: `"arrow"`, `"hdf5"`, `"both"`
- **Description**: Default storage format
- **Details**:
  - `"arrow"`: Apache Arrow IPC format (fast, columnar, interoperable)
  - `"hdf5"`: HDF5 format (hierarchical, large file support)
  - `"both"`: Write to both backends simultaneously
- **Example**: `default_backend = "hdf5"`

#### `output_dir` (required)
- **Type**: String (path)
- **Description**: Directory where data files are written
- **Notes**:
  - Must exist and be readable/writable by `rustdaq` user
  - Ensure sufficient disk space (100+ GB recommended)
  - Can be on network mount (NFS, SMB)
- **Example**: `output_dir = "/var/lib/rust-daq/data"`

#### `compression_level` (optional)
- **Type**: Integer
- **Default**: `6`
- **Range**: `0` - `9`
- **Description**: Compression level (0=none, 9=maximum)
- **Notes**:
  - Higher = better compression but slower writes
  - Recommended: `6` (good balance)
  - For HDF5: `0-9` supported
  - For Arrow: varies by codec
- **Example**: `compression_level = 6`

#### `auto_flush_interval_secs` (optional)
- **Type**: Integer (seconds)
- **Default**: `0` (manual flush only)
- **Range**: `0` - `3600`
- **Description**: Auto-flush interval for open files
- **Notes**:
  - `0` = only flush on explicit close
  - `30` = flush every 30 seconds
  - Larger intervals = better throughput
  - Smaller intervals = fresher data on disk
- **Example**: `auto_flush_interval_secs = 60`

### Example

```toml
[storage]
default_backend = "hdf5"
output_dir = "/var/lib/rust-daq/data"
compression_level = 6
auto_flush_interval_secs = 60
```

---

## Instrument Configuration

### Section: `[[instruments]]`

Defines individual instruments. Can have multiple entries (one per instrument).

#### Common Fields (All Instrument Types)

##### `id` (required)
- **Type**: String
- **Description**: Unique instrument identifier
- **Notes**:
  - Used in logs, API calls, and data headers
  - Alphanumeric + underscore: `^[a-zA-Z0-9_]+$`
  - Example: `scpi_1`, `esp300_stage`, `pvcam_main`
- **Example**: `id = "scpi_meter"`

##### `type` (required)
- **Type**: String (enum)
- **Valid Values**: `"ScpiInstrument"`, `"ESP300"`, `"PVCAMInstrument"`, `"Newport1830C"`, `"MaiTai"`
- **Description**: Instrument type (determines which actor to spawn)
- **Example**: `type = "ScpiInstrument"`

##### `enabled` (optional)
- **Type**: Boolean
- **Default**: `true`
- **Description**: Whether to spawn this instrument on startup
- **Notes**:
  - Set to `false` to disable without removing config
  - Can be overridden per deployment
- **Example**: `enabled = true`

### Instrument-Specific Configuration

#### SCPI Instrument

Generic SCPI-compliant instrument via VISA.

```toml
[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
enabled = true

[instruments.config]
# VISA resource string (required)
resource = "TCPIP0::192.168.1.100::INSTR"

# Command timeout in milliseconds (optional, default: 2000)
timeout_ms = 2000

# Command terminator (optional, default: "\n")
# terminator = "\n"

# Response timeout (optional, default: 2000)
# response_timeout_ms = 2000
```

**Resource String Examples**:
- Ethernet: `"TCPIP0::192.168.1.100::INSTR"`
- GPIB: `"GPIB0::20::INSTR"`
- USB: `"USB0::0x0699::0x0346::C123456::INSTR"`
- Serial: `"ASRL1::INSTR"`

---

#### ESP300 Motion Controller

Newport 3-axis motion stage controller.

```toml
[[instruments]]
id = "esp300_stage"
type = "ESP300"
enabled = true

[instruments.config]
# Serial port path (required)
serial_port = "/dev/ttyUSB0"

# Number of axes (optional, default: 3, range: 1-3)
axes = 3

# Baud rate (optional, default: 19200, fixed)
# Note: ESP300 requires 19200, changing not recommended
# baud = 19200

# Command timeout in milliseconds (optional, default: 2000)
# timeout_ms = 2000

# Homing velocity in mm/s (optional, default: 1.0)
# homing_velocity = 1.0
```

**Hardware Settings**:
- Baud Rate: Fixed at 19200 baud
- Flow Control: Hardware (RTS/CTS)
- Data Bits: 8
- Parity: None
- Stop Bits: 1
- Line Terminator: `\r\n`

**Serial Port Detection**:
```bash
# List available ports
ls /dev/ttyUSB*
ls /dev/ttyACM*

# Identify with udev rules
udevadm info /dev/ttyUSB0
```

---

#### PVCAM Camera

Photometrics camera via PVCAM SDK.

```toml
[[instruments]]
id = "pvcam_main"
type = "PVCAMInstrument"
enabled = true

[instruments.config]
# Camera name in PVCAM system (required)
camera_name = "PrimeBSI"

# Sensor temperature setpoint in Celsius (optional, default: -20.0)
# temperature_setpoint = -20.0

# Frame acquisition timeout in ms (optional, default: 5000)
# frame_timeout_ms = 5000

# ROI width in pixels (optional, default: full)
# roi_width = 2048

# ROI height in pixels (optional, default: full)
# roi_height = 2048

# ROI x offset in pixels (optional, default: 0)
# roi_x = 0

# ROI y offset in pixels (optional, default: 0)
# roi_y = 0
```

**Camera Discovery**:
```bash
# List available cameras (requires PVCAM SDK)
pvcam_list_cameras

# Example camera names:
# - "PrimeBSI"
# - "PrimeSC"
# - "Prime95B"
```

**PVCAM SDK Installation**:
- Install from Photometrics: https://www.photometrics.com/support/software
- Includes libraries and header files
- Runtime: `/usr/local/lib/libpvcam.so`

---

#### Newport 1830-C Power Meter

Newport optical power meter.

```toml
[[instruments]]
id = "newport_power"
type = "Newport1830C"
enabled = true

[instruments.config]
# VISA resource string (required)
resource = "ASRL1::INSTR"

# Wavelength in nanometers for calibration (optional, default: 1550)
wavelength_nm = 1550.0

# Command timeout in milliseconds (optional, default: 2000)
timeout_ms = 2000

# Auto-range enabled (optional, default: true)
# auto_range = true

# Measurement units (optional, default: "dBm")
# Valid: "dBm", "mW"
# units = "dBm"
```

**Resource Discovery**:
```bash
# List available serial VISA resources
visainfo | grep ASRL

# Example: ASRL1::INSTR, ASRL2::INSTR
```

**Wavelength Calibration**:
- C-band standard: 1550 nm
- Common values: 635, 785, 850, 1064, 1310, 1550 nm
- Power meter will calibrate for specified wavelength

---

#### MaiTai Tunable Laser

Spectra-Physics MaiTai Ti:Sapphire laser.

```toml
[[instruments]]
id = "maitai_laser"
type = "MaiTai"
enabled = true

[instruments.config]
# Serial port path (required)
serial_port = "/dev/ttyUSB1"

# Baud rate (optional, default: 115200, fixed)
# Note: MaiTai requires 115200, changing not recommended
# baud = 115200

# Command timeout in milliseconds (optional, default: 2000)
# timeout_ms = 2000

# Shutter default state on startup (optional, default: false=closed)
# shutter_open_on_startup = false

# Wavelength range in nm (optional)
# min_wavelength = 690.0
# max_wavelength = 1040.0
```

**Hardware Settings**:
- Baud Rate: Fixed at 115200 baud
- Flow Control: None
- Data Bits: 8
- Parity: None
- Stop Bits: 1
- Line Terminator: `\r\n`

**Safety Requirements**:
- Always verify laser safety officer approval before operation
- Shutter default is CLOSED (safe)
- Wavelength tuning takes ~2 seconds
- Power ramp-down required before shutdown

**Wavelength Range**:
- Standard: 690-1040 nm
- Actual range depends on pump power and optics
- Verify with `WAVE?` command

---

## Environment Variable Overrides

Configuration values can be overridden by environment variables. Prefix all variables with `RUST_DAQ_` and use `_` for nested paths.

### Override Syntax

```bash
# Application settings
export RUST_DAQ_APPLICATION_NAME="field-deployment"
export RUST_DAQ_APPLICATION_LOG_LEVEL="debug"
export RUST_DAQ_APPLICATION_DATA_DIR="/data"

# Actor settings
export RUST_DAQ_ACTORS_DEFAULT_MAILBOX_CAPACITY="200"
export RUST_DAQ_ACTORS_SPAWN_TIMEOUT_MS="5000"

# Storage settings
export RUST_DAQ_STORAGE_DEFAULT_BACKEND="arrow"
export RUST_DAQ_STORAGE_OUTPUT_DIR="/mnt/data"
export RUST_DAQ_STORAGE_COMPRESSION_LEVEL="6"

# Instrument settings (per instrument index)
# Note: Arrays use numeric index (0-based)
export RUST_DAQ_INSTRUMENTS_0_ID="scpi_1"
export RUST_DAQ_INSTRUMENTS_0_TYPE="ScpiInstrument"
export RUST_DAQ_INSTRUMENTS_0_CONFIG_RESOURCE="TCPIP0::192.168.1.100::INSTR"
```

### Use Cases

**Environment-Specific Overrides**:
```bash
# Production
export RUST_DAQ_APPLICATION_LOG_LEVEL="info"

# Staging
export RUST_DAQ_APPLICATION_LOG_LEVEL="debug"

# Development
export RUST_DAQ_APPLICATION_LOG_LEVEL="trace"
```

**Hardware-Specific Overrides**:
```bash
# Different serial ports per deployment
export RUST_DAQ_INSTRUMENTS_0_CONFIG_SERIAL_PORT="/dev/ttyUSB0"
export RUST_DAQ_INSTRUMENTS_1_CONFIG_SERIAL_PORT="/dev/ttyUSB1"
```

**Systemd Service Overrides**:
Edit `/etc/systemd/system/rust-daq.service`:
```ini
[Service]
Environment="RUST_DAQ_APPLICATION_LOG_LEVEL=debug"
Environment="RUST_DAQ_ACTORS_SPAWN_TIMEOUT_MS=10000"
```

---

## Configuration Validation

### Validate Configuration File

```bash
# Validate syntax
/opt/rust-daq/bin/rust-daq-v4 --validate-config /opt/rust-daq/config/config.v4.toml

# Should output:
# Configuration valid: OK
# All instruments: 5
# Enabled instruments: 5
```

### Common Validation Errors

**Error**: Invalid TOML syntax
```
Parse error: expected `=`, found `:`
```
Fix: Use `=` not `:` in TOML files

**Error**: Invalid log level
```
Validation error: Invalid log_level 'debug'. Must be one of: trace, debug, info, warn, error
```
Fix: Use exact lowercase level name

**Error**: Missing required field
```
Validation error: missing field `resource`
```
Fix: Add required field to instrument config

**Error**: Invalid resource string
```
Validation error: Invalid SCPI resource format
```
Fix: Check VISA resource syntax

---

## Example Configurations

### Example 1: Basic Single-Machine Setup

File: `/opt/rust-daq/config/config.v4.toml`

```toml
[application]
name = "rust-daq-lab"
log_level = "info"
data_dir = "/var/lib/rust-daq"

[actors]
default_mailbox_capacity = 100
spawn_timeout_ms = 5000
shutdown_timeout_ms = 5000

[storage]
default_backend = "hdf5"
output_dir = "/var/lib/rust-daq/data"
compression_level = 6
auto_flush_interval_secs = 60

# Generic SCPI instrument
[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
enabled = true
[instruments.config]
resource = "TCPIP0::192.168.1.100::INSTR"
timeout_ms = 2000

# Motion stage
[[instruments]]
id = "esp300_stage"
type = "ESP300"
enabled = true
[instruments.config]
serial_port = "/dev/ttyUSB0"
axes = 3

# Camera
[[instruments]]
id = "pvcam_main"
type = "PVCAMInstrument"
enabled = true
[instruments.config]
camera_name = "PrimeBSI"

# Power meter
[[instruments]]
id = "newport_1830c"
type = "Newport1830C"
enabled = true
[instruments.config]
resource = "ASRL1::INSTR"
wavelength_nm = 1550.0

# Tunable laser
[[instruments]]
id = "maitai_laser"
type = "MaiTai"
enabled = true
[instruments.config]
serial_port = "/dev/ttyUSB1"
```

### Example 2: Development Configuration (Extra Logging)

```toml
[application]
name = "rust-daq-dev"
log_level = "debug"  # Extra verbosity

[actors]
default_mailbox_capacity = 200  # Larger queues for testing
spawn_timeout_ms = 10000  # More lenient
shutdown_timeout_ms = 10000

[storage]
default_backend = "arrow"  # Faster writes for testing
output_dir = "/tmp/rust-daq-data"  # Temporary
compression_level = 0  # No compression for speed

[[instruments]]
id = "scpi_mock"
type = "ScpiInstrument"
enabled = true
[instruments.config]
resource = "TCPIP0::192.168.1.100::INSTR"
timeout_ms = 5000  # Lenient timeouts

# ... rest of instruments
```

### Example 3: Staging Configuration (Balanced)

```toml
[application]
name = "rust-daq-staging"
log_level = "info"

[actors]
default_mailbox_capacity = 150
spawn_timeout_ms = 5000
shutdown_timeout_ms = 10000

[storage]
default_backend = "hdf5"
output_dir = "/mnt/data/staging"
compression_level = 6
auto_flush_interval_secs = 30

# ... instrument configs
```

### Example 4: Distributed Setup (Multiple Machines)

**Machine 1** (`/opt/rust-daq/config/config.v4.toml`):
```toml
[application]
name = "rust-daq-machine1"
log_level = "info"

[storage]
default_backend = "hdf5"
output_dir = "/var/lib/rust-daq/data"

[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
[instruments.config]
resource = "TCPIP0::192.168.1.100::INSTR"

[[instruments]]
id = "esp300_stage"
type = "ESP300"
[instruments.config]
serial_port = "/dev/ttyUSB0"
axes = 3

[[instruments]]
id = "pvcam_main"
type = "PVCAMInstrument"
[instruments.config]
camera_name = "PrimeBSI"
```

**Machine 2** (`/opt/rust-daq/config/config.v4.toml`):
```toml
[application]
name = "rust-daq-machine2"
log_level = "info"

[storage]
default_backend = "hdf5"
output_dir = "/var/lib/rust-daq/data"

[[instruments]]
id = "newport_1830c"
type = "Newport1830C"
[instruments.config]
resource = "ASRL1::INSTR"
wavelength_nm = 1550.0

[[instruments]]
id = "maitai_laser"
type = "MaiTai"
[instruments.config]
serial_port = "/dev/ttyUSB0"
```

---

## Performance Tuning

### High-Throughput Configuration

For continuous high-speed acquisition:

```toml
[actors]
default_mailbox_capacity = 500  # Large buffers
spawn_timeout_ms = 10000
shutdown_timeout_ms = 10000

[storage]
compression_level = 0  # No compression
auto_flush_interval_secs = 120  # Batch flushes
```

### Low-Latency Configuration

For real-time measurement:

```toml
[actors]
default_mailbox_capacity = 50  # Small buffers
spawn_timeout_ms = 2000
shutdown_timeout_ms = 2000

[storage]
compression_level = 6
auto_flush_interval_secs = 10  # Frequent flushes
```

### Memory-Constrained Configuration

For embedded or limited-memory systems:

```toml
[actors]
default_mailbox_capacity = 20  # Minimal
spawn_timeout_ms = 5000

[storage]
compression_level = 9  # Maximum compression
default_backend = "arrow"  # More efficient than HDF5
```

---

## Troubleshooting Configuration

See [TROUBLESHOOTING.md](TROUBLESHOOTING.md) for common configuration issues.

---

**Version**: 1.0
**Last Updated**: 2025-11-17
**Maintained By**: Brian Squires
