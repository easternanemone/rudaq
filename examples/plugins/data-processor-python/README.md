# Python FFT Data Processor Plugin

A script-based module demonstrating Python integration via PyO3 for real-time FFT analysis and signal processing of acquired data streams.

## Overview

This plugin showcases how to use Python and NumPy within the rust_daq plugin system for data processing tasks. It demonstrates:

- **Python Module via PyO3Engine**: Script-based module using the `python` engine type
- **NumPy Integration**: FFT computation and signal processing using NumPy
- **Real-time Signal Processing**: Overlapping FFT windows for continuous analysis
- **Configurable Windowing**: Hanning, Hamming, Blackman, and Kaiser window functions

## Requirements

- rust_daq built with `python` feature enabled in `scripting`
- Python 3.8+ with NumPy installed

### Enable Python Support

In `Cargo.toml` for `scripting`, ensure the `python` feature is enabled:

```toml
[dependencies]
scripting = { path = "../scripting", features = ["python"] }
```

### Install Python Dependencies

```bash
pip install -r requirements.txt
# Or with uv:
uv pip install -r requirements.txt
```

## Usage

### Loading the Plugin

```rust
use scripting::plugins::ScriptPluginLoader;

// Create loader and add search path
let mut loader = ScriptPluginLoader::new();
loader.add_search_path("./examples/plugins/data-processor-python");

// Discover plugins
loader.discover().await?;

// Create module instance
let module = loader.create_module("python_fft_processor").await?;
```

### Configuration

Configure the module with FFT parameters:

```rust
use std::collections::HashMap;

let mut params = HashMap::new();
params.insert("fft_size".to_string(), "2048".to_string());
params.insert("window_type".to_string(), "hanning".to_string());
params.insert("overlap_percent".to_string(), "50.0".to_string());
params.insert("sample_rate_hz".to_string(), "44100.0".to_string());
params.insert("output_type".to_string(), "magnitude".to_string());

module.configure(params)?;
```

## Parameters

| Parameter | Type | Default | Range | Description |
|-----------|------|---------|-------|-------------|
| `fft_size` | int | 1024 | 64-16384 | FFT window size (power of 2) |
| `window_type` | enum | hanning | none/hanning/hamming/blackman/kaiser | Window function |
| `overlap_percent` | float | 50.0 | 0-90 | Overlap between windows (%) |
| `sample_rate_hz` | float | 10000.0 | 1-1000000 | Input sample rate (Hz) |
| `output_type` | enum | magnitude | magnitude/power/psd/phase | Output format |

## Output Types

- **magnitude**: |FFT| / N - normalized magnitude spectrum
- **power**: |FFT|² / N - power spectrum
- **psd**: |FFT|² / (fs × window_power) - power spectral density
- **phase**: arg(FFT) - phase spectrum in radians

## Module Interface

The Python script implements the standard rust_daq ScriptModule interface:

```python
def module_type_info() -> dict:
    """Return module metadata for registration."""

def configure(params: dict) -> list:
    """Configure module, return warnings."""

def get_config() -> dict:
    """Return current configuration."""

def stage(ctx: dict) -> None:
    """Prepare resources before start."""

def start(ctx: dict) -> None:
    """Main execution entry point."""

def pause() -> None:
    """Pause execution."""

def resume() -> None:
    """Resume execution."""

def stop() -> None:
    """Stop execution."""

def unstage(ctx: dict) -> None:
    """Release resources."""
```

## Signal Processing Functions

The module also exposes utility functions for data processing:

```python
def process_fft(data: np.ndarray) -> tuple:
    """Perform FFT on input data, return (frequencies, output)."""

def add_samples(samples: list) -> list:
    """Add samples to buffer, return FFT results for complete windows."""
```

## Example: Processing Acquired Data

```python
# In your acquisition loop:
results = processor.add_samples(new_samples)
for freqs, magnitudes in results:
    peak_idx = np.argmax(magnitudes)
    peak_freq = freqs[peak_idx]
    print(f"Peak frequency: {peak_freq:.2f} Hz")
```

## Architecture

```
rust_daq
├── scripting (PyO3Engine)
│   └── pyo3_engine.rs - Python interpreter integration
├── rust-daq/plugins
│   ├── loader.rs - ScriptPluginLoader
│   └── script_module.rs - ScriptModule trait impl
└── examples/plugins/data-processor-python
    ├── plugin.toml - Plugin manifest
    ├── processor.py - Python module implementation
    └── requirements.txt - Python dependencies
```

## Related

- `examples/scripts/power_logger.rhai` - Rhai script module example
- `crates/scripting/src/pyo3_engine.rs` - PyO3 engine implementation
- `crates/hardware/src/plugin/manifest.rs` - Plugin manifest format
