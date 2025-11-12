# V4 Tracing Infrastructure

**Status**: ✅ Implemented (bd-fxb7)
**Location**: `src/tracing_v4.rs`

## Overview

The V4 tracing infrastructure uses the [tracing](https://docs.rs/tracing/) and [tracing-subscriber](https://docs.rs/tracing-subscriber/) crates to provide structured, async-aware logging for the V4 architecture.

### Key Features

- **Structured Logging**: Fields and context instead of string interpolation
- **Async-Aware**: Automatic context propagation across async boundaries
- **Multiple Formats**: Pretty (development), Compact (production), JSON (log aggregation)
- **Environment Filtering**: `RUST_LOG` support for fine-grained control
- **Integration with V4Config**: Reads log level from configuration
- **Span Events**: Track operation entry/exit for performance analysis

## Quick Start

### Basic Usage

```rust
use rust_daq::{config_v4::V4Config, tracing_v4};
use tracing::{info, warn, error, debug};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = V4Config::load()?;

    // Initialize tracing
    tracing_v4::init_from_config(&config)?;

    // Use tracing macros
    info!("Application started");
    warn!(component = "instrument", "Connection timeout");
    error!(error = ?std::io::Error::from(std::io::ErrorKind::NotFound), "File not found");

    Ok(())
}
```

### Custom Configuration

```rust
use rust_daq::tracing_v4::{self, TracingConfig, OutputFormat};
use tracing::Level;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = TracingConfig::new(Level::DEBUG)
        .with_format(OutputFormat::Json)
        .with_span_events(false);

    tracing_v4::init(config)?;

    Ok(())
}
```

## Configuration Integration

The tracing system integrates with V4Config to read the log level:

**config/config.v4.toml:**
```toml
[application]
name = "Rust DAQ V4"
log_level = "info"  # trace, debug, info, warn, error
```

**Rust code:**
```rust
let config = V4Config::load()?;
tracing_v4::init_from_config(&config)?;  // Uses config.application.log_level
```

### Environment Override

Override the configured log level with environment variables:

```bash
# Override V4Config log level
RUST_DAQ_APPLICATION_LOG_LEVEL=debug cargo run

# Fine-grained filtering with RUST_LOG
RUST_LOG=rust_daq=debug cargo run
RUST_LOG=rust_daq::instrument=trace,rust_daq::data=debug cargo run
```

## Output Formats

### Pretty Format (Development)

Colorized, pretty-printed output for human readability during development:

```
  2024-01-15T10:30:45.123456Z  INFO rust_daq: Application started
    at src/main.rs:42

  2024-01-15T10:30:45.234567Z  WARN rust_daq::instrument: Connection timeout
    at src/instrument/mod.rs:123
    in rust_daq::instrument::connect with instrument: "ESP300"
```

### Compact Format (Production)

Concise, no-color output optimized for production logs:

```
2024-01-15T10:30:45.123Z INFO rust_daq: Application started
2024-01-15T10:30:45.234Z WARN rust_daq::instrument: Connection timeout instrument="ESP300"
```

### JSON Format (Log Aggregation)

Machine-parseable JSON for structured log aggregation systems (Elasticsearch, CloudWatch, etc.):

```json
{"timestamp":"2024-01-15T10:30:45.123456Z","level":"INFO","fields":{"message":"Application started"},"target":"rust_daq"}
{"timestamp":"2024-01-15T10:30:45.234567Z","level":"WARN","fields":{"message":"Connection timeout","instrument":"ESP300"},"target":"rust_daq::instrument"}
```

## API Reference

### Types

#### `OutputFormat`

```rust
pub enum OutputFormat {
    /// Pretty-printed format with colors (for development)
    Pretty,
    /// Compact format without colors (for production)
    Compact,
    /// JSON format for structured logging (for log aggregation)
    Json,
}
```

#### `TracingConfig`

```rust
pub struct TracingConfig {
    /// Log level (trace, debug, info, warn, error)
    pub level: Level,
    /// Output format
    pub format: OutputFormat,
    /// Whether to include span events (ENTER, EXIT, CLOSE)
    pub with_span_events: bool,
    /// Whether to include file and line numbers
    pub with_file_and_line: bool,
    /// Whether to include thread IDs
    pub with_thread_ids: bool,
    /// Whether to include thread names
    pub with_thread_names: bool,
    /// Whether to enable ANSI colors (only for Pretty format)
    pub with_ansi: bool,
}
```

### Functions

#### `init_from_config`

```rust
pub fn init_from_config(config: &V4Config) -> Result<(), String>
```

Initialize tracing from V4 configuration. This is the **recommended** initialization method for V4 applications.

**Example:**
```rust
let config = V4Config::load()?;
tracing_v4::init_from_config(&config)?;
```

#### `init`

```rust
pub fn init(config: TracingConfig) -> Result<(), String>
```

Initialize tracing with custom configuration. Use this for more fine-grained control over tracing setup.

**Example:**
```rust
let config = TracingConfig::new(Level::DEBUG)
    .with_format(OutputFormat::Json)
    .with_span_events(false);
tracing_v4::init(config)?;
```

### Builder Methods

#### `TracingConfig::new`

```rust
pub fn new(level: Level) -> Self
```

Create a new tracing config with the specified log level and default settings.

#### `TracingConfig::from_v4_config`

```rust
pub fn from_v4_config(config: &V4Config) -> Result<Self, String>
```

Create tracing config from V4 configuration, reading the log level from `config.application.log_level`.

#### `with_format`

```rust
pub fn with_format(mut self, format: OutputFormat) -> Self
```

Set the output format (Pretty, Compact, or Json).

#### `with_span_events`

```rust
pub fn with_span_events(mut self, enabled: bool) -> Self
```

Enable or disable span events (NEW, CLOSE). Useful for performance analysis.

#### `with_ansi`

```rust
pub fn with_ansi(mut self, enabled: bool) -> Self
```

Enable or disable ANSI colors (only affects Pretty format).

## Structured Logging

### Basic Structured Fields

```rust
use tracing::info;

info!(
    instrument = "mock_power_meter",
    reading = 42.5,
    unit = "mW",
    "Instrument reading received"
);
```

Output:
```
INFO rust_daq: Instrument reading received instrument="mock_power_meter" reading=42.5 unit="mW"
```

### Error Logging

```rust
use tracing::error;
use std::io;

let err = io::Error::new(io::ErrorKind::NotFound, "device not found");
error!(
    error = ?err,  // Debug formatting
    device_path = "/dev/ttyUSB0",
    "Failed to open serial device"
);
```

### Debugging with Display and Debug

```rust
use tracing::debug;

// Display formatting (%value)
debug!(value = %some_value, "Processing value");

// Debug formatting (?value)
debug!(value = ?some_complex_struct, "Inspecting struct");
```

## Spans for Async Context

Spans provide hierarchical context for operations, especially useful in async code:

### Basic Span Usage

```rust
use tracing::{info, info_span};

fn process_data() {
    let span = info_span!("data_processing", session_id = "abc-123");
    let _enter = span.enter();  // Enter span context

    info!("Processing data");  // Logged within span context
    // ... processing work ...

}  // Span automatically exits when _enter is dropped
```

### Async Spans

```rust
use tracing::{info, instrument};

#[instrument]  // Automatically creates a span with function name
async fn fetch_data(url: &str) -> Result<String, Error> {
    info!("Fetching data from URL");
    // ... async work ...
    Ok(response)
}
```

### Manual Span Management

```rust
use tracing::info_span;

async fn complex_operation() {
    let span = info_span!("complex_op", operation_id = 42);
    let _guard = span.enter();

    // All logs here will include the span context
    info!("Starting operation");

    // Spawn tasks that inherit span context
    tokio::spawn(async {
        info!("Background task running");  // Still in span context
    });
}
```

## Log Levels

### Level Hierarchy

From most to least verbose:

1. **TRACE**: Very detailed, low-level information
2. **DEBUG**: Debugging information useful during development
3. **INFO**: General informational messages about application flow
4. **WARN**: Warning messages for potentially problematic situations
5. **ERROR**: Error messages for failures and exceptions

### Level Selection Guidelines

- **Development**: Use `DEBUG` or `TRACE` for detailed visibility
- **Testing**: Use `INFO` for test output clarity
- **Production**: Use `INFO` or `WARN` to reduce log volume
- **Troubleshooting**: Temporarily increase to `DEBUG` or `TRACE`

## Environment Variable Filtering

The `RUST_LOG` environment variable provides fine-grained control:

```bash
# Set global log level
RUST_LOG=debug cargo run

# Filter by crate
RUST_LOG=rust_daq=debug cargo run

# Filter by module
RUST_LOG=rust_daq::instrument=trace cargo run

# Multiple targets with different levels
RUST_LOG=rust_daq=info,rust_daq::instrument=debug,rust_daq::data=trace cargo run

# Use exact target matching
RUST_LOG=rust_daq::instrument::esp300=trace cargo run
```

## Performance Considerations

### Span Events Overhead

Span events (NEW, CLOSE) add overhead. Disable for production if not needed:

```rust
let config = TracingConfig::from_v4_config(&v4_config)?
    .with_span_events(false);
tracing_v4::init(config)?;
```

### Field Allocation

Structured fields are allocated per-event. For high-frequency logging, consider:

```rust
// Less efficient: allocates strings
info!(msg = format!("Processed {} items", count));

// More efficient: direct formatting
info!(count = count, "Processed items");
```

### Conditional Logging

Use `debug_span!` and `trace!` macros - they compile to no-ops when disabled:

```rust
use tracing::{debug, debug_span};

// Only executed when DEBUG level is enabled
debug!(expensive_computation = ?compute_debug_info(), "Debug data");

// Span overhead only when enabled
let span = debug_span!("expensive_operation");
let _enter = span.enter();
```

## Integration with V4 Architecture

### Actor System

Tracing integrates with Kameo actors for distributed logging:

```rust
use tracing::{info, instrument};

#[instrument(skip(self))]
async fn handle_message(&mut self, msg: Message) -> Result<(), Error> {
    info!(message_type = ?msg, "Handling actor message");
    // ... actor logic ...
    Ok(())
}
```

### Instrument Drivers

Instrument implementations use tracing for debugging:

```rust
use tracing::{debug, info, warn};

impl InstrumentV3 for ESP300 {
    async fn connect(&mut self) -> Result<(), DaqError> {
        let span = info_span!("esp300_connect", device = %self.device_path);
        let _enter = span.enter();

        info!("Connecting to ESP300");
        debug!(timeout_ms = self.timeout_ms, "Connection parameters");

        // ... connection logic ...

        info!("ESP300 connected successfully");
        Ok(())
    }
}
```

### Storage Subsystem

Storage backends log write operations:

```rust
use tracing::{info, warn};

async fn write_batch(&mut self, data: &[DataPoint]) -> Result<(), Error> {
    info!(
        backend = "hdf5",
        batch_size = data.len(),
        "Writing data batch"
    );

    if data.len() > 1000 {
        warn!(batch_size = data.len(), "Large batch may impact performance");
    }

    // ... write logic ...
    Ok(())
}
```

## Testing

### Unit Tests

The tracing system includes comprehensive tests:

```bash
cargo test tracing_v4
```

Tests cover:
- Log level parsing (case insensitivity, invalid levels)
- Configuration building and validation
- Integration with V4Config

### Test Output

Use `tracing-test` crate for capturing logs in tests:

```rust
use tracing_test::traced_test;

#[traced_test]
#[test]
fn test_with_logs() {
    tracing::info!("This log is captured in test output");
    assert!(true);
}
```

## Example

See `examples/tracing_v4_demo.rs` for a complete demonstration:

```bash
# Run with default (info level)
cargo run --example tracing_v4_demo

# Run with debug level
RUST_DAQ_APPLICATION_LOG_LEVEL=debug cargo run --example tracing_v4_demo

# Run with trace level
RUST_DAQ_APPLICATION_LOG_LEVEL=trace cargo run --example tracing_v4_demo

# Use RUST_LOG for fine-grained control
RUST_LOG=rust_daq=debug cargo run --example tracing_v4_demo
```

## Migration from V1/V2/V3

### Old System (env_logger)

```rust
use env_logger;
use log::{info, warn};

env_logger::init();
info!("Application started");
warn!("Connection timeout");
```

### New System (tracing)

```rust
use rust_daq::{config_v4::V4Config, tracing_v4};
use tracing::{info, warn};

let config = V4Config::load()?;
tracing_v4::init_from_config(&config)?;

info!("Application started");
warn!(component = "instrument", "Connection timeout");
```

### Key Differences

1. **Configuration**: V1/V2/V3 used `RUST_LOG` only, V4 uses config file + `RUST_LOG` override
2. **Structured Fields**: V4 supports structured fields, V1/V2/V3 only string messages
3. **Async Context**: V4 automatically propagates context across async boundaries
4. **Multiple Formats**: V4 supports Pretty, Compact, and JSON formats
5. **Span Events**: V4 includes span lifecycle events for performance analysis

## Related Issues

- **bd-fxb7**: Initialize Tracing Infrastructure ✅ (this document)
- **bd-rir3**: Implement figment-based Configuration System ✅ (provides log_level)
- **bd-662d**: Create V4 Core Crate (will use this tracing system)

## References

- [tracing Documentation](https://docs.rs/tracing/)
- [tracing-subscriber Documentation](https://docs.rs/tracing-subscriber/)
- [Tokio Tracing Guide](https://tokio.rs/tokio/topics/tracing)
- [V4 Config System](./CONFIG_SYSTEM.md)
