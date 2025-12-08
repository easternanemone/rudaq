# Plugin Creation Guide

This guide explains how to create YAML-based instrument plugins for rust-daq. The plugin system allows you to control new instruments without writing Rust code by defining command/response patterns in a declarative YAML format.

## Quick Start

Create a new file in the `plugins/` directory with a `.yaml` extension:

```yaml
metadata:
  id: "my-instrument"
  name: "My Custom Instrument"
  version: "1.0.0"
  driver_type: "serial_scpi"

protocol:
  baud_rate: 9600
  termination: "\r\n"

capabilities:
  readable:
    - name: "power"
      command: "POW?"
      pattern: "{val:f}"
      unit: "W"
      mock:
        default: 1.0
        jitter: 0.1
```

## File Structure

Every plugin YAML file has these sections:

```yaml
metadata:      # Required: Plugin identification
protocol:      # Required: Communication settings
on_connect:    # Optional: Initialization commands
on_disconnect: # Optional: Cleanup commands
error_patterns: # Optional: Error detection strings
capabilities:  # Required: What the instrument can do
ui_layout:     # Optional: GUI control layout
```

## Metadata Section

```yaml
metadata:
  id: "unique-instrument-id"    # Unique identifier (lowercase, hyphens)
  name: "Human Readable Name"   # Display name for UI
  version: "1.0.0"              # Plugin version (semver)
  driver_type: "serial_scpi"    # Protocol type (see below)
```

### Driver Types

| Type | Description | Use Case |
|------|-------------|----------|
| `serial_scpi` | Serial port with text commands | Most lab instruments |
| `tcp_scpi` | TCP/IP with text commands | Network instruments |
| `serial_raw` | Serial with binary protocol | Binary/proprietary protocols |
| `tcp_raw` | TCP/IP with binary protocol | Network binary devices |

## Protocol Section

```yaml
protocol:
  # Serial settings (ignored for TCP)
  baud_rate: 9600              # Baud rate: 9600, 19200, 115200, etc.

  # TCP settings (required for tcp_* driver types)
  tcp_host: "192.168.1.100"    # IP address or hostname
  tcp_port: 5025               # Port number (SCPI default: 5025)

  # Common settings
  termination: "\r\n"          # Line ending: "\r\n", "\n", or "\r"
  command_delay_ms: 10         # Delay after each command (ms)
  timeout_ms: 1000             # Read timeout (ms)
```

## Lifecycle Commands

### on_connect

Commands run when the instrument connects. Use for initialization:

```yaml
on_connect:
  - cmd: "*IDN?"           # Query identification
    wait_ms: 100           # Wait for response
  - cmd: "*RST"            # Reset to defaults
    wait_ms: 500           # Longer wait for reset
  - cmd: "SYST:REM"        # Enable remote mode
```

### on_disconnect

Commands run when disconnecting. Use for cleanup:

```yaml
on_disconnect:
  - cmd: "SYST:LOC"        # Return to local mode
```

## Error Patterns

Strings that indicate an error response from the device:

```yaml
error_patterns:
  - "ERROR"
  - "INVALID"
  - "FAULT"
```

## Capabilities

Capabilities define what your instrument can do. Each capability type maps to a trait in the rust-daq system.

### Readable (Sensor Values)

Read measurements from the instrument:

```yaml
capabilities:
  readable:
    - name: "temperature"
      command: "TEMP?"           # Command to query value
      pattern: "{val:f}C"        # Response pattern (see Pattern Syntax)
      unit: "C"                  # Optional unit string
      mock:                      # Optional mock data for testing
        default: 25.0
        jitter: 0.5              # Random variation ±0.5
```

### Settable (Configuration Parameters)

Parameters that can be read and written:

