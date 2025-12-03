Jupyter Notebook Integration
=============================

rust-daq works seamlessly in Jupyter notebooks for interactive data acquisition.

Basic Setup
-----------

Cell 1: Imports
~~~~~~~~~~~~~~~

.. code-block:: python

    %matplotlib inline
    import matplotlib.pyplot as plt
    import numpy as np
    from rust_daq import connect, Motor, Detector, scan

Cell 2: Connect
~~~~~~~~~~~~~~~

.. code-block:: python

    # Connection persists for notebook session
    from rust_daq import connect

    # Note: Use this at the start of your workflow
    conn = connect().__enter__()

Or use context manager in each cell (safer):

.. code-block:: python

    with connect():
        # Your code here
        pass

Interactive Motor Control
-------------------------

Create interactive controls using ipywidgets:

.. code-block:: python

    from ipywidgets import interact, FloatSlider

    with connect():
        motor = Motor("mock_stage")

        @interact(position=FloatSlider(min=0, max=100, step=1, value=0))
        def move_motor(position):
            motor.position = position
            print(f"Moved to {motor.position} {motor.units}")

Live Plotting
-------------

Simple Live Plot
~~~~~~~~~~~~~~~~

.. code-block:: python

    import matplotlib.pyplot as plt
    from IPython.display import clear_output

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        positions = np.linspace(0, 100, 51)
        readings = []

        plt.figure(figsize=(10, 6))

        for pos in positions:
            motor.position = pos
            val = detector.read()
            readings.append(val)

            # Update plot
            clear_output(wait=True)
            plt.clf()
            plt.plot(positions[:len(readings)], readings, 'o-')
            plt.xlabel('Position')
            plt.ylabel('Signal')
            plt.grid(True)
            plt.show()

Interactive Visualization
~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    %matplotlib notebook  # Enable interactive mode

    import matplotlib.pyplot as plt

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        fig, ax = plt.subplots(figsize=(10, 6))
        line, = ax.plot([], [], 'o-')
        ax.set_xlabel('Position')
        ax.set_ylabel('Signal')
        ax.grid(True)

        positions = []
        readings = []

        for i in range(50):
            pos = i * 2
            motor.position = pos
            val = detector.read()

            positions.append(pos)
            readings.append(val)

            # Update plot
            line.set_data(positions, readings)
            ax.relim()
            ax.autoscale_view()
            fig.canvas.draw()

Quick Data Exploration
----------------------

Run and Analyze Scan
~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        data = scan(
            detectors=[detector],
            motor=motor,
            start=0, stop=100, steps=51
        )

    # Analyze
    display(data.head())
    display(data.describe())

    # Plot
    data.plot(x='position', y='mock_power_meter', figsize=(10, 6))
    plt.grid(True)
    plt.show()

2D Heatmap
~~~~~~~~~~

.. code-block:: python

    import pandas as pd
    import seaborn as sns

    with connect():
        x_motor = Motor("stage_x")
        y_motor = Motor("stage_y")
        detector = Detector("power_meter")

        x_pos = np.linspace(0, 10, 11)
        y_pos = np.linspace(0, 10, 11)

        results = []
        for y in y_pos:
            y_motor.position = y
            for x in x_pos:
                x_motor.position = x
                val = detector.read()
                results.append({'x': x, 'y': y, 'signal': val})

        data = pd.DataFrame(results)

    # Visualize
    pivot = data.pivot(index='y', columns='x', values='signal')
    plt.figure(figsize=(10, 8))
    sns.heatmap(pivot, annot=True, fmt='.2f', cmap='viridis')
    plt.title('2D Scan')
    plt.show()

Interactive Dashboard
---------------------

Complete Dashboard Example
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    import ipywidgets as widgets
    from IPython.display import display

    # Create widgets
    motor_slider = widgets.FloatSlider(
        min=0, max=100, step=1, value=0,
        description='Position:'
    )

    detector_output = widgets.Output()
    status_label = widgets.Label(value='Ready')

    # Layout
    dashboard = widgets.VBox([
        widgets.Label('Motor Control Dashboard'),
        motor_slider,
        status_label,
        detector_output
    ])

    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        def on_slider_change(change):
            try:
                motor.position = change['new']
                val = detector.read()

                status_label.value = f'Position: {motor.position:.2f} {motor.units}'

                with detector_output:
                    clear_output(wait=True)
                    print(f"Reading: {val:.3f} {detector.units}")

            except Exception as e:
                status_label.value = f'Error: {e}'

        motor_slider.observe(on_slider_change, names='value')

        display(dashboard)

