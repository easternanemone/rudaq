Data Acquisition Tutorial
==========================

Learn how to acquire and analyze data with rust-daq.

Simple 1D Scans
---------------

Basic Line Scan
~~~~~~~~~~~~~~~

The ``scan()`` function provides automated 1D scans:

.. code-block:: python

    from rust_daq import connect, Motor, Detector, scan

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        data = scan(
            detectors=[detector],
            motor=motor,
            start=0,
            stop=100,
            steps=11
        )

        print(data)  # pandas DataFrame

The result is a DataFrame with columns:

- ``position`` - Motor position
- ``<detector_id>`` - Reading for each detector

Multiple Detectors
~~~~~~~~~~~~~~~~~~

Scan with multiple detectors simultaneously:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")
        det1 = Detector("power_meter")
        det2 = Detector("photodiode")

        data = scan(
            detectors=[det1, det2],
            motor=motor,
            start=0,
            stop=50,
            steps=26
        )

        print(data.head())

DataFrame output:

.. code-block:: text

       position  power_meter  photodiode
    0       0.0     0.123456    0.234567
    1       2.0     0.134567    0.245678
    2       4.0     0.145678    0.256789
    ...

Scan Parameters
---------------

Dwell Time
~~~~~~~~~~

Add settling time at each position:

.. code-block:: python

    data = scan(
        detectors=[detector],
        motor=motor,
        start=0, stop=100, steps=11,
        dwell_time=0.5  # Wait 500ms at each point
    )

Step Calculation
~~~~~~~~~~~~~~~~

The ``steps`` parameter is the number of points:

.. code-block:: python

    # 11 points from 0 to 100 (step size = 10)
    data = scan(..., start=0, stop=100, steps=11)

    # 101 points from 0 to 100 (step size = 1)
    data = scan(..., start=0, stop=100, steps=101)

Return Format
~~~~~~~~~~~~~

Get dict instead of DataFrame:

.. code-block:: python

    data = scan(
        detectors=[detector],
        motor=motor,
        start=0, stop=100, steps=11,
        return_dict=True
    )

    # data is dict: {'position': [...], 'detector_id': [...]}

Working with Scan Data
----------------------

Basic Analysis
~~~~~~~~~~~~~~

.. code-block:: python

    import pandas as pd

    data = scan(...)  # Returns DataFrame

    # Basic statistics
    print(data.describe())

    # Find maximum
    max_idx = data['detector'].idxmax()
    max_pos = data.loc[max_idx, 'position']
    max_val = data.loc[max_idx, 'detector']

    print(f"Peak at {max_pos}: {max_val}")

Plotting
~~~~~~~~

.. code-block:: python

    import matplotlib.pyplot as plt

    data = scan(...)

    plt.figure(figsize=(10, 6))
    plt.plot(data['position'], data['detector'], 'o-')
    plt.xlabel(f'Position ({motor.units})')
    plt.ylabel(f'Signal ({detector.units})')
    plt.title('1D Scan')
    plt.grid(True)
    plt.show()

Saving Data
~~~~~~~~~~~

.. code-block:: python

    # Save to CSV
    data.to_csv('scan_data.csv', index=False)

    # Save to Excel
    data.to_excel('scan_data.xlsx', index=False)

    # Save to HDF5
    data.to_hdf('scan_data.h5', key='scan', mode='w')

Custom Scans
------------

Manual Scan Implementation
~~~~~~~~~~~~~~~~~~~~~~~~~~

For more control, implement your own scan:

.. code-block:: python

    import numpy as np
    import pandas as pd

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        positions = np.linspace(0, 100, 51)
        readings = []

        for pos in positions:
            motor.position = pos
            val = detector.read()
            readings.append(val)

        data = pd.DataFrame({
            'position': positions,
            'signal': readings
        })

With Progress Bar
~~~~~~~~~~~~~~~~~

.. code-block:: python

    from tqdm import tqdm

    positions = np.linspace(0, 100, 101)
    readings = []

    for pos in tqdm(positions, desc='Scanning'):
        motor.position = pos
        val = detector.read()
        readings.append(val)

Adaptive Scanning
~~~~~~~~~~~~~~~~~

Adjust step size based on data:

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        positions = [0]
        readings = []

        current_pos = 0
        while current_pos < 100:
            motor.position = current_pos
            val = detector.read()

            readings.append(val)
            positions.append(current_pos)

            # Adaptive step size
            if len(readings) >= 2 and abs(readings[-1] - readings[-2]) > 0.1:
                step = 1.0  # Small steps in changing region
            else:
                step = 5.0  # Large steps in flat region

            current_pos += step

2D Scans
--------

