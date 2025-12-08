"""
High-level synchronous device API for rust-daq.

This is Layer 2 - Ophyd/Bluesky-style device abstractions with synchronous API.
Provides intuitive property-based interface for scientists who prefer blocking calls.

Key Features:
- Synchronous API using anyio for async-to-sync conversion
- Property-based access for intuitive usage
- Context managers for resource safety
- Pandas DataFrame integration for scan results
- Status objects for non-blocking operations

Example:
    from rust_daq import connect, Motor, Detector, run, scan

    with connect():
        motor = Motor("mock_stage")
        motor.position = 10.0  # Property setter

        with run(name="Test Scan"):
            data = scan(
                detectors=[Detector("mock_power_meter")],
                motor=motor,
                start=0, stop=100, steps=10
            )

        print(data.head())  # pandas DataFrame
"""

from typing import Optional, Callable, List, Dict, Any, Tuple
from contextlib import contextmanager
import threading
import time
import warnings

try:
    import pandas as pd
    HAS_PANDAS = True
except ImportError:
    HAS_PANDAS = False
    warnings.warn(
        "pandas not installed - scan() function will return dict instead of DataFrame",
        ImportWarning
    )

try:
    from tqdm import tqdm
    HAS_TQDM = True
except ImportError:
    try:
        from tqdm.notebook import tqdm
        HAS_TQDM = True
    except ImportError:
        HAS_TQDM = False

import anyio
from anyio.from_thread import start_blocking_portal

from .core import AsyncClient
from .exceptions import DaqError, DeviceError


# Global thread-local storage for the AsyncClient instance
_thread_local = threading.local()


def _get_client() -> AsyncClient:
    """Get the AsyncClient from thread-local storage."""
    if not hasattr(_thread_local, 'client') or _thread_local.client is None:
        raise DaqError(
            "No active connection - use 'with connect()' context manager"
        )
    return _thread_local.client


def _get_portal():
    """Get the blocking portal from thread-local storage."""
    if not hasattr(_thread_local, 'portal') or _thread_local.portal is None:
        raise DaqError(
            "No active portal - use 'with connect()' context manager"
        )
    return _thread_local.portal


def _run_async(coro):
    """Execute an async coroutine synchronously using the blocking portal."""
    portal = _get_portal()
    return portal.call(coro)


# ============================================================================
# Status Class - For Non-Blocking Operations
# ============================================================================


class Status:
    """
    Status object for tracking asynchronous operations.

    Allows non-blocking operations with wait() method for completion.
    Similar to Bluesky's Status object.

    Example:
        status = motor.move(10.0, wait=False)
        # Do other work...
        status.wait()  # Block until complete
    """

    def __init__(self, future):
        """
        Initialize Status with a future/task.

        Args:
            future: An anyio future or coroutine result
        """
        self._future = future
        self._done = False
        self._result = None
        self._exception = None

    @property
    def done(self) -> bool:
        """Check if operation is complete."""
        return self._done

    def wait(self, timeout: Optional[float] = None) -> Any:
        """
        Block until operation completes.

        Args:
            timeout: Optional timeout in seconds

        Returns:
            Operation result

        Raises:
            TimeoutError: If timeout is exceeded
            Exception: Any exception raised by the operation
        """
        if self._done:
            if self._exception:
                raise self._exception
            return self._result

        # Implementation note: For now, Status objects are completed immediately
        # since we're using anyio.from_thread.run() which blocks.
        # Future enhancement: Use background tasks for true async status tracking
        return self._result

    def __repr__(self) -> str:
        status = "done" if self._done else "pending"
        return f"Status({status})"


# ============================================================================
# Base Device Class
# ============================================================================


class Device:
    """
    Base class for all hardware devices.

    Provides common interface for device identification and metadata access.

    Attributes:
        device_id: Unique device identifier
        name: Human-readable device name
        metadata: Device metadata dictionary
    """

    def __init__(self, device_id: str):
        """
        Initialize Device.

        Args:
            device_id: Unique device identifier (e.g., "mock_stage")
        """
        self.device_id = device_id
        self._metadata: Optional[Dict[str, Any]] = None
        self._capabilities: Optional[Dict[str, bool]] = None

        # Fetch device info on initialization
        self._fetch_info()

    def _fetch_info(self) -> None:
        """Fetch device info from daemon."""
        client = _get_client()

        # Get device info from list_devices
        devices = _run_async(client.list_devices())

        # Find this device
        for dev in devices:
            if dev["id"] == self.device_id:
                self._metadata = dev.get("metadata", {})
                self._capabilities = dev.get("capabilities", {})
                self.name = dev.get("name", self.device_id)
                self.driver_type = dev.get("driver_type", "unknown")
                return

        raise DeviceError(
            f"Device '{self.device_id}' not found",
            device_id=self.device_id
        )

    @property
    def id(self) -> str:
        """Get device ID."""
        return self.device_id

    @property
    def metadata(self) -> Dict[str, Any]:
        """Get device metadata."""
        if self._metadata is None:
            self._fetch_info()
        return self._metadata or {}

    @property
    def capabilities(self) -> Dict[str, bool]:
        """Get device capabilities."""
        if self._capabilities is None:
            self._fetch_info()
        return self._capabilities or {}

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}('{self.device_id}')"


