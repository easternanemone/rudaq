Exceptions Module
=================

Custom exception hierarchy for rust-daq.

Exception Hierarchy
-------------------

.. code-block:: text

    DaqError (base)
    ├── DeviceError
    ├── CommunicationError
    │   └── TimeoutError
    └── ConfigurationError

Base Exception
--------------

DaqError
~~~~~~~~

.. autoexception:: rust_daq.DaqError
   :members:
   :show-inheritance:

Specific Exceptions
-------------------

DeviceError
~~~~~~~~~~~

.. autoexception:: rust_daq.DeviceError
   :members:
   :show-inheritance:

CommunicationError
~~~~~~~~~~~~~~~~~~

.. autoexception:: rust_daq.CommunicationError
   :members:
   :show-inheritance:

TimeoutError
~~~~~~~~~~~~

.. autoexception:: rust_daq.TimeoutError
   :members:
   :show-inheritance:

ConfigurationError
~~~~~~~~~~~~~~~~~~

.. autoexception:: rust_daq.ConfigurationError
   :members:
   :show-inheritance:

Utility Functions
-----------------

translate_grpc_error
~~~~~~~~~~~~~~~~~~~~

.. autofunction:: rust_daq.exceptions.translate_grpc_error
