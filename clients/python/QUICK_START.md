# Quick Start Guide - rust-daq Python Client

## One-Command Setup

```bash
cd clients/python
./setup.sh
```

This will:
1. Create a Python virtual environment
2. Install all dependencies
3. Generate gRPC code from proto files
4. Fix import paths
5. Verify the installation

## Running Examples

### 1. Start the Daemon

In one terminal:
```bash
cd /path/to/rust-daq
cargo run -- daemon --port 50051
```

### 2. Activate Virtual Environment

In another terminal:
```bash
cd clients/python
source venv/bin/activate
```

### 3. Run Examples

**Basic usage** (upload, execute, monitor):
```bash
python examples/basic_usage.py
```

**Stream system status** (real-time monitoring):
```bash
python examples/stream_status.py
```

**Stream measurements** (instrument-specific):
```bash
python examples/stream_measurements.py stage
```

**Main demo** (all features):
```bash
python daq_client.py
```

## Example Code

```python
from daq_client import DaqClient

# Connect to daemon
client = DaqClient(host='localhost', port=50051)

# Upload and run script
script = """
stage.move_abs(5.0);
camera.trigger();
"""
script_id = client.upload_script(script, name="test")
exec_id = client.start_script(script_id)

# Check status
status = client.get_status(exec_id)
print(f"State: {status['state']}")

# Clean up
client.close()
```

## Troubleshooting

**"Module not found" errors**
```bash
./generate_proto.sh
```

**"Connection refused"**
- Make sure daemon is running
- Check port number (default: 50051)

**Python version issues**
- Requires Python 3.7+
- Tested with Python 3.14

## API Methods

- `upload_script(content, name)` - Upload Rhai script
- `start_script(script_id)` - Start execution
- `stop_script(exec_id)` - Stop execution
- `get_status(exec_id)` - Get execution status
- `stream_status()` - Real-time system updates
- `stream_measurements(instrument)` - Real-time data
- `close()` - Close connection

See `README.md` for complete documentation.