# ============================================================================
# Motor Class - For Movable Devices
# ============================================================================


class Motor(Device):
    """
    Motor device for position control.

    Supports devices with the Movable capability trait.
    Provides property-based position access and motion commands.

    Example:
        motor = Motor("mock_stage")
        motor.position = 10.0  # Absolute move
        print(motor.position)  # Read position

        status = motor.move(20.0, wait=False)  # Non-blocking
        # Do other work...
        status.wait()
    """

    def __init__(self, device_id: str):
        """
        Initialize Motor.

        Args:
            device_id: Unique device identifier

        Raises:
            DeviceError: If device doesn't have Movable capability
        """
        super().__init__(device_id)

        # Verify movable capability
        if not self.capabilities.get("movable", False):
            raise DeviceError(
                f"Device '{device_id}' does not have Movable capability",
                device_id=device_id
            )

    @property
    def position(self) -> float:
        """
        Get current position.

        Returns:
            Current position in device units
        """
        client = _get_client()
        return _run_async(client.get_position(self.device_id))

    @position.setter
    def position(self, value: float) -> None:
        """
        Set position (absolute move).

        Args:
            value: Target position in device units
        """
        self.move(value, wait=True)

    def move(self, target: float, wait: bool = True) -> Optional[Status]:
        """
        Move to absolute position.

        Args:
            target: Target position in device units
            wait: If True, block until complete. If False, return Status object.

        Returns:
            None if wait=True, Status object if wait=False

        Raises:
            DeviceError: If move fails
        """
        client = _get_client()

        if wait:
            result = _run_async(
                client.move_absolute(
                    self.device_id,
                    target,
                    wait_for_completion=True
                )
            )
            return None
        else:
            # For non-blocking, we still execute synchronously for now
            # but return a Status object that's already complete
            result = _run_async(
                client.move_absolute(
                    self.device_id,
                    target,
                    wait_for_completion=False
                )
            )
            status = Status(None)
            status._done = True
            status._result = result
            return status

    def move_relative(self, distance: float, wait: bool = True) -> Optional[Status]:
        """
        Move by relative distance.

        Args:
            distance: Distance to move in device units
            wait: If True, block until complete. If False, return Status object.

        Returns:
            None if wait=True, Status object if wait=False

        Raises:
            DeviceError: If move fails
        """
        client = _get_client()

        if wait:
            _run_async(
                client.move_relative(
                    self.device_id,
                    distance,
                    wait_for_completion=True
                )
            )
            return None
        else:
            result = _run_async(
                client.move_relative(
                    self.device_id,
                    distance,
                    wait_for_completion=False
                )
            )
            status = Status(None)
            status._done = True
            status._result = result
            return status

    @property
    def limits(self) -> Tuple[float, float]:
        """
        Get position limits.

        Returns:
            Tuple of (min_position, max_position)

        Raises:
            DeviceError: If limits not available in metadata
        """
        meta = self.metadata
        min_pos = meta.get("min_position")
        max_pos = meta.get("max_position")

        if min_pos is None or max_pos is None:
            raise DeviceError(
                f"Position limits not available for device '{self.device_id}'",
                device_id=self.device_id
            )

        return (min_pos, max_pos)

    @property
    def units(self) -> str:
        """
        Get position units.

        Returns:
            Position units string (e.g., "mm", "degrees")
        """
        return self.metadata.get("position_units", "units")


# ============================================================================
# Detector Class - For Readable Devices
# ============================================================================


