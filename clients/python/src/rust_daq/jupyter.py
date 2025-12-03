"""
Jupyter notebook integration for rust-daq.

Provides rich HTML representations, interactive widgets, live plotting,
and dashboard creation for Jupyter notebooks.

Features:
- Rich HTML device representations
- Interactive slider widgets for motor control
- Live plotting during scans
- Progress bars with tqdm.notebook
- Quick connection utilities

Example:
    from rust_daq import Motor, Detector
    from rust_daq.jupyter import quick_connect, dashboard

    with quick_connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        # Display devices with rich HTML
        display(motor, detector)

        # Create interactive dashboard
        dashboard(motor, detector)
"""

from typing import Optional, List, Any, Dict
import warnings
import time

# Optional dependencies - graceful degradation
try:
    import ipywidgets as widgets
    from IPython.display import display, HTML
    HAS_WIDGETS = True
except ImportError:
    HAS_WIDGETS = False
    widgets = None
    display = None
    HTML = None

try:
    import matplotlib.pyplot as plt
    from matplotlib.figure import Figure
    HAS_MATPLOTLIB = True
except ImportError:
    HAS_MATPLOTLIB = False
    plt = None
    Figure = None

try:
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots
    HAS_PLOTLY = True
except ImportError:
    HAS_PLOTLY = False
    go = None
    make_subplots = None

try:
    from tqdm.notebook import tqdm as tqdm_notebook
    HAS_TQDM_NOTEBOOK = True
except ImportError:
    HAS_TQDM_NOTEBOOK = False
    tqdm_notebook = None

try:
    import numpy as np
    HAS_NUMPY = True
except ImportError:
    HAS_NUMPY = False
    np = None


# Re-export from devices for convenience
from .devices import connect, Motor, Detector, scan


# ============================================================================
# Quick Connection Utilities
# ============================================================================


def quick_connect(host: str = "localhost:50051", timeout: float = 10.0):
    """
    Quick connection context manager for notebook use.

    Simplified version of connect() with sensible defaults for Jupyter.

    Args:
        host: Daemon address (default: "localhost:50051")
        timeout: Connection timeout in seconds

    Returns:
        Context manager for connection

    Example:
        with quick_connect():
            motor = Motor("mock_stage")
            print(motor.position)
    """
    return connect(host=host, timeout=timeout)


def show_devices():
    """
    Display all available devices in rich HTML table format.

    Returns:
        IPython.display.HTML object (auto-displayed in notebooks)

    Example:
        with quick_connect():
            show_devices()
    """
    if not HAS_WIDGETS:
        warnings.warn(
            "IPython not available - install with: pip install rust-daq-client[jupyter]",
            ImportWarning
        )
        return None

    from .core import AsyncClient
    from .devices import _get_client, _run_async

    client = _get_client()
    devices = _run_async(client.list_devices())

    # Build HTML table
    html = """
    <style>
        .daq-device-table {
            border-collapse: collapse;
            width: 100%;
            margin: 10px 0;
        }
        .daq-device-table th {
            background-color: #4CAF50;
            color: white;
            text-align: left;
            padding: 8px;
        }
        .daq-device-table td {
            border: 1px solid #ddd;
            padding: 8px;
        }
        .daq-device-table tr:nth-child(even) {
            background-color: #f2f2f2;
        }
        .daq-status-ready {
            color: green;
            font-weight: bold;
        }
    </style>
    <table class="daq-device-table">
        <tr>
            <th>Device ID</th>
            <th>Type</th>
            <th>Capabilities</th>
            <th>Status</th>
        </tr>
    """

    for dev in devices:
        dev_id = dev.get("id", "unknown")
        dev_type = dev.get("driver_type", "unknown")
        caps = dev.get("capabilities", {})
        cap_list = [k for k, v in caps.items() if v]
        cap_str = ", ".join(cap_list) if cap_list else "none"

        html += f"""
        <tr>
            <td><b>{dev_id}</b></td>
            <td>{dev_type}</td>
            <td>{cap_str}</td>
            <td class="daq-status-ready">● Ready</td>
        </tr>
        """

    html += "</table>"

    return HTML(html)


# ============================================================================
# Rich HTML Representations (Monkey-patching)
# ============================================================================


