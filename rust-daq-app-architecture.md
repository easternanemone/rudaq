# Rust DAQ Application Architecture Guide

## Overview

This document provides architectural guidelines for Rust DAQ application in Rust, similar to PyMoDAQ, ScopeFoundry, or Qudi. The architecture focuses on modularity, performance, and real-time capabilities while maintaining safety and reliability.

## Core Architecture Principles

### 1. Modular Plugin System
- **Plugin-Based Architecture**: Design the application around a plugin system where instruments, GUIs, and data processors are separate modules
- **Trait-Based Interfaces**: Use Rust traits to define common interfaces for different component types
- **Dynamic Loading**: Support runtime plugin loading and unloading for flexibility

### 2. Async-First Design
- **Tokio Runtime**: Use Tokio as the primary async runtime for I/O operations
- **Channel-Based Communication**: Implement actor-like patterns using channels for inter-component communication
- **Non-Blocking Operations**: Ensure all instrument operations are non-blocking to maintain UI responsiveness

### 3. Type Safety and Error Handling
- **Strong Typing**: Leverage Rust's type system to prevent common scientific computing errors
- **Result-Based Error Handling**: Use Result types consistently throughout the application
- **Custom Error Types**: Define domain-specific error types for better error reporting

## System Components

### Core Framework Layer

```rust
// Core trait definitions
pub trait Instrument: Send + Sync {
    type Config: serde::Serialize + serde::DeserializeOwned;
    type Data: Send + Clone;
    
    async fn initialize(&mut self, config: Self::Config) -> Result<(), InstrumentError>;
    async fn acquire_data(&mut self) -> Result<Self::Data, InstrumentError>;
    async fn configure(&mut self, config: Self::Config) -> Result<(), InstrumentError>;
    async fn shutdown(&mut self) -> Result<(), InstrumentError>;
}

pub trait DataProcessor: Send + Sync {
    type Input: Send + Clone;
    type Output: Send + Clone;
    
    async fn process(&self, data: Self::Input) -> Result<Self::Output, ProcessError>;
}

pub trait GuiComponent: Send {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame);
    fn handle_event(&mut self, event: GuiEvent);
}
```

### Plugin Management System

```rust
use async_trait::async_trait;
use tokio::sync::mpsc;

pub struct PluginManager {
    instruments: HashMap<String, Box<dyn Instrument>>,
    processors: HashMap<String, Box<dyn DataProcessor>>,
    gui_components: HashMap<String, Box<dyn GuiComponent>>,
}

impl PluginManager {
    pub async fn load_plugin(&mut self, plugin_path: &str) -> Result<(), PluginError> {
        // Dynamic plugin loading implementation
    }
    
    pub async fn unload_plugin(&mut self, plugin_name: &str) -> Result<(), PluginError> {
        // Plugin cleanup and removal
    }
}
```

### Data Flow Architecture

#### Message-Based Communication
```rust
#[derive(Debug, Clone)]
pub enum SystemMessage {
    InstrumentData { source: String, data: Vec<u8>, timestamp: std::time::Instant },
    ConfigUpdate { target: String, config: serde_json::Value },
    Command { target: String, command: String, params: Vec<String> },
    Error { source: String, error: String },
}

pub struct MessageBus {
    sender: mpsc::UnboundedSender<SystemMessage>,
    receiver: mpsc::UnboundedReceiver<SystemMessage>,
    subscribers: HashMap<String, Vec<mpsc::UnboundedSender<SystemMessage>>>,
}
```

#### Data Pipeline
```rust
pub struct DataPipeline {
    stages: Vec<Box<dyn DataProcessor>>,
    input: mpsc::Receiver<RawData>,
    output: mpsc::Sender<ProcessedData>,
}

impl DataPipeline {
    pub async fn process_stream(&mut self) {
        while let Some(data) = self.input.recv().await {
            let mut current_data = data;
            
            for stage in &self.stages {
                current_data = stage.process(current_data).await?;
            }
            
            self.output.send(current_data).await?;
        }
    }
}
```

