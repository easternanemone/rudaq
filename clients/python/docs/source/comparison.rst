Comparison with Other Frameworks
=================================

How rust-daq compares to popular Python DAQ frameworks.

Overview
--------

rust-daq vs Other Frameworks:

================ ============ ========= ============ =============
Feature          rust-daq     PyMoDAQ   Bluesky      ScopeFoundry
================ ============ ========= ============ =============
Language         Rust+Python  Python    Python       Python
Architecture     Headless     GUI-first Plan-based   Qt-based
Performance      High         Medium    Medium       Medium
Async Support    Native       Limited   Advanced     Limited
Scripting        Rhai+Python  Python    Python       Python
Learning Curve   Low          Medium    High         Medium
Remote Control   gRPC         Custom    REST         Custom
================ ============ ========= ============ =============

rust-daq vs PyMoDAQ
--------------------

Philosophy
~~~~~~~~~~

**PyMoDAQ**: GUI-first framework with plugin architecture

**rust-daq**: Headless-first with optional remote GUI

Code Comparison
~~~~~~~~~~~~~~~

**PyMoDAQ**:

.. code-block:: python

    from pymodaq.control_modules.move_utility_classes import DAQ_Move
    from pymodaq.utils.daq_utils import ThreadCommand

    class DAQ_Move_MyStage(DAQ_Move):
        def __init__(self, parent=None, params_state=None):
            super().__init__(parent, params_state)
            # Setup code...

        def move_abs(self, position):
            # Move implementation
            pass

**rust-daq**:

.. code-block:: python

    from rust_daq import connect, Motor

    with connect():
        motor = Motor("my_stage")
        motor.position = position  # Simple!

Migration from PyMoDAQ
~~~~~~~~~~~~~~~~~~~~~~

**Concepts Mapping**:

=================== ===================
PyMoDAQ             rust-daq
=================== ===================
DAQ_Move            Motor
DAQ_Viewer          Detector
Move_Abs            motor.position = x
Get_Position        motor.position
Grab_Data           detector.read()
=================== ===================

**Example Migration**:

PyMoDAQ code:

.. code-block:: python

    # Create move module
    move = DAQ_Move("MyStage")
    move.move_abs(10.0)
    pos = move.get_position()

    # Create viewer module
    viewer = DAQ_Viewer("MyDetector")
    data = viewer.grab_data()

rust-daq equivalent:

.. code-block:: python

    with connect():
        motor = Motor("my_stage")
        motor.position = 10.0
        pos = motor.position

        detector = Detector("my_detector")
        data = detector.read()

rust-daq vs Bluesky/Ophyd
--------------------------

Philosophy
~~~~~~~~~~

**Bluesky**: Plan-based data acquisition with document model

**rust-daq**: Direct hardware control with flexible scripting

Code Comparison
~~~~~~~~~~~~~~~

**Bluesky/Ophyd**:

.. code-block:: python

    from ophyd import EpicsMotor, Device
    from bluesky import RunEngine
    from bluesky.plans import scan

    motor = EpicsMotor('XF:31IDA-OP{Tbl-Ax:X1}Mtr', name='motor')
    det = Device('XF:31IDA-BI{Det:1}', name='det')

    RE = RunEngine({})
    RE(scan([det], motor, -1, 1, 50))

**rust-daq**:

.. code-block:: python

    from rust_daq import connect, Motor, Detector, scan

    with connect():
        motor = Motor("motor_x")
        det = Detector("detector_1")

        data = scan([det], motor, start=-1, stop=1, steps=50)

Migration from Bluesky
~~~~~~~~~~~~~~~~~~~~~~

**Concepts Mapping**:

=================== ===================
Bluesky/Ophyd       rust-daq
=================== ===================
EpicsMotor          Motor
Device (detector)   Detector
RunEngine           connect()
scan plan           scan()
count plan          detector.read()
mv (move)           motor.position = x
=================== ===================

**Key Differences**:

1. **No RunEngine**: rust-daq uses direct hardware control
2. **No plans**: Use Python code or Rhai scripts instead
3. **Simpler API**: Property-based access vs callbacks
4. **No document model**: Direct DataFrame output

**Example Migration**:

Bluesky code:

.. code-block:: python

    from bluesky.plans import scan, count
    from bluesky import RunEngine

    RE = RunEngine({})

    # Scan
    RE(scan([det], motor, 0, 100, 11))

    # Count
    RE(count([det], num=10))

rust-daq equivalent:

.. code-block:: python

    with connect():
        # Scan
        data = scan([det], motor, 0, 100, 11)

        # Multiple readings
        readings = [det.read() for _ in range(10)]

rust-daq vs ScopeFoundry
-------------------------

Philosophy
~~~~~~~~~~

**ScopeFoundry**: Qt-based framework for microscopy

**rust-daq**: Headless with remote GUI option

Code Comparison
~~~~~~~~~~~~~~~

**ScopeFoundry**:

.. code-block:: python

    from ScopeFoundry import BaseApp, HardwareComponent

    class MyHardware(HardwareComponent):
        name = 'my_hardware'

        def setup(self):
            self.settings.New('position', dtype=float, unit='mm')

        def move(self, pos):
            # Implementation
            pass

    app = BaseApp()
    hw = app.add_hw(MyHardware)

**rust-daq**:

.. code-block:: python

    with connect():
        motor = Motor("my_hardware")
        motor.position = 10.0

Migration Guide
---------------

From PyMoDAQ
~~~~~~~~~~~~

1. Replace DAQ_Move/DAQ_Viewer with Motor/Detector
2. Remove GUI-related code (handled by daemon)
3. Use ``with connect():`` for session management
4. Direct property access instead of method calls

From Bluesky
~~~~~~~~~~~~

1. Replace RunEngine with ``connect()`` context manager
2. Convert plans to direct Python code or Rhai scripts
3. Replace Ophyd devices with Motor/Detector
4. Use DataFrame instead of document model

From ScopeFoundry
~~~~~~~~~~~~~~~~~

1. Remove Qt/GUI code
2. Replace HardwareComponent with Motor/Detector
3. Use gRPC daemon instead of in-process hardware
4. Migrate measurement logic to Python/Rhai scripts

Advantages of rust-daq
-----------------------

1. **Performance**: Rust core provides high performance
2. **Simplicity**: Minimal boilerplate, intuitive API
3. **Headless-first**: No GUI required for automation
4. **Modern**: gRPC, async/await, type hints
5. **Flexible**: Python, Rhai, or gRPC control
6. **Lightweight**: Small memory footprint
7. **Safe**: Rust's safety guarantees in core

When to Choose Each
-------------------

Choose **rust-daq** if you want:

- Headless automation
- High performance
- Simple API
- Modern architecture
- Flexible remote control

Choose **PyMoDAQ** if you want:

- Rich GUI out of the box
- Large plugin ecosystem
- Established community

Choose **Bluesky** if you want:

- Complex experimental plans
- Document model for data
- EPICS integration
- Synchrotron beamline features

Choose **ScopeFoundry** if you want:

- Microscopy-specific features
- Qt-based GUI
- Image acquisition focus

Next Steps
----------

Ready to migrate? See:

- :doc:`getting_started` for first steps
- :doc:`tutorials/index` for detailed examples
- :doc:`api/index` for complete API reference