Grid Scan
~~~~~~~~~

Scan two motors in a grid pattern:

.. code-block:: python

    import numpy as np
    import pandas as pd

    with connect():
        x_motor = Motor("stage_x")
        y_motor = Motor("stage_y")
        detector = Detector("power_meter")

        x_positions = np.linspace(0, 10, 11)
        y_positions = np.linspace(0, 10, 11)

        results = []

        for y in y_positions:
            y_motor.position = y
            for x in x_positions:
                x_motor.position = x
                val = detector.read()
                results.append({'x': x, 'y': y, 'signal': val})

        data = pd.DataFrame(results)

Visualizing 2D Data
~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import matplotlib.pyplot as plt

    # Pivot to 2D array
    pivot = data.pivot(index='y', columns='x', values='signal')

    plt.figure(figsize=(10, 8))
    plt.imshow(pivot, origin='lower', extent=[0, 10, 0, 10], aspect='auto')
    plt.colorbar(label='Signal')
    plt.xlabel('X Position')
    plt.ylabel('Y Position')
    plt.title('2D Scan')
    plt.show()

Snake Scan
~~~~~~~~~~

More efficient 2D scan pattern:

.. code-block:: python

    results = []
    for i, y in enumerate(y_positions):
        y_motor.position = y

        # Alternate scan direction
        if i % 2 == 0:
            x_scan = x_positions
        else:
            x_scan = x_positions[::-1]  # Reverse

        for x in x_scan:
            x_motor.position = x
            val = detector.read()
            results.append({'x': x, 'y': y, 'signal': val})

Time Series Acquisition
-----------------------

Continuous Reading
~~~~~~~~~~~~~~~~~~

Record detector over time:

.. code-block:: python

    import time
    import pandas as pd

    with connect():
        detector = Detector("power_meter")

        times = []
        readings = []

        start_time = time.time()
        duration = 10.0  # seconds
        interval = 0.1   # seconds

        while (time.time() - start_time) < duration:
            t = time.time() - start_time
            val = detector.read()

            times.append(t)
            readings.append(val)

            time.sleep(interval)

        data = pd.DataFrame({'time': times, 'signal': readings})

Triggered Acquisition
~~~~~~~~~~~~~~~~~~~~~

For devices with triggering capability:

.. code-block:: python

    # TODO: Add triggering examples when API is available
    pass

Metadata Recording
------------------

Store scan metadata:

.. code-block:: python

    import json
    from datetime import datetime

    with connect():
        # Run scan
        data = scan(...)

        # Add metadata
        metadata = {
            'timestamp': datetime.now().isoformat(),
            'operator': 'Alice',
            'sample': 'Sample 123',
            'motor': motor.device_id,
            'detector': detector.device_id,
            'motor_units': motor.units,
            'detector_units': detector.units,
        }

        # Save data and metadata
        data.to_csv('scan_data.csv', index=False)

        with open('scan_metadata.json', 'w') as f:
            json.dump(metadata, f, indent=2)

Data Analysis Examples
----------------------

Peak Finding
~~~~~~~~~~~~

.. code-block:: python

    from scipy.signal import find_peaks

    data = scan(...)

    peaks, properties = find_peaks(data['signal'], height=0.5)

    print(f"Found {len(peaks)} peaks")
    for idx in peaks:
        pos = data.loc[idx, 'position']
        val = data.loc[idx, 'signal']
        print(f"  Peak at {pos}: {val}")

Curve Fitting
~~~~~~~~~~~~~

.. code-block:: python

    from scipy.optimize import curve_fit

    data = scan(...)

    def gaussian(x, amp, mean, std):
        return amp * np.exp(-((x - mean) / std) ** 2 / 2)

    popt, pcov = curve_fit(
        gaussian,
        data['position'],
        data['signal'],
        p0=[1.0, 50.0, 10.0]
    )

    amp, mean, std = popt
    print(f"Gaussian fit: amplitude={amp}, mean={mean}, std={std}")

Baseline Subtraction
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # Subtract baseline (first point)
    baseline = data['signal'].iloc[0]
    data['corrected'] = data['signal'] - baseline

Best Practices
--------------

1. **Add dwell times**: Allow detector settling time
2. **Use progress bars**: For long scans (requires tqdm)
3. **Save data frequently**: Don't lose data from crashes
4. **Record metadata**: Store experimental conditions
5. **Verify motor positions**: Check actual vs target positions
6. **Handle errors**: Wrap scans in try/except for robustness

Next Steps
----------

- Learn Jupyter integration in :doc:`jupyter_notebooks`
- Explore async data acquisition with Layer 1 API
- See :doc:`../guides/best_practices` for production workflows
