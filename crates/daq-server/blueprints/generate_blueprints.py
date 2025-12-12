#!/usr/bin/env python3
"""
Generate Rerun Blueprint files (.rbl) for rust-daq visualization.

These blueprints can be loaded from Rust using `log_file_from_path`.

Usage:
    pip install rerun-sdk
    python generate_blueprints.py

Output:
    - daq_default.rbl: Default layout with camera view + time series
    - daq_camera_only.rbl: Full-screen camera/tensor view
    - daq_timeseries_only.rbl: Time series plots for scalar measurements
"""

import rerun.blueprint as rrb

# Application ID must match the Rust code
APP_ID = "rust-daq"


def create_default_blueprint() -> rrb.Blueprint:
    """Default layout: Camera view on left, time series on right."""
    return rrb.Blueprint(
        rrb.Horizontal(
            # Left panel: Camera/Image view
            rrb.TensorView(
                name="Camera",
                origin="/device",
                contents=["+ /device/**"],
            ),
            # Right panel: Time series for scalar values
            rrb.Vertical(
                rrb.TimeSeriesView(
                    name="Power",
                    origin="/device",
                    contents=["+ /device/power*/**"],
                ),
                rrb.TimeSeriesView(
                    name="Position",
                    origin="/device",
                    contents=["+ /device/position*/**", "+ /device/wavelength*/**"],
                ),
            ),
            column_shares=[2, 1],  # Camera gets 2/3 width
        ),
        rrb.BlueprintPanel(state="collapsed"),
        rrb.SelectionPanel(state="collapsed"),
        rrb.TimePanel(state="expanded"),
    )


def create_camera_only_blueprint() -> rrb.Blueprint:
    """Full-screen camera view for image inspection."""
    return rrb.Blueprint(
        rrb.TensorView(
            name="Camera Full",
            origin="/device",
            contents=["+ /device/**"],
        ),
        rrb.BlueprintPanel(state="collapsed"),
        rrb.SelectionPanel(state="collapsed"),
        rrb.TimePanel(state="expanded"),
    )


def create_timeseries_blueprint() -> rrb.Blueprint:
    """Time series view for scalar measurements (power meter, stage position)."""
    return rrb.Blueprint(
        rrb.Vertical(
            rrb.TimeSeriesView(
                name="All Measurements",
                origin="/device",
                contents=["+ /device/**"],
            ),
        ),
        rrb.BlueprintPanel(state="collapsed"),
        rrb.SelectionPanel(state="collapsed"),
        rrb.TimePanel(state="expanded"),
    )


def create_acquisition_blueprint() -> rrb.Blueprint:
    """Layout for active acquisition: camera + live stats."""
    return rrb.Blueprint(
        rrb.Horizontal(
            # Main camera view
            rrb.TensorView(
                name="Live Camera",
                origin="/device",
            ),
            # Stats panel
            rrb.Vertical(
                rrb.TimeSeriesView(
                    name="Frame Stats",
                    origin="/device",
                    contents=["+ /device/frame_rate/**", "+ /device/exposure/**"],
                ),
                rrb.TextLogView(
                    name="Events",
                    origin="/events",
                ),
            ),
            column_shares=[3, 1],
        ),
        rrb.BlueprintPanel(state="collapsed"),
        rrb.SelectionPanel(state="collapsed"),
        rrb.TimePanel(state="expanded"),
    )


def main():
    blueprints = {
        "daq_default.rbl": create_default_blueprint(),
        "daq_camera_only.rbl": create_camera_only_blueprint(),
        "daq_timeseries_only.rbl": create_timeseries_blueprint(),
        "daq_acquisition.rbl": create_acquisition_blueprint(),
    }

    for filename, blueprint in blueprints.items():
        blueprint.save(APP_ID, filename)
        print(f"Generated: {filename}")

    print(f"\nAll blueprints use application ID: '{APP_ID}'")
    print("Load in Rust with: rec.log_file_from_path(\"path/to/blueprint.rbl\", None, true)")


if __name__ == "__main__":
    main()
