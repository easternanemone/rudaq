# rust-daq Python Client

Modern Python client library for controlling the rust-daq headless daemon via gRPC.

## Overview

The `rust-daq-client` library provides four API layers:

- **Layer 0**: Auto-generated protobuf stubs (`.generated` submodule)
- **Layer 1**: `AsyncClient` - Robust async-first gRPC wrapper
- **Layer 2**: High-level synchronous API - Ophyd/Bluesky-style device abstractions
- **Layer 3**: Async streaming - Context-managed frame/parameter streams

**Choose your API based on your needs:**
- Use **Layer 2** for interactive use, scripts, and Jupyter notebooks (recommended)
- Use **Layer 3** for real-time streaming applications (cameras, live monitoring)
- Use **Layer 1** for async applications and maximum control
- Use **Layer 0** only if you need raw protobuf access

## Installation

### From PyPI (when published)

```bash
# Basic installation
pip install rust-daq-client

# With scan support (pandas, tqdm)
pip install rust-daq-client[scan]

# With Jupyter support (ipywidgets, notebook, matplotlib, plotly)
pip install rust-daq-client[jupyter]

# With dev dependencies
pip install rust-daq-client[dev]

# Everything
pip install rust-daq-client[all]
```

### From Source

```bash
cd clients/python

# Install in development mode
pip install -e .

# Or with optional dependencies
pip install -e ".[scan,dev]"
```

## Quick Start

### Layer 2: High-Level Synchronous API (Recommended)

Intuitive property-based interface for scientists:

```python
from rust_daq import connect, Motor, Detector, run, scan

# Connect to daemon
with connect("localhost:50051"):
    # Create devices
    motor = Motor("mock_stage")
    detector = Detector("mock_power_meter")

    # Property-based control
    motor.position = 10.0
    print(f"Position: {motor.position} {motor.units}")

    # Read detector
    value = detector.read()
    print(f"Reading: {value} {detector.units}")

    # Run a scan
    with run(name="Test Scan"):
        data = scan(
            detectors=[detector],
            motor=motor,
            start=0, stop=100, steps=11,
            dwell_time=0.1
        )

    print(data)  # pandas DataFrame
```

### Layer 1: Async Client API

For async applications and advanced control:

```python
import anyio
from rust_daq import AsyncClient

async def main():
    async with AsyncClient("localhost:50051") as client:
        # Get daemon info
        info = await client.get_daemon_info()
        print(f"Daemon version: {info['version']}")

        # List devices
        devices = await client.list_devices()
        for device in devices:
            print(f"Device: {device['id']} ({device['driver_type']})")

        # Move a motor
        await client.move_absolute("mock_stage", 10.0, wait_for_completion=True)

        # Get current position
        position = await client.get_position("mock_stage")
        print(f"Position: {position}")

anyio.run(main)
```

### Layer 3: Async Streaming API

For real-time data streaming with context managers:

```python
import asyncio
from rust_daq import AsyncClient, FrameStream, ParameterSubscription

async def main():
    async with AsyncClient() as client:
        # Stream camera frames with automatic cleanup
        async with FrameStream(client, "camera_0", max_frames=100) as stream:
            async for frame in stream:
                arr = frame.to_numpy()  # Convert to numpy array
                print(f"Frame {frame.frame_number}: mean={arr.mean():.1f}")

        # Subscribe to parameter changes
        async with ParameterSubscription(client, "laser") as sub:
            async for change in sub:
                print(f"{change.name}: {change.old_value} -> {change.new_value}")

asyncio.run(main)
```

## Layer 2 API Reference

### Context Managers

#### `connect(host, timeout)`

Connect to the rust-daq daemon.

```python
with connect("localhost:50051", timeout=10.0):
    # Your code here
    pass
```

**Parameters:**
- `host` (str): Daemon address in "host:port" format. Default: "localhost:50051"
- `timeout` (float): Default timeout for operations in seconds. Default: 10.0

#### `run(name, metadata)`

Start a data acquisition run (placeholder for future StartRun/StopRun).

```python
with run(name="My Scan", metadata={"operator": "Alice"}):
    # Acquisition code
    pass
```

**Parameters:**
- `name` (str): Run name/identifier
- `metadata` (dict, optional): Metadata dictionary

### Device Classes

#### `Device(device_id)`

Base class for all devices.

**Attributes:**
- `device_id` (str): Unique device identifier
- `name` (str): Human-readable device name
- `metadata` (dict): Device metadata
- `capabilities` (dict): Device capabilities

**Methods:**
- `id` (property): Get device ID

#### `Motor(device_id)`

Motor device for position control (requires Movable capability).

**Properties:**
- `position` (float): Current position (getter) or move to position (setter)
- `limits` (tuple): Position limits as (min, max)
- `units` (str): Position units

**Methods:**
- `move(target, wait=True)`: Move to absolute position
  - Returns: `None` if wait=True, `Status` if wait=False
- `move_relative(distance, wait=True)`: Move by relative distance
  - Returns: `None` if wait=True, `Status` if wait=False

**Example:**
```python
motor = Motor("mock_stage")
motor.position = 10.0  # Absolute move
print(motor.position)  # Read position
motor.move_relative(5.0)  # Relative move
```

