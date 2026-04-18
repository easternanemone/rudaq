# Driver Builder Reference

Detailed reference for rust-daq driver development patterns.

## Complete Capability Trait Signatures

### Movable (Motion Control)

```rust
#[async_trait]
pub trait Movable: Send + Sync {
    /// Move to absolute position
    async fn move_abs(&self, position: f64) -> Result<()>;

    /// Move by relative amount
    async fn move_rel(&self, delta: f64) -> Result<()>;

    /// Get current position
    async fn position(&self) -> Result<f64>;

    /// Wait until motion is complete
    async fn wait_until_settled(&self) -> Result<()>;

    /// Emergency stop
    async fn stop(&self) -> Result<()>;
}
```

### Readable (Scalar Measurements)

```rust
#[async_trait]
pub trait Readable: Send + Sync {
    /// Read a scalar value from the device
    async fn read_value(&self) -> Result<f64>;
}
```

### FrameProducer (2D Image Acquisition)

```rust
#[async_trait]
pub trait FrameProducer: Send + Sync {
    /// Acquire a single frame
    async fn acquire_frame(&self) -> Result<Frame>;

    /// Start continuous streaming to sender
    async fn start_streaming(&self, sender: FrameSender) -> Result<()>;

    /// Stop continuous streaming
    async fn stop_streaming(&self) -> Result<()>;

    /// Get frame dimensions (width, height)
    fn frame_shape(&self) -> (u32, u32);
}
```

### Triggerable (External Trigger Support)

```rust
#[async_trait]
pub trait Triggerable: Send + Sync {
    /// Arm the device for triggering
    async fn arm(&self) -> Result<()>;

    /// Execute a software trigger
    async fn trigger(&self) -> Result<()>;

    /// Disarm the device
    async fn disarm(&self) -> Result<()>;

    /// Check if device is armed
    async fn is_armed(&self) -> Result<bool>;
}
```

### ExposureControl (Integration Time)

```rust
#[async_trait]
pub trait ExposureControl: Send + Sync {
    /// Set exposure time in seconds
    async fn set_exposure(&self, seconds: f64) -> Result<()>;

    /// Get current exposure time
    async fn exposure(&self) -> Result<f64>;

    /// Get valid exposure range (min, max)
    fn exposure_range(&self) -> (f64, f64);
}
```

### WavelengthTunable (Lasers, Monochromators)

```rust
#[async_trait]
pub trait WavelengthTunable: Send + Sync {
    /// Set wavelength in nanometers
    async fn set_wavelength(&self, nm: f64) -> Result<()>;

    /// Get current wavelength
    async fn wavelength(&self) -> Result<f64>;

    /// Get tuning range (min, max)
    fn wavelength_range(&self) -> (f64, f64);
}
```

### ShutterControl (Beam Shutter)

```rust
#[async_trait]
pub trait ShutterControl: Send + Sync {
    /// Open the shutter
    async fn open_shutter(&self) -> Result<()>;

    /// Close the shutter
    async fn close_shutter(&self) -> Result<()>;

    /// Check if shutter is open
    async fn is_shutter_open(&self) -> Result<bool>;
}
```

### EmissionControl (Laser Emission)

```rust
#[async_trait]
pub trait EmissionControl: Send + Sync {
    /// Enable emission
    async fn enable_emission(&self) -> Result<()>;

    /// Disable emission
    async fn disable_emission(&self) -> Result<()>;

    /// Check if emission is enabled
    async fn is_emission_enabled(&self) -> Result<bool>;
}
```

### Parameterized (Observable State)

```rust
pub trait Parameterized: Send + Sync {
    /// Get list of all observable parameters
    fn parameters(&self) -> Vec<&dyn Observable>;

    /// Get parameter by name
    fn parameter(&self, name: &str) -> Option<&dyn Observable>;
}
```

---

## Complete DriverFactory Implementation

