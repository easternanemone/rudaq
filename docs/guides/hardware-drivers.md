# Hardware Driver Development Guide

A comprehensive guide for developers implementing new hardware drivers for rust-daq.

## Table of Contents

- [Getting Started](#getting-started)
- [Driver Architecture](#driver-architecture)
- [Implementing DriverFactory](#implementing-driverfactory)
- [Implementing Capability Traits](#implementing-capability-traits)
- [Serial Device Patterns](#serial-device-patterns)
- [Testing Your Driver](#testing-your-driver)
- [Common Protocols](#common-protocols)
- [Reference Implementations](#reference-implementations)
- [Troubleshooting](#troubleshooting)

---

## Getting Started

### Prerequisites

- Rust 1.70+
- Basic understanding of async/await
- Familiarity with the hardware you're implementing (protocol docs, command specs)

### What You'll Build

A **driver** is the bridge between the hardware and rust-daq's application layer. It:

1. Manages communication with a physical device (serial port, USB, Ethernet, etc.)
2. Implements capability traits that define what the device can do (Movable, Readable, etc.)
3. Registers with the DeviceRegistry via a DriverFactory
4. Exposes configuration through TOML files

### Development Path

```
1. Create driver crate (daq-driver-yourdevice)
2. Implement DriverFactory
3. Implement capability trait(s)
4. Write unit tests with mock hardware
5. Write hardware integration tests (optional)
6. Register factory at startup
7. Add TOML config example
```

---

## Driver Architecture

### High-Level Flow

```text
┌─────────────────────────────────────────────────┐
│           Application Code (Rhai/gRPC)          │
├─────────────────────────────────────────────────┤
│          DeviceRegistry (trait objects)         │
├──────────────┬──────────────┬───────────────────┤
│   Movable    │   Readable   │  FrameProducer    │
├──────────────┼──────────────┼───────────────────┤
│         Driver Implementation                   │
│  (Your driver struct + trait impls)             │
├──────────────┼──────────────┼───────────────────┤
│    Serial/USB/Ethernet I/O                      │
├─────────────────────────────────────────────────┤
│           Physical Hardware                      │
└─────────────────────────────────────────────────┘
```

### Component Model

Each driver returns a **DeviceComponents** struct containing trait objects for all the capabilities it supports:

```rust
pub struct DeviceComponents {
    pub movable: Option<Arc<dyn Movable>>,
    pub readable: Option<Arc<dyn Readable>>,
    pub triggerable: Option<Arc<dyn Triggerable>>,
    pub frame_producer: Option<Arc<dyn FrameProducer>>,
    // ... other capabilities
}
```

This decoupled design means:
- A driver only implements the capabilities it actually provides
- Multiple trait objects can point to the same driver instance
- The registry performs capability-based lookups (e.g., "get all Movable devices")

---

## Implementing DriverFactory

The **DriverFactory** trait is the entry point for driver registration. Every driver must implement it.

### Template Structure

```rust
use common::driver::{DriverFactory, DeviceComponents, Capability};
use futures::future::BoxFuture;
use anyhow::Result;
use serde::Deserialize;
use std::sync::Arc;

// ============================================================================
// Configuration Struct (Serde-based)
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct YourDeviceConfig {
    /// Serial port path (e.g., "/dev/ttyUSB0")
    pub port: String,

    /// Device address (for multidrop buses like RS-485)
    #[serde(default)]
    pub address: Option<String>,

    /// Baud rate for serial communication
    #[serde(default = "default_baud")]
    pub baud_rate: u32,

    /// Optional: Custom calibration parameter
    #[serde(default)]
    pub calibration: Option<f64>,

    /// Optional: Timeout for device initialization (ms)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

fn default_baud() -> u32 {
    9600
}

// ============================================================================
// Driver Factory Implementation
// ============================================================================

pub struct YourDeviceFactory;

/// Declare capabilities statically to avoid repeated allocations
static CAPABILITIES: &[Capability] = &[
    Capability::Movable,
    Capability::Parameterized,
];

impl DriverFactory for YourDeviceFactory {
    fn driver_type(&self) -> &'static str {
        "your_device"  // Must match TOML config: type = "your_device"
    }

    fn name(&self) -> &'static str {
        "Your Device Name"  // User-facing name
    }

    fn capabilities(&self) -> &'static [Capability] {
        CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        // Parse config to validate structure
        let _: YourDeviceConfig = config.clone().try_into()?;

        // Validate specific fields
        let table = config.as_table().ok_or_else(||
            anyhow::anyhow!("expected table")
        )?;

        if !table.contains_key("port") {
            anyhow::bail!("missing required 'port' field");
        }

        // Validate baud rate is reasonable (e.g., not 1)
        if let Some(baud) = table.get("baud_rate").and_then(|v| v.as_integer()) {
            if baud < 1200 || baud > 921600 {
                anyhow::bail!("baud_rate must be 1200-921600, got {}", baud);
            }
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            // Parse configuration
            let cfg: YourDeviceConfig = config.try_into()?;

            // Create driver instance
            let driver = Arc::new(
                YourDevice::new(&cfg.port, cfg.baud_rate).await?
            );

            // Return components with implemented capabilities
            Ok(DeviceComponents::new()
                .with_movable(driver.clone())
                .with_parameterized(driver))
        })
    }
}
```

### Key Points

1. **driver_type()** - This must match the TOML config exactly:
   ```toml
   [[devices]]
   type = "your_device"  # Matches driver_type() return value
   ```

2. **validate()** - Called before build(), should fail fast with clear errors

3. **build()** - Async factory method where you:
   - Parse config (safe after validation)
   - Open serial ports, USB connections, etc.
   - Perform device identity verification (query version strings)
   - Return trait objects for each implemented capability

4. **Capabilities** - Declare statically to avoid allocations on each factory call

---

## Implementing Capability Traits

### Movable Trait (Motion Control)

Used for stages, rotation mounts, linear actuators.

```rust
use common::capabilities::Movable;
use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
impl Movable for YourDevice {
    /// Move to an absolute position
    async fn move_abs(&self, position: f64) -> Result<()> {
        // Convert position to device-specific format
        // Send command
        // Wait for completion

        let command = format!("MA{:06.0}", position);
        self.send_command(&command).await?;

        // Poll or wait for motion complete
        self.wait_for_status_ready().await?;

        Ok(())
    }

    /// Move relative to current position
    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = self.position().await?;
        self.move_abs(current + distance).await
    }

    /// Get current position
    async fn position(&self) -> Result<f64> {
        let response = self.send_command("GP").await?;
        // Parse response to extract position
        let pos = parse_position_from_response(&response)?;
        Ok(pos)
    }

    /// Home to mechanical zero
    async fn home(&self) -> Result<()> {
        self.send_command("HO").await?;
        self.wait_for_status_ready().await?;
        Ok(())
    }

    /// Wait until motion settles (position stable)
    async fn wait_settled(&self) -> Result<()> {
        let timeout = std::time::Duration::from_secs(15);
        let start = std::time::Instant::now();

        loop {
            if self.is_settled().await? {
                return Ok(());
            }

            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for motion to settle");
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Get soft limit range (optional, for safety)
    async fn get_soft_limits(&self) -> Result<(f64, f64)> {
        // Query device for min/max position
        // Or return from cached metadata
        Ok((self.min_pos, self.max_pos))
    }
}
```

**Implementation Checklist:**
- [ ] move_abs() - most critical
- [ ] move_rel() - can be default (= current + distance)
- [ ] position() - query current position
- [ ] home() - return to zero
- [ ] wait_settled() - block until motion complete
- [ ] get_soft_limits() - optional but recommended

### Readable Trait (Scalar Measurements)

Used for power meters, temperature sensors, photodiodes.

```rust
use common::capabilities::Readable;
use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
impl Readable for YourDevice {
    /// Read a single scalar value
    async fn read(&self) -> Result<f64> {
        let response = self.send_command("READ").await?;
        let value = parse_value_from_response(&response)?;
        Ok(value)
    }

    /// Read and average multiple samples (optional)
    async fn read_averaged(&self, num_samples: usize) -> Result<f64> {
        let mut sum = 0.0;

        for _ in 0..num_samples {
            let value = self.read().await?;
            sum += value;

            // Space samples 50ms apart to avoid noise
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        Ok(sum / num_samples as f64)
    }
}
```

**Common Examples:**
- Power meters: return watts
- Temperature sensors: return degrees C/F
- Photodiodes: return voltage (0-10V)

### Triggerable Trait (Camera/Pulse Generators)

Used for cameras, pulse generators, data acquisition devices.

```rust
use common::capabilities::Triggerable;
use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
impl Triggerable for YourDevice {
    /// Prepare device for acquisition (arm state)
    async fn arm(&self) -> Result<()> {
        self.send_command("ARM").await?;
        Ok(())
    }

    /// Send acquisition trigger
    async fn trigger(&self) -> Result<()> {
        self.send_command("TRIGGER").await?;
        Ok(())
    }

    /// Stop acquisition and return to idle
    async fn disarm(&self) -> Result<()> {
        self.send_command("DISARM").await?;
        Ok(())
    }

    /// Optional: Get current trigger state
    fn is_armed(&self) -> bool {
        self.armed.load(std::sync::atomic::Ordering::Acquire)
    }
}
```

### FrameProducer Trait (Cameras)

Used for image acquisition. This is more complex due to buffering requirements.

```rust
use common::capabilities::FrameProducer;
use common::data::Frame;
use async_trait::async_trait;
use anyhow::Result;
use std::sync::Arc;

#[async_trait]
impl FrameProducer for YourDevice {
    /// Get camera resolution (width, height)
    async fn resolution(&self) -> Result<(u32, u32)> {
        Ok((self.width, self.height))
    }

    /// Get bits per pixel (8, 12, 16)
    async fn bits_per_pixel(&self) -> Result<u32> {
        Ok(self.bits_per_pixel)
    }

    /// Get frame from internal buffer (non-blocking if available)
    async fn get_frame(&self) -> Result<Option<Arc<Frame>>> {
        // Check internal buffer for new frames
        match self.frame_buffer.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(_) => Ok(None),  // No frame available
        }
    }

    /// Wait for next frame with timeout
    async fn wait_frame(&self, timeout_ms: u64) -> Result<Arc<Frame>> {
        let timeout = std::time::Duration::from_millis(timeout_ms);

        tokio::time::timeout(
            timeout,
            self.frame_buffer.recv()
        )
        .await
        .map_err(|_| anyhow::anyhow!("frame timeout"))?
    }
}
```

### Parameterized Trait (Observable Parameters)

Allows reading/writing device parameters with change notifications.

```rust
use common::capabilities::Parameterized;
use common::observable::ParameterSet;
use common::parameter::Parameter;
use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
impl Parameterized for YourDevice {
    /// Get the parameter set (lazy init)
    async fn parameters(&self) -> Result<Arc<ParameterSet>> {
        Ok(self.params.clone())
    }

    /// Subscribe to parameter changes
    async fn subscribe(&self, param_id: &str) -> Result<tokio::sync::broadcast::Receiver<f64>> {
        // Return broadcast receiver for this parameter
        let (tx, rx) = tokio::sync::broadcast::channel(100);
        // Store tx for when values change
        Ok(rx)
    }

    /// Update a parameter value
    async fn set_parameter(&self, param_id: &str, value: f64) -> Result<()> {
        match param_id {
            "velocity" => {
                self.send_command(&format!("SV{}", value as u8)).await?;
                self.velocity.set(value).await?;
            }
            _ => anyhow::bail!("unknown parameter: {}", param_id),
        }
        Ok(())
    }
}
```

---

## Serial Device Patterns

### Pattern 1: Simple Request-Response

For straightforward devices that respond to ASCII commands:

```rust
use tokio_serial::SerialPortBuilderExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::{anyhow, Context, Result};

pub struct SimpleDevice {
    port: Box<dyn tokio::io::AsyncReadWrite + Unpin + Send>,
}

impl SimpleDevice {
    pub async fn new(port_path: &str, baud_rate: u32) -> Result<Self> {
        // Open serial port asynchronously
        let port = tokio::task::spawn_blocking({
            let path = port_path.to_string();
            move || {
                tokio_serial::new(&path, baud_rate)
                    .open_native_async()
                    .context("Failed to open serial port")
            }
        }).await??;

        Ok(Self { port })
    }

    async fn send_command(&mut self, cmd: &str) -> Result<String> {
        // Send command (append newline if needed)
        self.port.write_all(cmd.as_bytes()).await?;
        self.port.write_all(b"\n").await?;

        // Read response (up to newline)
        let mut response = String::new();
        let mut buf = [0u8; 256];

        let n = self.port.read(&mut buf).await?;
        response.push_str(&String::from_utf8_lossy(&buf[..n]));

        Ok(response.trim().to_string())
    }
}
```

**Best Practices:**
- Use `spawn_blocking` to open serial ports (not async)
- Set appropriate timeouts on ports
- Handle both expected responses and timeouts
- Validate device identity on initialization

### Pattern 2: RS-485 Multidrop Bus

For devices that share a single serial port via address:

```rust
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedPort = Arc<Mutex<Box<dyn AsyncReadWrite>>>;

pub struct MultidropDevice {
    port: SharedPort,
    address: String,
}

impl MultidropDevice {
    pub async fn new(port: SharedPort, address: &str) -> Result<Self> {
        let device = Self {
            port,
            address: address.to_string(),
        };

        // Validate device at this address
        device.validate_device().await?;

        Ok(device)
    }

    async fn validate_device(&self) -> Result<()> {
        // Query device identity at this address
        let response = self.send_command("*IDN?").await?;

        if !response.contains("EXPECTED_MODEL") {
            anyhow::bail!(
                "wrong device at address {}: got {}",
                self.address,
                response
            );
        }

        Ok(())
    }

    async fn send_command(&self, cmd: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Prefix command with device address
        let full_cmd = format!("{}{}\n", self.address, cmd);

        port.write_all(full_cmd.as_bytes()).await?;

        // Read response
        let mut response = String::new();
        let mut buf = [0u8; 256];

        let n = port.read(&mut buf).await?;
        response.push_str(&String::from_utf8_lossy(&buf[..n]));

        Ok(response)
    }
}
```

**Key Pattern:**
- Lock the shared port for exclusive access during command/response
- Prefix all commands with device address
- Validate device identity on connection to catch misconfigurations

### Pattern 3: Binary Protocol with CRC

For devices that use binary frames with checksums:

```rust
use crc::{Crc, CRC_16_MODBUS};

pub struct BinaryDevice {
    port: Box<dyn AsyncReadWrite>,
}

impl BinaryDevice {
    async fn send_command(&mut self, cmd: &[u8]) -> Result<Vec<u8>> {
        // Build frame: [SOF] [CMD] [DATA] [CRC16] [EOF]
        let mut frame = vec![0x02];  // SOF (STX)
        frame.extend_from_slice(cmd);

        // Calculate CRC16
        let crc = Crc::<u16>::new(&CRC_16_MODBUS);
        let checksum = crc.checksum(&frame);
        frame.extend_from_slice(&checksum.to_be_bytes());

        frame.push(0x03);  // EOF (ETX)

        // Send frame
        self.port.write_all(&frame).await?;

        // Read response
        let response = self.read_frame().await?;

        // Verify CRC
        if !self.verify_crc(&response) {
            anyhow::bail!("CRC mismatch in response");
        }

        Ok(response)
    }

    async fn read_frame(&mut self) -> Result<Vec<u8>> {
        let mut frame = Vec::new();
        let mut buf = [0u8; 1];

        // Wait for SOF
        loop {
            self.port.read_exact(&mut buf).await?;
            if buf[0] == 0x02 { break; }
        }

        // Read until EOF
        loop {
            self.port.read_exact(&mut buf).await?;
            if buf[0] == 0x03 { break; }
            frame.push(buf[0]);
        }

        Ok(frame)
    }

    fn verify_crc(&self, data: &[u8]) -> bool {
        if data.len() < 2 { return false; }

        let crc = Crc::<u16>::new(&CRC_16_MODBUS);
        let expected = u16::from_be_bytes([data[data.len()-2], data[data.len()-1]]);
        let actual = crc.checksum(&data[..data.len()-2]);

        actual == expected
    }
}
```

### Device Identity Validation

Always validate the device on connection to catch misconfigurations:

```rust
async fn validate_device(&self) -> Result<()> {
    // Query device for version/model string
    let response = self.send_command("*IDN?").await?;

    // Check it's the right device
    if !response.contains("EXPECTED_MODEL_NAME") {
        anyhow::bail!(
            "Device identity mismatch. Expected 'EXPECTED_MODEL_NAME', got: {}",
            response
        );
    }

    tracing::info!("Device validated: {}", response);
    Ok(())
}
```

---

## Testing Your Driver

### Unit Tests with Mock Hardware

Test capability traits without actual hardware:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// Mock device for testing
    struct MockDevice {
        position: f64,
        armed: bool,
    }

    impl MockDevice {
        fn new() -> Self {
            Self {
                position: 0.0,
                armed: false,
            }
        }
    }

    #[async_trait]
    impl Movable for MockDevice {
        async fn move_abs(&self, pos: f64) -> Result<()> {
            // Validate range
            if pos < -100.0 || pos > 100.0 {
                anyhow::bail!("position out of range");
            }
            Ok(())
        }

        async fn position(&self) -> Result<f64> {
            Ok(self.position)
        }

        async fn home(&self) -> Result<()> {
            Ok(())
        }

        async fn wait_settled(&self) -> Result<()> {
            Ok(())
        }

        async fn get_soft_limits(&self) -> Result<(f64, f64)> {
            Ok((-100.0, 100.0))
        }
    }

    #[tokio::test]
    async fn test_move_abs() -> Result<()> {
        let device = MockDevice::new();

        device.move_abs(50.0).await?;
        assert_eq!(device.position().await?, 50.0);

        Ok(())
    }

    #[tokio::test]
    async fn test_soft_limits() -> Result<()> {
        let device = MockDevice::new();
        let (min, max) = device.get_soft_limits().await?;

        assert!(min < max);
        Ok(())
    }

    #[tokio::test]
    async fn test_position_out_of_range() -> Result<()> {
        let device = MockDevice::new();

        // Should fail with out-of-range position
        let result = device.move_abs(150.0).await;
        assert!(result.is_err());

        Ok(())
    }
}
```

### Testing with Mock Serial Port

For testing serial communication without real hardware:

```rust
#[cfg(test)]
mod tests {
    use tokio::io::DuplexStream;

    #[tokio::test]
    async fn test_command_parsing() -> Result<()> {
        // Create in-memory pipe for testing
        let (client, server) = tokio::io::duplex(64);

        // Send command on one end
        let cmd = "MA045000\n";
        let mut client_write = client;
        client_write.write_all(cmd.as_bytes()).await?;

        // Server receives and responds
        let mut server_read = server;
        let mut buf = [0u8; 256];
        let n = server_read.read(&mut buf).await?;

        assert_eq!(&buf[..n], cmd.as_bytes());

        Ok(())
    }
}
```

### Feature-Gated Hardware Tests

For tests that require actual hardware:

```rust
#[cfg(feature = "hardware_tests")]
mod hardware_tests {
    #[tokio::test]
    #[ignore = "requires /dev/ttyUSB0 with actual hardware"]
    async fn test_real_device() -> Result<()> {
        let device = YourDevice::new("/dev/ttyUSB0", 9600).await?;

        // Validate device responds
        device.move_abs(0.0).await?;
        device.home().await?;

        Ok(())
    }
}
```

Run hardware tests separately:
```bash
# Unit tests only (always fast)
cargo test --lib

# Include hardware tests (requires actual device)
cargo test --all-features
```

---

## Common Protocols

### SCPI (Standard Commands for Programmable Instruments)

Used by many RF, power, and meter devices:

```rust
// SCPI command structure: [:SYSTem]:SUBSYS:PARAM VALUE UNIT
// Example: CONF:VOLT:DC 10V

async fn configure_dc_voltage(&mut self, range: f64) -> Result<()> {
    let cmd = format!("CONF:VOLT:DC {}", range);
    self.send_command(&cmd).await?;
    Ok(())
}

async fn read_voltage(&mut self) -> Result<f64> {
    let response = self.send_command("READ?").await?;
    let value: f64 = response.parse()?;
    Ok(value)
}
```

### Modbus RTU (Industrial Standard)

Binary protocol with CRC checking, common on sensors and controllers:

```rust
// Modbus: [ADDR] [FUNCTION] [DATA...] [CRC_LOW] [CRC_HIGH]
// Function 3: Read holding registers
// Function 16: Write multiple registers

async fn read_register(&mut self, addr: u8, reg: u16) -> Result<u16> {
    // Build Modbus frame
    let mut frame = vec![
        addr,           // Slave address
        0x03,           // Read holding registers
        (reg >> 8) as u8, (reg & 0xFF) as u8,  // Register address
        0x00, 0x01,     // Quantity of registers
    ];

    // Calculate and append CRC
    let crc = calculate_modbus_crc(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);

    // Send and parse response
    let response = self.send_binary_command(&frame).await?;

    // Response format: [ADDR] [FUNC] [BYTE_COUNT] [DATA...] [CRC_LOW] [CRC_HIGH]
    if response.len() < 5 {
        anyhow::bail!("response too short");
    }

    let value = u16::from_be_bytes([response[3], response[4]]);
    Ok(value)
}
```

### Ethernet (TCP/IP)

For devices with network interfaces:

```rust
use tokio::net::TcpStream;

pub struct NetworkDevice {
    stream: TcpStream,
    timeout: Duration,
}

impl NetworkDevice {
    pub async fn new(ip: &str, port: u16) -> Result<Self> {
        let addr = format!("{}:{}", ip, port);
        let stream = tokio::net::TcpStream::connect(&addr).await?;

        Ok(Self {
            stream,
            timeout: Duration::from_secs(5),
        })
    }

    async fn send_command(&mut self, cmd: &str) -> Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Send
        self.stream.write_all(cmd.as_bytes()).await?;
        self.stream.write_all(b"\n").await?;

        // Receive with timeout
        let mut response = String::new();
        let mut buf = [0u8; 1024];

        let n = tokio::time::timeout(
            self.timeout,
            self.stream.read(&mut buf)
        ).await??;

        response.push_str(&String::from_utf8_lossy(&buf[..n]));
        Ok(response)
    }
}
```

---

## Reference Implementations

### ELL14 Rotator (RS-485 Multidrop)

Location: `crates/daq-driver-thorlabs/src/ell14.rs`

**Key Features:**
- Multidrop RS-485 bus with address 0-F
- Velocity control (0-100%)
- Cached settings for non-blocking queries
- Device identity validation on connection

**Study this for:**
- How to handle shared serial ports
- Parameter caching patterns
- Motor optimization sequences

### Newport 1830-C Power Meter (Simple Serial ASCII)

Location: `crates/daq-driver-newport/src/newport_1830c.rs`

**Key Features:**
- Simple ASCII command protocol
- Zero calibration with/without attenuator
- Readable trait implementation
- Port path discovery

**Study this for:**
- Simple Readable trait implementation
- Error handling patterns
- Device validation without complex state

### PVCAM Camera (Complex FrameProducer)

Location: `crates/daq-driver-pvcam/src/lib.rs`

**Key Features:**
- FrameProducer trait with continuous buffering
- Hardware frame modes (circular, non-circular)
- Exposure control
- Triggering modes

**Study this for:**
- FrameProducer implementation patterns
- Ring buffers for continuous acquisition
- Handling high-speed data streams
- Async frame delivery to clients

### MaiTai Laser (Wavelength + Shutter Control)

Location: `crates/daq-driver-spectra_physics/src/maitai.rs`

**Key Features:**
- Wavelength tuning (700-1050nm)
- Shutter control for safety
- Emission control
- Parameter queries

**Study this for:**
- WavelengthTunable trait implementation
- ShutterControl trait implementation
- Safety-critical device patterns
- Metadata for wavelength ranges

---

## Troubleshooting

### "Failed to open serial port"

**Causes:**
- Port path doesn't exist (e.g., `/dev/ttyUSB0` isn't connected)
- Permissions issue (not in dialout group)
- Port is in use by another process

**Solutions:**
```bash
# Check if port exists
ls -la /dev/serial/by-id/

# Add user to dialout group (Linux)
sudo usermod -a -G dialout $USER

# Check what's using the port
lsof /dev/ttyUSB0

# Use stable port paths (USB devices)
# Instead of: /dev/ttyUSB0 (changes on reboot)
# Use: /dev/serial/by-id/usb-FTDI_... (stable)
```

### "Device identity mismatch"

**Cause:** Wrong device connected to the port, or identity query failed

**Debug:**
```rust
// Add logging in validation
let response = self.send_command("*IDN?").await?;
tracing::info!("Device identity response: {}", response);

// Should show model number
// If garbled: check baud rate, line endings, encoding
```

### "CRC mismatch"

**Cause:** Corrupted data in transmission, or incorrect CRC algorithm

**Solutions:**
```rust
// Verify CRC algorithm matches device docs
// Common: CRC16-CCITT, CRC16-MODBUS, CRC32

// Add validation
let crc = calculate_crc(&data);
if crc != expected {
    tracing::warn!(
        "CRC mismatch: calculated={:04X}, expected={:04X}",
        crc, expected
    );
}

// Reduce baud rate if frequency-dependent
```

### "Timeout waiting for response"

**Cause:** Device not responding, wrong baud rate, or protocol mismatch

**Solutions:**
```rust
// Increase timeout for slow devices
let timeout = Duration::from_secs(10);
tokio::time::timeout(timeout, read_response()).await?

// Check baud rate matches device
// Verify line endings (CR, LF, CR+LF)
// Check hardware connections (cables, terminators)

// Add debug logging
tracing::debug!("Sending: {}", cmd);
let response = self.read_response().await?;
tracing::debug!("Received: {:?}", response);
```

### "Position out of soft limits"

**Cause:** Script or user attempted to move beyond declared range

**Prevention:**
```rust
// Validate before moving
let (min, max) = device.get_soft_limits().await?;
if target < min || target > max {
    return Err(anyhow::anyhow!(
        "Position {} outside limits [{}, {}]",
        target, min, max
    ));
}

// Don't silently clamp - fail with clear error
```

### "Shared port deadlock"

**Cause:** Holding Mutex guard across await point

**WRONG:**
```rust
let mut port = self.port.lock().await;
// Never do this - holding lock across await!
some_long_operation().await;
```

**CORRECT:**
```rust
// Release lock before await
let data = {
    let mut port = self.port.lock().await;
    read_from_port(&mut port).await?
};
// Lock released here
some_long_operation(&data).await;
```

---

## Registering Your Driver

After implementing DriverFactory, register it at startup:

```rust
// In main.rs or your initialization code
use daq_driver_yourdevice::YourDeviceFactory;
use common::registry::DeviceRegistry;

#[tokio::main]
async fn main() -> Result<()> {
    let registry = DeviceRegistry::new();

    // Register your driver factory
    registry.register_factory(Box::new(YourDeviceFactory));

    // Other factories...
    registry.register_factory(Box::new(Ell14Factory));
    registry.register_factory(Box::new(Newport1830cFactory));

    // Continue with normal startup...
}
```

---

## Next Steps

1. **Choose a reference implementation** - Pick one that matches your device type
2. **Start with DriverFactory** - Implement configuration and validation first
3. **Implement one capability trait** - Usually Movable or Readable to start
4. **Write unit tests** - Test with mock objects before touching hardware
5. **Add hardware tests** - Optional but recommended (feature-gated)
6. **Create TOML config example** - Document for users
7. **Add to project README** - List your driver in supported hardware

---

## See Also

- [common driver.rs](../../crates/common/src/driver.rs) - DriverFactory trait definition
- [common capabilities.rs](../../crates/common/src/capabilities.rs) - Capability trait definitions
- [Scripting Guide](./scripting.md) - How users control drivers via Rhai
- [Testing Guide](./testing.md) - General testing strategies for rust-daq
