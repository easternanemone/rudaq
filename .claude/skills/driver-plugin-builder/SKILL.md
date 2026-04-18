---
name: "rust-daq Driver Builder"
description: "Create new hardware drivers, plugins, and extensions for the rust-daq data acquisition system. Use when adding new instruments, implementing DriverFactory, creating capability traits (Movable, Readable, FrameProducer), building serial device drivers, or extending the plugin system."
---

# rust-daq Driver Builder

## What This Skill Does

Guides you through creating new hardware drivers and plugins for the rust-daq data acquisition system:

1. **New Driver Crates** - Scaffold `daq-driver-*` crates with proper structure
2. **DriverFactory Implementations** - Registry-compatible factory patterns
3. **Capability Traits** - Implement Movable, Readable, FrameProducer, etc.
4. **Serial Device Drivers** - RS-232/RS-485 communication patterns
5. **Declarative Plugins** - Config-driven drivers without Rust code

## Prerequisites

- Rust toolchain (1.75+)
- Understanding of async Rust (`tokio`)
- Serial port knowledge (for hardware drivers)
- Access to device documentation (protocol specs)

---

## Quick Start: New Driver Crate

### 1. Create Crate Structure

```bash
# Create driver crate directory
mkdir -p crates/driver-mydevice/src

# Create Cargo.toml
cat > crates/driver-mydevice/Cargo.toml << 'EOF'
[package]
name = "daq-driver-mydevice"
version = "0.1.0"
edition = "2021"
description = "Driver for MyDevice instrument"

[dependencies]
daq-core = { path = "../daq-core" }
anyhow = "1.0"
async-trait = "0.1"
futures = "0.3"
tokio = { version = "1", features = ["sync", "time"] }
tracing = "0.1"
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# For serial devices:
tokio-serial = { version = "5.4", optional = true }

[features]
default = []
serial = ["tokio-serial"]
EOF
```

### 2. Create lib.rs with Factory Export

```rust
// crates/driver-mydevice/src/lib.rs
mod driver;

pub use driver::{MyDevice, MyDeviceFactory};

/// Force linker to include this crate's factories
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<MyDeviceFactory>());
}
```

### 3. Implement the Driver and Factory

```rust
// crates/driver-mydevice/src/driver.rs
use anyhow::Result;
use async_trait::async_trait;
use daq_core::{
    capabilities::Readable,
    driver::{Capability, DeviceComponents, DriverFactory},
    parameter::Parameter,
};
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct MyDeviceConfig {
    pub port: String,
    #[serde(default = "default_baud")]
    pub baud_rate: u32,
}

fn default_baud() -> u32 { 9600 }

pub struct MyDevice {
    value: Parameter<f64>,
}

impl MyDevice {
    pub async fn new_async(config: &MyDeviceConfig) -> Result<Arc<Self>> {
        // Connect to hardware, validate identity
        let device = Arc::new(Self {
            value: Parameter::new("value", 0.0)
                .with_description("Measured value")
                .with_unit("V"),
        });
        Ok(device)
    }
}

#[async_trait]
impl Readable for MyDevice {
    async fn read_value(&self) -> Result<f64> {
        // Query hardware and return value
        Ok(self.value.get())
    }
}

// --- Factory ---

pub struct MyDeviceFactory;

static CAPABILITIES: &[Capability] = &[Capability::Readable];

impl DriverFactory for MyDeviceFactory {
    fn driver_type(&self) -> &'static str { "mydevice" }
    fn name(&self) -> &'static str { "My Custom Device" }
    fn capabilities(&self) -> &'static [Capability] { CAPABILITIES }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let _: MyDeviceConfig = config.clone().try_into()?;
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MyDeviceConfig = config.try_into()?;
            let device = MyDevice::new_async(&cfg).await?;
            Ok(DeviceComponents::new().with_readable(device))
        })
    }
}
```

### 4. Register in Workspace

Add to root `Cargo.toml`:

```toml
[workspace]
members = [
    # ... existing crates
    "crates/driver-mydevice",
]
```

### 5. Register Factory

In `crates/hardware/src/registry.rs`, add:

