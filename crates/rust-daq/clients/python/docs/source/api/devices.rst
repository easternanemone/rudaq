Devices Module
==============

High-level device classes for synchronous operation.

Context Managers
----------------

connect
~~~~~~~

.. autofunction:: rust_daq.connect

run
~~~

.. autofunction:: rust_daq.run

Device Classes
--------------

Device
~~~~~~

.. autoclass:: rust_daq.Device
   :members:
   :undoc-members:
   :show-inheritance:

   .. automethod:: __init__

Motor
~~~~~

.. autoclass:: rust_daq.Motor
   :members:
   :undoc-members:
   :show-inheritance:

   .. automethod:: __init__
   .. autoproperty:: position
   .. automethod:: move
   .. automethod:: move_relative
   .. autoproperty:: limits
   .. autoproperty:: units

Detector
~~~~~~~~

.. autoclass:: rust_daq.Detector
   :members:
   :undoc-members:
   :show-inheritance:

   .. automethod:: __init__
   .. automethod:: read
   .. autoproperty:: units

Status
~~~~~~

.. autoclass:: rust_daq.Status
   :members:
   :undoc-members:
   :show-inheritance:

   .. autoproperty:: done
   .. automethod:: wait

Scan Function
-------------

scan
~~~~

.. autofunction:: rust_daq.scan