def _motor_repr_html(self) -> str:
    """
    Rich HTML representation for Motor devices.

    Returns:
        HTML string with device info
    """
    try:
        pos = self.position
        limits = self.limits
        units = self.units
        status_color = "green"
        status_text = "Ready"
    except Exception as e:
        pos = "N/A"
        limits = ("N/A", "N/A")
        units = ""
        status_color = "red"
        status_text = f"Error: {e}"

    return f"""
    <div style="border: 2px solid #4CAF50; border-radius: 5px; padding: 15px;
                background-color: #f9f9f9; font-family: monospace; max-width: 400px;">
        <div style="font-size: 18px; font-weight: bold; color: #4CAF50; margin-bottom: 10px;">
            Motor: {self.device_id}
        </div>
        <table style="width: 100%; border-collapse: collapse;">
            <tr>
                <td style="padding: 5px;"><b>Position:</b></td>
                <td style="padding: 5px; text-align: right;">{pos:.4f} {units}</td>
            </tr>
            <tr>
                <td style="padding: 5px;"><b>Limits:</b></td>
                <td style="padding: 5px; text-align: right;">[{limits[0]:.2f}, {limits[1]:.2f}] {units}</td>
            </tr>
            <tr>
                <td style="padding: 5px;"><b>Driver:</b></td>
                <td style="padding: 5px; text-align: right;">{self.driver_type}</td>
            </tr>
            <tr>
                <td style="padding: 5px;"><b>Status:</b></td>
                <td style="padding: 5px; text-align: right;">
                    <span style="color: {status_color}; font-size: 16px;">●</span> {status_text}
                </td>
            </tr>
        </table>
    </div>
    """


def _detector_repr_html(self) -> str:
    """
    Rich HTML representation for Detector devices.

    Returns:
        HTML string with device info
    """
    try:
        value = self.read()
        units = self.units
        status_color = "green"
        status_text = "Ready"
    except Exception as e:
        value = "N/A"
        units = ""
        status_color = "red"
        status_text = f"Error: {e}"

    return f"""
    <div style="border: 2px solid #2196F3; border-radius: 5px; padding: 15px;
                background-color: #f9f9f9; font-family: monospace; max-width: 400px;">
        <div style="font-size: 18px; font-weight: bold; color: #2196F3; margin-bottom: 10px;">
            Detector: {self.device_id}
        </div>
        <table style="width: 100%; border-collapse: collapse;">
            <tr>
                <td style="padding: 5px;"><b>Reading:</b></td>
                <td style="padding: 5px; text-align: right;">{value:.6e} {units}</td>
            </tr>
            <tr>
                <td style="padding: 5px;"><b>Driver:</b></td>
                <td style="padding: 5px; text-align: right;">{self.driver_type}</td>
            </tr>
            <tr>
                <td style="padding: 5px;"><b>Status:</b></td>
                <td style="padding: 5px; text-align: right;">
                    <span style="color: {status_color}; font-size: 16px;">●</span> {status_text}
                </td>
            </tr>
        </table>
    </div>
    """


def enable_rich_repr():
    """
    Enable rich HTML representations for Device classes.

    Call this function once to monkey-patch _repr_html_ methods
    onto Motor and Detector classes.

    Example:
        from rust_daq.jupyter import enable_rich_repr
        enable_rich_repr()

        # Now devices will display with rich HTML in notebooks
        motor = Motor("mock_stage")
        motor  # Displays rich HTML
    """
    if not HAS_WIDGETS:
        warnings.warn(
            "IPython not available - rich repr requires: pip install rust-daq-client[jupyter]",
            ImportWarning
        )
        return

    Motor._repr_html_ = _motor_repr_html
    Detector._repr_html_ = _detector_repr_html


# ============================================================================
# Interactive Widgets
# ============================================================================


