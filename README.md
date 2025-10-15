# Rust DAQ: Modular Scientific Data Acquisition

A modular, high-performance, and type-safe scientific data acquisition (DAQ) application written in Rust.

## Features

- **Asynchronous Core**: Built on the `tokio` async runtime for high-performance, non-blocking I/O with scientific instruments.
- **Modular & Extensible**: Plugin-based architecture for adding new instruments, data processors, and storage formats.
- **Real-time GUI**: A responsive user interface built with `egui` for real-time data visualization and control.
- **Robust Data Handling**:
  - Ring buffer-based streaming for efficient memory usage.
  - Support for multiple data storage backends (CSV, HDF5, Arrow).
- **Type Safety & Performance**: Leverages Rust's safety guarantees to prevent common bugs in concurrent systems.

## Architecture Overview

The application consists of several key components:

- **Core**: Defines the essential traits (`Instrument`, `DataProcessor`, `StorageWriter`) and data types.
- **Instruments**: Responsible for communicating with hardware. They run on a dedicated `tokio` thread pool.
- **Data Pipeline**: Data from instruments is streamed through `tokio::sync::broadcast` channels. This allows multiple consumers (GUI, processors, storage) to access the data concurrently.
- **GUI**: The `egui`-based interface runs on the main thread and communicates with the async backend via channels.
- **Plugins**: A static plugin system allows for compile-time registration of new components.

### Performance Architecture

**Rust Core** (current):
- Real-time data acquisition and processing (10-100x faster than Python)
- Thread-safe concurrent operations with zero-cost abstractions
- Memory-efficient ring buffers for continuous streaming
- Async I/O for non-blocking hardware communication

**Python Integration** (planned):
- High-level experiment scripting and orchestration
- Jupyter notebook support for rapid prototyping
- NumPy/Pandas integration for analysis workflows
- PyO3 bindings expose Rust performance to Python users

This hybrid architecture follows the proven pattern of NumPy, TensorFlow, and PyTorch: high-performance compiled core with accessible scripting layer.

## Getting Started

### Prerequisites

