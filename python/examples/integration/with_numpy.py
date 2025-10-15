#!/usr/bin/env python3
"""
Integration example with NumPy for FFT processing.

Demonstrates how to use rust_daq data structures with NumPy arrays for
signal processing tasks like FFT analysis.
"""

import numpy as np
import rust_daq
from datetime import datetime, timezone, timedelta


def generate_mock_timeseries(duration_sec=1.0, sample_rate_hz=1000.0,
                               frequencies=[50.0, 120.0]):
    """
    Generate mock time-series data with multiple frequency components.

    Args:
        duration_sec: Duration of signal in seconds
        sample_rate_hz: Sampling rate in Hz
        frequencies: List of frequency components to include (Hz)

    Returns:
        List of DataPoint objects
    """
    n_samples = int(duration_sec * sample_rate_hz)
    t = np.linspace(0, duration_sec, n_samples)
    dt = 1.0 / sample_rate_hz

    # Generate signal with multiple frequencies
    signal = np.zeros_like(t)
    for freq in frequencies:
        signal += np.sin(2 * np.pi * freq * t)

    # Add noise
    signal += 0.1 * np.random.randn(len(t))

    # Convert to DataPoint objects
    start_time = datetime.now(timezone.utc)
    data_points = []

    for i, value in enumerate(signal):
        point = rust_daq.DataPoint(
            timestamp=start_time + timedelta(seconds=i * dt),
            channel="signal",
            value=float(value),
            unit="V",
            metadata={
                "sample_index": i,
                "sample_rate_hz": sample_rate_hz
            }
        )
        data_points.append(point)

    return data_points


def extract_numpy_array(data_points):
    """Extract values and timestamps from DataPoints into NumPy arrays."""
    values = np.array([p.value for p in data_points])
    timestamps = np.array([p.timestamp.timestamp() for p in data_points])
    return timestamps, values


def compute_fft(values, sample_rate_hz):
    """
    Compute FFT and return frequency bins and magnitudes.

    Args:
        values: NumPy array of signal values
        sample_rate_hz: Sampling rate in Hz

    Returns:
        (frequencies, magnitudes) tuple of NumPy arrays
    """
    n = len(values)
    fft = np.fft.rfft(values)
    magnitudes = np.abs(fft) * 2 / n
    frequencies = np.fft.rfftfreq(n, 1 / sample_rate_hz)

    return frequencies, magnitudes


def find_peaks(frequencies, magnitudes, threshold=0.5):
    """Find frequency peaks above threshold."""
    peaks = []
    for i in range(1, len(magnitudes) - 1):
        if magnitudes[i] > threshold:
            if magnitudes[i] > magnitudes[i-1] and magnitudes[i] > magnitudes[i+1]:
                peaks.append((frequencies[i], magnitudes[i]))
    return peaks


def main():
    print("=== rust_daq + NumPy FFT Example ===\n")

    # Generate mock data
    print("Generating mock time-series data...")
    target_frequencies = [50.0, 120.0, 200.0]
    data_points = generate_mock_timeseries(
        duration_sec=1.0,
        sample_rate_hz=1000.0,
        frequencies=target_frequencies
    )
    print(f"Generated {len(data_points)} data points")
    print(f"Target frequencies: {target_frequencies} Hz\n")

    # Extract to NumPy arrays
    print("Extracting to NumPy arrays...")
    timestamps, values = extract_numpy_array(data_points)
    print(f"Array shapes: timestamps={timestamps.shape}, values={values.shape}")
    print(f"Value range: [{values.min():.3f}, {values.max():.3f}] V\n")

    # Compute FFT
    print("Computing FFT...")
    sample_rate = 1000.0  # Hz
    frequencies, magnitudes = compute_fft(values, sample_rate)
    print(f"Frequency resolution: {frequencies[1] - frequencies[0]:.2f} Hz\n")

    # Find peaks
    print("Detecting frequency peaks (threshold=0.5)...")
    peaks = find_peaks(frequencies, magnitudes, threshold=0.5)
    print(f"Found {len(peaks)} peaks:")
    for freq, mag in sorted(peaks, key=lambda x: x[1], reverse=True):
        print(f"  {freq:.1f} Hz: magnitude {mag:.3f}")

    # Verify detection
    print("\nVerification:")
    detected_freqs = set(round(f) for f, _ in peaks)
    target_freqs = set(target_frequencies)
    if detected_freqs == target_freqs:
        print("✓ All target frequencies detected correctly")
    else:
        print(f"✗ Mismatch: detected={detected_freqs}, expected={target_freqs}")

    # Statistics
    print(f"\nStatistics:")
    print(f"  Mean: {np.mean(values):.6f} V")
    print(f"  Std Dev: {np.std(values):.6f} V")
    print(f"  RMS: {np.sqrt(np.mean(values**2)):.6f} V")


if __name__ == "__main__":
    main()
