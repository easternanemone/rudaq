"""
Tests for Layer 2 high-level device API.

These tests cover:
- Device base class
- Motor class and properties
- Detector class
- Context managers (connect, run)
- scan() function
- Status objects

Most tests are integration tests requiring a running daemon.
"""

import pytest
from unittest.mock import MagicMock, patch
import warnings

from rust_daq import (
    Device,
    Motor,
    Detector,
    Status,
    connect,
    run,
    scan,
)
from rust_daq.exceptions import DaqError, DeviceError


# ============================================================================
# Unit Tests - Status Class
# ============================================================================


def test_status_initialization():
    """Test Status object initialization."""
    status = Status(None)
    assert not status.done
    assert repr(status) == "Status(pending)"


def test_status_completion():
    """Test Status object completion."""
    status = Status(None)
    status._done = True
    status._result = "test_result"

    assert status.done
    assert status.wait() == "test_result"
    assert repr(status) == "Status(done)"


def test_status_exception():
    """Test Status object with exception."""
    status = Status(None)
    status._done = True
    status._exception = ValueError("Test error")

    with pytest.raises(ValueError, match="Test error"):
        status.wait()


# ============================================================================
# Integration Tests - Context Manager
# ============================================================================


@pytest.mark.integration
def test_connect_context_manager():
    """
    Integration test - requires rust-daq daemon running.

    Run with: pytest -m integration

    Tests:
    - connect() context manager
    - Thread-local client storage
    - Proper cleanup on exit
    """
    # Test that connect() works
    with connect("localhost:50051", timeout=5.0):
        # Should be able to create devices inside context
        from rust_daq.devices import _get_client
        client = _get_client()
        assert client is not None
        assert client.address == "localhost:50051"

    # After context, should raise error
    from rust_daq.devices import _get_client
    with pytest.raises(DaqError, match="No active connection"):
        _get_client()


@pytest.mark.integration
def test_run_context_manager():
    """
    Integration test - requires rust-daq daemon running.

    Run with: pytest -m integration

    Tests:
    - run() context manager (currently placeholder)
    - Warning about unimplemented functionality
    """
    with connect("localhost:50051", timeout=5.0):
        # run() should warn that it's not implemented
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")

            with run(name="Test Run", metadata={"test": True}):
                pass

            # Check that warning was raised
            assert len(w) == 1
            assert "placeholder" in str(w[0].message).lower()


# ============================================================================
# Integration Tests - Device Base Class
# ============================================================================


@pytest.mark.integration
def test_device_initialization():
    """
    Integration test - requires rust-daq daemon with at least one device.

    Run with: pytest -m integration

    Tests:
    - Device initialization
    - Metadata fetching
    - Property access
    """
    with connect("localhost:50051", timeout=5.0):
        # Get first available device
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        if len(devices) > 0:
            device_id = devices[0]["id"]

            # Create Device instance
            device = Device(device_id)

            # Test properties
            assert device.id == device_id
            assert device.device_id == device_id
            assert isinstance(device.metadata, dict)
            assert isinstance(device.capabilities, dict)
            assert hasattr(device, 'name')
            assert hasattr(device, 'driver_type')

            # Test repr
            assert device_id in repr(device)


@pytest.mark.integration
def test_device_not_found():
    """
    Integration test - requires rust-daq daemon.

    Run with: pytest -m integration

    Tests:
    - Device initialization with invalid ID raises DeviceError
    """
    with connect("localhost:50051", timeout=5.0):
        with pytest.raises(DeviceError, match="not found"):
            Device("nonexistent_device_12345")


# ============================================================================
# Integration Tests - Motor Class
# ============================================================================


@pytest.mark.integration
def test_motor_initialization():
    """
    Integration test - requires rust-daq daemon with movable device.

    Run with: pytest -m integration

    Tests:
    - Motor initialization
    - Movable capability verification
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find movable device
        movable_device = None
        for dev in devices:
            if dev["capabilities"].get("movable"):
                movable_device = dev
                break

        if movable_device:
            motor = Motor(movable_device["id"])

            # Verify it's a motor
            assert isinstance(motor, Motor)
            assert motor.capabilities["movable"]
            assert "Motor" in repr(motor)


@pytest.mark.integration
def test_motor_wrong_capability():
    """
    Integration test - requires rust-daq daemon.

    Run with: pytest -m integration

    Tests:
    - Motor initialization with non-movable device raises DeviceError
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find non-movable device
        non_movable = None
        for dev in devices:
            if not dev["capabilities"].get("movable"):
                non_movable = dev
                break

        if non_movable:
            with pytest.raises(DeviceError, match="does not have Movable capability"):
                Motor(non_movable["id"])


