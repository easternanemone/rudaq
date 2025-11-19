#!/usr/bin/env python3
"""
Measurement streaming example for rust-daq Python client.

This demonstrates real-time monitoring of instrument data.
"""

import sys
import os

# Add parent directory to path for imports
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from daq_client import DaqClient


def main():
    print("=" * 60)
    print("rust-daq Measurement Streaming Example")
    print("=" * 60)

    # Get instrument name from command line
    if len(sys.argv) < 2:
        print("\nUsage: python stream_measurements.py <instrument_name>")
        print("Example: python stream_measurements.py stage")
        sys.exit(1)

    instrument = sys.argv[1]
    print(f"\nStreaming measurements from: {instrument}")
    print("Press Ctrl+C to stop streaming")

    # Create client
    client = DaqClient(host='localhost', port=50051)
    print(f"\n✅ Connected to daemon at localhost:50051")

    try:
        print(f"\n📡 Streaming measurements from {instrument}:\n")

        for i, data in enumerate(client.stream_measurements(instrument)):
            # Display measurement
            if data['value'] is not None:
                # Scalar value
                print(f"[{i}] {data['instrument']:10} | "
                      f"Value: {data['value']:10.4f} | "
                      f"Time: {data['timestamp']}")
            elif data['image'] is not None:
                # Image data
                print(f"[{i}] {data['instrument']:10} | "
                      f"Image: {len(data['image'])} bytes | "
                      f"Time: {data['timestamp']}")
            else:
                print(f"[{i}] {data['instrument']:10} | No data")

    except KeyboardInterrupt:
        print("\n\n⚠️  Stream stopped by user")
    except RuntimeError as e:
        print(f"\n\n❌ Error: {e}")
    finally:
        client.close()
        print("\n👋 Disconnected")


if __name__ == "__main__":
    main()
