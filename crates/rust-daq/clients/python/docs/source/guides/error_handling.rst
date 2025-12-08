Error Handling
==============

Comprehensive guide to handling errors in rust-daq.

Exception Hierarchy
-------------------

rust-daq provides a hierarchy of custom exceptions:

.. code-block:: text

    DaqError (base)
    ├── DeviceError
    ├── CommunicationError
    │   └── TimeoutError
    └── ConfigurationError

All exceptions inherit from ``DaqError``, so you can catch all rust-daq errors with:

.. code-block:: python

    from rust_daq import DaqError

    try:
        # ... rust-daq operations
        pass
    except DaqError as e:
        print(f"rust-daq error: {e}")

Exception Types
---------------

DaqError
~~~~~~~~

Base exception for all rust-daq errors.

**Attributes:**

- ``message`` - Human-readable error message
- ``details`` - Optional technical details

**Example:**

.. code-block:: python

    from rust_daq import DaqError

    try:
        # ... operations
        pass
    except DaqError as e:
        print(f"Error: {e.message}")
        if e.details:
            print(f"Details: {e.details}")

DeviceError
~~~~~~~~~~~

Raised for device-specific errors.

**When raised:**

- Device not found
- Device operation failed
- Wrong capability for operation
- Device in invalid state

**Attributes:**

- ``device_id`` - ID of the device
- ``message`` - Error description

**Example:**

.. code-block:: python

    from rust_daq import DeviceError, Motor

    try:
        motor = Motor("nonexistent_device")
    except DeviceError as e:
        print(f"Device '{e.device_id}' error: {e.message}")

CommunicationError
~~~~~~~~~~~~~~~~~~

Raised for network/gRPC errors.

**When raised:**

- Connection to daemon failed
- Network timeout
- gRPC channel error
- Daemon unreachable

**Attributes:**

- ``grpc_code`` - gRPC status code
- ``message`` - Error description

**Example:**

.. code-block:: python

    from rust_daq import CommunicationError, connect

    try:
        with connect("invalid_host:12345"):
            pass
    except CommunicationError as e:
        print(f"Communication failed: {e.message}")
        print(f"gRPC code: {e.grpc_code}")

TimeoutError
~~~~~~~~~~~~

Raised when operations time out.

**When raised:**

- gRPC call exceeds timeout
- Device operation takes too long
- Streaming stalls

**Attributes:**

- ``timeout_seconds`` - Timeout value
- ``message`` - Error description

**Example:**

.. code-block:: python

    from rust_daq import TimeoutError, AsyncClient

    try:
        async with AsyncClient(timeout=1.0) as client:
            await client.move_absolute("slow_stage", 1000.0)
    except TimeoutError as e:
        print(f"Timed out after {e.timeout_seconds}s")

ConfigurationError
~~~~~~~~~~~~~~~~~~

Raised for invalid configuration or parameters.

**When raised:**

- Invalid parameter value
- Configuration validation fails
- Incompatible settings

**Attributes:**

- ``parameter_name`` - Name of invalid parameter
- ``message`` - Error description

**Example:**

.. code-block:: python

    from rust_daq import ConfigurationError, AsyncClient

    try:
        await client.set_parameter("device", "invalid_param", "bad_value")
    except ConfigurationError as e:
        print(f"Bad parameter '{e.parameter_name}': {e.message}")

Handling Specific Errors
-------------------------

Device Not Found
~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import connect, Motor, DeviceError

    with connect():
        try:
            motor = Motor("my_stage")
        except DeviceError as e:
            if "not found" in str(e).lower():
                print(f"Device '{e.device_id}' not found")
                # List available devices
                from rust_daq import AsyncClient
                import anyio

                async def list_devices():
                    async with AsyncClient() as client:
                        devices = await client.list_devices()
                        print("Available devices:")
                        for dev in devices:
                            print(f"  - {dev['id']}")

                anyio.run(list_devices())