@pytest.mark.integration
def test_motor_position_property():
    """
    Integration test - requires rust-daq daemon with movable device.

    Run with: pytest -m integration

    Tests:
    - Motor.position getter
    - Motor.position setter (absolute move)
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find movable device
        movable_device = None
        for dev in devices:
            if dev["capabilities"].get("movable"):
                movable_device = dev
                break

        if movable_device:
            motor = Motor(movable_device["id"])

            # Get position
            pos = motor.position
            assert isinstance(pos, float)

            # Set position
            target = 5.0
            motor.position = target

            # Verify position (might not be exact due to hardware limitations)
            new_pos = motor.position
            assert isinstance(new_pos, float)


@pytest.mark.integration
def test_motor_move_methods():
    """
    Integration test - requires rust-daq daemon with movable device.

    Run with: pytest -m integration

    Tests:
    - Motor.move() with wait=True
    - Motor.move() with wait=False (Status object)
    - Motor.move_relative()
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find movable device
        movable_device = None
        for dev in devices:
            if dev["capabilities"].get("movable"):
                movable_device = dev
                break

        if movable_device:
            motor = Motor(movable_device["id"])

            # Test blocking move
            result = motor.move(10.0, wait=True)
            assert result is None  # Blocking move returns None

            # Test non-blocking move
            status = motor.move(15.0, wait=False)
            assert isinstance(status, Status)
            assert status.done  # Should be done immediately in current impl

            # Test relative move
            start_pos = motor.position
            motor.move_relative(1.0, wait=True)
            end_pos = motor.position
            # Position should have changed (might not be exactly 1.0)
            assert end_pos != start_pos


@pytest.mark.integration
def test_motor_limits_and_units():
    """
    Integration test - requires rust-daq daemon with movable device.

    Run with: pytest -m integration

    Tests:
    - Motor.limits property
    - Motor.units property
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find movable device
        movable_device = None
        for dev in devices:
            if dev["capabilities"].get("movable"):
                movable_device = dev
                break

        if movable_device:
            motor = Motor(movable_device["id"])

            # Test units
            units = motor.units
            assert isinstance(units, str)

            # Test limits (may not be available for all devices)
            try:
                limits = motor.limits
                assert isinstance(limits, tuple)
                assert len(limits) == 2
                min_pos, max_pos = limits
                assert min_pos < max_pos
            except DeviceError:
                # Limits not available - that's okay
                pass


# ============================================================================
# Integration Tests - Detector Class
# ============================================================================


@pytest.mark.integration
def test_detector_initialization():
    """
    Integration test - requires rust-daq daemon with readable device.

    Run with: pytest -m integration

    Tests:
    - Detector initialization
    - Readable capability verification
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find readable device
        readable_device = None
        for dev in devices:
            if dev["capabilities"].get("readable"):
                readable_device = dev
                break

        if readable_device:
            detector = Detector(readable_device["id"])

            # Verify it's a detector
            assert isinstance(detector, Detector)
            assert detector.capabilities["readable"]
            assert "Detector" in repr(detector)


@pytest.mark.integration
def test_detector_wrong_capability():
    """
    Integration test - requires rust-daq daemon.

    Run with: pytest -m integration

    Tests:
    - Detector initialization with non-readable device raises DeviceError
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find non-readable device
        non_readable = None
        for dev in devices:
            if not dev["capabilities"].get("readable"):
                non_readable = dev
                break

        if non_readable:
            with pytest.raises(DeviceError, match="does not have Readable capability"):
                Detector(non_readable["id"])


@pytest.mark.integration
def test_detector_read():
    """
    Integration test - requires rust-daq daemon with readable device.

    Run with: pytest -m integration

    Tests:
    - Detector.read() method
    - Detector.units property
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find readable device
        readable_device = None
        for dev in devices:
            if dev["capabilities"].get("readable"):
                readable_device = dev
                break

        if readable_device:
            detector = Detector(readable_device["id"])

            # Test read
            value = detector.read()
            assert isinstance(value, float)

            # Test units
            units = detector.units
            assert isinstance(units, str)


# ============================================================================
# Integration Tests - scan() Function
# ============================================================================


