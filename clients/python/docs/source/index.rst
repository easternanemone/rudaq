rust-daq Python Client Documentation
=====================================

Modern Python client library for controlling the rust-daq headless daemon via gRPC.

The ``rust-daq-client`` provides three API layers designed for different use cases:

- **Layer 2 (High-Level)**: Intuitive synchronous API for scripts and Jupyter notebooks (recommended)
- **Layer 1 (AsyncClient)**: Robust async-first interface for async applications
- **Layer 0 (Protobuf)**: Auto-generated gRPC stubs for low-level access

Quick Example
-------------

.. code-block:: python

    from rust_daq import connect, Motor, Detector, scan

    # Connect to daemon
    with connect("localhost:50051"):
        # Create devices
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        # Simple property-based control
        motor.position = 10.0
        print(f"Position: {motor.position} {motor.units}")

        # Run a scan
        data = scan(
            detectors=[detector],
            motor=motor,
            start=0, stop=100, steps=11
        )

        print(data)  # pandas DataFrame

Getting Started
---------------

.. toctree::
   :maxdepth: 2

   installation
   getting_started

Tutorials
---------

.. toctree::
   :maxdepth: 2

   tutorials/index

How-To Guides
-------------

.. toctree::
   :maxdepth: 2

   guides/index

API Reference
-------------

.. toctree::
   :maxdepth: 2

   api/index

Comparison with Other Frameworks
---------------------------------

.. toctree::
   :maxdepth: 1

   comparison

Indices and tables
==================

* :ref:`genindex`
* :ref:`modindex`
* :ref:`search`

