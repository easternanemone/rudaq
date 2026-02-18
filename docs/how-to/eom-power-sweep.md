# EOM Power Sweep Test Guide

## Overview

The EOM (Electro-Optic Modulator) Power Sweep test characterizes the optical power control system on the maitai workstation. It sweeps the EOM control voltage while measuring output power, revealing the transfer function of the Pockels cell-based power control.

## Hardware Configuration

### Physical Setup

```
MaiTai Ti:Sapphire Laser
       ↓
   [Shutter]
       ↓
   [Pockels Cell EOM] ←── DAC0 (Comedi)
       ↓
   [Polarizer]
       ↓
 Newport 1830-C Power Meter
```

### Device Connections

| Device | Port | Protocol | Description |
|--------|------|----------|-------------|
| **MaiTai Laser** | `/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0` | Serial 115200 8N1 | Shutter control |
| **Comedi DAQ** | `/dev/comedi0` | Comedi API | DAC0 → EOM amplifier |
| **Newport 1830-C** | `/dev/ttyS0` | Serial 9600 8N1 | Power measurement |

### BNC-2110 Channel Assignment

| Channel | Signal | Notes |
|---------|--------|-------|
| **DAC0 (AO0)** | EOM Amplifier | Controls Pockels cell bias voltage |
| **DAC1 (AO1)** | Loopback Test | Connected to ACH0 for self-test |
| **ACH0** | DAC1 Loopback | Test input only |

**WARNING**: DAC0 controls laser power. Do not write arbitrary voltages during unrelated tests.

## Test Procedure

### Phase 1: Safety Initialization
1. Set EOM to 0V (safe default state)
2. Verify shutter is closed
3. Open shutter (enable laser output)

### Phase 2: Voltage Sweep
1. Sweep DAC0 from -5V to +5V in 0.1V steps (101 points)
2. At each step:
   - Set voltage via Comedi DAQ
   - Wait 500ms for settling
   - Read power from Newport 1830-C
3. Record voltage-power pairs

### Phase 3: Safe Shutdown
1. Reset EOM to 0V
2. Close shutter
3. Save data to HDF5

## Running the Test

### Prerequisites

```bash
# On maitai machine
cd ~/rust-daq

# Verify devices are accessible
ls -la /dev/comedi0
ls -la /dev/serial/by-id/usb-Silicon_Labs*
ls -la /dev/ttyS0
```

### Execute Test

```bash
# Enable the test (disabled by default for safety)
export EOM_SWEEP_TEST=1

# Run with default parameters (-5V to +5V, 0.1V step)
cargo test --features hardware -p driver-comedi \
    --test eom_power_sweep -- --nocapture --test-threads=1

# Custom voltage range
export EOM_VOLTAGE_MIN=-2.0
export EOM_VOLTAGE_MAX=2.0
export EOM_VOLTAGE_STEP=0.05
cargo test --features hardware -p driver-comedi \
    --test eom_power_sweep -- --nocapture --test-threads=1
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `EOM_SWEEP_TEST` | (unset) | Must be `1` to enable test |
| `COMEDI_DEVICE` | `/dev/comedi0` | Comedi DAQ device |
| `MAITAI_PORT` | (by-id path) | MaiTai serial port |
| `NEWPORT_PORT` | `/dev/ttyS0` | Power meter serial port |
| `EOM_VOLTAGE_MIN` | `-5.0` | Minimum sweep voltage |
| `EOM_VOLTAGE_MAX` | `5.0` | Maximum sweep voltage |
| `EOM_VOLTAGE_STEP` | `0.1` | Voltage step size |
| `EOM_OUTPUT_DIR` | `~/rust-daq/data` | HDF5 output directory |

## Output Data Format

### HDF5 Structure (xarray-compatible)

```
eom_sweep_YYYYMMDD_HHMMSS.h5
├── voltage (1D array, 101 elements)
│   └── attrs:
│       ├── units = "V"
│       └── long_name = "EOM Control Voltage"
├── power (1D array, 101 elements)
│   └── attrs:
│       ├── units = "W"
│       ├── long_name = "Optical Power"
│       └── _ARRAY_DIMENSIONS = ["voltage"]
└── Root attrs:
    ├── experiment = "EOM Power Sweep"
    ├── timestamp = "2026-01-26T20:33:59..."
    ├── instrument = "MaiTai + Comedi DAQ + Newport 1830-C"
    ├── voltage_min = -5.0
    ├── voltage_max = 5.0
    ├── voltage_step = 0.1
    ├── n_points = 101
    ├── min_power_W = 0.002246
    ├── max_power_W = 0.015886
    ├── extinction_ratio = 7.07
    └── voltage_at_min_power = 0.7
```

## Python Analysis

### Load and Plot

```python
import xarray as xr
import matplotlib.pyplot as plt

