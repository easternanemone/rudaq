# daq-driver-red-pitaya

Rust driver for Red Pitaya FPGA-based instruments. Provides control of custom PID feedback loops and signal acquisition via SCPI over TCP.

## Hardware Supported

- **Red Pitaya STEMlab 125-14** - FPGA-based instrument with analog I/O and custom bitstreams
- **Custom PID Bitstream** - Laser power stabilization feedback controller

## Quick Start

### Hardware Setup

Red Pitaya communicates via SCPI protocol over TCP network connection:

- **Communication:** TCP socket (standard Ethernet)
- **Default Port:** 5000
- **Protocol:** SCPI (Simplified Command Set for Programmable Instruments) over TCP
- **Default IP:** 192.168.1.100 (or configure via hostname)

### Network Configuration

First, ensure the Red Pitaya is powered and connected to your network:

```bash
# Ping the Red Pitaya to verify connection
ping 192.168.1.100

# SSH to configure network (optional)
ssh root@192.168.1.100
```

### Configuration Example

```toml
[[devices]]
id = "red_pitaya_pid"
type = "red_pitaya_pid"
enabled = true

[devices.config]
host = "192.168.1.100"
port = 5000

# For testing without hardware:
# mock = true
```

### Usage in Rust

```rust
use daq_driver_red_pitaya::RedPitayaPidFactory;
use common::driver::DriverFactory;

// Register the factory
registry.register_factory(Box::new(RedPitayaPidFactory));

// Create via config
let config = toml::toml! {
    host = "192.168.1.100"
    port = 5000
};
let components = factory.build(config.into()).await?;

// Read current power level
let readable = components.readable.unwrap();
let power = readable.read_value().await?;
println!("Current power: {:.2} W", power);

// Access PID parameters
let params = components.parameterized.unwrap();
let setpoint = params.get_parameter("setpoint").await?;
println!("PID setpoint: {}", setpoint);
```

### Usage in Rhai Scripts

```rhai
let pid = create_red_pitaya("192.168.1.100", 5000);

// Read current power
let power = pid.read_power();
print(`Power: ${power} W`);

// Set PID parameters
pid.set_kp(0.5);     // Proportional gain
pid.set_ki(0.1);     // Integral gain
pid.set_kd(0.05);    // Derivative gain

// Set output limits
pid.set_output_min(0.0);
pid.set_output_max(5.0);

// Enable/disable PID loop
pid.enable_pid();
sleep(1.0);
pid.disable_pid();
```

## Features

### PID Feedback Control

The Red Pitaya PID bitstream implements a closed-loop feedback controller:

```rust
// Read current measured value
let power = driver.read_value().await?;

// PID loop parameters
driver.set_kp(0.5).await?;   // Proportional: immediate response
driver.set_ki(0.1).await?;   // Integral: steady-state error correction
driver.set_kd(0.05).await?;  // Derivative: damping oscillations

// Setpoint (target value)
driver.set_setpoint(2.0).await?;

// Output limits (prevent saturation)
driver.set_output_min(0.0).await?;
driver.set_output_max(5.0).await?;
```

### Signal Acquisition

Read voltage levels from input channels:

```rust
// Read analog input (typically filtered sensor feedback)
let voltage = driver.read_input_voltage().await?;
println!("Input: {:.3} V", voltage);

// Read output voltage (PID control signal)
let output = driver.read_output_voltage().await?;
println!("Output: {:.3} V", output);
```

### Parametrized Control

All PID parameters are exposed via the Parameterized trait:

```rust
let parameterized = components.parameterized.unwrap();

// Query parameter value
let kp = parameterized.get_parameter("kp").await?;
println!("Kp = {}", kp);

// Set parameter value
parameterized.set_parameter("setpoint", 2.5).await?;
```

### Mock Mode

For testing without hardware, enable mock mode:

```toml
[[devices]]
id = "red_pitaya_pid_mock"
type = "red_pitaya_pid"
enabled = true

[devices.config]
host = "192.168.1.100"  # Ignored in mock mode
port = 5000              # Ignored in mock mode
mock = true              # Enable simulated device
```

In mock mode:
- Power readings are synthetic (triangular wave)
- PID parameters are stored locally
- No network connection required
- Useful for UI development and testing

## Protocol Reference

### SCPI Command Format

Red Pitaya uses SCPI (Simplified Command Set for Programmable Instruments):

```
Format: {command}:{subcommand} {value}\n
Example: "OUTPUT:DIRECT:VOLTAGE 3.3\n"

Response: {value}\n
Example: "2.45\n"
```

### Common Commands (PID Bitstream)

