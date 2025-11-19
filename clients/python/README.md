# rust-daq Python Client

Remote control client for the rust-daq headless daemon using gRPC.

## Overview

This Python client allows you to:
- Upload Rhai scripts remotely
- Start script execution on the daemon
- Monitor execution status
- Stream real-time system status updates

## Installation

### Option 1: Using Virtual Environment (Recommended)

```bash
cd clients/python

# Create virtual environment
python3 -m venv venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Generate Python code from proto file
chmod +x generate_proto.sh
./generate_proto.sh

# Fix generated imports (run once after generation)
echo "from . import daq_pb2" > generated/__init__.py
sed -i '' 's/import daq_pb2/from . import daq_pb2/g' generated/daq_pb2_grpc.py
```

### Option 2: System-wide Installation

```bash
cd clients/python

# Install dependencies
pip3 install -r requirements.txt

# Generate Python code from proto file
chmod +x generate_proto.sh
./generate_proto.sh

# Fix generated imports
echo "from . import daq_pb2" > generated/__init__.py
sed -i '' 's/import daq_pb2/from . import daq_pb2/g' generated/daq_pb2_grpc.py
```

## Starting the Daemon

Before using the client, start the rust-daq daemon:

```bash
# From the rust-daq project root
cargo run -- daemon --port 50051
```

## Usage

### Basic Example

```python
from daq_client import DaqClient

# Connect to daemon
client = DaqClient(host='localhost', port=50051)

# Upload a script
script = """
print("Hello from Rhai!");
stage.move_abs(5.0);
camera.trigger();
"""
script_id = client.upload_script(script, name="my_experiment")

# Start execution
execution_id = client.start_script(script_id)

# Check status
status = client.get_status(execution_id)
print(f"State: {status['state']}")

# Clean up
client.close()
```

### Streaming Status Updates

```python
from daq_client import DaqClient

client = DaqClient()

# Stream real-time updates
for update in client.stream_status():
    print(f"State: {update['state']}")
    print(f"Memory: {update['memory_mb']:.1f} MB")
    print(f"Live values: {update['live_values']}")
    # Ctrl+C to stop
```

### Running Examples

```bash
# Activate virtual environment if using one
source venv/bin/activate

# Run the main demo
python daq_client.py

# Or run specific examples
python examples/basic_usage.py
python examples/stream_status.py
python examples/stream_measurements.py stage
```

## API Reference

### `DaqClient(host='localhost', port=50051)`

Create a new client connection.

**Parameters:**
- `host` (str): Hostname or IP address of the daemon
- `port` (int): gRPC port number

### `upload_script(script_content, name='experiment')`

Upload a Rhai script to the daemon.

**Parameters:**
- `script_content` (str): The Rhai script source code
- `name` (str): Optional name for the script

**Returns:**
- `str`: The script ID assigned by the daemon

**Raises:**
- `RuntimeError`: If upload fails

### `start_script(script_id)`

Start executing an uploaded script.

**Parameters:**
- `script_id` (str): The ID of the script to execute

**Returns:**
- `str`: The execution ID for tracking progress

**Raises:**
- `RuntimeError`: If start fails

### `stop_script(execution_id)`

Stop a running script execution.

**Parameters:**
- `execution_id` (str): The execution ID to stop

**Returns:**
- `bool`: True if stopped successfully

**Raises:**
- `RuntimeError`: If stop fails

### `get_status(execution_id)`

Get current status of script execution.

**Parameters:**
- `execution_id` (str): The execution ID to query

**Returns:**
- `dict`: Status information with keys:
  - `execution_id`: The execution ID
  - `state`: Current state ('IDLE', 'RUNNING', 'COMPLETED', 'ERROR')
  - `error`: Error message if any
  - `start_time`: Start timestamp in nanoseconds
  - `end_time`: End timestamp in nanoseconds

### `stream_status()`

Stream real-time system status updates.

**Yields:**
- `dict`: Status updates with keys:
  - `state`: Current system state ('IDLE', 'RUNNING', 'ERROR')
  - `memory_mb`: Current memory usage in MB
  - `live_values`: Dictionary of live instrument values
  - `timestamp`: Timestamp in nanoseconds

### `stream_measurements(instrument)`

Stream real-time measurement data from a specific instrument.

**Parameters:**
- `instrument` (str): Name of the instrument to stream from

**Yields:**
- `dict`: Measurement data with keys:
  - `instrument`: Instrument name
  - `value`: Scalar value (float) or None
  - `image`: Image data (bytes) or None
  - `timestamp`: Timestamp in nanoseconds

### `close()`

Close the gRPC channel.

## Protocol Buffer Definition

The client uses the proto definition at `../../src/network/proto/daq.proto`.

The service provides:
- `UploadScript`: Upload Rhai scripts
- `StartScript`: Start script execution
- `StopScript`: Stop running execution
- `GetScriptStatus`: Query execution status
- `StreamStatus`: Real-time status streaming
- `StreamMeasurements`: Real-time instrument data streaming

## Error Handling

All methods raise `RuntimeError` on failure with descriptive error messages.
Network errors are wrapped and re-raised with context.

## Example Workflow

```bash
# Terminal 1: Start daemon
cd /path/to/rust-daq
cargo run -- daemon --port 50051

# Terminal 2: Run client
cd clients/python
pip install -r requirements.txt
./generate_proto.sh
python daq_client.py
```

## Requirements

- Python 3.7+
- grpcio >= 1.59.0
- grpcio-tools >= 1.59.0
- rust-daq daemon running

## Troubleshooting

**"No module named 'generated'"**
- Run `./generate_proto.sh` to generate Python code from proto file

**"Failed to connect to daemon"**
- Ensure daemon is running: `cargo run -- daemon --port 50051`
- Check firewall settings
- Verify port number matches

**"Upload failed: ..."**
- Check Rhai script syntax
- Verify daemon has write permissions

## License

Same as rust-daq project.
