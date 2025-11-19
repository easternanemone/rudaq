"""
rust-daq Python Client

Remote control client for rust-daq headless daemon using gRPC.
"""

import grpc
from generated import daq_pb2, daq_pb2_grpc
import time
from typing import Dict, Generator, Optional


class DaqClient:
    """Client for controlling rust-daq daemon remotely."""

    def __init__(self, host: str = 'localhost', port: int = 50051):
        """
        Initialize the DAQ client.

        Args:
            host: Hostname or IP address of the rust-daq daemon
            port: gRPC port number
        """
        self.address = f'{host}:{port}'
        self.channel = grpc.insecure_channel(self.address)
        self.stub = daq_pb2_grpc.ControlServiceStub(self.channel)

    def upload_script(self, script_content: str, name: str = "experiment") -> str:
        """
        Upload a Rhai script to the daemon.

        Args:
            script_content: The Rhai script source code
            name: Optional name for the script

        Returns:
            The script_id assigned by the daemon

        Raises:
            RuntimeError: If upload fails
        """
        request = daq_pb2.UploadRequest(
            script_content=script_content,
            name=name
        )

        try:
            response = self.stub.UploadScript(request)
        except grpc.RpcError as e:
            raise RuntimeError(f"gRPC error during upload: {e.details()}") from e

        if not response.success:
            raise RuntimeError(f"Upload failed: {response.error_message}")

        return response.script_id

    def start_script(self, script_id: str) -> str:
        """
        Start executing an uploaded script.

        Args:
            script_id: The ID of the script to execute

        Returns:
            The execution_id for tracking progress

        Raises:
            RuntimeError: If start fails
        """
        request = daq_pb2.StartRequest(script_id=script_id)

        try:
            response = self.stub.StartScript(request)
        except grpc.RpcError as e:
            raise RuntimeError(f"gRPC error during start: {e.details()}") from e

        if not response.started:
            raise RuntimeError(f"Start failed (started={response.started})")

        return response.execution_id

    def get_status(self, execution_id: str) -> Dict[str, any]:
        """
        Get current status of script execution.

        Args:
            execution_id: The execution ID to query

        Returns:
            Dictionary with status information:
            - execution_id: The execution ID
            - state: Current execution state (str: IDLE, RUNNING, COMPLETED, ERROR)
            - error: Error message if any
            - start_time: Start timestamp in nanoseconds
            - end_time: End timestamp in nanoseconds
        """
        request = daq_pb2.StatusRequest(execution_id=execution_id)

        try:
            response = self.stub.GetScriptStatus(request)
        except grpc.RpcError as e:
            raise RuntimeError(f"gRPC error getting status: {e.details()}") from e

        return {
            'execution_id': response.execution_id,
            'state': response.state,
            'error': response.error_message,
            'start_time': response.start_time_ns,
            'end_time': response.end_time_ns
        }

    def stop_script(self, execution_id: str) -> bool:
        """
        Stop a running script execution.

        Args:
            execution_id: The execution ID to stop

        Returns:
            True if stopped successfully

        Raises:
            RuntimeError: If stop fails
        """
        request = daq_pb2.StopRequest(execution_id=execution_id)

        try:
            response = self.stub.StopScript(request)
        except grpc.RpcError as e:
            raise RuntimeError(f"gRPC error stopping script: {e.details()}") from e

        return response.stopped

    def stream_status(self) -> Generator[Dict[str, any], None, None]:
        """
        Stream real-time system status updates.

        Yields:
            Dictionary with status information for each update:
            - state: Current system state (str: IDLE, RUNNING, ERROR)
            - memory_mb: Current memory usage in MB
            - live_values: Dictionary of live instrument values
            - timestamp: Timestamp in nanoseconds
        """
        request = daq_pb2.StatusRequest()

        try:
            for status in self.stub.StreamStatus(request):
                yield {
                    'state': status.current_state,
                    'memory_mb': status.current_memory_usage_mb,
                    'live_values': dict(status.live_values),
                    'timestamp': status.timestamp_ns
                }
        except grpc.RpcError as e:
            print(f"Stream error: {e.details()}")
            return

    def stream_measurements(self, instrument: str) -> Generator[Dict[str, any], None, None]:
        """
        Stream real-time measurement data from a specific instrument.

        Args:
            instrument: Name of the instrument to stream from

        Yields:
            Dictionary with measurement data:
            - instrument: Instrument name
            - value: Scalar value (if scalar data) or None
            - image: Byte data (if image data) or None
            - timestamp: Timestamp in nanoseconds
        """
        request = daq_pb2.MeasurementRequest(instrument=instrument)

        try:
            for data_point in self.stub.StreamMeasurements(request):
                result = {
                    'instrument': data_point.instrument,
                    'timestamp': data_point.timestamp_ns
                }

                # Handle oneof value field
                if data_point.HasField('scalar'):
                    result['value'] = data_point.scalar
                    result['image'] = None
                elif data_point.HasField('image'):
                    result['value'] = None
                    result['image'] = data_point.image
                else:
                    result['value'] = None
                    result['image'] = None

                yield result
        except grpc.RpcError as e:
            print(f"Measurement stream error: {e.details()}")
            return

    def close(self):
        """Close the gRPC channel."""
        self.channel.close()


# Example usage
if __name__ == "__main__":
    print("🚀 rust-daq Python Client Demo")
    print("=" * 60)

    # Create client instance
    client = DaqClient()
    print(f"✅ Connected to daemon at {client.address}")

    # Example Rhai script
    script = """
    print("Hello from remote Rhai!");
    stage.move_abs(5.0);
    sleep(0.5);
    camera.trigger();
    print("Movement complete");
    """

    try:
        # Upload the script
        print("\n📤 Uploading script...")
        print(f"Script content:\n{script}")
        script_id = client.upload_script(script, name="remote_test")
        print(f"✅ Script uploaded: {script_id}")

        # Start execution
        print("\n▶️  Starting execution...")
        exec_id = client.start_script(script_id)
        print(f"✅ Execution started: {exec_id}")

        # Monitor status
        print("\n📊 Monitoring execution...")
        time.sleep(0.5)  # Wait for execution to progress

        status = client.get_status(exec_id)
        print(f"   Execution ID: {status['execution_id']}")
        print(f"   State: {status['state']}")
        print(f"   Start time: {status['start_time']}")
        if status['error']:
            print(f"   Error: {status['error']}")

        # Stream status updates (limited to 5 for demo)
        print("\n📡 Streaming status updates (showing first 5):")
        try:
            count = 0
            for update in client.stream_status():
                print(f"   [{count}] State: {update['state']}, "
                      f"Memory: {update['memory_mb']:.1f} MB, "
                      f"Values: {len(update['live_values'])} live")
                count += 1
                if count >= 5:
                    break
        except KeyboardInterrupt:
            print("\n⚠️  Stream interrupted by user")

        print("\n✅ Demo complete!")

    except RuntimeError as e:
        print(f"\n❌ Error: {e}")
    except Exception as e:
        print(f"\n❌ Unexpected error: {e}")
    finally:
        client.close()
        print("\n👋 Disconnected from daemon")