#### `Detector(device_id)`

Detector device for scalar measurements (requires Readable capability).

**Properties:**
- `units` (str): Reading units

**Methods:**
- `read()`: Read current value
  - Returns: `float`

**Example:**
```python
detector = Detector("mock_power_meter")
value = detector.read()
print(f"{value} {detector.units}")
```

#### `Status`

Status object for tracking non-blocking operations.

**Properties:**
- `done` (bool): Whether operation is complete

**Methods:**
- `wait(timeout=None)`: Block until operation completes

**Example:**
```python
status = motor.move(20.0, wait=False)
# Do other work...
status.wait()  # Block until complete
```

### Scan Function

#### `scan(detectors, motor, start, stop, steps, dwell_time, return_dict)`

Execute a 1D scan of detectors vs motor position.

**Parameters:**
- `detectors` (list): List of Detector objects to read
- `motor` (Motor): Motor object to scan
- `start` (float): Starting position
- `stop` (float): Ending position
- `steps` (int): Number of steps (positions)
- `dwell_time` (float, optional): Time to wait at each position (seconds). Default: 0.0
- `return_dict` (bool, optional): Return dict instead of DataFrame. Default: False

**Returns:**
- `pandas.DataFrame` with columns: position, <detector_names> (if pandas installed)
- `dict` if return_dict=True or pandas not installed

**Example:**
```python
data = scan(
    detectors=[det1, det2],
    motor=motor,
    start=0, stop=100, steps=11,
    dwell_time=0.1
)

print(data.head())  # pandas DataFrame
```

## Layer 1 API Reference

### AsyncClient

Full async API documentation is available in the docstrings. Key methods:

**Connection:**
- `async with AsyncClient(address, timeout) as client:` - Context manager
- `await client.connect()` - Manual connection
- `await client.close()` - Close connection

**Device Discovery:**
- `await client.list_devices(capability_filter)` - List all devices
- `await client.get_device_state(device_id)` - Get device state

**Motion Control:**
- `await client.move_absolute(device_id, position, wait_for_completion, timeout_ms)`
- `await client.move_relative(device_id, distance, wait_for_completion, timeout_ms)`
- `await client.get_position(device_id)`

**Parameters:**
- `await client.set_parameter(device_id, parameter_name, value)`
- `await client.get_parameter(device_id, parameter_name)`

**Streaming:**
- `async for update in client.stream_device_state(device_ids, max_rate_hz):`

## Layer 3 API Reference

### Streaming Classes

#### `FrameStream(client, device_id, include_pixel_data, max_frames, on_frame)`

Async context manager for streaming camera frames.

```python
async with FrameStream(client, "camera_0", max_frames=100) as stream:
    async for frame in stream:
        arr = frame.to_numpy()
        print(f"Frame {frame.frame_number}: shape={arr.shape}")
    print(f"Total: {stream.frame_count} frames")
```

**Parameters:**
- `client` (AsyncClient): Connected client instance
- `device_id` (str): Camera device ID
- `include_pixel_data` (bool): Include raw pixel bytes. Default: True
- `max_frames` (int, optional): Stop after N frames. Default: None (unlimited)
- `on_frame` (callable, optional): Callback for each frame

**Properties:**
- `frame_count` (int): Number of frames received
- `is_active` (bool): Whether stream is active

#### `Frame`

Camera frame dataclass with numpy conversion.

**Attributes:**
- `device_id` (str): Camera device ID
- `frame_number` (int): Sequential frame number
- `width` (int): Frame width in pixels
- `height` (int): Frame height in pixels
- `timestamp_ns` (int): Capture timestamp (nanoseconds)
- `pixel_data` (bytes | None): Raw pixel bytes
- `pixel_format` (str): Format string ("u8", "u16_le", "f32_le")

**Methods:**
- `to_numpy()`: Convert to numpy array with shape (height, width)

#### `ParameterSubscription(client, device_id, parameter_names, on_change)`

Async context manager for monitoring parameter changes.

```python
async with ParameterSubscription(client, device_id="laser") as sub:
    async for change in sub:
        print(f"{change.name}: {change.old_value} -> {change.new_value}")
        if sub.change_count >= 10:
            break
```

**Parameters:**
- `client` (AsyncClient): Connected client instance
- `device_id` (str, optional): Filter by device ID. Default: None (all devices)
- `parameter_names` (list[str], optional): Filter by parameter names
- `on_change` (callable, optional): Callback for each change

**Properties:**
- `change_count` (int): Number of changes received
- `is_active` (bool): Whether subscription is active

#### `ParameterChange`

Parameter change event dataclass.

**Attributes:**
- `device_id` (str): Device ID
- `name` (str): Parameter name
- `old_value` (str): Previous value
- `new_value` (str): New value
- `units` (str): Parameter units

**Methods:**
- `old_as_float()`: Parse old_value as float
- `new_as_float()`: Parse new_value as float

#### `DeviceStateStream(client, device_ids, max_rate_hz, include_snapshot)`

Async context manager for streaming device state updates.

