# rust-daq Client Library

A high-level gRPC client library for the rust-daq daemon.

## Overview

The `client` crate provides a Rust API for communicating with the rust-daq daemon over gRPC. It is UI-agnostic and can be used by CLI tools, test harnesses, web frontends, and alternative interfaces.

## Key Features

- **DaqClient** - Main client for daemon communication
- **Automatic Reconnection** - Configurable reconnection logic for resilience
- **Connection Management** - Address resolution, URL normalization, and daemon discovery
- **Error Handling** - Comprehensive error types for network and gRPC failures

## Key Types

### DaqClient
The primary entry point for daemon communication.

```rust
use client::{DaqClient, DEFAULT_DAEMON_URL};

let client = DaqClient::connect(DEFAULT_DAEMON_URL).await?;
let devices = client.list_devices().await?;
```

### ConnectionManager
Handles automatic reconnection with exponential backoff.

```rust
use client::{ConnectionManager, ReconnectConfig};

let config = ReconnectConfig::default()
    .with_max_retries(5)
    .with_backoff_ms(100);

let manager = ConnectionManager::new(
    "http://localhost:50051",
    config,
)?;

// Manager automatically reconnects on failure
```

### Address Resolution
Utilities for finding and connecting to daemons.

```rust
use client::{resolve_address, AddressSource, DaemonAddress};

// Resolve daemon address from multiple sources
let addr = resolve_address(
    Some("localhost:50051"),  // explicit
    Some("http://localhost:50051"),  // or persisted from storage
);
```

## Usage Examples

### Basic Connection

```rust
use client::DaqClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = DaqClient::connect("http://localhost:50051").await?;

    // List all devices
    let devices = client.list_devices().await?;
    for device in &devices {
        println!("Device: {}", device.id);
    }

    // Read a value from a sensor
    let value = client.read_value("power_meter").await?;
    println!("Power: {} {}", value.value, value.units);

    Ok(())
}
```

### With Automatic Reconnection

```rust
use client::{ConnectionManager, ReconnectConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ReconnectConfig::default();
    let mut manager = ConnectionManager::new(
        "http://localhost:50051",
        config,
    )?;

    // Monitor connection state
    loop {
        match manager.get_client().await {
            Some(client) => {
                // Use client
                if let Ok(devices) = client.list_devices().await {
                    println!("Connected with {} devices", devices.len());
                }
            }
            None => {
                println!("Disconnected, waiting for reconnection...");
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
```

### Streaming Frames from Camera

```rust
use client::DaqClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = DaqClient::connect("http://localhost:50051").await?;

    let mut stream = client.stream_frames(
        "camera",
        30,  // max_fps
        Default::default(),  // quality
    ).await?;

    while let Some(frame) = stream.message().await? {
        println!("Got frame: {}x{}", frame.width, frame.height);
    }

    Ok(())
}
```

## Module Structure

- **client** - Main `DaqClient` implementation
- **connection** - Address resolution and URL handling
- **error** - Error types and result types
- **reconnect** - Automatic reconnection logic

## Configuration

### Environment Variables

- `DAQ_DAEMON_URL` - Daemon address (e.g., `http://127.0.0.1:50051`)

### Default Settings

- **Default URL**: `http://127.0.0.1:50051`
- **Default Port**: `50051`
- **Storage Key**: `daemon_address` (for persistent config)

### Address Resolution Priority

Addresses are resolved in this order (highest priority first):
1. User input (via API)
2. Persisted address from previous session (storage)
3. `DAQ_DAEMON_URL` environment variable
4. Default fallback: `http://127.0.0.1:50051`

## Error Handling

The crate provides a `Result<T>` type alias and `ClientError` enum for error cases:

```rust
use client::{ClientError, Result};

async fn example() -> Result<String> {
    // network error, auth error, or internal grpc error
    Err(ClientError::Connection("failed to connect".into()))?
}
```

## Related Documentation

- [Main README](../../README.md) - Project overview
- [gRPC API Documentation](../../docs/api/grpc.md) - Full API specification
- [Testing Guide](../../docs/guides/testing.md) - Test patterns

## See Also

- `server` crate - Server-side gRPC implementation
- `protocol` crate - Protobuf definitions