def create_motor_slider(motor: Motor) -> Any:
    """
    Create interactive slider widget for motor control.

    Args:
        motor: Motor device instance

    Returns:
        ipywidgets.FloatSlider widget

    Example:
        with quick_connect():
            motor = Motor("mock_stage")
            slider = create_motor_slider(motor)
            display(slider)
    """
    if not HAS_WIDGETS:
        warnings.warn(
            "ipywidgets not available - install with: pip install rust-daq-client[jupyter]",
            ImportWarning
        )
        return None

    try:
        limits = motor.limits
        current = motor.position
        units = motor.units
    except Exception as e:
        warnings.warn(f"Failed to get motor info: {e}")
        limits = (0.0, 100.0)
        current = 0.0
        units = "units"

    slider = widgets.FloatSlider(
        value=current,
        min=limits[0],
        max=limits[1],
        step=(limits[1] - limits[0]) / 100.0,
        description=f"{motor.device_id}:",
        readout_format=".4f",
        continuous_update=False,
        layout=widgets.Layout(width="500px"),
        style={"description_width": "150px"},
    )

    # Output widget for status messages
    output = widgets.Output()

    def on_change(change):
        """Handle slider value changes."""
        new_pos = change["new"]
        with output:
            output.clear_output(wait=True)
            try:
                motor.position = new_pos
                print(f"✓ Moved to {new_pos:.4f} {units}")
            except Exception as e:
                print(f"✗ Error: {e}")

    slider.observe(on_change, names="value")

    return widgets.VBox([slider, output])


def create_detector_display(detector: Detector, update_interval: float = 1.0) -> Any:
    """
    Create live-updating display widget for detector.

    Args:
        detector: Detector device instance
        update_interval: Update interval in seconds

    Returns:
        ipywidgets.VBox widget with live readout

    Example:
        with quick_connect():
            det = Detector("mock_power_meter")
            display_widget = create_detector_display(det)
            display(display_widget)
    """
    if not HAS_WIDGETS:
        warnings.warn(
            "ipywidgets not available - install with: pip install rust-daq-client[jupyter]",
            ImportWarning
        )
        return None

    # Label and readout
    label = widgets.Label(value=f"{detector.device_id}:")
    readout = widgets.FloatText(
        value=0.0,
        description="",
        disabled=True,
        layout=widgets.Layout(width="150px"),
    )

    # Update button
    update_btn = widgets.Button(
        description="Read",
        button_style="info",
        layout=widgets.Layout(width="100px"),
    )

    def update_value(b=None):
        """Update readout with current value."""
        try:
            value = detector.read()
            readout.value = value
        except Exception as e:
            warnings.warn(f"Failed to read detector: {e}")

    update_btn.on_click(update_value)

    # Initial read
    update_value()

    return widgets.HBox([label, readout, update_btn])


def dashboard(*devices) -> Any:
    """
    Create interactive dashboard for multiple devices.

    Args:
        *devices: Variable number of Device instances (Motor or Detector)

    Returns:
        ipywidgets.VBox widget with controls for all devices

    Example:
        with quick_connect():
            motor1 = Motor("stage_x")
            motor2 = Motor("stage_y")
            det = Detector("power_meter")

            dashboard(motor1, motor2, det)
    """
    if not HAS_WIDGETS:
        warnings.warn(
            "ipywidgets not available - install with: pip install rust-daq-client[jupyter]",
            ImportWarning
        )
        return None

    widgets_list = []

    # Create header
    header = widgets.HTML(
        value="<h3 style='color: #4CAF50;'>rust-daq Device Dashboard</h3>"
    )
    widgets_list.append(header)

    # Create widget for each device
    for device in devices:
        if isinstance(device, Motor):
            widget = create_motor_slider(device)
        elif isinstance(device, Detector):
            widget = create_detector_display(device)
        else:
            # Generic device - just show info
            widget = widgets.HTML(
                value=f"<div style='padding: 5px;'><b>{device.device_id}</b> ({device.__class__.__name__})</div>"
            )

        if widget is not None:
            widgets_list.append(widget)

    # Add separator
    widgets_list.append(widgets.HTML(value="<hr>"))

    dashboard_widget = widgets.VBox(widgets_list)

    # Auto-display if in notebook
    if display is not None:
        display(dashboard_widget)

    return dashboard_widget


# ============================================================================
# Live Plotting
# ============================================================================