```rust
#[cfg(feature = "mydevice")]
{
    use daq_driver_mydevice::MyDeviceFactory;
    registry.register_factory(Box::new(MyDeviceFactory));
}
```

And add feature to `crates/hardware/Cargo.toml`:

```toml
[features]
mydevice = ["daq-driver-mydevice"]

[dependencies.daq-driver-mydevice]
path = "../daq-driver-mydevice"
optional = true
```

---

## Capability Traits Reference

Select traits based on device type:

| Device Type | Required Traits | Optional Traits |
|-------------|-----------------|-----------------|
| **Motor/Stage** | `Movable` | `Parameterized` |
| **Sensor/Meter** | `Readable` | `Parameterized`, `WavelengthTunable` |
| **Camera** | `FrameProducer` | `Triggerable`, `ExposureControl` |
| **Laser** | `WavelengthTunable`, `ShutterControl` | `EmissionControl`, `Readable` |
| **Shutter** | `ShutterControl` | `Parameterized` |

### Movable (Motion Control)

```rust
#[async_trait]
impl Movable for MyStage {
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.position_param.set(position).await
    }

    async fn move_rel(&self, delta: f64) -> Result<()> {
        let current = self.position_param.get();
        self.position_param.set(current + delta).await
    }

    async fn position(&self) -> Result<f64> {
        self.position_param.read_from_hardware().await
    }

    async fn wait_until_settled(&self) -> Result<()> {
        // Poll until motion complete
        loop {
            if !self.is_moving().await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn stop(&self) -> Result<()> {
        // Send stop command to hardware
        Ok(())
    }
}
```

### Readable (Scalar Measurements)

```rust
#[async_trait]
impl Readable for MySensor {
    async fn read_value(&self) -> Result<f64> {
        // Query hardware and return measurement
        let response = self.port.query("READ?").await?;
        response.parse().map_err(Into::into)
    }
}
```

### FrameProducer (Cameras/Detectors)

```rust
#[async_trait]
impl FrameProducer for MyCamera {
    async fn acquire_frame(&self) -> Result<Frame> {
        // Capture and return a frame
        let buffer = self.internal_acquire().await?;
        Ok(Frame::new(buffer, self.width, self.height))
    }

    async fn start_streaming(&self, sender: FrameSender) -> Result<()> {
        // Start continuous acquisition
        Ok(())
    }

    async fn stop_streaming(&self) -> Result<()> {
        // Stop continuous acquisition
        Ok(())
    }

    fn frame_shape(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
```

---

## Serial Driver Pattern

For RS-232/RS-485 devices, follow this pattern:

### 1. Use `new_async()` as Primary Constructor

```rust
impl MySerialDevice {
    /// Internal constructor (test use only)
    pub fn new(port: SharedPort) -> Self {
        Self { port }
    }

    /// PRIMARY: Validates device identity before returning
    pub async fn new_async(port_path: &str) -> Result<Arc<Self>> {
        let port_path = port_path.to_string();
        let port = tokio::task::spawn_blocking(move || {
            tokio_serial::new(&port_path, 9600)
                .open_native_async()
                .context("Failed to open port")
        }).await??;

        let device = Arc::new(Self::new(Arc::new(Mutex::new(port))));

        // Validate device identity
        let response = device.query("*IDN?").await?;
        if !response.contains("EXPECTED_DEVICE") {
            anyhow::bail!("Wrong device on port: got '{}'", response);
        }

        Ok(device)
    }
}
```

### 2. Shared Port for Multi-Device Buses (RS-485)

```rust
// For ELL14-style RS-485 buses with multiple addresses
pub type SharedPort = Arc<Mutex<Box<dyn SerialPort>>>;

pub async fn get_or_open_port(port_path: &str) -> Result<SharedPort> {
    static PORTS: Lazy<Mutex<HashMap<String, SharedPort>>> =
        Lazy::new(|| Mutex::new(HashMap::new()));

    let mut ports = PORTS.lock().await;
    if let Some(port) = ports.get(port_path) {
        return Ok(port.clone());
    }

    let path = port_path.to_string();
    let port = tokio::task::spawn_blocking(move || {
        tokio_serial::new(&path, 9600).open_native_async()
    }).await??;

    let shared = Arc::new(Mutex::new(port));
    ports.insert(port_path.to_string(), shared.clone());
    Ok(shared)
}
```