Scan Control Panel
~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # Scan parameters
    start_widget = widgets.FloatText(value=0, description='Start:')
    stop_widget = widgets.FloatText(value=100, description='Stop:')
    steps_widget = widgets.IntText(value=51, description='Steps:')
    run_button = widgets.Button(description='Run Scan')

    output = widgets.Output()

    def run_scan(b):
        with output:
            clear_output(wait=True)
            with connect():
                motor = Motor("mock_stage")
                detector = Detector("mock_power_meter")

                data = scan(
                    detectors=[detector],
                    motor=motor,
                    start=start_widget.value,
                    stop=stop_widget.value,
                    steps=steps_widget.value
                )

                data.plot(x='position', y='mock_power_meter')
                plt.show()

    run_button.on_click(run_scan)

    display(widgets.VBox([
        start_widget,
        stop_widget,
        steps_widget,
        run_button,
        output
    ]))

Data Analysis Workflow
----------------------

Complete Analysis Pipeline
~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # 1. Acquire data
    with connect():
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

        data = scan(detectors=[detector], motor=motor,
                   start=0, stop=100, steps=101)

    # 2. Initial visualization
    data.plot(x='position', y='mock_power_meter', figsize=(12, 6))
    plt.title('Raw Data')
    plt.grid(True)
    plt.show()

    # 3. Process data
    from scipy.signal import savgol_filter

    data['smoothed'] = savgol_filter(data['mock_power_meter'], 11, 3)

    # 4. Find peaks
    from scipy.signal import find_peaks

    peaks, props = find_peaks(data['smoothed'], height=0.5, distance=10)

    # 5. Visualize results
    plt.figure(figsize=(12, 6))
    plt.plot(data['position'], data['mock_power_meter'],
             'o', alpha=0.5, label='Raw')
    plt.plot(data['position'], data['smoothed'],
             '-', linewidth=2, label='Smoothed')
    plt.plot(data.loc[peaks, 'position'],
             data.loc[peaks, 'smoothed'],
             'rx', markersize=10, label='Peaks')
    plt.xlabel('Position')
    plt.ylabel('Signal')
    plt.legend()
    plt.grid(True)
    plt.show()

    # 6. Report results
    print(f"Found {len(peaks)} peaks:")
    for i, idx in enumerate(peaks):
        pos = data.loc[idx, 'position']
        val = data.loc[idx, 'smoothed']
        print(f"  Peak {i+1}: position={pos:.2f}, signal={val:.3f}")

Saving Results
--------------

Save to Multiple Formats
~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    from datetime import datetime

    # Create filename with timestamp
    timestamp = datetime.now().strftime('%Y%m%d_%H%M%S')

    # Save data
    data.to_csv(f'scan_{timestamp}.csv', index=False)
    data.to_excel(f'scan_{timestamp}.xlsx', index=False)

    # Save figure
    plt.savefig(f'scan_{timestamp}.png', dpi=300, bbox_inches='tight')

    print(f"Saved: scan_{timestamp}.*")

Notebook Templates
------------------

Quick Scan Template
~~~~~~~~~~~~~~~~~~~

.. code-block:: python

    # ==== QUICK SCAN TEMPLATE ====

    # Setup
    from rust_daq import connect, Motor, Detector, scan
    import matplotlib.pyplot as plt
    %matplotlib inline

    # Parameters
    MOTOR_ID = "mock_stage"
    DETECTOR_ID = "mock_power_meter"
    START = 0
    STOP = 100
    STEPS = 51

    # Run scan
    with connect():
        motor = Motor(MOTOR_ID)
        detector = Detector(DETECTOR_ID)

        data = scan(
            detectors=[detector],
            motor=motor,
            start=START,
            stop=STOP,
            steps=STEPS
        )

    # Visualize
    data.plot(x='position', y=DETECTOR_ID, figsize=(10, 6))
    plt.grid(True)
    plt.show()

    # Analyze
    display(data.describe())

Best Practices
--------------

1. **Use context managers**: Prevents connection leaks
2. **Clear outputs**: Use ``clear_output(wait=True)`` for live updates
3. **Interactive plots**: Use ``%matplotlib notebook`` for interactivity
4. **Save frequently**: Save data after each scan
5. **Add markdown**: Document your experiments
6. **Version control**: Save notebooks with git

Common Issues
-------------

Kernel Restart
~~~~~~~~~~~~~~

If connection is lost after kernel restart:

.. code-block:: python

    # Re-run connection cell
    with connect():
        # Recreate devices
        motor = Motor("mock_stage")
        detector = Detector("mock_power_meter")

Plot Not Updating
~~~~~~~~~~~~~~~~~

Try different matplotlib backends:

.. code-block:: python

    %matplotlib inline    # Static plots
    %matplotlib notebook  # Interactive plots
    %matplotlib widget    # ipympl widgets

Connection Issues
~~~~~~~~~~~~~~~~~

Check daemon status in terminal:

.. code-block:: bash

    # In separate terminal
    cargo run --features networking -- daemon --port 50051

Next Steps
----------

- Explore advanced analysis in Python
- Build custom dashboards with ipywidgets
- Integrate with Holoviews/Plotly for advanced visualization
- See :doc:`../guides/best_practices` for production tips
