# rust-daq Python Examples

This directory contains practical examples demonstrating how to use the `rust_daq` Python bindings for scientific data acquisition and instrument control.

## Prerequisites

1. **Install the rust_daq package:**
   ```bash
   cd python
   python3 -m venv .venv
   source .venv/bin/activate  # On Windows: .venv\Scripts\activate
   pip install maturin
   maturin develop
   ```

2. **Install example dependencies:**
   ```bash
   pip install -r examples/requirements.txt
   ```

## Directory Structure

```
examples/
├── scripts/           Standalone Python scripts with CLI interfaces
├── notebooks/         Jupyter notebooks for interactive exploration
├── integration/       Integration examples with NumPy, pandas, etc.
├── advanced/          Advanced usage patterns and techniques
├── requirements.txt   Python dependencies for examples
└── README.md          This file
```

## Quick Start Examples

### Simple Acquisition (`scripts/simple_acquisition.py`)

Minimal example showing basic instrument initialization and data point creation:

```bash
python examples/scripts/simple_acquisition.py
```

**What it demonstrates:**
- Instrument initialization (MaiTai laser, Newport power meter)
- Single measurement acquisition
- DataPoint creation with metadata

**Expected output:** Console output showing instrument initialization, power measurement, and data point details.

### Wavelength Sweep (`scripts/wavelength_sweep.py`)

Production-ready automated wavelength scan with CLI arguments:

```bash
python examples/scripts/wavelength_sweep.py \
    --start 700 --stop 900 --step 10 \
    --output scan_results.csv
```

**What it demonstrates:**
- Argument parsing with argparse
- Automated scan control
- Progress logging
- CSV data export

**Expected output:** CSV file with wavelength vs power data, console log of scan progress.

### Continuous Monitoring (`scripts/continuous_monitor.py`)

Background acquisition with real-time statistics:

```bash
python examples/scripts/continuous_monitor.py --rate 10.0
```

**What it demonstrates:**
- Continuous data acquisition
- Rolling statistics window
- Graceful interrupt handling (Ctrl+C)
- Logging to file

**Expected output:** Real-time console display of statistics, log file with all samples.

## Integration Examples

### NumPy Integration (`integration/with_numpy.py`)

FFT analysis using NumPy:

```bash
python examples/integration/with_numpy.py
```

**What it demonstrates:**
- Converting DataPoints to NumPy arrays
- FFT signal processing
- Peak detection
- Statistical analysis

### pandas Integration (`integration/with_pandas.py`)

Time-series analysis and export with pandas:

```bash
python examples/integration/with_pandas.py
```

**What it demonstrates:**
- DataPoint to DataFrame conversion
- Time-based resampling
- Statistical operations
- Multi-format export (CSV, JSON, Excel, Parquet)

## Advanced Examples

### Async Acquisition (`advanced/async_acquisition.py`)

Concurrent instrument control with asyncio:

```bash
python examples/advanced/async_acquisition.py
```

**What it demonstrates:**
- Parallel instrument control
- Asyncio task coordination
- Performance comparison (sequential vs concurrent)

## Hardware vs Mock Instruments

All examples use **mock instruments** by default, which simulate hardware behavior without requiring physical connections. This allows you to:

- Test code without hardware
- Develop acquisition scripts offline
- Verify logic before deployment

### Using Real Hardware

To use real instruments, modify the connection parameters:

**For Serial Instruments (MaiTai):**
```python
# Mock (default)
laser = rust_daq.MaiTai(port="COM3")

# Real hardware - update port to match your system
laser = rust_daq.MaiTai(port="/dev/ttyUSB0")  # Linux
laser = rust_daq.MaiTai(port="COM5")          # Windows
```

**For VISA Instruments (Newport 1830C):**
```python
# Mock (default)
meter = rust_daq.Newport1830C(resource_string="USB0::0x104D::0xC0DE::SN12345::INSTR")

# Real hardware - get actual resource string with:
# python -m pyvisa-shell
# list
meter = rust_daq.Newport1830C(resource_string="USB0::0x104D::0xCEC7::PM002345::INSTR")
```

## Common Command-Line Options

Most scripts support these options:

- `--help` - Show all available options
- `--verbose` - Enable debug logging
- `--log-file PATH` - Log to file in addition to console
- `--output PATH` - Specify output file path

Example:
```bash
python script.py --help
python script.py --verbose --log-file debug.log
```

## Troubleshooting

### Import Error: `rust_daq` not found

Solution: Install the package with `maturin develop` from the `python/` directory.

### Serial Port Permission Denied (Linux)

Solution: Add your user to the `dialout` group:
```bash
sudo usermod -a -G dialout $USER
# Log out and back in for changes to take effect
```

### VISA Error: Resource not found

Solution:
1. Install VISA backend: `pip install pyvisa-py`
2. List available devices: `python -m pyvisa-shell`, then `list`
3. Update resource string in your script

## Next Steps

1. **Read the API Guide:** `docs/api_guide.md` - Full class reference
2. **Developer Guide:** `docs/developer_guide.md` - Extend bindings with new instruments
3. **Performance Guide:** `docs/performance.md` - Optimization and benchmarking
4. **Run Tests:** `pytest python/tests/` - Verify installation

## Contributing

To add new examples:

1. Follow existing naming patterns (`verb_noun.py`)
2. Include docstring with purpose and usage
3. Add argument parsing for scripts
4. Include expected output in comments
5. Test with both mock and real hardware (if available)

## License

See the main project LICENSE file.
