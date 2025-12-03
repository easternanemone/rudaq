"""
rust-daq Python Client Library

A Python client for controlling the rust-daq headless daemon via gRPC.

The library provides three layers:
- Layer 0: Auto-generated protobuf stubs (in .generated submodule)
- Layer 1: AsyncClient - Async-first gRPC wrapper
- Layer 2: High-level synchronous API (Device, Motor, Detector, scan)
- Jupyter integration: Interactive widgets, live plotting (rust_daq.jupyter)

Example usage (Layer 1 - Async):

    import anyio
    from rust_daq import AsyncClient

    async def main():
        async with AsyncClient("localhost:50051") as client:
            devices = await client.list_devices()
            for device in devices:
                print(f"Found device: {device['id']}")

    anyio.run(main)

Example usage (Layer 2 - Sync):

    from rust_daq import connect, Motor, Detector, run, scan

    with connect():
        motor = Motor("mock_stage")
        motor.position = 10.0

        with run(name="Test Scan"):
            data = scan(
                detectors=[Detector("mock_power_meter")],
                motor=motor,
                start=0, stop=100, steps=10
            )

        print(data.head())  # pandas DataFrame
"""

from ._version import __version__
from .core import AsyncClient
from .exceptions import (
    DaqError,
    DeviceError,
    CommunicationError,
    TimeoutError,
    ConfigurationError,
)
from .devices import (
    Device,
    Motor,
    Detector,
    Status,
    connect,
    run,
    scan,
)

__all__ = [
    "__version__",
    # Layer 1: AsyncClient
    "AsyncClient",
    # Layer 2: High-level API
    "Device",
    "Motor",
    "Detector",
    "Status",
    "connect",
    "run",
    "scan",
    # Exceptions
    "DaqError",
    "DeviceError",
    "CommunicationError",
    "TimeoutError",
    "ConfigurationError",
]
