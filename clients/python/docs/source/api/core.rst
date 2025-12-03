Core Module
===========

The core module provides the AsyncClient class for async operations.

AsyncClient
-----------

.. autoclass:: rust_daq.AsyncClient
   :members:
   :undoc-members:
   :show-inheritance:

   .. automethod:: __init__
   .. automethod:: __aenter__
   .. automethod:: __aexit__

Connection Management
~~~~~~~~~~~~~~~~~~~~~

.. automethod:: rust_daq.AsyncClient.connect
.. automethod:: rust_daq.AsyncClient.close

Control Service Methods
~~~~~~~~~~~~~~~~~~~~~~~

.. automethod:: rust_daq.AsyncClient.get_daemon_info

Hardware Service Methods
~~~~~~~~~~~~~~~~~~~~~~~~

Device Discovery
^^^^^^^^^^^^^^^^

.. automethod:: rust_daq.AsyncClient.list_devices
.. automethod:: rust_daq.AsyncClient.get_device_state

Motion Control
^^^^^^^^^^^^^^

.. automethod:: rust_daq.AsyncClient.move_absolute
.. automethod:: rust_daq.AsyncClient.move_relative
.. automethod:: rust_daq.AsyncClient.get_position

Parameter Control
^^^^^^^^^^^^^^^^^

.. automethod:: rust_daq.AsyncClient.set_parameter
.. automethod:: rust_daq.AsyncClient.get_parameter

State Streaming
^^^^^^^^^^^^^^^

.. automethod:: rust_daq.AsyncClient.stream_device_state
