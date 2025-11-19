#!/usr/bin/env python3
"""
Basic usage example for rust-daq Python client.

This demonstrates uploading a script, starting execution, and monitoring status.
"""

import sys
import os
import time

# Add parent directory to path for imports
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from daq_client import DaqClient


def main():
    print("=" * 60)
    print("Basic rust-daq Client Usage Example")
    print("=" * 60)

    # Create client
    client = DaqClient(host='localhost', port=50051)
    print(f"\n✅ Connected to daemon at localhost:50051")

    # Define experiment script
    script = """
    // Simple movement and capture experiment
    print("Starting experiment...");

    // Move stage to position
    stage.move_abs(5.0);
    sleep(0.5);

    // Trigger camera
    camera.trigger();

    // Move back
    stage.move_abs(0.0);

    print("Experiment complete!");
    """

    try:
        # Upload the script
        print("\n📤 Uploading script...")
        script_id = client.upload_script(script, name="basic_experiment")
        print(f"   Script ID: {script_id}")

        # Start execution
        print("\n▶️  Starting execution...")
        exec_id = client.start_script(script_id)
        print(f"   Execution ID: {exec_id}")

        # Poll status until complete
        print("\n📊 Monitoring execution:")
        max_wait = 10  # seconds
        start_time = time.time()

        while time.time() - start_time < max_wait:
            status = client.get_status(exec_id)

            print(f"   [{time.time() - start_time:.1f}s] State: {status['state']}")

            if status['state'] in ['COMPLETED', 'ERROR']:
                if status['error']:
                    print(f"   ❌ Error: {status['error']}")
                else:
                    print(f"   ✅ Completed successfully!")
                break

            time.sleep(0.5)
        else:
            print("   ⚠️  Timeout waiting for completion")

        print("\n✅ Example complete!")

    except RuntimeError as e:
        print(f"\n❌ Error: {e}")
    except KeyboardInterrupt:
        print("\n⚠️  Interrupted by user")
    finally:
        client.close()
        print("\n👋 Disconnected")


if __name__ == "__main__":
    main()
