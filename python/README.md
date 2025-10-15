# rust-daq Python Bindings

This directory contains the Python bindings for the `rust-daq` project, enabling high-performance data acquisition and instrument control from Python.

The bindings are built using [PyO3](https://pyo3.rs/) and packaged with [maturin](https://www.maturin.rs/).

## Development Setup

These instructions guide you through setting up a local development environment to work on the Python bindings.

### Prerequisites

-   [Rust toolchain](https://www.rust-lang.org/tools/install) (latest stable version recommended)
-   Python 3.8+
-   A Python virtual environment tool (e.g., `venv`, `virtualenv`)

### Installation

1.  **Navigate to the `python` directory:**

    ```bash
    cd python
    ```

2.  **Create and activate a Python virtual environment:**

    ```bash
    python3 -m venv .venv
    source .venv/bin/activate
    ```
    *On Windows, use `.venv\Scripts\activate`*

3.  **Install `maturin`:**

    `maturin` is the build tool used to compile the Rust code into a Python extension module.

    ```bash
    pip install maturin
    ```

4.  **Install the package in editable mode:**

    This command compiles the Rust extension and links it into your current environment. The `--release` flag is optional but recommended for performance testing. For development and debugging, you can omit it.

    ```bash
    maturin develop
    ```
    Alternatively, you can use `pip`:
    ```bash
    pip install -e .
    ```

The package `rust_daq` is now installed and ready to use in your active virtual environment.

## API Overview

The initial Python surface area focuses on the most commonly scripted components from the Rust core:

- `rust_daq.DataPoint` — Rich measurement container with timestamp, channel, engineering unit, and optional JSON metadata. Useful for assembling synthetic traces or inspecting acquisition output.
- `rust_daq.MaiTai` — Mock MaiTai laser driver shell exposing wavelength control. Real hardware integrations can extend this class via the guidance in `docs/developer_guide.md`.
- `rust_daq.Newport1830C` — Mock Newport 1830C power meter with a simple `read_power()` entry point that returns a floating-point watt measurement.

Refer to `docs/api_guide.md` for full signatures, type hints, and advanced usage patterns, and `docs/developer_guide.md` for extending the bindings with additional instruments.

## Usage Example

Once installed, you can import and use the exposed Rust objects just like any other Python package.

```python
import rust_daq
from datetime import datetime, timezone

def main():
    print("--- Testing MaiTai Laser ---")
    laser = rust_daq.MaiTai(port="COM3")
    laser.set_wavelength(800.5)
    print()

    print("--- Testing Newport Power Meter ---")
    power_meter = rust_daq.Newport1830C(resource_string="USB0::0x1234::0x5678::SN910::INSTR")
    power = power_meter.read_power()
    print(f"Measured power: {power:.6f} W")
    print()

    print("--- Testing DataPoint ---")
    dp = rust_daq.DataPoint(
        timestamp=datetime.now(timezone.utc),
        channel="power_reading",
        value=power,
        unit="W",
        metadata={"instrument": "Newport1830C", "status": "ok"}
    )
    print(repr(dp))
    print(f"Timestamp: {dp.timestamp}")
    print(f"Metadata: {dp.metadata}")


if __name__ == "__main__":
    main()

```

## Documentation

- [API Guide](docs/api_guide.md) — Detailed class reference with runnable examples.
- [Developer Guide](docs/developer_guide.md) — Instructions for adding new bindings, testing, and debugging the PyO3 layer.

## Building a Wheel

To distribute the package, you can build a standard Python wheel:

```bash
maturin build --release
```

This will create a `.whl` file in `target/wheels/` that can be installed with `pip`.
