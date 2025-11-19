#!/usr/bin/env python3
"""
Streaming status example for rust-daq Python client.

This demonstrates real-time monitoring of system status and live values.
"""

import sys
import os

# Add parent directory to path for imports
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from daq_client import DaqClient


def main():
    print("=" * 60)
    print("rust-daq Status Streaming Example")
    print("=" * 60)
    print("\nPress Ctrl+C to stop streaming")

    # Create client
    client = DaqClient(host='localhost', port=50051)
    print(f"\n✅ Connected to daemon at localhost:50051")

    try:
        print("\n📡 Streaming system status:\n")

        for i, update in enumerate(client.stream_status()):
            # Display status update
            print(f"[{i}] {update['state']:10} | "
                  f"Memory: {update['memory_mb']:6.1f} MB | "
                  f"Live values: {len(update['live_values'])}")

            # Show live values if any
            if update['live_values']:
                for name, value in update['live_values'].items():
                    print(f"    {name}: {value:.3f}")

    except KeyboardInterrupt:
        print("\n\n⚠️  Stream stopped by user")
    except RuntimeError as e:
        print(f"\n\n❌ Error: {e}")
    finally:
        client.close()
        print("\n👋 Disconnected")


if __name__ == "__main__":
    main()
