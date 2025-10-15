#!/usr/bin/env python3
"""
Production-ready wavelength sweep script with CLI arguments.

Performs an automated wavelength scan while measuring optical power at each
point. Supports both forward and bidirectional scans with configurable step
size and dwell time.
"""

import argparse
import logging
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

import rust_daq


def setup_logging(log_file=None, verbose=False):
    """Configure logging to console and optionally to file."""
    level = logging.DEBUG if verbose else logging.INFO
    handlers = [logging.StreamHandler(sys.stdout)]

    if log_file:
        handlers.append(logging.FileHandler(log_file))

    logging.basicConfig(
        level=level,
        format='%(asctime)s - %(levelname)s - %(message)s',
        handlers=handlers
    )


def wavelength_sweep(laser, meter, start_nm, stop_nm, step_nm, dwell_sec,
                     bidirectional=False):
    """
    Perform wavelength sweep and collect power measurements.

    Args:
        laser: MaiTai instance
        meter: Newport1830C instance
        start_nm: Starting wavelength in nanometers
        stop_nm: Ending wavelength in nanometers
        step_nm: Step size in nanometers
        dwell_sec: Dwell time at each point in seconds
        bidirectional: If True, scan forward then backward

    Returns:
        List of DataPoint objects
    """
    data_points = []

    # Generate wavelength points
    wavelengths = []
    current = start_nm
    while current <= stop_nm:
        wavelengths.append(current)
        current += step_nm

    if bidirectional:
        wavelengths.extend(reversed(wavelengths[:-1]))

    logging.info(f"Starting sweep: {start_nm} to {stop_nm} nm, "
                 f"step {step_nm} nm, {len(wavelengths)} points")

    for i, wavelength in enumerate(wavelengths, 1):
        logging.info(f"Point {i}/{len(wavelengths)}: λ = {wavelength:.1f} nm")

        # Set wavelength
        laser.set_wavelength(wavelength)

        # Wait for stabilization
        time.sleep(dwell_sec)

        # Measure power
        power = meter.read_power()

        # Create data point
        point = rust_daq.DataPoint(
            timestamp=datetime.now(timezone.utc),
            channel="optical_power",
            value=power,
            unit="W",
            metadata={
                "wavelength_nm": wavelength,
                "scan_point": i,
                "total_points": len(wavelengths),
                "dwell_time_sec": dwell_sec,
                "bidirectional": bidirectional
            }
        )

        data_points.append(point)
        logging.debug(f"  Power: {power:.6e} W")

    logging.info(f"Sweep complete: {len(data_points)} points collected")
    return data_points


def save_data(data_points, output_file):
    """Save data points to CSV file."""
    import csv

    with open(output_file, 'w', newline='') as f:
        writer = csv.writer(f)
        writer.writerow(['timestamp', 'wavelength_nm', 'power_W', 'channel', 'unit'])

        for point in data_points:
            writer.writerow([
                point.timestamp.isoformat(),
                point.metadata.get('wavelength_nm', ''),
                point.value,
                point.channel,
                point.unit
            ])

    logging.info(f"Data saved to {output_file}")


def main():
    parser = argparse.ArgumentParser(
        description='Perform automated wavelength sweep with power measurement',
        formatter_class=argparse.ArgumentDefaultsHelpFormatter
    )

    parser.add_argument('--start', type=float, default=700.0,
                        help='Starting wavelength (nm)')
    parser.add_argument('--stop', type=float, default=900.0,
                        help='Ending wavelength (nm)')
    parser.add_argument('--step', type=float, default=10.0,
                        help='Step size (nm)')
    parser.add_argument('--dwell', type=float, default=0.5,
                        help='Dwell time at each point (seconds)')
    parser.add_argument('--bidirectional', action='store_true',
                        help='Scan forward then backward')

    parser.add_argument('--laser-port', default='COM3',
                        help='Serial port for MaiTai laser')
    parser.add_argument('--meter-resource',
                        default='USB0::0x104D::0xC0DE::SN12345::INSTR',
                        help='VISA resource string for power meter')

    parser.add_argument('--output', type=Path, default='wavelength_sweep.csv',
                        help='Output CSV file')
    parser.add_argument('--log-file', type=Path,
                        help='Log file (defaults to stdout only)')
    parser.add_argument('--verbose', action='store_true',
                        help='Enable verbose logging')

    args = parser.parse_args()

    # Setup logging
    setup_logging(args.log_file, args.verbose)

    # Validate parameters
    if args.start >= args.stop:
        logging.error("Start wavelength must be less than stop wavelength")
        return 1

    if args.step <= 0:
        logging.error("Step size must be positive")
        return 1

    try:
        # Initialize instruments
        logging.info(f"Initializing MaiTai laser on {args.laser_port}")
        laser = rust_daq.MaiTai(port=args.laser_port)

        logging.info(f"Initializing Newport 1830C: {args.meter_resource}")
        meter = rust_daq.Newport1830C(resource_string=args.meter_resource)

        # Perform sweep
        data_points = wavelength_sweep(
            laser, meter,
            args.start, args.stop, args.step, args.dwell,
            args.bidirectional
        )

        # Save results
        save_data(data_points, args.output)

        logging.info("Scan completed successfully")
        return 0

    except KeyboardInterrupt:
        logging.warning("Scan interrupted by user")
        return 130

    except Exception as e:
        logging.error(f"Scan failed: {e}", exc_info=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