```yaml
capabilities:
  settable:
    - name: "wavelength"
      set_cmd: "WAV {val}"       # Command to set value
      get_cmd: "WAV?"            # Command to read current value
      pattern: "{val:f}"         # Response pattern
      value_type: float          # Type: float, int, string, enum, bool
      unit: "nm"
      min: 400.0                 # Optional minimum
      max: 1100.0                # Optional maximum
      mock:
        default: 780.0
        jitter: 0.0

    # Enum example
    - name: "mode"
      set_cmd: "MODE {val}"
      get_cmd: "MODE?"
      pattern: "{val}"
      value_type: enum
      options: ["AUTO", "MANUAL", "REMOTE"]
```

### Switchable (ON/OFF Controls)

Binary switches:

```yaml
capabilities:
  switchable:
    - name: "shutter"
      on_cmd: "SHUT OPEN"        # Command to turn on
      off_cmd: "SHUT CLOSE"      # Command to turn off
      status_cmd: "SHUT?"        # Command to query state (optional)
      pattern: "{val}"           # Response pattern for status
      mock:
        default: 0.0             # 0.0 = off, non-zero = on
        jitter: 0.0
```

### Movable (Motion Control)

Multi-axis motion control:

```yaml
capabilities:
  movable:
    axes:
      - name: "x"
        unit: "mm"
        min: 0.0
        max: 100.0
      - name: "y"
        unit: "mm"
        min: 0.0
        max: 100.0
    set_cmd: "POS {val}"         # {val} = "x,y" for multi-axis
    get_cmd: "POS?"
    get_pattern: "{x:f},{y:f}"   # Parse multi-axis response
```

### Actionable (One-Shot Commands)

Commands that trigger actions (no return value):

```yaml
capabilities:
  actionable:
    - name: "home"
      cmd: "HOME"
      wait_ms: 5000              # Wait for action to complete
    - name: "calibrate"
      cmd: "CAL"
      wait_ms: 10000
```

### Loggable (Static Metadata)

Information queried once and logged:

```yaml
capabilities:
  loggable:
    - name: "serial_number"
      cmd: "*SN?"
      pattern: "SN:{val}"
    - name: "firmware"
      cmd: "VERS?"
      pattern: "{val}"
```

### Triggerable (External Triggering)

For devices that support hardware triggers:

```yaml
capabilities:
  triggerable:
    arm_cmd: "TRIG:ARM"          # Arm the trigger
    trigger_cmd: "TRIG:IMM"      # Software trigger
    status_cmd: "TRIG:STAT?"     # Optional status query
    status_pattern: "{val}"
    armed_value: "ARMED"         # Expected value when armed
```

### Exposure Control (Camera Exposure)

For camera-like devices:

```yaml
capabilities:
  exposure_control:
    set_cmd: "EXP {val}"
    get_cmd: "EXP?"
    get_pattern: "{val:f}"
    min_seconds: 0.001           # Minimum exposure time
    max_seconds: 60.0            # Maximum exposure time
    mock:
      default: 0.1
      jitter: 0.0
```

### Frame Producer (Cameras)

For devices that produce image frames:

```yaml
capabilities:
  frame_producer:
    width: 1024                  # Frame width in pixels
    height: 1024                 # Frame height in pixels
    start_cmd: "START"           # Start acquisition
    stop_cmd: "STOP"             # Stop acquisition
    frame_cmd: "FRAME?"          # Get frame data
    status_cmd: "ACQ?"           # Optional status query
    status_pattern: "{val}"
    mock:
      pattern: "checkerboard"    # Mock pattern type
      intensity: 1000            # Base intensity (0-65535)
```

### Scriptable (Complex Operations)

Embed Rhai scripts for complex multi-step operations:

```yaml
capabilities:
  scriptable:
    - name: "safe_startup"
      description: "Safely start up with full checks"
      timeout_ms: 60000
      script: |
        // Check if already running
        let power = driver.read("power");
        if power > 0.1 {
          return "Already running";
        }

        // Enable step by step
        driver.switch_on("enable");
        sleep(1.0);

        // Verify
        let final_power = driver.read("power");
        if final_power > 0.5 {
          return "SUCCESS";
        } else {
          driver.switch_off("enable");
          return "FAILED: No power detected";
        }
```

