# rust-daq Jupyter Notebooks

Example notebooks demonstrating the rust-daq Python client in Jupyter environments.

## Prerequisites

1. **Install the Python client with Jupyter support:**
   ```bash
   pip install rust-daq-client[jupyter]
   ```

2. **Start the rust-daq daemon:**
   ```bash
   # From the rust-daq repository root
   cargo run --features networking -- daemon --port 50051
   ```

3. **Launch Jupyter:**
   ```bash
   jupyter notebook
   ```

## Notebooks

### 01_getting_started.ipynb
Introduction to rust-daq in Jupyter notebooks.

**Topics:**
- Connecting to the daemon
- Rich HTML device representations
- Basic motor control and detector reads
- Simple 1D scans
- Plotting results with matplotlib

**Best for:** First-time users learning the basics.

---

### 02_interactive_control.ipynb
Interactive device control with ipywidgets.

**Topics:**
- Motor slider widgets
- Live detector displays
- Multi-device dashboards
- Custom widget layouts
- Combining widgets with programmatic control

**Best for:** Building interactive GUIs in notebooks.

---

### 03_live_plotting.ipynb
Real-time data visualization during experiments.

**Topics:**
- Live matplotlib plots
- Live plotly plots (interactive)
- Progress bars with tqdm.notebook
- Multi-dataset plotting
- Exporting data for analysis

**Best for:** Monitoring experiments in real-time.

---

### 04_advanced_scans.ipynb
Complex experimental patterns and adaptive scanning.

**Topics:**
- 2D grid scans with heatmap visualization
- Adaptive scans (data-driven positioning)
- Custom scan patterns (spiral, circular)
- Multi-detector experiments
- Advanced data analysis and export

**Best for:** Complex experiments and optimization.

---

## Features

### Rich HTML Representations
Devices automatically display formatted information in Jupyter:

```python
from rust_daq import Motor
from rust_daq.jupyter import quick_connect

with quick_connect():
    motor = Motor("mock_stage")
    motor  # Displays rich HTML with position, limits, status
```

### Interactive Widgets
Control hardware with sliders and buttons:

```python
from rust_daq.jupyter import create_motor_slider, dashboard

slider = create_motor_slider(motor)
display(slider)

# Or create a full dashboard
dashboard(motor1, motor2, detector)
```

### Live Plotting
Visualize data as it's acquired:

```python
from rust_daq.jupyter import live_scan_plot

data = live_scan_plot(
    motor=motor,
    detector=detector,
    start=0, stop=100, steps=50,
    backend='matplotlib'  # or 'plotly'
)
```

## Tips

### Connection Management
For interactive notebook use, manually manage the connection:

```python
conn = quick_connect()
conn.__enter__()

# ... your work ...

conn.__exit__(None, None, None)
```

### Progress Bars
The library automatically uses `tqdm.notebook` for progress bars when available:

```python
# Progress bars appear automatically in scan()
data = scan(detectors=[det], motor=motor, start=0, stop=100, steps=50)
```

### Graceful Degradation
All Jupyter features are optional. The library works without:
- `ipywidgets` - widgets won't be available
- `matplotlib` - plotting won't work
- `plotly` - alternative to matplotlib
- `notebook` - can use JupyterLab instead

Missing dependencies produce warnings but don't break core functionality.

## Troubleshooting

### "No active connection" error
Make sure the connection context manager is active:
```python
with quick_connect():
    # Your code here
    pass
```

### Widgets not displaying
1. Ensure ipywidgets is installed: `pip install ipywidgets`
2. Enable the extension: `jupyter nbextension enable --py widgetsnbextension`
3. For JupyterLab: `jupyter labextension install @jupyter-widgets/jupyterlab-manager`

### Matplotlib plots not updating
Use `%matplotlib notebook` for interactive plots:
```python
%matplotlib notebook
# Now live plots will work
```

### Daemon not found
Verify the daemon is running and accessible:
```bash
# Check if daemon is listening
netstat -an | grep 50051

# Test connection
grpcurl -plaintext localhost:50051 list
```

## Additional Resources

- [Python Client Documentation](../README.md)
- [rust-daq Main Documentation](../../../docs/)
- [gRPC API Reference](../../../proto/daq.proto)

## Contributing

Found an issue or have an idea for a new notebook example? Please open an issue or pull request!