### 3. Query Pattern with Timeout

```rust
async fn query(&self, command: &str) -> Result<String> {
    let mut port = self.port.lock().await;

    // Clear input buffer
    let mut discard = vec![0u8; 256];
    let _ = port.try_read(&mut discard);

    // Send command
    port.write_all(command.as_bytes()).await?;
    port.write_all(b"\r\n").await?;
    port.flush().await?;

    // Read response with timeout
    let mut buffer = vec![0u8; 1024];
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        port.read(&mut buffer)
    ).await;

    match result {
        Ok(Ok(n)) => {
            let response = String::from_utf8_lossy(&buffer[..n]);
            Ok(response.trim().to_string())
        }
        Ok(Err(e)) => Err(e.into()),
        Err(_) => anyhow::bail!("Timeout waiting for response"),
    }
}
```

---

## Parameter<T> (Reactive State)

**MANDATORY**: Use `Parameter<T>` instead of raw `Mutex<T>` for device state.

### Basic Usage

```rust
use daq_core::parameter::Parameter;

let wavelength = Parameter::new("wavelength_nm", 800.0)
    .with_description("Laser wavelength")
    .with_unit("nm")
    .with_range(690.0, 1040.0);
```

### Hardware Callbacks

```rust
use futures::future::BoxFuture;

// Write callback: called when parameter is set
wavelength.connect_to_hardware_write({
    let port = port.clone();
    move |val: f64| -> BoxFuture<'static, Result<()>> {
        Box::pin(async move {
            port.lock().await.write_all(
                format!("WAVELENGTH:{}\r\n", val).as_bytes()
            ).await?;
            Ok(())
        })
    }
});

// Read callback: called when reading from hardware
wavelength.connect_to_hardware_read({
    let port = port.clone();
    move || -> BoxFuture<'static, Result<f64>> {
        Box::pin(async move {
            let response = port.lock().await.query("WAVELENGTH?").await?;
            response.parse().map_err(Into::into)
        })
    }
});
```

### In Trait Methods

```rust
#[async_trait]
impl WavelengthTunable for MyLaser {
    async fn set_wavelength(&self, nm: f64) -> Result<()> {
        // Delegates to parameter, which calls hardware callback
        self.wavelength.set(nm).await
    }

    async fn wavelength(&self) -> Result<f64> {
        // Returns cached value (or reads from hardware)
        Ok(self.wavelength.get())
    }
}
```

---

## Declarative Config-Driven Drivers

For simple serial devices, use TOML configs in `config/devices/`:

```toml
# config/devices/mydevice.toml
[device]
name = "My Simple Device"
capabilities = ["Readable"]
manufacturer = "Acme Corp"
category = "sensor"

[connection]
type = "serial"
baud_rate = 9600
data_bits = 8
stop_bits = 1
parity = "None"
terminator_tx = "\r\n"
terminator_rx = "\r\n"

[parameters.value]
type = "float"
unit = "V"
description = "Measured voltage"

[commands.read_value]
template = "READ?"
expects_response = true
timeout_ms = 1000

[responses.value]
pattern = "^(?P<value>[\\d.]+)$"

[responses.value.fields.value]
type = "float"

[trait_mapping.Readable.read_value]
command = "read_value"
output_field = "value"
```

The registry auto-loads configs and creates factories at startup.

---

## DeviceComponents Builder

Return capabilities from your factory:

```rust
fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
    Box::pin(async move {
        let device = Arc::new(MyDevice::new().await?);

        // Single device implementing multiple traits
        Ok(DeviceComponents::new()
            .with_movable(device.clone())
            .with_parameterized(device.clone())
            .with_metadata(DeviceMetadata {
                name: "My Device".to_string(),
                description: Some("Custom instrument".to_string()),
                ..Default::default()
            }))
    })
}
```