```rust
use anyhow::Result;
use daq_core::driver::{Capability, DeviceComponents, DeviceMetadata, DriverFactory};
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;

/// Configuration parsed from TOML
#[derive(Debug, Clone, Deserialize)]
pub struct MyDeviceConfig {
    /// Serial port path
    pub port: String,

    /// Baud rate (default: 9600)
    #[serde(default = "default_baud")]
    pub baud_rate: u32,

    /// Device address for multi-drop buses
    #[serde(default)]
    pub address: Option<String>,

    /// Connection timeout in milliseconds
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_baud() -> u32 { 9600 }
fn default_timeout() -> u64 { 5000 }

/// Factory that creates MyDevice instances
pub struct MyDeviceFactory;

/// Static capability list (compile-time constant)
static MY_DEVICE_CAPABILITIES: &[Capability] = &[
    Capability::Readable,
    Capability::Parameterized,
];

impl DriverFactory for MyDeviceFactory {
    /// Type identifier matching TOML config "type" field
    fn driver_type(&self) -> &'static str {
        "mydevice"
    }

    /// Human-readable name for UI
    fn name(&self) -> &'static str {
        "My Custom Device"
    }

    /// Declared capabilities for matching
    fn capabilities(&self) -> &'static [Capability] {
        MY_DEVICE_CAPABILITIES
    }

    /// Validate configuration before building
    /// Called during config load, not at build time
    fn validate(&self, config: &toml::Value) -> Result<()> {
        // Parse config to catch errors early
        let cfg: MyDeviceConfig = config.clone().try_into()?;

        // Additional validation
        if cfg.baud_rate == 0 {
            anyhow::bail!("baud_rate must be non-zero");
        }

        Ok(())
    }

    /// Build device instance from configuration
    /// This is async and may perform I/O (opening ports, etc.)
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MyDeviceConfig = config.try_into()?;

            // Create device (validates identity)
            let device = MyDevice::new_async(&cfg).await?;

            // Build components bag with metadata
            Ok(DeviceComponents::new()
                .with_readable(device.clone())
                .with_parameterized(device)
                .with_metadata(DeviceMetadata {
                    name: "My Device".to_string(),
                    description: Some("Custom measurement device".to_string()),
                    manufacturer: Some("Acme Corp".to_string()),
                    model: Some("Model X".to_string()),
                    serial_number: None,
                    measurement_units: Some("V".to_string()),
                }))
        })
    }
}
```

---

## Hardware Configuration Files

### maitai_hardware.toml Example

```toml
# Hardware configuration for maitai machine
# Location: config/maitai_hardware.toml

[[devices]]
id = "laser"
type = "maitai"
enabled = true

[devices.config]
port = "/dev/ttyUSB5"
baud_rate = 115200

[[devices]]
id = "power_meter"
type = "newport_1830c"
enabled = true

[devices.config]
port = "/dev/ttyS0"
baud_rate = 9600

[[devices]]
id = "rotator_hwp"
type = "ell14"
enabled = true

[devices.config]
port = "/dev/ttyUSB1"
address = "2"

[[devices]]
id = "rotator_qwp"
type = "ell14"
enabled = true

[devices.config]
port = "/dev/ttyUSB1"
address = "3"

[[devices]]
id = "stage"
type = "esp300"
enabled = true

[devices.config]
port = "/dev/ttyUSB0"
baud_rate = 19200
axis = 1
```

---

## Declarative Config Schema

Full TOML schema for config-driven drivers:

```toml
# Device metadata
[device]
name = "Device Name"                    # Required
capabilities = ["Movable", "Readable"]  # Required: trait list
manufacturer = "Vendor"                 # Optional
model = "Model X"                       # Optional
category = "stage"                      # Optional: stage, sensor, camera, laser

# Connection settings
[connection]
type = "serial"                         # serial | tcp | usb
baud_rate = 9600                        # For serial
data_bits = 8                           # 5, 6, 7, 8
stop_bits = 1                           # 1, 2
parity = "None"                         # None, Odd, Even
flow_control = "None"                   # None, Hardware, Software
terminator_tx = "\r\n"                  # Command terminator
terminator_rx = "\r\n"                  # Response terminator
timeout_ms = 2000                       # Read timeout

# For RS-485 buses
[connection.bus]
type = "rs485"
address_format = "hex_char"             # hex_char | decimal | ascii

# Parameters (observable state)
[parameters.position]
type = "float"                          # float | int | bool | string
unit = "degrees"
description = "Current position"
range = [0.0, 360.0]                    # Optional: [min, max]
default = 0.0                           # Optional

[parameters.velocity]
type = "float"
unit = "deg/s"
range = [0.1, 100.0]

# Commands
[commands.move_absolute]
template = "${address}ma${position:08X}"  # Template with variables
expects_response = true
timeout_ms = 10000

[commands.move_absolute.retry]
max_retries = 3
initial_delay_ms = 500
backoff_multiplier = 2.0

[commands.get_position]
template = "${address}gp"
expects_response = true
timeout_ms = 1000

[commands.stop]
template = "${address}st"
expects_response = false

# Response parsing
[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{1,8})$"

[responses.position.fields.addr]
type = "string"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true

# Trait method mappings
[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_param = "position"
input_conversion = "degrees_to_pulses"

[trait_mapping.Movable.position]
command = "get_position"
output_conversion = "pulses_to_degrees"
output_field = "pulses"

[trait_mapping.Movable.stop]
command = "stop"

# Value conversions
[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"
constants = { pulses_per_degree = 398.222222 }

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"
constants = { pulses_per_degree = 398.222222 }
```