# Load data
ds = xr.open_dataset('eom_sweep_20260126_203359.h5', engine='h5netcdf')

# Quick plot
ds.power.plot()
plt.xlabel('EOM Voltage (V)')
plt.ylabel('Power (W)')
plt.title('EOM Transfer Function')
plt.grid(True)
plt.savefig('eom_transfer_function.png')
```

### Analysis with Scipp

```python
import scipp as sc

# Load as scipp dataset
data = sc.io.load_hdf5('eom_sweep_20260126_203359.h5')

# Convert to mW
power_mw = data['power'] * sc.scalar(1000.0, unit='mW/W')

# Find minimum
min_idx = sc.argmin(power_mw)
print(f"Minimum power: {power_mw[min_idx].value:.3f} mW")
print(f"At voltage: {data.coords['voltage'][min_idx].value:.2f} V")
```

### Extract Summary Statistics

```python
import h5py

with h5py.File('eom_sweep_20260126_203359.h5', 'r') as f:
    print(f"Experiment: {f.attrs['experiment']}")
    print(f"Timestamp: {f.attrs['timestamp']}")
    print(f"Min power: {f.attrs['min_power_W']*1000:.3f} mW")
    print(f"Max power: {f.attrs['max_power_W']*1000:.3f} mW")
    print(f"Extinction ratio: {f.attrs['extinction_ratio']:.1f}:1")
    print(f"Optimal voltage: {f.attrs['voltage_at_min_power']:.2f} V")
```

## Expected Results

### Typical Transfer Function

The EOM produces a sinusoidal power response:

```
Power (mW)
   16 ┤                                                    ╭─
   14 ┤╭─╮                                              ╭─╯
   12 ┤│  ╰╮                                          ╭╯
   10 ┤│    ╰╮                                      ╭╯
    8 ┤│      ╰╮                                  ╭╯
    6 ┤│        ╰╮                              ╭╯
    4 ┤│          ╰╮                          ╭╯
    2 ┤│            ╰──────────────────────╯
    0 ┼┼────────────────────────────────────────────────────
     -5  -4  -3  -2  -1   0   1   2   3   4   5
                    EOM Voltage (V)
```

### Key Metrics (maitai, 2026-01-26)

| Metric | Value |
|--------|-------|
| Minimum Power | 2.25 mW at +0.7V |
| Maximum Power | 15.9 mW at +4.8V |
| Extinction Ratio | 7.1:1 (8.5 dB) |
| Power at 0V | 3.1 mW |

### Physical Interpretation

- **Minimum (~0.7V)**: Pockels cell achieves maximum extinction
- **Maximum (~±5V)**: Approaches half-wave voltage (Vπ)
- **Sinusoidal shape**: Expected for crossed polarizer + Pockels cell
- **Asymmetry**: Due to residual birefringence or alignment

## Troubleshooting

### "Failed to open Comedi device"
```bash
# Check permissions
ls -la /dev/comedi0
# Should show: crw-rw---- 1 root comedi

# Add user to comedi group
sudo usermod -aG comedi $USER
# Log out and back in
```

### "Failed to open MaiTai serial port"
```bash
# Check USB connection
ls -la /dev/serial/by-id/

# Verify port permissions
sudo chmod 666 /dev/serial/by-id/usb-Silicon_Labs*
```

### "Power meter read timeout"
- Check `/dev/ttyS0` cable connection
- Verify power meter is in remote mode
- Check RS-232 null modem cable orientation

### Low Extinction Ratio (<5:1)
- Check polarizer alignment
- Verify Pockels cell crystal alignment
- Check for beam clipping in EOM aperture

## Safety Considerations

### Laser Safety
- This test opens the MaiTai shutter and produces laser output
- Ensure proper laser safety enclosure is in place
- Wear appropriate laser safety glasses (OD 5+ at 700-1000nm)
- Verify interlock system is functional

### Electrical Safety
- DAC0 connects to high-voltage EOM amplifier
- Do not exceed ±10V DAC range
- Test automatically resets to 0V on completion

### Software Safety
- Test is disabled by default (requires `EOM_SWEEP_TEST=1`)
- Shutter closes automatically on test completion
- EOM voltage resets to 0V on completion

## Related Tests

- `analog_loopback.rs` - DAC/ADC loopback verification (uses DAC1→ACH0)
- `hardware_smoke.rs` - Basic Comedi device connectivity
- `maitai_validation.rs` - MaiTai laser communication tests
- `newport1830c_validation.rs` - Power meter communication tests

## Version History

| Date | Version | Changes |
|------|---------|---------|
| 2026-01-26 | 1.0 | Initial implementation with HDF5 output |

## See Also

- [Comedi Setup Guide](comedi-setup.md)
- [BNC-2110 Channel Mapping](../../CLAUDE.md#comedi-daq-ni-pci-mio-16xe-10)
- [Storage Formats Guide](storage-formats.md)