**Script API:**
- `driver.read(name)` - Read a readable value
- `driver.set(name, value)` - Set a settable parameter
- `driver.get(name)` - Get current settable value
- `driver.switch_on(name)` - Turn on a switchable
- `driver.switch_off(name)` - Turn off a switchable
- `driver.action(name)` - Execute an actionable
- `driver.command(cmd)` - Send raw command
- `sleep(seconds)` - Pause execution
- `print(msg)` - Log a message

## Pattern Syntax

Patterns describe how to parse instrument responses.

### Placeholders

| Placeholder | Meaning | Matches |
|------------|---------|---------|
| `{val}` | Any string | `"hello"`, `"12.5"`, etc. |
| `{val:f}` | Float number | `12.5`, `-3.14`, `1e-6` |
| `{val:i}` | Integer | `42`, `-10` |
| `{name:f}` | Named float | Same as `{val:f}` but captured as `name` |

### Examples

| Response | Pattern | Extracted Value |
|----------|---------|-----------------|
| `"25.5"` | `"{val:f}"` | 25.5 |
| `"TEMP:25.5C"` | `"TEMP:{val:f}C"` | 25.5 |
| `"10.5,20.3"` | `"{x:f},{y:f}"` | x=10.5, y=20.3 |
| `"STATUS:ON"` | `"STATUS:{val}"` | "ON" |
| `"0.0025W"` | `"{val:f}W"` | 0.0025 |

## UI Layout

Define how the instrument appears in the GUI:

```yaml
ui_layout:
  - type: "group"
    label: "Output Control"
    children:
      - type: "toggle"
        target: "output"           # Links to switchable name
        label: "Output Enable"

      - type: "slider"
        target: "voltage_setpoint" # Links to settable name
        label: "Voltage (V)"

      - type: "readout"
        source: "voltage"          # Links to readable name
        label: "Measured Voltage"

      - type: "dropdown"
        target: "mode"             # Links to enum settable
        label: "Operating Mode"

      - type: "button"
        action: "calibrate"        # Links to actionable name
        label: "Run Calibration"
```

### UI Element Types

| Type | Purpose | Key Properties |
|------|---------|----------------|
| `group` | Container | `label`, `children` |
| `slider` | Numeric input | `target` (settable/movable axis) |
| `readout` | Display value | `source` (readable) |
| `toggle` | ON/OFF switch | `target` (switchable) |
| `button` | Action trigger | `action` (actionable) |
| `dropdown` | Enum selection | `target` (enum settable) |

## Mock Mode

Every capability can have mock data for testing without hardware:

```yaml
mock:
  default: 25.0    # Default value
  jitter: 0.5      # Random variation ±jitter
```

When the plugin is loaded without a physical connection, mock values are returned instead of communicating with the device.

## Complete Example

Here's a complete plugin for a simple power meter:

