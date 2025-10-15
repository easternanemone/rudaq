#!/usr/bin/env python3
"""
Advanced example: Concurrent instrument control with asyncio.

Demonstrates how to control multiple instruments concurrently using Python's
asyncio for efficient parallel data acquisition.
"""

import asyncio
import rust_daq
from datetime import datetime, timezone
from typing import List


async def control_laser_async(laser, wavelengths: List[float], delay: float = 0.5):
    """
    Asynchronously step through wavelengths.

    Args:
        laser: MaiTai instance
        wavelengths: List of wavelengths to set (nm)
        delay: Delay between steps (seconds)
    """
    print(f"[Laser] Starting wavelength sequence...")
    for i, wl in enumerate(wavelengths, 1):
        print(f"[Laser] Setting wavelength {i}/{len(wavelengths)}: {wl} nm")
        laser.set_wavelength(wl)
        await asyncio.sleep(delay)

    print("[Laser] Sequence complete")


async def acquire_power_async(meter, n_samples: int, sample_rate: float = 2.0):
    """
    Asynchronously acquire power measurements.

    Args:
        meter: Newport1830C instance
        n_samples: Number of samples to acquire
        sample_rate: Sampling rate (Hz)

    Returns:
        List of DataPoint objects
    """
    print(f"[Power Meter] Starting acquisition: {n_samples} samples at {sample_rate} Hz")

    data_points = []
    interval = 1.0 / sample_rate

    for i in range(n_samples):
        power = meter.read_power()

        point = rust_daq.DataPoint(
            timestamp=datetime.now(timezone.utc),
            channel="optical_power",
            value=power,
            unit="W",
            metadata={
                "sample_index": i + 1,
                "sample_rate_hz": sample_rate
            }
        )

        data_points.append(point)
        print(f"[Power Meter] Sample {i+1}/{n_samples}: {power:.6e} W")

        await asyncio.sleep(interval)

    print("[Power Meter] Acquisition complete")
    return data_points


async def monitor_status(duration: float, check_interval: float = 1.0):
    """
    Monitor system status during acquisition.

    Args:
        duration: Duration to monitor (seconds)
        check_interval: Status check interval (seconds)
    """
    print(f"[Monitor] Starting status monitoring for {duration}s...")

    start_time = asyncio.get_event_loop().time()
    while asyncio.get_event_loop().time() - start_time < duration:
        elapsed = asyncio.get_event_loop().time() - start_time
        print(f"[Monitor] System running... ({elapsed:.1f}s elapsed)")
        await asyncio.sleep(check_interval)

    print("[Monitor] Monitoring complete")


async def coordinated_acquisition():
    """
    Coordinate multiple instruments concurrently.

    This function demonstrates running laser control, power acquisition,
    and status monitoring in parallel using asyncio tasks.
    """
    print("=== Async Coordinated Acquisition ===\n")

    # Initialize instruments
    print("Initializing instruments...")
    laser = rust_daq.MaiTai(port="COM3")
    meter = rust_daq.Newport1830C(
        resource_string="USB0::0x104D::0xC0DE::SN12345::INSTR"
    )
    print()

    # Define wavelength sequence
    wavelengths = [750.0, 800.0, 850.0, 900.0]

    # Create concurrent tasks
    print("Launching concurrent tasks...\n")

    tasks = [
        asyncio.create_task(control_laser_async(laser, wavelengths, delay=1.0)),
        asyncio.create_task(acquire_power_async(meter, n_samples=10, sample_rate=2.0)),
        asyncio.create_task(monitor_status(duration=5.0, check_interval=1.0))
    ]

    # Wait for all tasks to complete
    results = await asyncio.gather(*tasks, return_exceptions=True)

    # Check for errors
    for i, result in enumerate(results):
        if isinstance(result, Exception):
            print(f"\nTask {i} failed with error: {result}")

    # Extract data from power acquisition task (task index 1)
    if not isinstance(results[1], Exception):
        data_points = results[1]
        print(f"\n=== Results ===")
        print(f"Collected {len(data_points)} data points")
        print(f"Mean power: {sum(p.value for p in data_points) / len(data_points):.6e} W")

    print("\n=== Acquisition Complete ===")


async def sequential_vs_concurrent_demo():
    """
    Demonstrate the performance difference between sequential and concurrent execution.
    """
    print("\n=== Sequential vs Concurrent Comparison ===\n")

    # Sequential execution
    print("--- Sequential Execution ---")
    start = asyncio.get_event_loop().time()

    await asyncio.sleep(1.0)  # Simulate task 1
    await asyncio.sleep(1.0)  # Simulate task 2
    await asyncio.sleep(1.0)  # Simulate task 3

    sequential_time = asyncio.get_event_loop().time() - start
    print(f"Sequential time: {sequential_time:.2f}s\n")

    # Concurrent execution
    print("--- Concurrent Execution ---")
    start = asyncio.get_event_loop().time()

    await asyncio.gather(
        asyncio.sleep(1.0),  # Task 1
        asyncio.sleep(1.0),  # Task 2
        asyncio.sleep(1.0),  # Task 3
    )

    concurrent_time = asyncio.get_event_loop().time() - start
    print(f"Concurrent time: {concurrent_time:.2f}s\n")

    print(f"Speedup: {sequential_time / concurrent_time:.2f}x")


def main():
    """Main entry point."""
    # Run coordinated acquisition
    asyncio.run(coordinated_acquisition())

    # Run comparison demo
    asyncio.run(sequential_vs_concurrent_demo())


if __name__ == "__main__":
    main()
