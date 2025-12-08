Installation
============

Requirements
------------

- Python 3.8 or higher
- rust-daq daemon running (see :ref:`starting-the-daemon`)

Core Dependencies
~~~~~~~~~~~~~~~~~

The following dependencies are installed automatically:

- ``grpcio >= 1.50`` - gRPC Python library
- ``grpcio-tools >= 1.50`` - gRPC code generation tools
- ``protobuf >= 4.20`` - Protocol Buffers
- ``anyio >= 3.0`` - Async I/O framework
- ``numpy >= 1.20`` - Numerical computing

Optional Dependencies
~~~~~~~~~~~~~~~~~~~~~

For scan support (recommended):

- ``pandas >= 1.3`` - DataFrame support for scan results
- ``tqdm >= 4.60`` - Progress bars

For development:

- ``pytest >= 7.0`` - Testing framework
- ``pytest-asyncio >= 0.21`` - Async test support
- ``pytest-mock >= 3.10`` - Mocking support
- ``black >= 23.0`` - Code formatting
- ``mypy >= 1.0`` - Type checking
- ``ruff >= 0.1`` - Fast linting

For documentation:

- ``sphinx >= 7.0`` - Documentation generator
- ``sphinx-rtd-theme >= 2.0`` - Read the Docs theme

Installation Methods
--------------------

From PyPI (Recommended)
~~~~~~~~~~~~~~~~~~~~~~~

Once published on PyPI:

.. code-block:: bash

    # Basic installation
    pip install rust-daq-client

    # With scan support (recommended for most users)
    pip install rust-daq-client[scan]

    # With all optional features
    pip install rust-daq-client[all]

From Source
~~~~~~~~~~~

For development or to get the latest features:

.. code-block:: bash

    # Clone the repository
    git clone https://github.com/yourusername/rust-daq.git
    cd rust-daq/clients/python

    # Install in editable mode
    pip install -e .

    # Or with optional dependencies
    pip install -e ".[scan,dev,docs]"

Verify Installation
-------------------

Test that the installation succeeded:

.. code-block:: python

    import rust_daq
    print(rust_daq.__version__)  # Should print: 0.1.0

Test the AsyncClient import:

.. code-block:: python

    from rust_daq import AsyncClient, Motor, Detector
    print("Installation successful!")

.. _starting-the-daemon:

Starting the rust-daq Daemon
-----------------------------

Before using the Python client, you need to start the rust-daq daemon.

Basic Daemon Start
~~~~~~~~~~~~~~~~~~

From the rust-daq project root:

.. code-block:: bash

    # Start with mock hardware (for testing)
    cargo run --features networking -- daemon --port 50051

With Real Hardware
~~~~~~~~~~~~~~~~~~

To use specific hardware drivers:

.. code-block:: bash

    # All hardware drivers
    cargo run --features "networking,all_hardware" -- daemon --port 50051

    # Specific hardware only
    cargo run --features "networking,instrument_thorlabs" -- daemon --port 50051

Configuration
~~~~~~~~~~~~~

The daemon can be configured via:

- Command-line arguments
- Configuration file (``config/default.toml``)
- Environment variables

See the main rust-daq documentation for details.

Verify Daemon is Running
~~~~~~~~~~~~~~~~~~~~~~~~~

Test the daemon connection:

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient

    async def test_connection():
        async with AsyncClient("localhost:50051") as client:
            info = await client.get_daemon_info()
            print(f"Daemon version: {info['version']}")

    anyio.run(test_connection)

If this succeeds, you're ready to use the client!

Troubleshooting
---------------

Import Errors
~~~~~~~~~~~~~

If you get import errors:

.. code-block:: python

    ModuleNotFoundError: No module named 'rust_daq'

Make sure you've installed the package:

.. code-block:: bash

    pip install -e .

Connection Refused
~~~~~~~~~~~~~~~~~~

If you get connection errors:

.. code-block:: python

    CommunicationError: Daemon unavailable - is the daemon running?

Verify:

1. The daemon is running (see :ref:`starting-the-daemon`)
2. The port matches (default: 50051)
3. No firewall is blocking the connection

Protobuf Version Mismatch
~~~~~~~~~~~~~~~~~~~~~~~~~~

If you get protobuf errors:

.. code-block:: bash

    pip install --upgrade protobuf grpcio grpcio-tools

Missing pandas
~~~~~~~~~~~~~~

If scan() doesn't work:

.. code-block:: bash

    pip install pandas tqdm

Or use ``return_dict=True`` in scan() to get dict instead of DataFrame.

Platform-Specific Notes
-----------------------

Windows
~~~~~~~

On Windows, you may need to install Visual C++ build tools for some dependencies.

macOS
~~~~~

No special requirements. Works on both Intel and Apple Silicon.

Linux
~~~~~

No special requirements. Tested on Ubuntu 20.04+, Debian, and Fedora.

Next Steps
----------

Now that you've installed the client, continue to :doc:`getting_started` for your first program.