@pytest.mark.integration
def test_scan_basic():
    """
    Integration test - requires rust-daq daemon with movable and readable devices.

    Run with: pytest -m integration

    Tests:
    - scan() function with basic parameters
    - DataFrame return type
    - Correct data structure
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find movable and readable devices
        movable_device = None
        readable_device = None

        for dev in devices:
            if dev["capabilities"].get("movable") and not movable_device:
                movable_device = dev
            if dev["capabilities"].get("readable") and not readable_device:
                readable_device = dev

        if movable_device and readable_device:
            motor = Motor(movable_device["id"])
            detector = Detector(readable_device["id"])

            # Execute scan
            data = scan(
                detectors=[detector],
                motor=motor,
                start=0.0,
                stop=10.0,
                steps=5,
                dwell_time=0.0,
            )

            # Check result type
            try:
                import pandas as pd
                assert isinstance(data, pd.DataFrame)

                # Check structure
                assert "position" in data.columns
                assert detector.device_id in data.columns
                assert len(data) == 5

                # Check position values
                import numpy as np
                expected_positions = np.linspace(0.0, 10.0, 5)
                assert np.allclose(data["position"].values, expected_positions)

            except ImportError:
                # pandas not installed - should be dict
                assert isinstance(data, dict)
                assert "position" in data
                assert detector.device_id in data
                assert len(data["position"]) == 5


@pytest.mark.integration
def test_scan_multiple_detectors():
    """
    Integration test - requires rust-daq daemon with movable and multiple readable devices.

    Run with: pytest -m integration

    Tests:
    - scan() with multiple detectors
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find movable and readable devices
        movable_device = None
        readable_devices = []

        for dev in devices:
            if dev["capabilities"].get("movable") and not movable_device:
                movable_device = dev
            if dev["capabilities"].get("readable"):
                readable_devices.append(dev)

        if movable_device and len(readable_devices) >= 1:
            motor = Motor(movable_device["id"])
            detectors = [Detector(dev["id"]) for dev in readable_devices[:2]]

            # Execute scan with multiple detectors
            data = scan(
                detectors=detectors,
                motor=motor,
                start=5.0,
                stop=15.0,
                steps=3,
                dwell_time=0.0,
            )

            # Check that all detectors are present
            try:
                import pandas as pd
                assert isinstance(data, pd.DataFrame)

                for det in detectors:
                    assert det.device_id in data.columns

            except ImportError:
                assert isinstance(data, dict)
                for det in detectors:
                    assert det.device_id in data


@pytest.mark.integration
def test_scan_return_dict():
    """
    Integration test - requires rust-daq daemon with movable and readable devices.

    Run with: pytest -m integration

    Tests:
    - scan() with return_dict=True
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find movable and readable devices
        movable_device = None
        readable_device = None

        for dev in devices:
            if dev["capabilities"].get("movable") and not movable_device:
                movable_device = dev
            if dev["capabilities"].get("readable") and not readable_device:
                readable_device = dev

        if movable_device and readable_device:
            motor = Motor(movable_device["id"])
            detector = Detector(readable_device["id"])

            # Execute scan with return_dict=True
            data = scan(
                detectors=[detector],
                motor=motor,
                start=0.0,
                stop=5.0,
                steps=3,
                dwell_time=0.0,
                return_dict=True,
            )

            # Should always be dict
            assert isinstance(data, dict)
            assert "position" in data
            assert detector.device_id in data
            assert len(data["position"]) == 3


# ============================================================================
# Integration Tests - Complete Workflow
# ============================================================================


@pytest.mark.integration
def test_complete_workflow():
    """
    Integration test - requires rust-daq daemon with movable and readable devices.

    Run with: pytest -m integration

    Tests complete workflow:
    - Connect
    - Create devices
    - Move motor
    - Read detector
    - Execute scan
    - Cleanup
    """
    with connect("localhost:50051", timeout=5.0):
        from rust_daq import AsyncClient
        from rust_daq.devices import _run_async, _get_client

        client = _get_client()
        devices = _run_async(client.list_devices())

        # Find devices
        movable_device = None
        readable_device = None

        for dev in devices:
            if dev["capabilities"].get("movable") and not movable_device:
                movable_device = dev
            if dev["capabilities"].get("readable") and not readable_device:
                readable_device = dev

        if movable_device and readable_device:
            # Create devices
            motor = Motor(movable_device["id"])
            detector = Detector(readable_device["id"])

            # Test motor control
            motor.position = 10.0
            pos = motor.position
            assert isinstance(pos, float)

            # Test detector reading
            value = detector.read()
            assert isinstance(value, float)

            # Test scan
            with run(name="Integration Test Scan"):
                data = scan(
                    detectors=[detector],
                    motor=motor,
                    start=0.0,
                    stop=20.0,
                    steps=5,
                    dwell_time=0.0,
                )

            # Verify data
            try:
                import pandas as pd
                assert isinstance(data, pd.DataFrame)
                assert len(data) == 5
            except ImportError:
                assert isinstance(data, dict)
                assert len(data["position"]) == 5
