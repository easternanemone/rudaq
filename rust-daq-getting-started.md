# Getting Started Guide: Rust Scientific Data Acquisition Application

## Prerequisites

### Required Tools
1. **Rust Toolchain** (latest stable)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source ~/.cargo/env
   ```

2. **Additional Tools**
   ```bash
   # For cross-compilation and embedded targets
   rustup target add x86_64-pc-windows-gnu
   rustup target add aarch64-apple-darwin
   
   # Development tools
   cargo install cargo-watch
   cargo install cargo-edit
   cargo install cargo-audit
   ```

3. **System Dependencies**
   ```bash
   # Ubuntu/Debian
   sudo apt-get install build-essential pkg-config libssl-dev libusb-1.0-0-dev
   
   # macOS
   brew install pkg-config openssl libusb
   
   # Windows (using vcpkg)
   vcpkg install openssl libusb
   ```

## Project Setup

### 1. Initialize the Project
```bash
cargo new scientific-daq --bin
cd scientific-daq
```

### 2. Project Structure
```
scientific-daq/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── app.rs
│   ├── app_actor.rs
│   ├── messages.rs
│   ├── core.rs
│   ├── instrument/
│   ├── data/
│   └── gui/
├── crates/
│   └── daq-core/
├── config/
└── tests/
```

### 3. Core Dependencies (Cargo.toml)
```toml
[package]
name = "scientific-daq"
version = "0.1.0"
edition = "2021"

[dependencies]
# Async runtime
tokio = { version = "1.35", features = ["full"] }
async-trait = "0.1"

# GUI framework
eframe = { version = "0.25", features = ["persistence"] }
egui = "0.25"
egui_plot = "0.25"

# Data handling
ndarray = "0.15"
polars = { version = "0.36", features = ["lazy", "temporal", "csv-file"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Scientific computing
nalgebra = "0.32"
plotters = "0.3"

# Instrument control
scpi = "1.0"
serialport = "4.3"

# Configuration and logging
config = "0.14"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Error handling
thiserror = "1.0"
anyhow = "1.0"

# Data structures
dashmap = "5.5"
parking_lot = "0.12"
ringbuf = "0.3"

# File I/O
hdf5 = { version = "0.8", optional = true }
arrow = { version = "52.0", optional = true }

[features]
default = ["hdf5-support", "arrow-support"]
hdf5-support = ["hdf5"]
arrow-support = ["arrow"]

[dev-dependencies]
tokio-test = "0.4"
tempfile = "3.8"
```

## V2 Architecture and Actor Model

The `rust-daq` application has been refactored to use a modern, actor-based architecture. This design eliminates performance bottlenecks and provides a more robust and scalable foundation for the application. For more details, see the "V2 Architecture Migration" section in the `rust-daq-app-architecture.md` file.

## Initial Implementation

### 1. Main Application Structure (src/main.rs)
```rust
use rust_daq::{app::DaqApp, config::Settings, gui::Gui, log_capture::LogBuffer};
use std::sync::Arc;
use eframe::egui;

fn main() -> anyhow::Result<()> {
    // ... initialization ...

    let settings = Arc::new(Settings::load()?);
    let app = DaqApp::new(
        settings.clone(),
        // ... registries ...
    )?;

    // ... eframe setup ...
}
```

### 2. Core Application (src/app.rs and src/app_actor.rs)
The core application logic is now split between `src/app.rs` and `src/app_actor.rs`. `DaqApp` in `src/app.rs` is a lightweight handle that provides access to the application's functionality, while `DaqManagerActor` in `src/app_actor.rs` is the actor that owns and manages the application state.
```rust
// src/app.rs
pub struct DaqApp {
    pub command_tx: mpsc::Sender<DaqCommand>,
    // ...
}

// src/app_actor.rs
pub struct DaqManagerActor {
    settings: Settings,
    instruments: HashMap<String, InstrumentHandle>,
    // ...
}
```

### 3. Configuration Module (src/utils/config.rs)
```rust
use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    pub application: ApplicationConfig,
    pub instruments: HashMap<String, InstrumentConfig>,
    pub data_acquisition: DataAcquisitionConfig,
    pub gui: GuiConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ApplicationConfig {
    pub name: String,
    pub version: String,
    pub log_level: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InstrumentConfig {
    pub plugin: String,
    pub connection: ConnectionConfig,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConnectionConfig {
    pub protocol: String,
    pub address: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DataAcquisitionConfig {
    pub buffer_size: usize,
    pub sample_rate: f64,
    pub auto_save: bool,
    pub save_format: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GuiConfig {
    pub theme: String,
    pub update_rate: f64,
    pub plot_buffer_size: usize,
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        let config = Config::builder()
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name("config/local").required(false))
            .add_source(Environment::with_prefix("SCIDAQ"))
            .build()?;
        
        config.try_deserialize()
    }
}
```

### 4. Basic Configuration Files

**config/default.toml**
```toml
[application]
name = "Scientific DAQ"
version = "0.1.0"
log_level = "info"

[data_acquisition]
buffer_size = 10000
sample_rate = 1000.0
auto_save = true
save_format = "hdf5"

[gui]
theme = "dark"
update_rate = 30.0
plot_buffer_size = 1000

[instruments.mock_instrument]
plugin = "mock"
[instruments.mock_instrument.connection]
protocol = "tcp"
address = "localhost:5555"
timeout_ms = 1000
[instruments.mock_instrument.parameters]
channels = 4
range = [-10.0, 10.0]
```

### 5. Logging Setup (src/utils/logging.rs)
```rust
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_thread_ids(true)
                .with_file(true)
                .with_line_number(true)
        )
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info"))
        )
        .init();
    
    Ok(())
}
```

## Development Workflow

### 1. Running the Application
```bash
# Development mode with hot reload
cargo watch -x run

# Release mode
cargo run --release

# With specific features
cargo run --features hdf5-support
```

### 2. Testing
```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_instrument_initialization

# Run integration tests
cargo test --test integration
```

### 3. Code Quality
```bash
# Format code
cargo fmt

# Check for issues
cargo clippy

# Audit dependencies
cargo audit

# Check for unused dependencies
cargo machete
```

### 4. Documentation
```bash
# Generate documentation
cargo doc --open

# Check documentation
cargo doc --no-deps --document-private-items
```

## Next Steps

1. **Implement Core Traits**: Start with the basic `Instrument` and `DataProcessor` traits
2. **Create Mock Instrument**: Implement a simple mock instrument for testing
3. **Basic GUI**: Create the main window with instrument controls
4. **Data Pipeline**: Implement basic data flow from instrument to GUI
5. **Configuration Loading**: Test configuration system with different settings
6. **Plugin System**: Implement dynamic plugin loading
7. **Real Instruments**: Add support for actual hardware instruments

## Common Patterns

### Error Handling
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),
    
    #[error("Instrument error: {0}")]
    Instrument(String),
    
    #[error("GUI error: {0}")]
    Gui(String),
}

type Result<T> = std::result::Result<T, AppError>;
```

### Async Patterns
```rust
use tokio::sync::{mpsc, RwLock};
use std::sync::Arc;

// Shared state pattern
pub type SharedState<T> = Arc<RwLock<T>>;

// Channel pattern for data streaming
pub fn create_data_channel<T>() -> (mpsc::Sender<T>, mpsc::Receiver<T>) {
    mpsc::channel(1000)
}
```

This getting started guide provides the foundation for building a robust scientific data acquisition application in Rust. The modular structure allows for incremental development while maintaining clean separation of concerns.