- **Rust Toolchain**: Install Rust via [rustup](https://rustup.rs/).
- **System Dependencies**:
  - **HDF5**: To enable the HDF5 storage backend, you need the HDF5 library installed.
    - On Ubuntu/Debian: `sudo apt-get install libhdf5-dev`
    - On macOS: `brew install hdf5`

### Building and Running

1.  **Clone the repository:**
    ```sh
    git clone https://github.com/TheFermiSea/rust-daq.git
    cd rust-daq
    ```

2.  **Build the application:**
    - To build with default features (CSV storage, Serial instruments):
      ```sh
      cargo build --release
      ```
    - To build with all features (HDF5, Arrow, etc.):
      ```sh
      cargo build --release --features full
      ```

3.  **Run the application:**
    ```sh
    cargo run --release
    ```

### Configuration

Application settings can be configured in `config/default.toml`. This includes log levels, instrument parameters, and default storage paths.

## Directory Structure

```
.
├── config/
│   └── default.toml    # Default configuration
├── src/
│   ├── main.rs         # Application entry point
│   ├── lib.rs          # Library root
│   ├── app.rs          # Core application state
│   ├── core.rs         # Core traits and types
│   ├── config.rs       # Configuration loading
│   ├── error.rs        # Custom error types
│   ├── gui.rs          # Egui implementation
│   ├── data/           # Data processing & storage
│   └── instrument/     # Instrument trait and implementations
└── tests/
    └── integration.rs  # Integration tests
```

For contributor workflow details (worktrees, beads tracker, Jules isolation), see [AGENTS.md](AGENTS.md) and [BD_JULES_INTEGRATION.md](BD_JULES_INTEGRATION.md).

## Roadmap

### Current Phase: Rust Core Implementation
- ✅ Async runtime with Tokio
- ✅ Real-time data processors (FFT, IIR, Trigger)
- ✅ Multiple storage backends (CSV, HDF5, Arrow)
- ✅ Instrument drivers (ESP300, MaiTai, Newport 1830C, SCPI, VISA)
- ✅ Real-time GUI with egui
- 🔄 Comprehensive test coverage (in progress)
- 🔄 Documentation improvements (in progress)

### Next Phase: Python Integration (Q2 2025)
- 🔮 PyO3 bindings for core functionality
- 🔮 Python package (`pip install rust-daq`)
- 🔮 Jupyter kernel integration
- 🔮 High-level experiment scripting API
- 🔮 NumPy/Pandas data interoperability

### Future: Ecosystem Development
- 🔮 Plugin marketplace for community instruments
- 🔮 Visual experiment builder
- 🔮 Distributed multi-computer experiments
- 🔮 Cloud experiment orchestration

**Design Philosophy**: Maintain Rust performance for real-time operations while providing Python accessibility for experiment design and analysis. See [FINAL_CONSENSUS_REPORT.md](FINAL_CONSENSUS_REPORT.md) for detailed strategy and performance boundaries.

## Examples

### Basic Usage

This example demonstrates how to instantiate the `DaqApp`, acquire data from a mock instrument, and print it to the console.

```rust
use rust_daq::app::DaqApp;
use rust_daq::config::Settings;
use rust_daq::instrument::{InstrumentRegistry, mock::MockInstrument};
use rust_daq::data::registry::ProcessorRegistry;
use rust_daq::log_capture::LogBuffer;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Create a default configuration
    let settings = Arc::new(Settings::new(None)?);

    // 2. Create instrument and processor registries
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);
    let processor_registry = Arc::new(ProcessorRegistry::new());

    // 3. Set up a logger (optional)
    let log_buffer = LogBuffer::new();

    // 4. Create the main application
    let app = DaqApp::new(settings, instrument_registry, processor_registry, log_buffer)?;

    // 5. Spawn a mock instrument
    app.with_inner(|inner| {
        inner.spawn_instrument("mock").unwrap();
    });

    // 6. Subscribe to the data stream
    let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

    // 7. Receive and print a few data points
    for _ in 0..5 {
        if let Ok(data_point) = data_rx.recv().await {
            println!("Received data: {:?}", data_point);
        }
    }

    // 8. Shut down the application
    app.shutdown();

    Ok(())
}
```

### Instrument Setup

This example shows how to create an `InstrumentRegistry`, register a mock instrument, and configure it with specific parameters.

```rust
use rust_daq::app::DaqApp;
use rust_daq::config::{self, Settings};
use rust_daq::instrument::{InstrumentRegistry, mock::MockInstrument};
use rust_daq::data::registry::ProcessorRegistry;
use rust_daq::log_capture::LogBuffer;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Define custom settings for the instrument
    let mut settings = Settings::new(None)?;
    let mut instrument_config = toml::map::Map::new();
    instrument_config.insert("type".to_string(), toml::Value::String("mock".to_string()));
    instrument_config.insert("name".to_string(), toml::Value::String("My Mock Instrument".to_string()));
    let mut params = toml::map::Map::new();
    params.insert("channel_count".to_string(), toml::Value::Integer(4));
    params.insert("sample_rate_hz".to_string(), toml::Value::Float(1000.0));
    instrument_config.insert("params".to_string(), toml::Value::Table(params));
    settings.instruments.insert(
        "my_mock".to_string(),
        toml::Value::Table(instrument_config),
    );
    let settings = Arc::new(settings);

    // 2. Create and configure the instrument registry
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);
    let processor_registry = Arc::new(ProcessorRegistry::new());
    let log_buffer = LogBuffer::new();

    // 3. Create the app and start data acquisition
    let app = DaqApp::new(settings, instrument_registry, processor_registry, log_buffer)?;
    app.with_inner(|inner| {
        inner.spawn_instrument("my_mock").unwrap();
    });

    let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

    for _ in 0..10 {
        if let Ok(data_point) = data_rx.recv().await {
            println!("Received data from {}: {}", data_point.channel, data_point.value);
        }
    }

    app.shutdown();
    Ok(())
}
```

### Data Processing

This example demonstrates how to add a simple data processor to the pipeline. In this case, we'll add an FFT processor to transform time-domain data into frequency-domain data.

```rust
use rust_daq::app::DaqApp;
use rust_daq::config::Settings;
use rust_daq::core::DataProcessor;
use rust_daq::data::fft::{FFTConfig, FFTProcessor};
use rust_daq::data::registry::ProcessorRegistry;
use rust_daq::instrument::{mock::MockInstrument, InstrumentRegistry};
use rust_daq::log_capture::LogBuffer;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let settings = Arc::new(Settings::new(None)?);

    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);

    let processor_registry = Arc::new(ProcessorRegistry::new());
    let log_buffer = LogBuffer::new();

    let app = DaqApp::new(settings, instrument_registry, processor_registry, log_buffer)?;

    app.with_inner(|inner| {
        inner.spawn_instrument("mock").unwrap();
    });

    let mut data_rx = app.with_inner(|inner| inner.data_sender.subscribe());

    // Create an FFT processor
    let config = FFTConfig {
        window_size: 1024,
        overlap: 512,
        sampling_rate: 1024.0,
    };
    let mut fft_processor = FFTProcessor::new(config);

    // Collect data and process it
    let mut collected_data = Vec::new();
    for _ in 0..1024 {
        if let Ok(data_point) = data_rx.recv().await {
            collected_data.push(data_point);
        }
    }

    let spectrum = fft_processor.process(&collected_data);
    println!("FFT processed {} frequency bins", spectrum.len());

    app.shutdown();
    Ok(())
}
```

### Launching the GUI

This example shows how to launch the full GUI application.

```rust
use anyhow::Result;
use eframe::NativeOptions;
use log::{info, LevelFilter};
use rust_daq::{
    app::DaqApp,
    config::Settings,
    data::registry::ProcessorRegistry,
    gui::Gui,
    instrument::{mock::MockInstrument, InstrumentRegistry},
    log_capture::{LogBuffer, LogCollector},
};
use std::sync::Arc;

fn main() -> Result<()> {
    // Initialize logging
    let log_buffer = LogBuffer::new();
    let gui_logger = LogCollector::new(log_buffer.clone());
    let log_level_filter = LevelFilter::Info;
    let console_logger = env_logger::Builder::new()
        .filter_level(log_level_filter)
        .build();
    log::set_max_level(log_level_filter);
    multi_log::MultiLogger::init(
        vec![Box::new(console_logger), Box::new(gui_logger)],
        log::Level::Info,
    )
    .map_err(|e| anyhow::anyhow!("Failed to initialize logger: {}", e))?;

    // Load configuration and create registries
    let settings = Arc::new(Settings::new(None)?);
    info!("Configuration loaded successfully.");

    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    let instrument_registry = Arc::new(instrument_registry);

    let processor_registry = Arc::new(ProcessorRegistry::new());

    // Create the core application
    let app = DaqApp::new(
        settings.clone(),
        instrument_registry,
        processor_registry,
        log_buffer,
    )?;
    let app_clone = app.clone();

    // Launch the GUI
    let options = NativeOptions::default();
    info!("Starting GUI...");

    eframe::run_native(
        "Rust DAQ",
        options,
        Box::new(move |cc| Box::new(Gui::new(cc, app_clone))),
    )
    .map_err(|e| anyhow::anyhow!("Eframe run error: {}", e))?;

    // Clean shutdown
    info!("GUI closed. Shutting down.");
    app.shutdown();

    Ok(())
}
```

## How to Add a New Instrument

1.  Create a new file in `src/instrument/`, e.g., `my_instrument.rs`.
2.  Implement the `Instrument` trait from `src/core.rs`.
3.  In `main.rs`, register your new instrument in the `instrument_registry`.
4.  Add any necessary configuration for your instrument to `config/default.toml` and `src/config.rs`.

## Comparison to PyMoDAQ, Qudi, ScopeFoundry

rust-daq provides similar modular experiment control capabilities with key advantages:

**Performance**: 10-100x faster data processing and acquisition (Rust vs Python)
**Reliability**: Thread-safe, memory-safe concurrent operations
**Flexibility**: Both low-level Rust and high-level Python APIs (planned)
**Real-time**: Predictable latency without garbage collection pauses

See [FRAMEWORK_COMPARISON_ANALYSIS.md](FRAMEWORK_COMPARISON_ANALYSIS.md) for detailed comparison.

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue.

See [AGENTS.md](AGENTS.md) for repository guidelines and development workflow.

## License

This project is licensed under the MIT License.
