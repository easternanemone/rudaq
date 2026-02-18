# bin

Command-line interface and daemon for rust-daq.

## Overview

This crate provides the main entry points for running rust-daq:

- **`daq-daemon`**: Long-running gRPC server for remote control
- **`daq-cli`**: Command-line tools for configuration and testing

## Installation

```bash
# Build from source
cargo build --release -p bin

# Install to PATH
cargo install --path crates/bin
```

## Usage

### Running the Daemon

```bash
# Start with default configuration
daq-daemon

# Specify configuration file
daq-daemon --config /path/to/config.toml

# Specify gRPC port
daq-daemon --port 50051

# Enable verbose logging
RUST_LOG=debug daq-daemon
```

### Command-Line Interface

```bash
# List available devices
daq-cli devices list

# Check device status
daq-cli devices status camera_1

# Run a quick test
daq-cli test camera_1

# Execute a script
daq-cli run script.rhai
```

## Configuration

Configuration is loaded from (in order of precedence):

1. Command-line arguments
2. Environment variables (`DAQ_*`)
3. Configuration file (`config.toml`)
4. Default values

### Example Configuration

```toml
# config.toml

[server]
host = "0.0.0.0"
port = 50051
enable_tls = false

[devices]
# Device configurations loaded from devices.toml
config_path = "./devices.toml"

[storage]
ring_buffer_path = "/dev/shm/daq_ring"
ring_buffer_frames = 1000
data_directory = "./data"

[logging]
level = "info"
format = "json"  # or "pretty" for development
```

### Device Configuration

```toml
# devices.toml

[[device]]
id = "camera_1"
name = "Prime BSI Camera"
driver = "pvcam"
camera_name = "PMUSBCam00"

[[device]]
id = "stage_x"
name = "Sample X Stage"
driver = "esp300"
port = "/dev/ttyUSB1"
axis = 1

[[device]]
id = "power_meter"
name = "Newport 1830-C"
driver = "newport_1830c"
port = "/dev/ttyS0"
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DAQ_CONFIG` | Path to configuration file | `./config.toml` |
| `DAQ_PORT` | gRPC server port | `50051` |
| `DAQ_LOG_LEVEL` | Logging level | `info` |
| `RUST_LOG` | Fine-grained log control | - |

## Features

| Feature | Description |
|---------|-------------|
| `server` | Enable gRPC server (required for daemon) |
| `scripting` | Enable Rhai script execution |
| `pvcam` | PVCAM camera support |
| `serial` | Serial device support (ESP300, Newport, etc.) |

## Daemon Architecture

```
┌─────────────────────────────────────────────────────┐
│                    daq-daemon                        │
├─────────────────────────────────────────────────────┤
│  gRPC Server (tonic)                                │
│  ├── HardwareService    - Device control            │
│  ├── RunEngineService   - Plan execution            │
│  ├── StorageService     - Data management           │
│  ├── ControlService     - Script execution          │
│  └── HealthService      - System monitoring         │
├─────────────────────────────────────────────────────┤
│  DeviceRegistry         - Hardware abstraction      │
│  RunEngine              - Experiment orchestration  │
│  RingBuffer             - High-speed data buffer    │
└─────────────────────────────────────────────────────┘
```

## Signals

The daemon handles these signals:

| Signal | Behavior |
|--------|----------|
| `SIGTERM` | Graceful shutdown |
| `SIGINT` | Graceful shutdown (Ctrl+C) |
| `SIGHUP` | Reload configuration |

## Health Checks

The daemon exposes health endpoints:

```bash
# gRPC health check
grpcurl -plaintext localhost:50051 grpc.health.v1.Health/Check

# HTTP health (if enabled)
curl http://localhost:8080/health
```

## Logging

Structured logging with tracing:

```bash
# Development (pretty output)
RUST_LOG=debug,hyper=warn daq-daemon

# Production (JSON output)
DAQ_LOG_FORMAT=json daq-daemon 2>&1 | jq .

# Specific module debugging
RUST_LOG=server::grpc=trace daq-daemon
```

## Systemd Service

Example systemd unit file:

```ini
# /etc/systemd/system/daq-daemon.service
[Unit]
Description=rust-daq Data Acquisition Daemon
After=network.target

[Service]
Type=simple
User=daq
ExecStart=/usr/local/bin/daq-daemon --config /etc/daq/config.toml
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Normal exit |
| 1 | Configuration error |
| 2 | Device initialization failed |
| 3 | Server startup failed |

## See Also

- [`server`](../server/) - gRPC service implementations
- [`hardware`](../hardware/) - Device registry and drivers
- [`scripting`](../scripting/) - Script execution engine
- [DEMO.md](../../docs/tutorials/demo-mode.md) - Getting started guide
