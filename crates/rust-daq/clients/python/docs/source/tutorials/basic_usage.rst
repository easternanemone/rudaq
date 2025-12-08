Basic Usage Tutorial
====================

This tutorial covers the fundamental operations with rust-daq.

Connecting to the Daemon
-------------------------

Always use the ``connect()`` context manager:

.. code-block:: python

    from rust_daq import connect

    with connect("localhost:50051", timeout=10.0):
        # Your code here
        pass

The context manager:

- Establishes the gRPC connection
- Provides automatic cleanup
- Handles errors gracefully

Creating Devices
----------------

Create device objects using the appropriate class:

.. code-block:: python

    from rust_daq import connect, Motor, Detector

    with connect():
        # Create a motor (requires movable capability)
        motor = Motor("mock_stage")

        # Create a detector (requires readable capability)
        detector = Detector("mock_power_meter")

Device Information
~~~~~~~~~~~~~~~~~~

Access device metadata:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        print(f"ID: {motor.id}")
        print(f"Name: {motor.name}")
        print(f"Driver: {motor.driver_type}")
        print(f"Capabilities: {motor.capabilities}")
        print(f"Metadata: {motor.metadata}")

For motors, you can access position limits and units:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        min_pos, max_pos = motor.limits
        print(f"Limits: {min_pos} to {max_pos} {motor.units}")

Moving Motors
-------------

Property-Based Movement
~~~~~~~~~~~~~~~~~~~~~~~

The simplest way to move:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Set position (blocks until complete)
        motor.position = 10.0

        # Read position
        current = motor.position
        print(f"At {current}")

Method-Based Movement
~~~~~~~~~~~~~~~~~~~~~

For more control:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Absolute move (blocking)
        motor.move(10.0, wait=True)

        # Relative move
        motor.move_relative(5.0, wait=True)  # Now at 15.0

        # Non-blocking move
        status = motor.move(20.0, wait=False)
        # Do other work...
        status.wait()  # Block when ready

Reading Detectors
-----------------

Simple Read
~~~~~~~~~~~

.. code-block:: python

    with connect():
        detector = Detector("mock_power_meter")

        value = detector.read()
        print(f"{value} {detector.units}")

Multiple Readings
~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        detector = Detector("mock_power_meter")

        readings = []
        for i in range(10):
            value = detector.read()
            readings.append(value)

        import numpy as np
        mean = np.mean(readings)
        std = np.std(readings)
        print(f"Mean: {mean:.3f} ± {std:.3f} {detector.units}")

Coordinated Motion and Reading
-------------------------------

Move a motor and read at each position:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        positions = [0, 10, 20, 30, 40, 50]
        results = []

        for pos in positions:
            motor.position = pos
            value = detector.read()
            results.append((pos, value))

        for pos, val in results:
            print(f"{pos} {motor.units}: {val} {detector.units}")

Using Multiple Devices
----------------------

You can create and use multiple devices:

.. code-block:: python

    with connect():
        # Create multiple motors
        x_stage = Motor("stage_x")
        y_stage = Motor("stage_y")

        # Create multiple detectors
        power_meter = Detector("power_meter")
        photodiode = Detector("photodiode")

        # Move both motors
        x_stage.position = 10.0
        y_stage.position = 20.0

        # Read both detectors
        power = power_meter.read()
        pd_signal = photodiode.read()

        print(f"Position: ({x_stage.position}, {y_stage.position})")
        print(f"Power: {power}, PD: {pd_signal}")

Error Handling Best Practices
------------------------------

Always handle errors appropriately:

.. code-block:: python

    from rust_daq import (
        connect, Motor, DeviceError,
        CommunicationError, TimeoutError
    )

    try:
        with connect("localhost:50051"):
            motor = Motor("my_stage")
            motor.position = 10.0

    except DeviceError as e:
        print(f"Device error: {e}")
        print(f"Device ID: {e.device_id}")

    except CommunicationError as e:
        print(f"Communication failed: {e}")
        print("Is the daemon running?")

    except TimeoutError as e:
        print(f"Operation timed out: {e}")

    except Exception as e:
        print(f"Unexpected error: {e}")

Resource Management
-------------------

The ``connect()`` context manager handles cleanup automatically:

.. code-block:: python

    # Good: Automatic cleanup
    with connect():
        motor = Motor("mock_stage")
        motor.position = 10.0
    # Connection closed here

    # Also works with exceptions
    try:
        with connect():
            motor = Motor("mock_stage")
            motor.position = 10.0
            raise ValueError("Oops!")
    except ValueError:
        pass
    # Connection still closed properly

Best Practices
--------------

1. **Always use context managers**: Use ``with connect():`` instead of manual connect/close
2. **Check device capabilities**: Verify a device has the right capabilities before creating Motor/Detector
3. **Handle errors**: Always wrap operations in try/except blocks
4. **Use appropriate wait modes**: Use ``wait=False`` for long operations you can parallelize
5. **Reuse device objects**: Create device objects once, reuse them multiple times
6. **Close connections**: Let the context manager handle cleanup

Anti-Patterns to Avoid
----------------------

.. code-block:: python

    # BAD: No context manager
    client = AsyncClient("localhost:50051")
    await client.connect()
    # ... might not close properly

    # BAD: Creating device objects repeatedly
    for i in range(100):
        motor = Motor("mock_stage")  # Wasteful
        motor.position = i

    # GOOD: Reuse device object
    motor = Motor("mock_stage")
    for i in range(100):
        motor.position = i

    # BAD: No error handling
    motor.position = invalid_value  # Might crash

    # GOOD: With error handling
    try:
        motor.position = value
    except DeviceError as e:
        print(f"Move failed: {e}")

Next Steps
----------

- Learn advanced motor control in :doc:`motor_control`
- Learn about scanning in :doc:`data_acquisition`
- Explore Jupyter integration in :doc:`jupyter_notebooks`
