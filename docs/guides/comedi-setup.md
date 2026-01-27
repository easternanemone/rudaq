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

### Analog Input Channels (maitai Configuration)

| BNC Label | Signal | Description |
|-----------|--------|-------------|
| **ACH0** | DAC1 Loopback | Test loopback from AO1 (permanently connected) |
| **ACH1** | ESP300 Encoder | Encoder signal from Newport ESP300 motion controller |
| **ACH2** | MaiTai Rep Rate | ~40MHz signal (half of laser repetition rate) |
| ACH3-ACH7 | Available | Unassigned, available on BNC connectors |
| ACH8-ACH15 | Terminal Block | Spring terminals only (not BNC accessible) |

### Analog Output Channels

| BNC Label | Signal | Description |
|-----------|--------|-------------|
| **DAC0 (AO0)** | EOM Amplifier | **CAUTION:** Controls laser power via electro-optic modulator |
| **DAC1 (AO1)** | Test Loopback | Connected to ACH0 for self-test |

> **Warning:** Do NOT write arbitrary voltages to DAC0 during testing as this 
> directly controls laser power through the EOM amplifier. Always use DAC1→ACH0
> for loopback tests.

### Digital I/O

| Connector | Signal | Notes |
|-----------|--------|-------|
| P0.0-P0.7 | DIO0-DIO7 | 8 bidirectional digital lines |

### Reference Mode Switch

The ACH<0..7> BNC inputs have a switch for each channel:

- **GS (Ground-referenced Source)**: Source is grounded elsewhere
- **FS (Floating Source)**: Source has no ground reference

For loopback testing (DAC1 → ACH0), set the ACH0 switch to **FS**.

### Hardware Accuracy

The NI PCI-MIO-16XE-10 without calibration typically has:
- DC offset: ~50mV
- Expected accuracy: ±100mV for loopback tests

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

## Calibration

The NI PCI-MIO-16XE-10 supports software calibration via `comedi_calibrate`. Without
calibration, expect ~50mV DC offset and ±100mV accuracy. After calibration, accuracy
improves to ~1-2mV.

### Prerequisites

```bash
# Ensure comedilib is installed
sudo apt-get install libcomedi-dev comedilib

# Verify comedi_calibrate is available
which comedi_calibrate
```

### Running Calibration

```bash
# Full autocalibration (recommended)
sudo comedi_calibrate -f /dev/comedi0

# Reset and fresh calibration
sudo comedi_calibrate --reset --calibrate -f /dev/comedi0

# Verbose output for debugging
sudo comedi_calibrate -v -f /dev/comedi0
```

### Calibration Script

A convenience script is provided at `scripts/calibrate-comedi.sh`:

```bash
# Run calibration on maitai
sudo bash scripts/calibrate-comedi.sh

# Verify calibration
sudo bash scripts/calibrate-comedi.sh --verify
```

### Calibration Files

Calibration data is stored in:
- System: `/etc/comedi/calibrations/`
- User: `~/.comedi_calibrations/`

```bash
# Check existing calibrations
ls -la /etc/comedi/calibrations/
ls -la ~/.comedi_calibrations/

# View calibration details
comedi_calibrate -f /dev/comedi0 --dump
```

### How It Works

The NI PCI-MIO-16XE-10 has internal precision voltage references. The calibration
utility:

1. Measures internal reference voltages
2. Calculates gain and offset corrections
3. Programs the onboard calibration DACs
4. Stores coefficients in a calibration file

This is **soft calibration** - no EEPROM is modified, so it's safe to run repeatedly.

### When to Recalibrate

- After significant temperature changes (>10°C)
- After the system has been powered off for extended periods
- If measurement accuracy degrades
- Periodically (monthly recommended for precision work)

### Applying Calibration in Rust

The Comedi driver automatically looks for calibration files. To ensure calibration
is applied, the calibration file must exist before opening the device:

```rust
use daq_driver_comedi::ComediDevice;

// Calibration is automatically applied if file exists
let device = ComediDevice::open("/dev/comedi0")?;

// Check if calibration was loaded (future feature)
// device.is_calibrated()
```

### Troubleshooting Calibration

**"comedi_calibrate: command not found"**
```bash
sudo apt-get install comedilib
```

**"Cannot open /dev/comedi0"**
```bash
# Check device exists
ls -la /dev/comedi0

# Load driver if needed
sudo modprobe ni_pcimio
```

**"Calibration failed: unsupported board"**
```bash
# Check board type
comedi_board_info /dev/comedi0

# Some boards don't support software calibration
```

**Calibration doesn't improve accuracy**
- Ensure the calibration file is being read (check permissions)
- Verify the file timestamp matches the last calibration run
- Try removing old calibration files and recalibrating

## References

- [Comedi Project](https://www.comedi.org/)
- [Comedilib Documentation](https://www.comedi.org/doc/index.html)
- [NI PCI-MIO-16XE-10 Specifications](https://www.ni.com/docs/en-US/bundle/pci-mio-16xe-10-specs/page/specs.html)
- [comedi_calibrate Man Page](https://manpages.ubuntu.com/manpages/focal/man8/comedi_calibrate.8.html)
