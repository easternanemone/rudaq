Motor Control Tutorial
======================

Advanced motor control techniques.

Understanding Motor Capabilities
---------------------------------

Before using a motor, check its capabilities:

.. code-block:: python

    from rust_daq import connect, Motor

    with connect():
        motor = Motor("mock_stage")

        # Check capabilities
        caps = motor.capabilities
        print(f"Movable: {caps['movable']}")
        print(f"Triggerable: {caps.get('triggerable', False)}")

        # Get metadata
        meta = motor.metadata
        print(f"Units: {meta.get('position_units', 'unknown')}")
        print(f"Limits: {meta.get('min_position')} to {meta.get('max_position')}")

Position Control
----------------

Absolute Positioning
~~~~~~~~~~~~~~~~~~~~

Move to an absolute position:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Using property (recommended)
        motor.position = 10.0

        # Using method (more control)
        motor.move(10.0, wait=True)

Relative Positioning
~~~~~~~~~~~~~~~~~~~~

Move relative to current position:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Start at 10
        motor.position = 10.0

        # Move +5 (now at 15)
        motor.move_relative(5.0)

        # Move -3 (now at 12)
        motor.move_relative(-3.0)

        print(f"Final position: {motor.position}")

Working with Limits
-------------------

Respecting Position Limits
~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        min_pos, max_pos = motor.limits

        # Safe move
        target = 50.0
        if min_pos <= target <= max_pos:
            motor.position = target
        else:
            print(f"Target {target} out of range [{min_pos}, {max_pos}]")

Scanning Within Limits
~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import numpy as np

    with connect():
        motor = Motor("mock_stage")
        min_pos, max_pos = motor.limits

        # Generate positions within limits
        positions = np.linspace(min_pos, max_pos, 10)

        for pos in positions:
            motor.position = pos
            print(f"At {pos} {motor.units}")

Non-Blocking Operations
-----------------------

Starting Non-Blocking Moves
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Start move without waiting
        status = motor.move(100.0, wait=False)

        # status.done is False
        print(f"Moving: {not status.done}")

        # Wait for completion when ready
        status.wait()
        print(f"Arrived at {motor.position}")

Parallel Motor Moves
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        x_motor = Motor("stage_x")
        y_motor = Motor("stage_y")

        # Start both moves
        x_status = x_motor.move(10.0, wait=False)
        y_status = y_motor.move(20.0, wait=False)

        # Wait for both to complete
        x_status.wait()
        y_status.wait()

        print(f"Position: ({x_motor.position}, {y_motor.position})")

Timeouts
~~~~~~~~

.. code-block:: python

    from rust_daq import TimeoutError

    with connect():
        motor = Motor("mock_stage")

        try:
            status = motor.move(100.0, wait=False)
            status.wait(timeout=5.0)  # Wait max 5 seconds
        except TimeoutError:
            print("Move timed out!")

Multi-Axis Coordination
------------------------

2D Grid Positioning
~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import numpy as np

    with connect():
        x_motor = Motor("stage_x")
        y_motor = Motor("stage_y")

        x_positions = np.linspace(0, 10, 5)
        y_positions = np.linspace(0, 10, 5)

        for y in y_positions:
            y_motor.position = y
            for x in x_positions:
                x_motor.position = x
                print(f"At ({x}, {y})")

Synchronized Moves
~~~~~~~~~~~~~~~~~~

For motors that need to move together:

.. code-block:: python

    with connect():
        x_motor = Motor("stage_x")
        y_motor = Motor("stage_y")

        # Define trajectory
        steps = 10
        for i in range(steps):
            t = i / (steps - 1)  # 0 to 1

            # Linear trajectory
            x = 0 + t * 10  # 0 to 10
            y = 0 + t * 5   # 0 to 5

            # Move both (serial)
            x_motor.position = x
            y_motor.position = y

Circular Motion
~~~~~~~~~~~~~~~

.. code-block:: python

    import numpy as np

    with connect():
        x_motor = Motor("stage_x")
        y_motor = Motor("stage_y")

        # Circle parameters
        center_x, center_y = 5.0, 5.0
        radius = 2.0
        n_points = 36

        for i in range(n_points):
            angle = 2 * np.pi * i / n_points

            x = center_x + radius * np.cos(angle)
            y = center_y + radius * np.sin(angle)

            x_motor.position = x
            y_motor.position = y

            print(f"Position {i+1}/{n_points}: ({x:.2f}, {y:.2f})")

Motor Sequencing
----------------

Sequential Operations
~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        # Define sequence
        sequence = [0, 10, 20, 30, 20, 10, 0]

        for i, pos in enumerate(sequence):
            motor.position = pos
            print(f"Step {i+1}: moved to {pos}")

With Dwell Times
~~~~~~~~~~~~~~~~

.. code-block:: python

    import time

    with connect():
        motor = Motor("mock_stage")
        positions = [0, 10, 20, 30]
        dwell_time = 0.5  # seconds

        for pos in positions:
            motor.position = pos
            time.sleep(dwell_time)  # Wait at position

Backlash Compensation
---------------------

If your motor has backlash, approach from one direction:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        def move_with_backlash_comp(target, overshoot=2.0):
            """Move to target with backlash compensation"""
            # Overshoot in positive direction
            motor.position = target + overshoot
            # Approach from above
            motor.position = target

        move_with_backlash_comp(10.0)

Error Recovery
--------------

Handling Move Failures
~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import DeviceError

    with connect():
        motor = Motor("mock_stage")

        try:
            motor.position = 1000.0  # Might be out of range
        except DeviceError as e:
            print(f"Move failed: {e}")
            # Recovery: move to safe position
            motor.position = 0.0

Position Verification
~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")

        target = 10.0
        motor.position = target

        actual = motor.position
        tolerance = 0.1

        if abs(actual - target) > tolerance:
            print(f"Warning: Position error {abs(actual - target)}")
        else:
            print("Position verified")

Best Practices
--------------

1. **Check limits**: Always verify target is within limits
2. **Use backlash compensation**: For precision applications
3. **Verify position**: Check actual position after critical moves
4. **Handle errors**: Wrap moves in try/except blocks
5. **Use non-blocking for parallel moves**: Start multiple motors simultaneously
6. **Add dwell times**: Allow settling time for precision measurements

Performance Tips
----------------

Minimize Communication
~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # BAD: Queries position every iteration
    for i in range(1000):
        current = motor.position  # gRPC call
        next_pos = current + 1
        motor.position = next_pos

    # GOOD: Calculate positions locally
    start = motor.position
    for i in range(1000):
        motor.position = start + i

Batch Moves
~~~~~~~~~~~

For sequential moves, consider using a trajectory:

.. code-block:: python

    import numpy as np

    positions = np.linspace(0, 100, 100)
    for pos in positions:
        motor.position = pos

Next Steps
----------

- Learn about data acquisition in :doc:`data_acquisition`
- Combine motor control with scanning
- Explore async motor control with Layer 1 API
