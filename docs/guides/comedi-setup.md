# Comedi DAQ Setup Guide

This guide covers setting up Comedi (Control and Measurement Device Interface) for use with the rust-daq system on Linux.

## Supported Hardware

The Comedi framework supports a wide variety of data acquisition hardware. This project has been tested with:

| Device | Description | Subdevices |
|--------|-------------|------------|
| NI PCI-MIO-16XE-10 | 16-ch AI, 2-ch AO, 8 DIO, counters | AI, AO, DIO, Counter, Timer |

## Prerequisites

### 1. Linux Kernel Headers

```bash
# Ubuntu/Debian
sudo apt-get install linux-headers-$(uname -r)

# Fedora/RHEL
sudo dnf install kernel-devel
```

### 2. Comedi Kernel Modules

```bash
# Ubuntu/Debian
sudo apt-get install comedi-modules

# Or build from source
git clone https://github.com/Linux-Comedi/comedi.git
cd comedi
./autogen.sh
./configure
make
sudo make install
```

### 3. Comedilib (User-space Library)

```bash
# Ubuntu/Debian
sudo apt-get install libcomedi-dev comedilib

# Or build from source
git clone https://github.com/Linux-Comedi/comedilib.git
cd comedilib
./autogen.sh
./configure
make
sudo make install
sudo ldconfig
```

## Loading Kernel Modules

### NI PCI Cards

```bash
# Load the ni_pcimio driver
sudo modprobe ni_pcimio

# Verify the device appeared
ls -la /dev/comedi0
comedi_info  # Shows device details
```

### Configure Device (if needed)

```bash
# For NI PCI cards, usually auto-configured
# Manual configuration if needed:
sudo comedi_config /dev/comedi0 ni_pcimio
```

## User Permissions

### Option 1: udev Rule (Recommended)

Create `/etc/udev/rules.d/99-comedi.rules`:

```udev
KERNEL=="comedi*", MODE="0666"
```

Reload udev:

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
```

### Option 2: Group Membership

```bash
# Add user to iocard group (if exists)
sudo usermod -a -G iocard $USER

# Or create a comedi group
sudo groupadd comedi
sudo chgrp comedi /dev/comedi0
sudo chmod 660 /dev/comedi0
sudo usermod -a -G comedi $USER
```

Log out and back in for group changes to take effect.

## Verifying Installation

### Check Device

```bash
# List Comedi devices
ls -la /dev/comedi*

# Get device info
comedi_info /dev/comedi0
```

Expected output for NI PCI-MIO-16XE-10:

```
overall info:
  version code: 0x000809
  driver name: ni_pcimio
  board name: pci-mio-16xe-10
  number of subdevices: 10
subdevice 0:
  type: analog input
  number of channels: 16
  max data value: 65535
  ...
```

### Test Read

```bash
# Read single sample from channel 0
comedi_test -t read /dev/comedi0

# Read with specific options
comedi_test -t read -s 0 -c 0 /dev/comedi0
```

## BNC 2110 Breakout Board Wiring

The BNC 2110 provides easy access to DAQ signals via BNC connectors:

### Analog Input Channels

| BNC Label | Signal | Notes |
|-----------|--------|-------|
| ACH0-ACH7 | AI0-AI7 | Single-ended or differential |
| ACH8-ACH15 | AI8-AI15 | Spring terminals only |

### Analog Output Channels

| BNC Label | Signal | Notes |
|-----------|--------|-------|
| DAC0 | AO0 | ±10V range |
| DAC1 | AO1 | ±10V range |

### Digital I/O

| Connector | Signal | Notes |
|-----------|--------|-------|
| P0.0-P0.7 | DIO0-DIO7 | Directly on 68-pin connector |

### Reference Mode Switch

The ACH<0..7> BNC inputs have a switch for each channel:

- **GS (Ground-referenced Source)**: Source is grounded elsewhere
- **FS (Floating Source)**: Source has no ground reference

For loopback testing (AO → AI), set the switch to **FS**.

## Rust Driver Setup

### Build with Hardware Feature

```bash
# Build the Comedi driver crate
cargo build -p daq-driver-comedi --features hardware

# Run tests (requires hardware)
export COMEDI_SMOKE_TEST=1
cargo nextest run -p daq-driver-comedi --features hardware -- --nocapture
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `COMEDI_DEVICE` | Device path | `/dev/comedi0` |
| `COMEDI_SMOKE_TEST` | Enable smoke tests | `0` |
| `COMEDI_LOOPBACK_TEST` | Enable loopback tests | `0` |
| `COMEDI_DIO_TEST` | Enable DIO tests | `0` |
| `COMEDI_DIO_LOOPBACK` | Enable DIO loopback | `0` |
| `COMEDI_COUNTER_TEST` | Enable counter tests | `0` |
| `COMEDI_HAL_TEST` | Enable HAL tests | `0` |
| `COMEDI_ERROR_TEST` | Enable error tests | `0` |
| `COMEDI_STORAGE_TEST` | Enable storage tests | `0` |

## Troubleshooting

### Device Not Found

```
Error: No such file or directory (os error 2)
```

**Solution**: Load the kernel module:
```bash
sudo modprobe ni_pcimio
# Or for other cards: sudo modprobe <driver_name>
```

### Permission Denied

```
Error: Permission denied (os error 13)
```

**Solution**: Set up udev rules or add user to appropriate group (see above).

### Wrong Board Type

```
Error: Expected NI PCI-MIO-16XE-10 board, got: unknown
```

**Solution**: The driver may need manual configuration:
```bash
sudo comedi_config /dev/comedi0 ni_pcimio
```

### Buffer Overflows During Streaming

**Symptoms**: `overflows` counter incrementing, data gaps

**Solutions**:
1. Reduce sample rate
2. Increase buffer size in StreamConfig
3. Read data more frequently
4. Check system load

## References

- [Comedi Project](https://www.comedi.org/)
- [Comedilib Documentation](https://www.comedi.org/doc/index.html)
- [NI PCI-MIO-16XE-10 Specifications](https://www.ni.com/docs/en-US/bundle/pci-mio-16xe-10-specs/page/specs.html)