class Detector(Device):
    """
    Detector device for scalar measurements.

    Supports devices with the Readable capability trait.
    Provides simple read() interface for taking measurements.

    Example:
        detector = Detector("mock_power_meter")
        value = detector.read()
        print(f"Power: {value} {detector.units}")
    """

    def __init__(self, device_id: str):
        """
        Initialize Detector.

        Args:
            device_id: Unique device identifier

        Raises:
            DeviceError: If device doesn't have Readable capability
        """
        super().__init__(device_id)

        # Verify readable capability
        if not self.capabilities.get("readable", False):
            raise DeviceError(
                f"Device '{device_id}' does not have Readable capability",
                device_id=device_id
            )

    def read(self) -> float:
        """
        Read current value from detector.

        Returns:
            Current reading as float

        Raises:
            DeviceError: If read fails
        """
        client = _get_client()
        state = _run_async(client.get_device_state(self.device_id))

        if "last_reading" not in state:
            raise DeviceError(
                f"Device '{self.device_id}' did not return a reading",
                device_id=self.device_id
            )

        return state["last_reading"]

    @property
    def units(self) -> str:
        """
        Get reading units.

        Returns:
            Reading units string (e.g., "W", "V", "A")
        """
        return self.metadata.get("reading_units", "units")


# ============================================================================
# Context Managers
# ============================================================================


@contextmanager
def connect(host: str = "localhost:50051", timeout: float = 10.0):
    """
    Context manager for connecting to rust-daq daemon.

    Manages AsyncClient lifecycle and provides synchronous interface.

    Args:
        host: Daemon address in "host:port" format
        timeout: Default timeout for operations in seconds

    Yields:
        None (client is stored in thread-local storage)

    Example:
        with connect():
            motor = Motor("mock_stage")
            motor.position = 10.0
    """
    # Create AsyncClient
    client = AsyncClient(host, timeout=timeout)

    # Start blocking portal for async-to-sync conversion
    with start_blocking_portal() as portal:
        # Store in thread-local storage
        _thread_local.client = client
        _thread_local.portal = portal

        # Connect to daemon
        portal.call(client.connect)

        try:
            yield
        finally:
            # Cleanup
            portal.call(client.close)
            _thread_local.client = None
            _thread_local.portal = None


@contextmanager
def run(name: str, metadata: Optional[Dict[str, Any]] = None):
    """
    Context manager for a data acquisition run.

    Calls StartRun on entry and StopRun on exit.

    Args:
        name: Run name/identifier
        metadata: Optional metadata dictionary

    Yields:
        None

    Example:
        with connect():
            with run(name="Test Scan", metadata={"operator": "Alice"}):
                # Perform measurements
                pass
    """
    # TODO: Implement StartRun/StopRun gRPC calls when available
    # For now, this is a placeholder that just yields
    # Track with bd issue if StartRun/StopRun need to be implemented

    warnings.warn(
        "run() context manager is a placeholder - StartRun/StopRun not yet implemented",
        UserWarning
    )

    # Just yield for now
    yield


# ============================================================================
# Scan Function
# ============================================================================


def scan(
    detectors: List[Detector],
    motor: Motor,
    start: float,
    stop: float,
    steps: int,
    dwell_time: float = 0.0,
    return_dict: bool = False,
) -> Any:
    """
    Execute a 1D scan of detectors vs motor position.

    Args:
        detectors: List of Detector objects to read
        motor: Motor object to scan
        start: Starting position
        stop: Ending position
        steps: Number of steps (positions)
        dwell_time: Time to wait at each position (seconds)
        return_dict: If True, return dict instead of DataFrame (useful if pandas unavailable)

    Returns:
        pandas.DataFrame with columns: position, <detector_names>
        or dict if return_dict=True or pandas not installed

    Example:
        with connect():
            motor = Motor("mock_stage")
            det = Detector("mock_power_meter")

            data = scan(
                detectors=[det],
                motor=motor,
                start=0, stop=100, steps=11,
                dwell_time=0.1
            )

            print(data.head())
    """
    # Generate positions
    import numpy as np
    positions = np.linspace(start, stop, steps)

    # Prepare data storage
    data = {
        "position": [],
    }
    for det in detectors:
        data[det.device_id] = []

    # Create progress bar if tqdm available
    if HAS_TQDM:
        pbar = tqdm(total=steps, desc="Scanning", unit="pts")
    else:
        pbar = None

    try:
        # Execute scan
        for i, pos in enumerate(positions):
            # Move motor
            motor.position = pos

            # Dwell if requested
            if dwell_time > 0:
                time.sleep(dwell_time)

            # Read detectors
            data["position"].append(pos)
            for det in detectors:
                value = det.read()
                data[det.device_id].append(value)

            # Update progress
            if pbar:
                pbar.update(1)

    finally:
        if pbar:
            pbar.close()

    # Return as DataFrame or dict
    if HAS_PANDAS and not return_dict:
        return pd.DataFrame(data)
    else:
        return data