## Configuration Management

### Hierarchical Configuration
```rust
use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ApplicationConfig {
    pub instruments: HashMap<String, InstrumentConfig>,
    pub data_acquisition: DataAcquisitionConfig,
    pub gui: GuiConfig,
    pub logging: LoggingConfig,
}

impl ApplicationConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Config::builder()
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name("config/local").required(false))
            .add_source(Environment::with_prefix("RUSTDAQ"))
            .build()?
            .try_deserialize()
    }
}
```

## Real-Time Considerations

### Buffer Management
```rust
use ringbuf::{SharedRb, storage::Heap};

pub struct RealTimeBuffer<T> {
    buffer: SharedRb<Heap<T>>,
    sample_rate: f64,
    buffer_size: usize,
}

impl<T: Clone> RealTimeBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: SharedRb::new(capacity),
            sample_rate: 1000.0,
            buffer_size: capacity,
        }
    }
    
    pub fn push(&mut self, item: T) -> Result<(), BufferError> {
        self.buffer.push_overwrite(item);
        Ok(())
    }
    
    pub fn latest_samples(&self, count: usize) -> Vec<T> {
        self.buffer.iter().rev().take(count).cloned().collect()
    }
}
```

### Timing and Synchronization
```rust
use std::time::{Duration, Instant};
use tokio::time;

pub struct TimingController {
    sample_interval: Duration,
    last_sample: Instant,
}

impl TimingController {
    pub async fn wait_for_next_sample(&mut self) {
        let elapsed = self.last_sample.elapsed();
        if elapsed < self.sample_interval {
            time::sleep(self.sample_interval - elapsed).await;
        }
        self.last_sample = Instant::now();
    }
}
```

## Error Handling Strategy

### Custom Error Types
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApplicationError {
    #[error("Instrument error: {0}")]
    Instrument(#[from] InstrumentError),
    
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),
    
    #[error("Data processing error: {0}")]
    DataProcessing(String),
    
    #[error("Communication error: {0}")]
    Communication(String),
}

#[derive(Error, Debug)]
pub enum InstrumentError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    
    #[error("Timeout occurred")]
    Timeout,
}
```

## Memory Management

### Data Lifecycle
```rust
use std::sync::Arc;
use parking_lot::RwLock;

pub struct DataManager {
    active_datasets: Arc<RwLock<HashMap<String, Arc<Dataset>>>>,
    archive_path: PathBuf,
}

impl DataManager {
    pub fn store_dataset(&self, name: String, data: Dataset) -> Result<(), StorageError> {
        let dataset = Arc::new(data);
        self.active_datasets.write().insert(name.clone(), dataset.clone());
        
        // Async write to disk
        tokio::spawn(async move {
            self.persist_dataset(&name, &dataset).await
        });
        
        Ok(())
    }
    
    async fn persist_dataset(&self, name: &str, dataset: &Dataset) -> Result<(), StorageError> {
        // Implementation for persisting data to disk
    }
}
```

## Performance Optimization Guidelines

### 1. Zero-Copy Data Handling
- Use `Arc<[T]>` for shared data that doesn't need mutation
- Implement custom serialization for high-frequency data types
- Use memory-mapped files for large datasets

### 2. Async Optimization
- Batch small operations to reduce context switching
- Use `tokio::select!` for handling multiple async operations
- Implement backpressure mechanisms to handle data overflow

### 3. SIMD Utilization
```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

pub fn process_samples_simd(samples: &[f32]) -> Vec<f32> {
    // SIMD implementation for high-performance data processing
    samples.chunks_exact(8)
        .map(|chunk| {
            // SIMD operations on chunks of 8 f32 values
        })
        .collect()
}
```

## Testing Strategy

### Unit Testing
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio_test;

    #[tokio::test]
    async fn test_instrument_initialization() {
        let mut instrument = MockInstrument::new();
        let config = InstrumentConfig::default();
        
        let result = instrument.initialize(config).await;
        assert!(result.is_ok());
    }
}
```