```yaml
# plugins/my-power-meter.yaml
metadata:
  id: "my-power-meter"
  name: "My Power Meter"
  version: "1.0.0"
  driver_type: "serial_scpi"

protocol:
  baud_rate: 9600
  termination: "\r\n"
  command_delay_ms: 50
  timeout_ms: 2000

on_connect:
  - cmd: "*IDN?"
    wait_ms: 100
  - cmd: "*RST"
    wait_ms: 500
  - cmd: "SYST:REM"

on_disconnect:
  - cmd: "SYST:LOC"

error_patterns:
  - "ERROR"
  - "INVALID"

capabilities:
  readable:
    - name: "power"
      command: "POW?"
      pattern: "{val:f}"
      unit: "W"
      mock:
        default: 0.001
        jitter: 0.0001

    - name: "wavelength_actual"
      command: "WAV:ACT?"
      pattern: "{val:f}"
      unit: "nm"
      mock:
        default: 780.0
        jitter: 0.1

  settable:
    - name: "wavelength"
      set_cmd: "WAV {val}"
      get_cmd: "WAV?"
      pattern: "{val:f}"
      value_type: float
      unit: "nm"
      min: 400.0
      max: 1100.0
      mock:
        default: 780.0
        jitter: 0.0

    - name: "range"
      set_cmd: "RANG {val}"
      get_cmd: "RANG?"
      pattern: "{val}"
      value_type: enum
      options: ["AUTO", "1mW", "10mW", "100mW", "1W"]

  switchable:
    - name: "autorange"
      on_cmd: "AUTO ON"
      off_cmd: "AUTO OFF"
      status_cmd: "AUTO?"
      pattern: "{val}"
      mock:
        default: 1.0
        jitter: 0.0

  actionable:
    - name: "zero"
      cmd: "ZERO"
      wait_ms: 2000

  loggable:
    - name: "serial_number"
      cmd: "*IDN?"
      pattern: "{manufacturer},{model},{val},{firmware}"

ui_layout:
  - type: "group"
    label: "Measurement"
    children:
      - type: "readout"
        source: "power"
        label: "Measured Power"
      - type: "readout"
        source: "wavelength_actual"
        label: "Actual Wavelength"

  - type: "group"
    label: "Configuration"
    children:
      - type: "slider"
        target: "wavelength"
        label: "Wavelength Setpoint"
      - type: "dropdown"
        target: "range"
        label: "Power Range"
      - type: "toggle"
        target: "autorange"
        label: "Auto-Range"
      - type: "button"
        action: "zero"
        label: "Zero Calibration"
```

## Using Your Plugin

### Configuration

Add your plugin to the hardware configuration:

```toml
# config/hardware.toml

# Optional: Add custom plugin search paths
plugin_paths = ["./plugins", "/opt/lab-plugins"]

[[devices]]
id = "my-power-meter"
name = "Lab Power Meter"
type = "plugin"                          # Use plugin driver
plugin_id = "my-power-meter"             # Plugin ID from metadata
connection = "serial:/dev/ttyUSB0"       # Or "tcp:192.168.1.100:5025"
mock = false                             # true for testing without hardware
```

### Via gRPC

```bash
# List available plugins
grpcurl -plaintext localhost:50051 daq.HardwareService/ListDevices

# Spawn a plugin device
grpcurl -plaintext -d '{
  "plugin_id": "my-power-meter",
  "device_id": "pm1",
  "connection": "serial:/dev/ttyUSB0"
}' localhost:50051 daq.HardwareService/SpawnPlugin
```

## Testing Your Plugin

1. **Mock Mode**: Set `mock = true` in config to test without hardware
2. **Unit Tests**: The plugin system has comprehensive test coverage
3. **Validation**: The system validates your YAML on load and reports errors

### Common Validation Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `Missing metadata.id` | No ID specified | Add unique `id` field |
| `Unknown driver_type` | Invalid type | Use: serial_scpi, tcp_scpi, serial_raw, tcp_raw |
| `No capabilities defined` | Empty capabilities | Add at least one capability |
| `Invalid pattern` | Malformed pattern | Check placeholder syntax |

## Best Practices

1. **Use Meaningful IDs**: `"keithley-2400"` not `"device1"`
2. **Document Units**: Always specify `unit` for numeric values
3. **Add Mock Data**: Enable testing without hardware
4. **Use Error Patterns**: Catch device errors early
5. **Group Related UI Elements**: Organize by function
6. **Add Wait Times**: Allow time for device operations
7. **Test with Mock First**: Verify patterns before connecting

## Troubleshooting

### Pattern Not Matching

1. Check the actual device response format
2. Verify termination characters
3. Add command delays if device is slow

### Timeouts

1. Increase `timeout_ms` in protocol
2. Check baud rate settings
3. Verify cable/network connection

### Commands Not Working

1. Check command syntax in device manual
2. Verify termination string
3. Enable remote mode in `on_connect`

## See Also

- [Example Plugins](../plugins/) - Working examples
- [Hardware Configuration](./HARDWARE_CONFIG.md) - Device configuration
- [gRPC API Reference](./GRPC_API.md) - Remote control API
