#!/usr/bin/env python3
"""
Integration example with pandas for data analysis and export.

Demonstrates converting rust_daq DataPoints to pandas DataFrames for
time-series analysis, statistical operations, and data export.
"""

import pandas as pd
import rust_daq
from datetime import datetime, timezone, timedelta
import json


def datapoints_to_dataframe(data_points):
    """
    Convert list of DataPoints to pandas DataFrame.

    Args:
        data_points: List of rust_daq.DataPoint objects

    Returns:
        pandas DataFrame with columns: timestamp, channel, value, unit, metadata
    """
    records = []
    for point in data_points:
        records.append({
            'timestamp': point.timestamp,
            'channel': point.channel,
            'value': point.value,
            'unit': point.unit,
            'metadata': point.metadata
        })

    df = pd.DataFrame(records)
    df['timestamp'] = pd.to_datetime(df['timestamp'])
    df = df.set_index('timestamp')

    return df


def generate_mock_multipoint_scan():
    """Generate mock data from a multi-point measurement scan."""
    data_points = []
    start_time = datetime.now(timezone.utc)

    wavelengths = range(700, 901, 10)  # 700-900 nm, 10 nm steps

    for i, wavelength in enumerate(wavelengths):
        # Simulate Gaussian response centered at 800 nm
        power = 1e-3 * pd.np.exp(-((wavelength - 800) ** 2) / (2 * 50 ** 2))
        power += 1e-5 * pd.np.random.randn()  # Add noise

        point = rust_daq.DataPoint(
            timestamp=start_time + timedelta(seconds=i * 0.5),
            channel="optical_power",
            value=power,
            unit="W",
            metadata={
                "wavelength_nm": wavelength,
                "scan_point": i + 1,
                "measurement_type": "wavelength_scan"
            }
        )
        data_points.append(point)

    return data_points


def main():
    print("=== rust_daq + pandas Integration Example ===\n")

    # Generate mock data
    print("Generating mock wavelength scan data...")
    data_points = generate_mock_multipoint_scan()
    print(f"Generated {len(data_points)} data points\n")

    # Convert to DataFrame
    print("Converting to pandas DataFrame...")
    df = datapoints_to_dataframe(data_points)
    print(f"DataFrame shape: {df.shape}")
    print(f"Columns: {df.columns.tolist()}\n")

    # Display first few rows
    print("First 5 rows:")
    print(df.head())
    print()

    # Extract wavelength from metadata
    print("Extracting wavelength from metadata...")
    df['wavelength_nm'] = df['metadata'].apply(
        lambda x: x.get('wavelength_nm') if x else None
    )
    print(df[['wavelength_nm', 'value', 'unit']].head())
    print()

    # Statistical analysis
    print("Statistical Analysis:")
    print(f"  Count: {df['value'].count()}")
    print(f"  Mean: {df['value'].mean():.6e} W")
    print(f"  Std Dev: {df['value'].std():.6e} W")
    print(f"  Min: {df['value'].min():.6e} W")
    print(f"  Max: {df['value'].max():.6e} W")
    print()

    # Find peak
    peak_idx = df['value'].idxmax()
    peak_wavelength = df.loc[peak_idx, 'wavelength_nm']
    peak_power = df.loc[peak_idx, 'value']
    print(f"Peak Detection:")
    print(f"  Wavelength: {peak_wavelength:.0f} nm")
    print(f"  Power: {peak_power:.6e} W")
    print(f"  Timestamp: {peak_idx}")
    print()

    # Resampling (if needed for different time intervals)
    print("Time-based resampling (1 second bins):")
    resampled = df[['value']].resample('1S').mean()
    print(resampled.head())
    print()

    # Export to various formats
    print("Exporting data...")

    # CSV export
    csv_file = 'wavelength_scan.csv'
    df[['wavelength_nm', 'value', 'unit']].to_csv(csv_file)
    print(f"  ✓ Saved to {csv_file}")

    # JSON export
    json_file = 'wavelength_scan.json'
    df.reset_index().to_json(json_file, orient='records', date_format='iso')
    print(f"  ✓ Saved to {json_file}")

    # Excel export (requires openpyxl)
    try:
        excel_file = 'wavelength_scan.xlsx'
        df[['wavelength_nm', 'value', 'unit']].to_excel(excel_file)
        print(f"  ✓ Saved to {excel_file}")
    except ImportError:
        print(f"  ⊘ Excel export skipped (install openpyxl)")

    # Parquet export (efficient binary format)
    try:
        parquet_file = 'wavelength_scan.parquet'
        df[['wavelength_nm', 'value', 'unit']].to_parquet(parquet_file)
        print(f"  ✓ Saved to {parquet_file}")
    except ImportError:
        print(f"  ⊘ Parquet export skipped (install pyarrow)")

    print("\n=== Analysis Complete ===")


if __name__ == "__main__":
    main()