---

## Error Handling Patterns

### Custom Error Types

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MyDeviceError {
    #[error("Communication timeout after {0}ms")]
    Timeout(u64),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Device not ready: {0}")]
    NotReady(String),

    #[error("Hardware error code {code}: {message}")]
    HardwareError { code: u8, message: String },

    #[error("Serial port error: {0}")]
    SerialError(#[from] tokio_serial::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// Convert to anyhow::Result for trait compatibility
impl From<MyDeviceError> for anyhow::Error {
    fn from(e: MyDeviceError) -> Self {
        anyhow::anyhow!("{}", e)
    }
}
```

### Retry with Backoff

```rust
use tokio::time::{sleep, Duration};

async fn with_retry<T, F, Fut>(
    max_attempts: u32,
    initial_delay: Duration,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut delay = initial_delay;

    for attempt in 1..=max_attempts {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if attempt < max_attempts => {
                tracing::warn!(
                    "Attempt {}/{} failed: {}. Retrying in {:?}",
                    attempt, max_attempts, e, delay
                );
                sleep(delay).await;
                delay *= 2; // Exponential backoff
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!()
}
```

---

## Testing Strategies

### Mock Port for Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use tokio::sync::Mutex;

    /// Mock serial port for testing
    struct MockPort {
        responses: Mutex<VecDeque<String>>,
        commands: Mutex<Vec<String>>,
    }

    impl MockPort {
        fn new() -> Self {
            Self {
                responses: Mutex::new(VecDeque::new()),
                commands: Mutex::new(Vec::new()),
            }
        }

        fn add_response(&self, response: &str) {
            self.responses.blocking_lock().push_back(response.to_string());
        }

        async fn query(&self, cmd: &str) -> Result<String> {
            self.commands.lock().await.push(cmd.to_string());
            self.responses
                .lock()
                .await
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("No mock response"))
        }
    }

    #[tokio::test]
    async fn test_read_value() {
        let port = Arc::new(MockPort::new());
        port.add_response("123.45");

        let device = MyDevice::with_mock_port(port.clone());
        let value = device.read_value().await.unwrap();

        assert!((value - 123.45).abs() < 0.001);
        assert_eq!(port.commands.lock().await[0], "READ?");
    }
}
```

### Integration Test with Real Hardware

```rust
// tests/integration/mydevice_hardware.rs

#[cfg(feature = "hardware_tests")]
mod hardware {
    use daq_driver_mydevice::MyDevice;

    fn get_port() -> String {
        std::env::var("MYDEVICE_PORT")
            .unwrap_or_else(|_| "/dev/ttyUSB0".to_string())
    }

    #[tokio::test]
    async fn test_connection() {
        let device = MyDevice::new_async(&get_port()).await;
        assert!(device.is_ok(), "Connection failed: {:?}", device.err());
    }

    #[tokio::test]
    async fn test_identity() {
        let device = MyDevice::new_async(&get_port()).await.unwrap();
        let identity = device.identify().await.unwrap();
        assert!(identity.contains("MyDevice"), "Wrong identity: {}", identity);
    }

    #[tokio::test]
    async fn test_read_value() {
        let device = MyDevice::new_async(&get_port()).await.unwrap();
        let value = device.read_value().await.unwrap();
        assert!(value.is_finite(), "Read returned NaN/Inf");
    }
}
```

---

## Logging Best Practices

```rust
use tracing::{debug, error, info, instrument, trace, warn};

impl MyDevice {
    #[instrument(skip(self), fields(port = %self.port_path))]
    pub async fn read_value(&self) -> Result<f64> {
        trace!("Sending READ command");

        let response = self.query("READ?").await.map_err(|e| {
            error!("Query failed: {}", e);
            e
        })?;

        debug!("Raw response: {:?}", response);

        let value: f64 = response.trim().parse().map_err(|e| {
            warn!("Parse error for '{}': {}", response, e);
            anyhow::anyhow!("Invalid response: {}", response)
        })?;

        info!("Read value: {}", value);
        Ok(value)
    }
}
```

Enable tracing in tests:

```rust
#[tokio::test]
async fn test_with_logging() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();

    // ... test code
}
```
