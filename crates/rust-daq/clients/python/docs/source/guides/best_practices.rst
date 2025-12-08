Best Practices
==============

Production-ready patterns for rust-daq applications.

Connection Management
---------------------

Always Use Context Managers
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # GOOD
    with connect():
        motor = Motor("stage")
        motor.position = 10.0

    # BAD - manual cleanup
    client = AsyncClient()
    await client.connect()
    # ... might not close properly

Reuse Connections
~~~~~~~~~~~~~~~~~

.. code-block:: python

    # GOOD - single connection
    with connect():
        motor = Motor("stage")
        for i in range(100):
            motor.position = i

    # BAD - multiple connections
    for i in range(100):
        with connect():  # Wasteful
            motor = Motor("stage")
            motor.position = i

Resource Management
-------------------

Reuse Device Objects
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # GOOD
    with connect():
        motor = Motor("stage")
        detector = Detector("power_meter")

        for i in range(100):
            motor.position = i
            value = detector.read()

    # BAD
    with connect():
        for i in range(100):
            motor = Motor("stage")  # Wasteful
            detector = Detector("power_meter")
            motor.position = i
            value = detector.read()

Error Handling
--------------

Always Handle Errors
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import DeviceError, CommunicationError

    try:
        with connect():
            motor = Motor("stage")
            motor.position = 10.0
    except DeviceError as e:
        logger.error(f"Device error: {e}")
    except CommunicationError as e:
        logger.error(f"Communication error: {e}")

Data Acquisition
----------------

Save Data Frequently
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import pandas as pd

    data_buffer = []

    with connect():
        motor = Motor("stage")
        detector = Detector("power_meter")

        for i, pos in enumerate(positions):
            motor.position = pos
            val = detector.read()
            data_buffer.append({'position': pos, 'value': val})

            # Save every 100 points
            if i % 100 == 0:
                df = pd.DataFrame(data_buffer)
                df.to_csv('data.csv', mode='a', header=(i==0))

Add Metadata
~~~~~~~~~~~~

.. code-block:: python

    from datetime import datetime
    import json

    metadata = {
        'timestamp': datetime.now().isoformat(),
        'operator': 'Alice',
        'sample': 'Sample 123',
        'notes': 'Initial characterization'
    }

    with open('metadata.json', 'w') as f:
        json.dump(metadata, f, indent=2)

Performance Optimization
------------------------

Minimize gRPC Calls
~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # GOOD - calculate locally
    start_pos = motor.position
    for i in range(100):
        motor.position = start_pos + i

    # BAD - query every iteration
    for i in range(100):
        current = motor.position  # gRPC call
        motor.position = current + 1

Use Appropriate Wait Modes
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # For parallel moves
    s1 = motor1.move(10.0, wait=False)
    s2 = motor2.move(20.0, wait=False)
    s1.wait()
    s2.wait()

    # For sequential moves
    motor.position = 10.0  # wait=True by default

Code Organization
-----------------

Separate Concerns
~~~~~~~~~~~~~~~~~

.. code-block:: python

    # hardware.py
    from rust_daq import connect, Motor, Detector

    class Hardware:
        def __init__(self):
            self.motor = None
            self.detector = None

        def __enter__(self):
            self.conn = connect().__enter__()
            self.motor = Motor("stage")
            self.detector = Detector("power_meter")
            return self

        def __exit__(self, *args):
            self.conn.__exit__(*args)

    # experiment.py
    from hardware import Hardware

    with Hardware() as hw:
        hw.motor.position = 10.0
        value = hw.detector.read()

Use Configuration Files
~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # config.yaml
    daemon:
      host: localhost
      port: 50051
      timeout: 10.0

    devices:
      motor: mock_stage
      detector: mock_power_meter

    # main.py
    import yaml

    with open('config.yaml') as f:
        config = yaml.safe_load(f)

    with connect(
        f"{config['daemon']['host']}:{config['daemon']['port']}",
        timeout=config['daemon']['timeout']
    ):
        motor = Motor(config['devices']['motor'])

Testing
-------

Use Mock Hardware
~~~~~~~~~~~~~~~~~

.. code-block:: python

    import pytest
    from rust_daq import connect, Motor

    @pytest.fixture
    def hardware():
        with connect():
            motor = Motor("mock_stage")
            yield motor

    def test_motor_move(hardware):
        hardware.position = 10.0
        assert abs(hardware.position - 10.0) < 0.1

Integration Tests
~~~~~~~~~~~~~~~~~

.. code-block:: python

    @pytest.mark.integration
    def test_real_hardware():
        """Requires running daemon with real hardware"""
        with connect():
            motor = Motor("real_stage")
            motor.position = 10.0

    # Run with: pytest -m integration

Logging
-------

Structured Logging
~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import logging

    logging.basicConfig(
        level=logging.INFO,
        format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
        handlers=[
            logging.FileHandler('experiment.log'),
            logging.StreamHandler()
        ]
    )

    logger = logging.getLogger(__name__)

    with connect():
        motor = Motor("stage")
        logger.info(f"Moving motor {motor.device_id} to 10.0")
        motor.position = 10.0
        logger.info(f"Move complete, at {motor.position}")

Security
--------

Never Hardcode Credentials
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import os

    # Use environment variables
    host = os.getenv('DAQ_HOST', 'localhost')
    port = os.getenv('DAQ_PORT', '50051')

    with connect(f"{host}:{port}"):
        pass

Validate Inputs
~~~~~~~~~~~~~~~

.. code-block:: python

    def safe_move(motor, position):
        """Move with validation"""
        min_pos, max_pos = motor.limits

        if not isinstance(position, (int, float)):
            raise TypeError("Position must be numeric")

        if not (min_pos <= position <= max_pos):
            raise ValueError(f"Position {position} out of bounds")

        motor.position = position

Documentation
-------------

Add Docstrings
~~~~~~~~~~~~~~

.. code-block:: python

    def acquire_scan(motor, detector, start, stop, steps):
        """
        Acquire a 1D scan.

        Args:
            motor: Motor device to scan
            detector: Detector to read
            start: Starting position
            stop: Ending position
            steps: Number of points

        Returns:
            pandas.DataFrame with scan data

        Raises:
            DeviceError: If device operation fails
        """
        return scan(
            detectors=[detector],
            motor=motor,
            start=start,
            stop=stop,
            steps=steps
        )

Production Checklist
--------------------

Before deploying to production:

- [ ] All operations in try/except blocks
- [ ] Logging configured
- [ ] Data saved frequently with metadata
- [ ] Configuration in files, not hardcoded
- [ ] Tests passing
- [ ] Error recovery implemented
- [ ] Resource cleanup verified
- [ ] Performance profiled
- [ ] Documentation complete

Next Steps
----------

- See :doc:`error_handling` for exception handling
- See :doc:`async_vs_sync` for performance optimization
