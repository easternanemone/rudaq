#!/usr/bin/env python3
"""
Continuous power monitoring with background acquisition and logging.

Monitors optical power continuously at a specified sampling rate, logging
data to file and displaying statistics. Runs until interrupted with Ctrl+C.
"""

import argparse
import logging
import signal
import sys
import time
from collections import deque
from datetime import datetime, timezone
from pathlib import Path

import rust_daq


class PowerMonitor:
    """Continuous power monitoring with statistics."""

    def __init__(self, meter, sample_rate_hz=1.0, window_size=100):
        self.meter = meter
        self.sample_interval = 1.0 / sample_rate_hz
        self.window = deque(maxlen=window_size)
        self.total_samples = 0
        self.running = True

    def handle_interrupt(self, signum, frame):
        """Handle Ctrl+C gracefully."""
        logging.info("\nInterrupt received, stopping acquisition...")
        self.running = False

    def acquire_sample(self):
        """Acquire single power measurement and return DataPoint."""
        power = self.meter.read_power()

        point = rust_daq.DataPoint(
            timestamp=datetime.now(timezone.utc),
            channel="continuous_power",
            value=power,
            unit="W",
            metadata={
                "sample_number": self.total_samples + 1,
                "monitor_mode": "continuous"
            }
        )

        self.window.append(power)
        self.total_samples += 1

        return point

    def get_statistics(self):
        """Calculate statistics from recent samples."""
        if not self.window:
            return {}

        values = list(self.window)
        return {
            'mean': sum(values) / len(values),
            'min': min(values),
            'max': max(values),
            'count': len(values),
            'total_samples': self.total_samples
        }

    def run(self, display_interval=10):
        """
        Run continuous monitoring loop.

        Args:
            display_interval: Print statistics every N samples
        """
        signal.signal(signal.SIGINT, self.handle_interrupt)

        logging.info(f"Starting continuous monitoring at {1/self.sample_interval:.1f} Hz")
        logging.info("Press Ctrl+C to stop")

        last_display = 0

        while self.running:
            try:
                # Acquire sample
                point = self.acquire_sample()

                # Log to file
                logging.debug(f"Sample {self.total_samples}: "
                             f"{point.value:.6e} {point.unit}")

                # Display statistics periodically
                if self.total_samples - last_display >= display_interval:
                    stats = self.get_statistics()
                    logging.info(
                        f"[{stats['total_samples']} samples] "
                        f"Mean: {stats['mean']:.6e} W, "
                        f"Min: {stats['min']:.6e} W, "
                        f"Max: {stats['max']:.6e} W "
                        f"(window: {stats['count']})"
                    )
                    last_display = self.total_samples

                # Wait for next sample
                time.sleep(self.sample_interval)

            except Exception as e:
                logging.error(f"Acquisition error: {e}")
                continue

        # Final statistics
        stats = self.get_statistics()
        logging.info("\n=== Final Statistics ===")
        logging.info(f"Total samples: {stats['total_samples']}")
        if stats['count'] > 0:
            logging.info(f"Mean power: {stats['mean']:.6e} W")
            logging.info(f"Min power: {stats['min']:.6e} W")
            logging.info(f"Max power: {stats['max']:.6e} W")


def main():
    parser = argparse.ArgumentParser(
        description='Continuous optical power monitoring',
        formatter_class=argparse.ArgumentDefaultsHelpFormatter
    )

    parser.add_argument('--rate', type=float, default=1.0,
                        help='Sampling rate (Hz)')
    parser.add_argument('--window', type=int, default=100,
                        help='Statistics window size (samples)')
    parser.add_argument('--display-interval', type=int, default=10,
                        help='Display statistics every N samples')

    parser.add_argument('--meter-resource',
                        default='USB0::0x104D::0xC0DE::SN12345::INSTR',
                        help='VISA resource string for power meter')

    parser.add_argument('--log-file', type=Path,
                        default='power_monitor.log',
                        help='Log file for data')
    parser.add_argument('--verbose', action='store_true',
                        help='Enable verbose logging')

    args = parser.parse_args()

    # Setup logging
    level = logging.DEBUG if args.verbose else logging.INFO
    logging.basicConfig(
        level=level,
        format='%(asctime)s - %(levelname)s - %(message)s',
        handlers=[
            logging.StreamHandler(sys.stdout),
            logging.FileHandler(args.log_file)
        ]
    )

    try:
        # Initialize power meter
        logging.info(f"Initializing Newport 1830C: {args.meter_resource}")
        meter = rust_daq.Newport1830C(resource_string=args.meter_resource)

        # Create monitor and run
        monitor = PowerMonitor(meter, args.rate, args.window)
        monitor.run(args.display_interval)

        return 0

    except Exception as e:
        logging.error(f"Monitor failed: {e}", exc_info=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