class LivePlot:
    """
    Real-time plotting during experiments.

    Supports matplotlib and plotly backends for live data visualization.

    Example:
        with quick_connect():
            motor = Motor("mock_stage")
            det = Detector("power_meter")

            plot = LivePlot(backend='matplotlib')

            for i in range(10):
                motor.position = i * 10.0
                value = det.read()
                plot.update([i * 10.0], [value])
    """

    def __init__(self, backend: str = "matplotlib", title: str = "Live Data"):
        """
        Initialize LivePlot.

        Args:
            backend: Plotting backend ('matplotlib' or 'plotly')
            title: Plot title
        """
        self.backend = backend
        self.title = title
        self.x_data = []
        self.y_data = []

        if backend == "matplotlib":
            if not HAS_MATPLOTLIB:
                raise ImportError(
                    "matplotlib not available - install with: pip install matplotlib"
                )
            self._init_matplotlib()
        elif backend == "plotly":
            if not HAS_PLOTLY:
                raise ImportError(
                    "plotly not available - install with: pip install plotly"
                )
            self._init_plotly()
        else:
            raise ValueError(f"Unknown backend: {backend}")

    def _init_matplotlib(self):
        """Initialize matplotlib figure."""
        plt.ion()  # Interactive mode
        self.fig, self.ax = plt.subplots(figsize=(10, 6))
        self.ax.set_title(self.title)
        self.ax.set_xlabel("Position")
        self.ax.set_ylabel("Reading")
        self.ax.grid(True, alpha=0.3)
        (self.line,) = self.ax.plot([], [], "o-", linewidth=2, markersize=6)
        plt.show()

    def _init_plotly(self):
        """Initialize plotly figure."""
        self.fig = go.FigureWidget(
            data=[go.Scatter(x=[], y=[], mode="lines+markers", name="Data")],
            layout=go.Layout(
                title=self.title,
                xaxis=dict(title="Position"),
                yaxis=dict(title="Reading"),
            ),
        )
        if display is not None:
            display(self.fig)

    def update(self, x_data: List[float], y_data: List[float]):
        """
        Update plot with new data.

        Args:
            x_data: X-axis data (positions)
            y_data: Y-axis data (readings)
        """
        self.x_data.extend(x_data)
        self.y_data.extend(y_data)

        if self.backend == "matplotlib":
            self.line.set_data(self.x_data, self.y_data)
            self.ax.relim()
            self.ax.autoscale_view()
            self.fig.canvas.draw()
            self.fig.canvas.flush_events()
        elif self.backend == "plotly":
            with self.fig.batch_update():
                self.fig.data[0].x = self.x_data
                self.fig.data[0].y = self.y_data

    def finalize(self):
        """Finalize plot (turn off interactive mode for matplotlib)."""
        if self.backend == "matplotlib":
            plt.ioff()
            plt.show()


def live_scan_plot(
    motor: Motor,
    detector: Detector,
    start: float,
    stop: float,
    steps: int,
    dwell_time: float = 0.0,
    backend: str = "matplotlib",
):
    """
    Execute scan with live plotting.

    Args:
        motor: Motor device to scan
        detector: Detector device to read
        start: Starting position
        stop: Ending position
        steps: Number of steps
        dwell_time: Dwell time at each position (seconds)
        backend: Plotting backend ('matplotlib' or 'plotly')

    Returns:
        Scan data (dict or DataFrame)

    Example:
        with quick_connect():
            motor = Motor("mock_stage")
            det = Detector("mock_power_meter")

            data = live_scan_plot(
                motor, det,
                start=0, stop=100, steps=20,
                backend='matplotlib'
            )
    """
    if not HAS_NUMPY:
        raise ImportError("numpy required for scans - install with: pip install numpy")

    # Create live plot
    plot = LivePlot(backend=backend, title=f"{detector.device_id} vs {motor.device_id}")

    # Generate positions
    positions = np.linspace(start, stop, steps)

    # Storage
    data = {"position": [], detector.device_id: []}

    # Progress bar
    if HAS_TQDM_NOTEBOOK:
        pbar = tqdm_notebook(total=steps, desc="Scanning", unit="pts")
    else:
        pbar = None

    try:
        for pos in positions:
            # Move and measure
            motor.position = pos
            if dwell_time > 0:
                time.sleep(dwell_time)
            value = detector.read()

            # Store data
            data["position"].append(pos)
            data[detector.device_id].append(value)

            # Update plot
            plot.update([pos], [value])

            # Update progress
            if pbar:
                pbar.update(1)

    finally:
        if pbar:
            pbar.close()
        plot.finalize()

    return data


# ============================================================================
# Auto-enable rich repr if in IPython
# ============================================================================

try:
    get_ipython()  # This will raise NameError if not in IPython
    enable_rich_repr()
except NameError:
    pass  # Not in IPython, skip auto-enable