```python
async with DeviceStateStream(client, max_rate_hz=10) as stream:
    async for update in stream:
        print(f"{update['device_id']}: {update['fields']}")
```

**Parameters:**
- `client` (AsyncClient): Connected client instance
- `device_ids` (list[str], optional): Filter by device IDs. Default: None (all)
- `max_rate_hz` (int): Maximum update rate. Default: 10
- `include_snapshot` (bool): Include full snapshot as first message. Default: True

**Properties:**
- `update_count` (int): Number of updates received
- `is_active` (bool): Whether stream is active

## Starting the Daemon

Before using the client, start the rust-daq daemon:

```bash
# From the rust-daq project root
cargo run --features networking -- daemon --port 50051
```

Or with specific hardware:

```bash
cargo run --features "networking,all_hardware" -- daemon --port 50051
```

## Examples

See the `examples/` directory:

- `01_basic_client.py` - Layer 1 async client basics
- `02_device_control.py` - Device control patterns
- `09_ring_buffer_tap.py` - Zero-copy mmap ring buffer access
- `10_async_streaming.py` - Layer 3 async streaming demo
- `layer2_demo.py` - Layer 2 synchronous API demo

Run examples:

```bash
# Ensure daemon is running
python examples/layer2_demo.py
```

## Jupyter Notebooks

Interactive example notebooks are available in `notebooks/`:

- `01_getting_started.ipynb` - Introduction to rust-daq in Jupyter
- `02_interactive_control.ipynb` - Widgets and dashboards
- `03_live_plotting.ipynb` - Real-time visualization
- `04_advanced_scans.ipynb` - Complex experimental patterns

**Install Jupyter support:**
```bash
pip install rust-daq-client[jupyter]
```

**Features:**
- Rich HTML device representations
- Interactive motor sliders
- Live detector displays
- Real-time plotting (matplotlib & plotly)
- Multi-device dashboards
- Progress bars with tqdm

See [notebooks/README.md](notebooks/README.md) for details.

## Testing

```bash
# Install dev dependencies
pip install -e ".[dev]"

# Run unit tests only
pytest -m "not integration"

# Run all tests (requires running daemon)
pytest

# Run integration tests only
pytest -m integration
```

## Error Handling

The library provides custom exceptions:

- `DaqError` - Base exception for all errors
- `DeviceError` - Device-specific errors (device not found, wrong capability, etc.)
- `CommunicationError` - Network/connection errors
- `TimeoutError` - Operation timeout
- `ConfigurationError` - Invalid configuration

```python
from rust_daq import connect, Motor, DeviceError

try:
    with connect():
        motor = Motor("nonexistent_device")
except DeviceError as e:
    print(f"Device error: {e}")
```

## Requirements

- Python >= 3.8
- grpcio >= 1.50
- protobuf >= 4.20
- anyio >= 3.0
- numpy >= 1.20

Optional (for scan support):
- pandas >= 1.3
- tqdm >= 4.60

## Development

### Building from Source

```bash
cd clients/python

# Install build dependencies
pip install build

# Build wheel
python -m build

# Install locally
pip install dist/*.whl
```

### Running Tests

```bash
# Install dev dependencies
pip install -e ".[dev]"

# Run tests
pytest

# Run with coverage
pytest --cov=rust_daq --cov-report=html
```

### Code Formatting

```bash
# Format with black
black src/ tests/

# Lint with ruff
ruff check src/ tests/

# Type check with mypy
mypy src/
```

## Architecture

The client uses a 4-layer architecture:

```
┌─────────────────────────────────────────┐
│  Layer 3: Async Streaming               │
│  (FrameStream, ParameterSubscription)   │
├─────────────────────────────────────────┤
│  Layer 2: High-Level Sync API           │
│  (Motor, Detector, scan)                │
├─────────────────────────────────────────┤
│  Layer 1: AsyncClient                   │
│  (async gRPC wrapper)                   │
├─────────────────────────────────────────┤
│  Layer 0: Auto-generated Protobuf       │
│  (daq_pb2.py, daq_pb2_grpc.py)          │
└─────────────────────────────────────────┘
         ↓ gRPC (HTTP/2)
┌─────────────────────────────────────────┐
│  rust-daq Daemon                        │
│  (Rust headless server)                 │
└─────────────────────────────────────────┘
```

**Layer 2** uses `anyio.from_thread.start_blocking_portal()` to provide a synchronous wrapper around Layer 1's async API.

**Layer 3** provides async context managers that wrap Layer 1's streaming methods for clean resource management.

## Troubleshooting

**"No active connection" error**
- Ensure you're using the `with connect():` context manager

**"Device not found" error**
- Check that the device ID is correct
- Use `AsyncClient.list_devices()` to see available devices

**"Device does not have X capability" error**
- You're trying to use a Motor on a non-movable device (or Detector on non-readable)
- Check device capabilities with `device.capabilities`

**Connection refused**
- Ensure daemon is running: `cargo run --features networking -- daemon --port 50051`
- Check firewall settings
- Verify port number matches

**pandas not installed**
- Install with: `pip install rust-daq-client[scan]`
- Or use `return_dict=True` in scan()

## License

Same as rust-daq project.
