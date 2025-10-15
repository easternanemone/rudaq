#!/usr/bin/env python3
"""
Simple data acquisition example using rust_daq Python bindings.

This minimal example demonstrates basic instrument initialization and data
point collection. It's suitable for quick testing and as a starting template.
"""

import rust_daq
from datetime import datetime, timezone


def main():
    # Initialize instruments
    print("Initializing MaiTai laser...")
    laser = rust_daq.MaiTai(port="COM3")

    print("Setting wavelength to 800 nm...")
    laser.set_wavelength(800.0)

    print("\nInitializing Newport 1830C power meter...")
    meter = rust_daq.Newport1830C(
        resource_string="USB0::0x104D::0xC0DE::SN12345::INSTR"
    )

    print("Reading power...")
    power = meter.read_power()
    print(f"Measured power: {power:.6f} W ({power * 1000:.3f} mW)")

    # Create a data point
    data_point = rust_daq.DataPoint(
        timestamp=datetime.now(timezone.utc),
        channel="power_reading",
        value=power,
        unit="W",
        metadata={
            "instrument": "Newport1830C",
            "laser_wavelength_nm": 800.0,
            "status": "ok"
        }
    )

    print(f"\nCreated data point: {data_point}")
    print(f"  Timestamp: {data_point.timestamp}")
    print(f"  Channel: {data_point.channel}")
    print(f"  Value: {data_point.value} {data_point.unit}")
    print(f"  Metadata: {data_point.metadata}")


if __name__ == "__main__":
    main()