| Command | Format | Description |
|---------|--------|-------------|
| Read Power | `MEAS:POWER?` | Read current measured power |
| Read Input | `MEAS:INPUT?` | Read analog input voltage |
| Read Output | `MEAS:OUTPUT?` | Read control output voltage |
| Set Kp | `PID:KP {value}` | Set proportional gain |
| Set Ki | `PID:KI {value}` | Set integral gain |
| Set Kd | `PID:KD {value}` | Set derivative gain |
| Set Setpoint | `PID:SETP {value}` | Set target value |
| Set Output Min | `PID:OMIN {value}` | Set lower output limit |
| Set Output Max | `PID:OMAX {value}` | Set upper output limit |
| Enable PID | `PID:ENABLE` | Start control loop |
| Disable PID | `PID:DISABLE` | Stop control loop |

### Capabilities Implemented

- **Readable:** `read_value()` → Current measured power
- **Parameterized:** PID gains (Kp, Ki, Kd), setpoint, output limits

## Hardware Inventory

### Typical Network Configuration

| Device | IP Address | Port | Purpose |
|--------|-----------|------|---------|
| Red Pitaya STEMlab | 192.168.1.100 | 5000 | Laser power PID feedback |

To discover Red Pitaya on network:

```bash
# Scan for Red Pitaya SCPI servers
sudo nmap -p 5000 192.168.1.0/24

# Or use avahi discovery
avahi-browse -a | grep "Red Pitaya"
```

## Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `host` | string | Required | Hostname or IP address |
| `port` | integer | 5000 | SCPI server port |
| `mock` | boolean | false | Enable mock mode (no hardware) |
| `timeout_secs` | integer | 5 | Connection timeout |

## Troubleshooting

### Connection Refused

```
Error: "Connection refused"
```

**Solution:** Verify Red Pitaya is accessible:

```bash
# Check network connectivity
ping 192.168.1.100

# Check if SCPI server is running
telnet 192.168.1.100 5000

# SSH to device and restart server (if available)
ssh root@192.168.1.100
systemctl restart redpitaya-scpi
```

### Timeout During Operation

```
Error: "Command timeout"
```

**Solution:** Increase timeout or check network latency:

```toml
[devices.config]
host = "192.168.1.100"
timeout_secs = 10  # Increase from default 5 seconds
```

Or check network:

```bash
ping -c 5 192.168.1.100  # Check latency
```

### PID Loop Not Responding

```
Power doesn't change when setpoint is adjusted
```

**Solution:** Check:

1. PID loop is enabled: `driver.enable_pid().await?`
2. Output limits allow changes: `set_output_min()` / `set_output_max()`
3. Kp, Ki, Kd gains are non-zero

### Simulating Without Hardware

For development, use mock mode:

```toml
[[devices]]
id = "red_pitaya_test"
type = "red_pitaya_pid"
enabled = true

[devices.config]
host = "localhost"  # Ignored in mock mode
port = 5000         # Ignored in mock mode
mock = true         # Enable simulation
```

Mock behavior:
- Power readings cycle between 0 and 5W (triangular)
- Parameters are stored locally
- No network required
- Useful for UI testing

## Example: Simple Laser Power Stabilization

```rust
use daq_driver_red_pitaya::RedPitayaPidFactory;
use std::time::Duration;

async fn stabilize_laser() -> anyhow::Result<()> {
    // Create PID controller
    let config = toml::toml! {
        host = "192.168.1.100"
        port = 5000
    };

    let factory = RedPitayaPidFactory;
    let components = factory.build(config.into()).await?;

    let driver = components.readable.unwrap();
    let param = components.parameterized.unwrap();

    // Configure PID gains (tune these for your system)
    param.set_parameter("kp", 0.5).await?;
    param.set_parameter("ki", 0.1).await?;
    param.set_parameter("kd", 0.05).await?;

    // Set target power (2W)
    param.set_parameter("setpoint", 2.0).await?;

    // Set output voltage limits (0-5V for typical DAC)
    param.set_parameter("output_min", 0.0).await?;
    param.set_parameter("output_max", 5.0).await?;

    // Enable PID loop
    param.set_parameter("enabled", 1.0).await?;

    // Monitor for 10 seconds
    for i in 0..10 {
        let power = driver.read_value().await?;
        println!("[{:>2}s] Power: {:.2} W", i, power);

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // Disable PID loop
    param.set_parameter("enabled", 0.0).await?;

    Ok(())
}
```

## Related Documentation

- [CLAUDE.md - Red Pitaya Setup](../../CLAUDE.md#red-pitaya) - Detailed hardware configuration
- [Hardware Drivers Guide](../../docs/guides/hardware-drivers.md) - General driver patterns

## See Also

- **SCPI Standard:** https://www.ivifoundation.org/specifications/default.aspx
- **Red Pitaya Official:** https://redpitaya.com/
- **FPGA Development:** https://redpitaya.readthedocs.io/

## Dependencies

- `tokio` - Async runtime and TCP I/O
- `anyhow` - Error handling
- `serde` - TOML configuration

---

**Note:** The Red Pitaya driver assumes a custom PID FPGA bitstream is loaded on the device. Standard Red Pitaya firmware uses a different command set. Contact your bitstream provider for command reference documentation.
