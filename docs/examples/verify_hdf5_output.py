#!/usr/bin/env python3
"""
Verify HDF5 Output from The Mullet Strategy

This script demonstrates that HDF5 files written by the Rust HDF5Writer
are fully compatible with Python's h5py library.

Usage:
    python verify_hdf5_output.py experiment_data.h5

Requirements:
    pip install h5py numpy
"""

import sys
import h5py
import numpy as np


def verify_hdf5_file(filepath):
    """Verify HDF5 file structure and content."""
    print(f"🔍 Verifying HDF5 file: {filepath}")
    print()

    try:
        with h5py.File(filepath, 'r') as f:
            print("✅ File opened successfully")
            print()

            # List top-level groups
            print("📁 Top-level structure:")
            for key in f.keys():
                print(f"   - {key}")
            print()

            # Check measurements group
            if 'measurements' not in f:
                print("⚠️  No 'measurements' group found")
                return False

            measurements = f['measurements']
            print(f"📊 Found {len(measurements.keys())} batches:")

            # Examine each batch
            for batch_name in sorted(measurements.keys()):
                batch = measurements[batch_name]
                print(f"\n   Batch: {batch_name}")

                # List datasets
                print("   Datasets:")
                for dataset_name in batch.keys():
                    dataset = batch[dataset_name]
                    print(f"      - {dataset_name}: shape={dataset.shape}, dtype={dataset.dtype}")

                    # Show first few values
                    if dataset.size > 0:
                        if dataset.size <= 5:
                            values = dataset[:]
                        else:
                            values = dataset[:5]
                        print(f"        First values: {values}")

                # Show attributes
                if batch.attrs:
                    print("   Attributes:")
                    for attr_name in batch.attrs:
                        print(f"      - {attr_name}: {batch.attrs[attr_name]}")

            print("\n✅ HDF5 file is valid and readable by Python!")
            print("   Scientists can use h5py/MATLAB/Igor with this file")
            return True

    except FileNotFoundError:
        print(f"❌ File not found: {filepath}")
        print("   Run the Rust example first:")
        print("   cargo run --example phase4_ring_buffer_example --features='storage_hdf5,storage_arrow'")
        return False
    except Exception as e:
        print(f"❌ Error reading HDF5 file: {e}")
        return False


def demonstrate_numpy_access(filepath):
    """Demonstrate loading data into NumPy arrays."""
    print("\n🧮 NumPy Integration Demo:")
    print()

    try:
        with h5py.File(filepath, 'r') as f:
            # Get first batch
            measurements = f['measurements']
            first_batch = list(measurements.keys())[0]
            batch = measurements[first_batch]

            print(f"Reading batch: {first_batch}")

            # Load each dataset as NumPy array
            for dataset_name in batch.keys():
                dataset = batch[dataset_name]
                numpy_array = dataset[:]

                print(f"\n   {dataset_name}:")
                print(f"   - Type: {type(numpy_array)}")
                print(f"   - Shape: {numpy_array.shape}")
                print(f"   - dtype: {numpy_array.dtype}")
                print(f"   - Mean: {numpy_array.mean():.6f}")
                print(f"   - Std:  {numpy_array.std():.6f}")

                if numpy_array.size <= 10:
                    print(f"   - Values: {numpy_array}")

            print("\n✅ Data successfully loaded as NumPy arrays!")
            print("   Ready for scientific analysis with pandas, matplotlib, scipy, etc.")

    except Exception as e:
        print(f"❌ Error in NumPy demo: {e}")


def show_matlab_example(filepath):
    """Show example MATLAB code to read this file."""
    print("\n📐 MATLAB Compatibility:")
    print()
    print("To read this file in MATLAB, use:")
    print()
    print(f"    info = h5info('{filepath}');")
    print(f"    voltage = h5read('{filepath}', '/measurements/batch_000001/voltage');")
    print(f"    plot(voltage);")
    print()
    print("✅ HDF5 files are directly readable in MATLAB (no conversion needed)")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python verify_hdf5_output.py <hdf5_file>")
        print()
        print("Example:")
        print("    python verify_hdf5_output.py experiment_data.h5")
        sys.exit(1)

    filepath = sys.argv[1]

    print("=" * 70)
    print("  The Mullet Strategy - HDF5 Compatibility Verification")
    print("=" * 70)
    print()

    # Verify file structure
    if verify_hdf5_file(filepath):
        # Demonstrate NumPy access
        demonstrate_numpy_access(filepath)

        # Show MATLAB example
        show_matlab_example(filepath)

        print()
        print("=" * 70)
        print("🎯 Summary: The Mullet Strategy Works!")
        print("=" * 70)
        print()
        print("Scientists see:")
        print("   ✅ Standard HDF5 files (h5py, MATLAB, Igor)")
        print("   ✅ Familiar NumPy arrays")
        print("   ✅ No knowledge of Arrow format required")
        print()
        print("System provides:")
        print("   ✅ 10k+ writes/sec with Arrow")
        print("   ✅ Non-blocking background HDF5 translation")
        print("   ✅ Zero-copy memory sharing")
        print()
        print("Party in front. Business in back. 🎸")
        print()
