Getting Started
===============

This guide will help you write your first rust-daq Python program.

Prerequisites
-------------

1. Install the client library (see :doc:`installation`)
2. Start the rust-daq daemon with mock hardware:

   .. code-block:: bash

       cargo run --features networking -- daemon --port 50051

Your First Program
------------------

Let's create a simple program that connects to the daemon, lists devices, and moves a motor.

Step 1: Import the Library
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import connect, Motor, Detector

Step 2: Connect to the Daemon
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Use the ``connect()`` context manager to establish a connection:

.. code-block:: python

    with connect("localhost:50051"):
        # Your code here
        pass

The context manager ensures the connection is properly closed when you're done.

Step 3: Create a Motor Device
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")
        print(f"Created motor: {motor.name}")
        print(f"Position units: {motor.units}")

Step 4: Move the Motor
~~~~~~~~~~~~~~~~~~~~~~

Use the ``position`` property for simple moves:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Absolute move
        motor.position = 10.0
        print(f"Moved to: {motor.position}")

        # Another move
        motor.position = 20.0
        print(f"Now at: {motor.position}")

Step 5: Read from a Detector
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        detector = Detector("mock_power_meter")

        value = detector.read()
        print(f"Reading: {value} {detector.units}")

Complete Example
~~~~~~~~~~~~~~~~

Here's a complete working program:

.. code-block:: python

    #!/usr/bin/env python3
    """My first rust-daq program"""

    from rust_daq import connect, Motor, Detector

    def main():
        with connect("localhost:50051"):
            # Create devices
            motor = Motor("mock_stage")
            detector = Detector("mock_power_meter")

            print(f"Motor: {motor.name}")
            print(f"Detector: {detector.name}")

            # Move motor and take readings
            for position in [0, 10, 20, 30]:
                motor.position = position
                value = detector.read()
                print(f"Position: {position} {motor.units}, "
                      f"Reading: {value} {detector.units}")

    if __name__ == "__main__":
        main()

Save this as ``first_program.py`` and run it:

.. code-block:: bash

    python first_program.py

Running Your First Scan
-----------------------

Now let's run a 1D scan:

.. code-block:: python

    from rust_daq import connect, Motor, Detector, scan

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        # Run scan from 0 to 100 with 11 points
        data = scan(
            detectors=[detector],
            motor=motor,
            start=0,
            stop=100,
            steps=11,
            dwell_time=0.1  # Wait 100ms at each point
        )

        # data is a pandas DataFrame
        print(data)
        print(f"\nMax reading: {data['mock_power_meter'].max()}")

The output will be a pandas DataFrame:

.. code-block:: text

       position  mock_power_meter
    0       0.0          0.123456
    1      10.0          0.234567
    2      20.0          0.345678
    ...

Understanding the API Layers
-----------------------------

rust-daq provides three API layers. As a beginner, you'll mostly use Layer 2.

Layer 2: High-Level Synchronous API
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

This is what we've been using. It provides:

- Simple property-based interface (``motor.position = 10.0``)
- Synchronous (blocking) calls
- Context managers for resource safety
- Ideal for scripts and interactive use

Layer 1: AsyncClient
~~~~~~~~~~~~~~~~~~~~~

For async applications:

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient

    async def main():
        async with AsyncClient("localhost:50051") as client:
            await client.move_absolute("mock_stage", 10.0)
            position = await client.get_position("mock_stage")
            print(f"Position: {position}")

    anyio.run(main)

Use Layer 1 when:

- Building async applications
- Need maximum control over concurrency
- Working in an existing async codebase

Layer 0: Raw Protobuf
~~~~~~~~~~~~~~~~~~~~~

Direct gRPC access (rarely needed):

.. code-block:: python

    from rust_daq.generated import daq_pb2, daq_pb2_grpc
    import grpc

    channel = grpc.insecure_channel("localhost:50051")
    stub = daq_pb2_grpc.HardwareServiceStub(channel)

    request = daq_pb2.MoveRequest(device_id="mock_stage", value=10.0)
    response = stub.MoveAbsolute(request)

**Recommendation**: Stick with Layer 2 unless you have a specific reason to use Layer 1 or 0.

Error Handling
--------------

The library provides custom exceptions:

.. code-block:: python

    from rust_daq import connect, Motor, DeviceError, CommunicationError

    try:
        with connect("localhost:50051"):
            motor = Motor("nonexistent_device")
    except DeviceError as e:
        print(f"Device error: {e}")
    except CommunicationError as e:
        print(f"Connection error: {e}")

Common exceptions:

- ``DeviceError`` - Device not found or operation failed
- ``CommunicationError`` - Network/connection issues
- ``TimeoutError`` - Operation timed out
- ``ConfigurationError`` - Invalid parameters

Exploring Available Devices
----------------------------

To see what devices are available:

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient

    async def list_devices():
        async with AsyncClient("localhost:50051") as client:
            devices = await client.list_devices()
            for dev in devices:
                print(f"\nDevice: {dev['id']}")
                print(f"  Name: {dev['name']}")
                print(f"  Type: {dev['driver_type']}")
                print(f"  Capabilities: {dev['capabilities']}")

    anyio.run(list_devices)

Device Capabilities
-------------------

Devices have different capabilities:

- ``movable`` - Can be moved (use Motor class)
- ``readable`` - Can be read (use Detector class)
- ``frame_producer`` - Produces image frames (cameras)
- ``exposure_controllable`` - Exposure can be set
- ``triggerable`` - Supports external triggering
- ``shutter_controllable`` - Has a shutter
- ``wavelength_tunable`` - Wavelength can be changed (lasers)
- ``emission_controllable`` - Emission can be toggled

Use the appropriate class based on capabilities:

- ``Motor`` requires ``movable``
- ``Detector`` requires ``readable``

Non-Blocking Operations
-----------------------

For long moves, you can use non-blocking mode:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Start move without waiting
        status = motor.move(100.0, wait=False)

        # Do other work here...
        print("Moving in background...")

        # Wait for completion when ready
        status.wait()
        print("Move complete!")

Next Steps
----------

Now that you've learned the basics:

1. Explore the :doc:`tutorials/index` for more complex examples
2. Read the :doc:`guides/index` for best practices
3. Consult the :doc:`api/index` for detailed API documentation

For Jupyter users, see :doc:`tutorials/jupyter_notebooks` for interactive examples.
