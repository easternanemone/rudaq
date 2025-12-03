Async vs Sync API
=================

Understanding when to use Layer 1 (AsyncClient) vs Layer 2 (high-level sync API).

When to Use Each Layer
----------------------

Use Layer 2 (Sync API)
~~~~~~~~~~~~~~~~~~~~~~

**Best for:**

- Interactive scripts
- Jupyter notebooks
- Simple automation
- Learning rust-daq
- Sequential operations

**Advantages:**

- Simpler code
- No async/await complexity
- Property-based interface
- Ideal for beginners

**Example:**

.. code-block:: python

    from rust_daq import connect, Motor

    with connect():
        motor = Motor("mock_stage")
        motor.position = 10.0  # Simple!

Use Layer 1 (AsyncClient)
~~~~~~~~~~~~~~~~~~~~~~~~~~

**Best for:**

- Async applications
- Concurrent operations
- Maximum performance
- Advanced users
- Integration with async frameworks

**Advantages:**

- True async concurrency
- Fine-grained control
- Better performance for parallel ops
- Full API access

**Example:**

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient

    async def main():
        async with AsyncClient() as client:
            await client.move_absolute("mock_stage", 10.0)

    anyio.run(main)

Side-by-Side Comparison
-----------------------

Simple Motor Move
~~~~~~~~~~~~~~~~~

Layer 2 (Sync):

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")
        motor.position = 10.0

Layer 1 (Async):

.. code-block:: python

    async with AsyncClient() as client:
        await client.move_absolute("mock_stage", 10.0)

**Winner**: Layer 2 for simplicity

Reading a Detector
~~~~~~~~~~~~~~~~~~

Layer 2 (Sync):

.. code-block:: python

    with connect():
        detector = Detector("power_meter")
        value = detector.read()

Layer 1 (Async):

.. code-block:: python

    async with AsyncClient() as client:
        state = await client.get_device_state("power_meter")
        value = state['last_reading']

**Winner**: Layer 2 for readability

Parallel Operations
~~~~~~~~~~~~~~~~~~~

Layer 2 (Sync):

.. code-block:: python

    # Limited parallelism (uses wait=False)
    with connect():
        motor1 = Motor("stage_x")
        motor2 = Motor("stage_y")

        s1 = motor1.move(10.0, wait=False)
        s2 = motor2.move(20.0, wait=False)

        s1.wait()
        s2.wait()

Layer 1 (Async):

.. code-block:: python

    # True async parallelism
    async with AsyncClient() as client:
        await anyio.create_task_group(
            client.move_absolute("stage_x", 10.0),
            client.move_absolute("stage_y", 20.0)
        )

**Winner**: Layer 1 for true concurrency

Migration Guide
---------------

From Sync to Async
~~~~~~~~~~~~~~~~~~

If you need to migrate from Layer 2 to Layer 1:

**Before (Layer 2)**:

.. code-block:: python

    from rust_daq import connect, Motor, Detector

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("power_meter")

        motor.position = 10.0
        value = detector.read()

**After (Layer 1)**:

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient

    async def main():
        async with AsyncClient() as client:
            await client.move_absolute("mock_stage", 10.0)

            state = await client.get_device_state("power_meter")
            value = state['last_reading']

    anyio.run(main)

Mixing Layers
-------------

You can mix layers in the same application:

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient, Motor, Detector

    # Async function using Layer 1
    async def async_operation():
        async with AsyncClient() as client:
            await client.move_absolute("mock_stage", 50.0)

    # Sync wrapper
    def sync_wrapper():
        anyio.run(async_operation)

    # Use in Layer 2 context
    from rust_daq import connect

    with connect():
        # Do sync stuff
        motor = Motor("mock_stage")
        motor.position = 10.0

    # Then do async stuff
    sync_wrapper()

Performance Comparison
----------------------

Sequential Operations
~~~~~~~~~~~~~~~~~~~~~

For sequential operations, performance is similar:

.. code-block:: python

    # Both take ~1 second for 10 moves
    # Layer 2
    for i in range(10):
        motor.position = i

    # Layer 1
    for i in range(10):
        await client.move_absolute("stage", i)

Parallel Operations
~~~~~~~~~~~~~~~~~~~

For parallel operations, Layer 1 is faster:

.. code-block:: python

    # Layer 2: ~1 second (pseudo-parallel)
    s1 = motor1.move(10.0, wait=False)
    s2 = motor2.move(20.0, wait=False)
    s1.wait()
    s2.wait()

    # Layer 1: ~0.5 seconds (true parallel)
    await anyio.create_task_group(
        client.move_absolute("stage_x", 10.0),
        client.move_absolute("stage_y", 20.0)
    )

Best Practices
--------------

1. **Start with Layer 2**: Learn with sync API first
2. **Migrate when needed**: Switch to Layer 1 for concurrency
3. **Don't mix in same function**: Pick one layer per function
4. **Use anyio for portability**: Works with both asyncio and trio
5. **Profile before optimizing**: Measure actual performance gains

Common Patterns
---------------

Pattern 1: Sync Script
~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from rust_daq import connect, Motor, scan

    def main():
        with connect():
            motor = Motor("stage")
            # ... do work

    if __name__ == "__main__":
        main()

Pattern 2: Async Application
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import anyio
    from rust_daq import AsyncClient

    async def main():
        async with AsyncClient() as client:
            # ... do async work
            pass

    if __name__ == "__main__":
        anyio.run(main)

Pattern 3: Web Framework Integration
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from fastapi import FastAPI
    from rust_daq import AsyncClient

    app = FastAPI()
    client = None

    @app.on_event("startup")
    async def startup():
        global client
        client = AsyncClient()
        await client.connect()

    @app.on_event("shutdown")
    async def shutdown():
        await client.close()

    @app.post("/move")
    async def move(device_id: str, position: float):
        await client.move_absolute(device_id, position)
        return {"status": "ok"}

Next Steps
----------

- See :doc:`error_handling` for exception handling
- Read :doc:`best_practices` for production tips