### Integration Testing
```rust
#[tokio::test]
async fn test_data_pipeline() {
    let (tx, rx) = mpsc::channel(1000);
    let mut pipeline = DataPipeline::new(rx);
    
    // Send test data through pipeline
    tx.send(test_data()).await.unwrap();
    
    // Verify processed output
    let result = pipeline.process_next().await.unwrap();
    assert_eq!(result.len(), expected_length);
}
```

## Deployment Considerations

### Cross-Platform Support
- Use `#[cfg]` attributes for platform-specific code
- Implement feature flags for optional components
- Provide Docker containers for consistent deployment

### Security
- Implement proper authentication for remote access
- Use TLS for network communications
- Sanitize all external inputs

## V2 Architecture Migration

The `rust-daq` application has recently undergone a significant architectural migration from a legacy V1 architecture to a more robust and scalable V2 architecture. This migration was undertaken to address several key issues with the original design, including performance bottlenecks, limited data type support, and a confusing mix of architectural patterns.

### Key Improvements

The V2 architecture introduces several key improvements over the legacy V1 design:

*   **Actor-Based State Management**: The V2 architecture is built around the actor model, which eliminates the lock contention and performance bottlenecks of the previous `Arc<Mutex<>>`-based design. All application state is now owned by a central `DaqManagerActor`, which processes commands and manages the lifecycle of all instruments and other components.
*   **Native `Measurement` Enum Support**: The V2 architecture natively supports the `Measurement` enum, which allows for the handling of rich data types like images and spectra. This is a significant improvement over the V1 architecture, which was limited to simple scalar values.
*   **Asynchronous-First Design**: The V2 architecture is designed to be fully asynchronous, which allows for highly performant, non-blocking I/O. This is a key requirement for a scientific data acquisition application, where real-time performance is critical.
*   **Clear Separation of Concerns**: The V2 architecture enforces a clear separation of concerns between the core application logic, the instrument drivers, and the GUI. This makes the codebase easier to understand, maintain, and extend.

### Spawning a V2 Instrument

The following code example illustrates how to spawn a V2 instrument in the new architecture:

```rust
async fn spawn_v2_instrument(
    &mut self,
    id: &str,
    mut instrument: std::pin::Pin<
        Box<dyn daq_core::Instrument + Send + Sync + 'static + Unpin>,
    >,
) -> Result<(), SpawnError> {
    // ...

    // Initialize the V2 instrument
    instrument
        .as_mut()
        .get_mut()
        .initialize()
        .await
        .map_err(|e| {
            SpawnError::ConnectionFailed(format!(
                "Failed to initialize V2 instrument '{}': {}",
                id, e
            ))
        })?;

    // Get measurement stream from instrument
    let measurement_rx = instrument.as_ref().get_ref().measurement_stream();
    let data_distributor = self.data_distributor.clone();
    let id_clone = id.to_string();

    // ...

    // Spawn task to handle V2 instrument lifecycle
    let mut instrument_handle = instrument;
    let mut measurement_rx = measurement_rx;

    let task: JoinHandle<Result<()>> = self.runtime.spawn(async move {
        loop {
            tokio::select! {
                measurement_result = measurement_rx.recv() => {
                    match measurement_result {
                        Ok(measurement) => {
                            // V2 instruments produce Arc<Measurement> directly
                            if let Err(e) = data_distributor.broadcast(measurement).await {
                                error!(
                                    "Failed to broadcast measurement from V2 instrument '{}': {}",
                                    id_clone, e
                                );
                            }
                        }
                        // ...
                    }
                }
                // ...
            }
        }
        // ...
    });

    // ...
}
```