Connection Refused
~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import connect, CommunicationError
    import time

    max_retries = 3
    for attempt in range(max_retries):
        try:
            with connect():
                print("Connected!")
                break
        except CommunicationError as e:
            if attempt < max_retries - 1:
                print(f"Connection failed, retrying in 2s... ({attempt+1}/{max_retries})")
                time.sleep(2)
            else:
                print(f"Failed after {max_retries} attempts")
                print("Is the daemon running?")
                raise

Operation Timeout
~~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import connect, Motor, TimeoutError

    with connect():
        motor = Motor("mock_stage")

        try:
            status = motor.move(100.0, wait=False)
            status.wait(timeout=5.0)
        except TimeoutError:
            print("Move timed out, but may still complete in background")
            # Check current position
            print(f"Current position: {motor.position}")

Best Practices
--------------

1. Catch Specific Exceptions First
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import DeviceError, CommunicationError, DaqError

    try:
        # ... operations
        pass
    except DeviceError as e:
        # Handle device-specific errors
        print(f"Device error: {e}")
    except CommunicationError as e:
        # Handle communication errors
        print(f"Communication error: {e}")
    except DaqError as e:
        # Catch-all for other rust-daq errors
        print(f"Other error: {e}")

2. Provide Context in Error Messages
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    try:
        motor.position = target
    except DeviceError as e:
        print(f"Failed to move {motor.device_id} to {target}: {e}")

3. Log Errors for Debugging
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import logging

    logging.basicConfig(level=logging.INFO)
    logger = logging.getLogger(__name__)

    try:
        motor.position = 10.0
    except DeviceError as e:
        logger.error(f"Device error: {e}", exc_info=True)

4. Implement Retry Logic
~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    def move_with_retry(motor, position, max_retries=3):
        for attempt in range(max_retries):
            try:
                motor.position = position
                return  # Success
            except DeviceError as e:
                if attempt < max_retries - 1:
                    print(f"Retry {attempt+1}/{max_retries}")
                    time.sleep(0.5)
                else:
                    raise  # Final attempt failed

5. Clean Up Resources
~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # Use context managers for automatic cleanup
    try:
        with connect():
            motor = Motor("stage")
            motor.position = 10.0
    except DaqError as e:
        print(f"Error: {e}")
    # Connection closed even if error occurred

Error Recovery Patterns
-----------------------

Graceful Degradation
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        # Try to use real hardware
        try:
            motor = Motor("real_stage")
        except DeviceError:
            print("Real hardware not found, using mock")
            motor = Motor("mock_stage")

        motor.position = 10.0

Fallback Values
~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        detector = Detector("power_meter")

        try:
            value = detector.read()
        except DeviceError:
            print("Read failed, using last known value")
            value = last_known_value  # Fallback

Safe Mode
~~~~~~~~~

.. code-block:: python

    def safe_move(motor, position):
        """Move with bounds checking"""
        try:
            min_pos, max_pos = motor.limits
            if not (min_pos <= position <= max_pos):
                raise ValueError(f"Position {position} out of bounds")

            motor.position = position
        except DeviceError as e:
            print(f"Move failed: {e}")
            # Return to safe position
            motor.position = 0.0

Debugging Tips
--------------

Enable Verbose Logging
~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import logging

    logging.basicConfig(
        level=logging.DEBUG,
        format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
    )

Check Daemon Status
~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient

    async def check_daemon():
        try:
            async with AsyncClient() as client:
                info = await client.get_daemon_info()
                print(f"Daemon version: {info['version']}")
                print(f"Features: {info['features']}")
                return True
        except Exception as e:
            print(f"Daemon check failed: {e}")
            return False

    anyio.run(check_daemon())

Inspect Device State
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    async with AsyncClient() as client:
        state = await client.get_device_state("device_id")
        print(f"Device state: {state}")

Common Error Scenarios
----------------------

See :doc:`best_practices` for production error handling patterns.
