# Production Deployment Guide - Rust DAQ V4

**Version**: 1.0
**Date**: 2025-11-17
**Architecture**: V4-Only (Clean Kameo Actor System)
**Status**: Production Ready

---

## Table of Contents

1. [Overview](#overview)
2. [System Requirements](#system-requirements)
3. [Pre-Deployment Checklist](#pre-deployment-checklist)
4. [Installation Steps](#installation-steps)
5. [Systemd Service Setup](#systemd-service-setup)
6. [Monitoring and Logging](#monitoring-and-logging)
7. [Deployment Verification](#deployment-verification)
8. [Production Checklist](#production-checklist)

---

## Overview

Rust DAQ V4 is a production-grade data acquisition system with 5 instrument actors managed by the Kameo actor framework. It provides:

- **Actor-Based Architecture**: Fault-tolerant message-passing concurrency
- **5 Instrument Drivers**: SCPI, ESP300, PVCAM, Newport 1830-C, MaiTai
- **Flexible Storage**: HDF5 and Apache Arrow backends
- **Hardware Acceleration**: Supports GPU processing via PVCAM
- **Production Monitoring**: Comprehensive logging and metrics

### Supported Instruments

| Actor | Hardware | Communication | Features |
|-------|----------|-----------------|----------|
| **SCPI** | Any SCPI-compliant instrument | GPIB, USB, Ethernet (VISA) | Query/command interface |
| **ESP300** | Newport 3-axis motion stage | RS-232 (19200 baud) | Position tracking, homing |
| **PVCAM** | Photometrics Prime camera | USB or Ethernet | Frame acquisition, ROI, streaming |
| **Newport 1830-C** | Newport power meter | RS-232 (9600 baud) | Wavelength calibration, power measurement |
| **MaiTai** | Spectra-Physics MaiTai laser | RS-232 (115200 baud) | Wavelength tuning, shutter control, power measurement |

---

## System Requirements

### Hardware Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU | 2 cores | 4+ cores |
| RAM | 2 GB | 8 GB |
| Disk | 10 GB | 100 GB (data storage) |
| USB Ports | 2 | 4+ (for serial adapters) |

### Software Requirements

#### Operating System

- Linux (Ubuntu 18.04+, Debian 10+, CentOS 7+, RHEL 7+)
- macOS 10.13+ (with Homebrew)
- Windows (WSL2 with Linux distribution)

#### Runtime Dependencies

```bash
# Ubuntu/Debian
sudo apt-get install -y \
  libssl-dev \
  pkg-config \
  build-essential \
  libhdf5-dev \
  libserialport-dev \
  curl

# CentOS/RHEL
sudo yum install -y \
  openssl-devel \
  pkgconfig \
  gcc \
  gcc-c++ \
  hdf5-devel \
  libserialport-devel \
  curl

# macOS
brew install \
  openssl \
  pkg-config \
  hdf5 \
  libserialport
```

#### Hardware Communication Libraries

**VISA Runtime** (for SCPI and Newport instruments):
- National Instruments VISA: https://www.ni.com/en-us/support/downloads/drivers/download.ni-visa.html
- Or Keysight IO Libraries: https://www.keysight.com/en/en/lib/software/application/IO_Libraries_Suite.html

**Serial Port Access**:
- Linux: Automatic (udev rules configured during installation)
- macOS: Automatic
- Windows: USB drivers for specific adapters (FTDI, Prolific, etc.)

### Build Requirements

- Rust 1.70+ (for building from source)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  source "$HOME/.cargo/env"
  ```

### Network Requirements

- Ethernet connection for Gigabit network instruments (SCPI via TCP/IP)
- Separate network segment recommended for hardware instruments (dedicated switch)
- Firewall rules allowing TCP 5025 (VISA default) if using network instruments

---

## Pre-Deployment Checklist

Before deploying to production, verify:

- [ ] **Hardware Inventory**: All 5 instruments available and accessible
- [ ] **Network Connectivity**: Test IP connectivity to network instruments
- [ ] **Serial Ports**: List all connected devices (`ls /dev/ttyUSB*`)
- [ ] **VISA Installation**: Verify VISA runtime installed and accessible
- [ ] **Disk Space**: At least 100 GB free for data storage
- [ ] **User Permissions**: Create `rustdaq` system user
- [ ] **SSL Certificates**: If using secure VISA connections
- [ ] **Backup Plan**: Document hardware configuration and network settings

### Hardware Verification

```bash
# List available serial ports
ls -la /dev/tty* | grep -E "USB|serial"

# Test network connectivity to SCPI instruments
ping 192.168.1.100  # Replace with actual IP

# Verify VISA installation (Linux/macOS)
which visainfo
visainfo  # Should list available VISA resources

# Check USB device permissions
lsusb
ls -la /dev/bus/usb
```

---

## Installation Steps

### Step 1: Create System User and Directories

```bash
# Create dedicated user for rust-daq service
sudo useradd -r -s /bin/bash -d /opt/rust-daq rustdaq

# Create application directories
sudo mkdir -p /opt/rust-daq/bin
sudo mkdir -p /opt/rust-daq/config
sudo mkdir -p /var/lib/rust-daq/data
sudo mkdir -p /var/log/rust-daq

# Set ownership and permissions
sudo chown -R rustdaq:rustdaq /opt/rust-daq
sudo chown -R rustdaq:rustdaq /var/lib/rust-daq
sudo chown -R rustdaq:rustdaq /var/log/rust-daq

chmod 750 /opt/rust-daq
chmod 755 /var/lib/rust-daq
chmod 755 /var/log/rust-daq
```

### Step 2: Configure Serial Port Access

```bash
# Add rustdaq user to dialout group (for serial port access)
sudo usermod -a -G dialout rustdaq

# Create udev rule for USB serial adapters (optional, for persistent naming)
sudo tee /etc/udev/rules.d/99-rust-daq-serial.rules > /dev/null <<'EOF'
# Newport ESP300
SUBSYSTEMS=="usb", ATTRS{idVendor}=="0403", ATTRS{idProduct}=="6001", SYMLINK+="ttyESP300", MODE="0666"

# MaiTai Control Module
SUBSYSTEMS=="usb", ATTRS{idVendor}=="0403", ATTRS{idProduct}=="6001", SYMLINK+="ttyMaiTai", MODE="0666"

# Generic FTDI devices
SUBSYSTEMS=="usb", ATTRS{idVendor}=="0403", MODE="0666"
EOF

# Reload udev rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

### Step 3: Build or Download Binary

#### Option A: Build from Source

```bash
# Clone repository
git clone https://github.com/your-org/rust-daq.git
cd rust-daq/v4-daq

# Build release binary
cargo build --release --features instrument_serial,storage_hdf5

# Copy binary to installation directory
sudo cp target/release/rust-daq /opt/rust-daq/bin/rust-daq-v4
sudo chmod 755 /opt/rust-daq/bin/rust-daq-v4
```

#### Option B: Use Pre-Built Binary

```bash
# Download from release page
wget https://github.com/your-org/rust-daq/releases/download/v0.1.0/rust-daq-v4-linux-x86_64
sudo install -m 755 rust-daq-v4-linux-x86_64 /opt/rust-daq/bin/rust-daq-v4
```

### Step 4: Configure Application

```bash
# Copy configuration template
sudo cp config/config.example.v4.toml /opt/rust-daq/config/config.v4.toml
sudo chown rustdaq:rustdaq /opt/rust-daq/config/config.v4.toml

# Edit configuration for your environment
sudo -u rustdaq nano /opt/rust-daq/config/config.v4.toml

# Verify configuration syntax
sudo -u rustdaq /opt/rust-daq/bin/rust-daq-v4 --validate-config /opt/rust-daq/config/config.v4.toml
```

See [CONFIGURATION_REFERENCE.md](CONFIGURATION_REFERENCE.md) for detailed configuration options.

---

## Systemd Service Setup

### Create Systemd Service Unit

Create `/etc/systemd/system/rust-daq.service`:

```ini
[Unit]
Description=Rust DAQ V4 - Actor-Based Data Acquisition System
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=rustdaq
Group=rustdaq
WorkingDirectory=/opt/rust-daq

# Main process
ExecStart=/opt/rust-daq/bin/rust-daq-v4 --config /opt/rust-daq/config/config.v4.toml

# Restart policy
Restart=on-failure
RestartSec=10

# Process management
KillMode=mixed
KillSignal=SIGTERM

# Timeouts
TimeoutStartSec=30
TimeoutStopSec=30

# Resource limits
LimitNOFILE=65536
LimitNPROC=65536

# Environment variables
Environment="RUST_LOG=info"
Environment="RUST_BACKTRACE=1"
Environment="RUSTDAQ_LOG_LEVEL=info"

# Output configuration
StandardOutput=journal
StandardError=journal
SyslogIdentifier=rust-daq

[Install]
WantedBy=multi-user.target
```

### Enable and Start Service

```bash
# Reload systemd daemon
sudo systemctl daemon-reload

# Enable service to start on boot
sudo systemctl enable rust-daq

# Start the service
sudo systemctl start rust-daq

# Verify service is running
sudo systemctl status rust-daq

# View service logs
sudo journalctl -u rust-daq -f
```

### Service Management Commands

```bash
# Start/stop/restart service
sudo systemctl start rust-daq
sudo systemctl stop rust-daq
sudo systemctl restart rust-daq

# Check service status
sudo systemctl status rust-daq

# View last 100 lines of logs
sudo journalctl -u rust-daq -n 100

# View logs from specific time
sudo journalctl -u rust-daq --since "2025-11-17 10:00:00"

# Monitor in real-time
sudo journalctl -u rust-daq -f
```

---

## Monitoring and Logging

### Log Configuration

Rust DAQ uses `tracing` with JSON structured logging. Configure via:

1. **Configuration File** (`config.v4.toml`):
   ```toml
   [application]
   log_level = "info"  # trace, debug, info, warn, error
   ```

2. **Environment Variable**:
   ```bash
   export RUST_LOG=debug
   export RUSTDAQ_LOG_LEVEL=debug
   ```

3. **Systemd Service**:
   Edit `Environment="RUST_LOG=debug"` in service unit

### Log Output Locations

```bash
# Real-time logs (journalctl)
sudo journalctl -u rust-daq -f

# Persistent logs (if forwarding to syslog)
tail -f /var/log/syslog | grep rust-daq
tail -f /var/log/messages | grep rust-daq

# Application-generated logs (if configured)
ls -la /var/log/rust-daq/
```

### Log Levels

| Level | Use Case | Example |
|-------|----------|---------|
| `error` | Critical failures | "Actor spawn failed" |
| `warn` | Potential issues | "Hardware timeout detected" |
| `info` | Normal operations | "Actor started", "Measurement complete" |
| `debug` | Troubleshooting | "Command sent: *IDN?", "Response received" |
| `trace` | Deep diagnostics | Message queue details, buffer operations |

### Production Logging Recommendations

```toml
# Production: Minimal logging
[application]
log_level = "info"

# Staging: More details
[application]
log_level = "debug"

# Development/Troubleshooting: Full verbosity
[application]
log_level = "trace"
```

### Log Rotation

Set up logrotate for persistent logs:

```bash
sudo tee /etc/logrotate.d/rust-daq > /dev/null <<'EOF'
/var/log/rust-daq/*.log {
    daily
    rotate 7
    compress
    delaycompress
    notifempty
    create 0640 rustdaq rustdaq
    postrotate
        systemctl reload rust-daq > /dev/null 2>&1 || true
    endscript
}
EOF
```

---

## Deployment Verification

### Verify Installation

```bash
# Check binary
/opt/rust-daq/bin/rust-daq-v4 --version

# Check configuration
/opt/rust-daq/bin/rust-daq-v4 --validate-config /opt/rust-daq/config/config.v4.toml

# Verify user and permissions
ls -la /opt/rust-daq/bin/rust-daq-v4
ps aux | grep rust-daq
```

### Test Service Start

```bash
# Start in foreground for testing
sudo -u rustdaq /opt/rust-daq/bin/rust-daq-v4 --config /opt/rust-daq/config/config.v4.toml

# Should see output similar to:
# 2025-11-17T12:34:56.789Z INFO rust_daq: Starting Rust DAQ V4
# 2025-11-17T12:34:57.012Z INFO rust_daq: SCPI actor spawned: scpi_meter
# 2025-11-17T12:34:57.234Z INFO rust_daq: ESP300 actor spawned: esp300_stage
```

### Verify Hardware Communication

```bash
# Test SCPI instrument (requires running service)
echo "*IDN?" | nc 192.168.1.100 5025

# Test serial ports
ls -la /dev/ttyUSB*
lsof /dev/ttyUSB0  # Show which process has the port

# Test VISA resources
visainfo

# Test PVCAM camera (if available)
# This is tested at runtime by the PVCAM actor
```

---

## Production Checklist

Use this checklist before deploying to production:

### Pre-Deployment (1-2 days before)

- [ ] **Hardware Testing**: Run 24-hour hardware stability test
- [ ] **Load Testing**: Verify throughput under typical workload
- [ ] **Disaster Recovery**: Test backup and restore procedures
- [ ] **Documentation**: Update network and hardware diagrams
- [ ] **Team Training**: Ensure operators know how to manage service

### Day of Deployment

- [ ] **Change Log**: Create entry in change management system
- [ ] **Notification**: Notify users of deployment window
- [ ] **Backup**: Create complete system backup
- [ ] **Build Verification**: Verify release binaries match source
- [ ] **Configuration Review**: Final review of production config
- [ ] **Service Installation**: Install and enable systemd service

### Post-Deployment (24-48 hours)

- [ ] **Log Monitoring**: Check for errors in first 24 hours
- [ ] **Performance Check**: Verify baseline metrics
- [ ] **Hardware Validation**: Confirm all 5 actors responding
- [ ] **User Acceptance**: Get operator sign-off
- [ ] **Documentation Update**: Record any changes made
- [ ] **Runbook Update**: Update operational procedures

### Rollback Plan (if needed)

```bash
# Stop service
sudo systemctl stop rust-daq

# Restore previous binary
sudo cp /opt/rust-daq/bin/rust-daq-v4.bak /opt/rust-daq/bin/rust-daq-v4

# Restore previous configuration
sudo cp /opt/rust-daq/config/config.v4.toml.bak /opt/rust-daq/config/config.v4.toml

# Restart service
sudo systemctl start rust-daq

# Verify
sudo journalctl -u rust-daq -f
```

---

## Appendix: Common Deployment Scenarios

### Scenario 1: Single Machine, All Instruments

```toml
[application]
name = "rust-daq-lab"
log_level = "info"

[actors]
default_mailbox_capacity = 200
spawn_timeout_ms = 5000
shutdown_timeout_ms = 5000

[[instruments]]
id = "scpi_1"
type = "ScpiInstrument"
enabled = true
config = { resource = "TCPIP0::192.168.1.100::INSTR", timeout_ms = 2000 }

[[instruments]]
id = "esp300"
type = "ESP300"
enabled = true
config = { serial_port = "/dev/ttyUSB0", axes = 3 }

[[instruments]]
id = "pvcam"
type = "PVCAMInstrument"
enabled = true
config = { camera_name = "PrimeBSI" }

[[instruments]]
id = "newport"
type = "Newport1830C"
enabled = true
config = { resource = "ASRL1::INSTR" }

[[instruments]]
id = "maitai"
type = "MaiTai"
enabled = true
config = { serial_port = "/dev/ttyUSB1" }

[storage]
default_backend = "hdf5"
output_dir = "/var/lib/rust-daq/data"
compression_level = 6
```

### Scenario 2: Distributed Deployment (Multiple Machines)

**Machine 1: Instruments A, B, C**
```toml
[[instruments]]
id = "scpi"
type = "ScpiInstrument"
config = { resource = "TCPIP0::192.168.1.100::INSTR" }

[[instruments]]
id = "esp300"
type = "ESP300"
config = { serial_port = "/dev/ttyUSB0", axes = 3 }

[[instruments]]
id = "pvcam"
type = "PVCAMInstrument"
config = { camera_name = "PrimeBSI" }
```

**Machine 2: Instruments D, E**
```toml
[[instruments]]
id = "newport"
type = "Newport1830C"
config = { resource = "ASRL1::INSTR" }

[[instruments]]
id = "maitai"
type = "MaiTai"
config = { serial_port = "/dev/ttyUSB0" }
```

### Scenario 3: High-Availability Setup (with monitoring)

Same as Single Machine setup, but add:

```bash
# Install monitoring agent (e.g., Prometheus node exporter)
sudo apt-get install prometheus-node-exporter

# Add metrics collection to systemd service
# See CONFIGURATION_REFERENCE.md for metrics configuration

# Setup alerts for service failure
sudo apt-get install alertmanager

# Configure systemd to restart on failure
# Already in service unit: Restart=on-failure, RestartSec=10
```

---

## Support and Troubleshooting

For detailed troubleshooting steps, see [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

For configuration options, see [CONFIGURATION_REFERENCE.md](CONFIGURATION_REFERENCE.md).

For build/development information, see main project README.

---

**Version**: 1.0
**Last Updated**: 2025-11-17
**Maintained By**: Brian Squires