For devices where different structs implement different capabilities:

```rust
let motor = Arc::new(MotorController::new().await?);
let encoder = Arc::new(EncoderReader::new().await?);

Ok(DeviceComponents::new()
    .with_movable(motor)
    .with_readable(encoder))
```

---

## Feature Flags

### In Driver Crate

```toml
[features]
default = []
serial = ["tokio-serial"]
hardware_tests = []  # For integration tests requiring real hardware
```

### In hardware

```toml
[features]
mydevice = ["dep:daq-driver-mydevice"]
maitai = ["thorlabs", "newport", "spectra_physics", "pvcam_hardware"]
```

### Conditional Registration

```rust
pub async fn register_all_factories(registry: &DeviceRegistry) -> Result<()> {
    // Always register mocks
    registry.register_factory(Box::new(MockStageFactory));
    registry.register_factory(Box::new(MockCameraFactory));

    // Hardware-specific (feature-gated)
    #[cfg(feature = "mydevice")]
    registry.register_factory(Box::new(MyDeviceFactory));

    Ok(())
}
```

---

## Testing Patterns

### Unit Tests (Mock Mode)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_factory_builds_device() {
        let config = toml::toml! {
            port = "/dev/null"
        };

        let factory = MyDeviceFactory;
        let result = factory.build(config.into()).await;
        assert!(result.is_ok());
    }
}
```

### Hardware Integration Tests

```rust
// In tests/mydevice_hardware.rs
#[cfg(feature = "hardware_tests")]
mod hardware_tests {
    use super::*;

    #[tokio::test]
    async fn test_real_device_connection() {
        let port = std::env::var("MYDEVICE_PORT")
            .unwrap_or("/dev/ttyUSB0".to_string());

        let device = MyDevice::new_async(&port).await;
        assert!(device.is_ok(), "Failed to connect: {:?}", device.err());
    }
}
```

Run hardware tests:

```bash
MYDEVICE_PORT=/dev/ttyUSB0 cargo test --features hardware_tests
```

---

## Troubleshooting

### Build Errors: Missing Traits

```
error[E0277]: the trait bound `MyDevice: Movable` is not satisfied
```

**Solution**: Implement the required trait or remove it from capabilities list.

### Runtime: Wrong Device on Port

**Symptom**: Commands fail or return garbage
**Solution**: Add identity validation in `new_async()`:

```rust
let response = device.query("*IDN?").await?;
if !response.contains("EXPECTED_MODEL") {
    anyhow::bail!("Wrong device: {}", response);
}
```

### Lock-Across-Await Deadlock

**Symptom**: Driver hangs on second operation
**Solution**: Don't hold MutexGuard across `.await`:

```rust
// WRONG
let guard = self.port.lock().await;
guard.write(cmd).await;  // Deadlock!

// CORRECT
let value = {
    let guard = self.port.lock().await;
    guard.some_sync_operation()
};
do_async_thing(value).await;
```

---

## Existing Driver Examples

| Driver | Path | Good Example Of |
|--------|------|-----------------|
| MockStage | `driver-mock/src/mock_stage.rs` | Basic factory pattern |
| MockCamera | `driver-mock/src/mock_camera.rs` | FrameProducer streaming |
| Ell14 (Thorlabs) | `driver-thorlabs/src/ell14.rs` | RS-485 shared bus |
| Esp300 (Newport) | `driver-newport/src/esp300.rs` | Multi-axis controller |
| Newport1830C | `driver-newport/src/newport_1830c.rs` | Simple query-response |
| MaiTai Laser | `driver-spectra-physics/src/maitai.rs` | Complex state machine |
| PVCAM | `driver-pvcam/src/lib.rs` | FFI/C library binding |

---

## Resources

- [DriverFactory Trait](crates/daq-core/src/driver.rs) - Core trait definition
- [Capability Traits](crates/daq-core/src/capabilities.rs) - All available traits
- [DeviceComponents](crates/daq-core/src/driver.rs) - Builder pattern
- [Registry](crates/hardware/src/registry.rs) - Factory registration
- [Config Examples](config/devices/) - Declarative driver configs
