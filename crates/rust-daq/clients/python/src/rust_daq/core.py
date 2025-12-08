"""
Core AsyncClient implementation for rust-daq.

Provides async-first gRPC client with:
- Async context manager for connection lifecycle
- Timeout support for all operations
- Automatic error translation to Python exceptions
- Type hints for better IDE support
"""

from typing import Optional, Dict, List, AsyncIterator, Any
import grpc
from grpc.aio import Channel, insecure_channel

from .exceptions import translate_grpc_error, DaqError, DeviceError


class AsyncClient:
    """
    Async gRPC client for rust-daq daemon.

    This is Layer 1 of the Python client - a robust async wrapper around
    the auto-generated gRPC stubs. Provides:
    - Connection management with async context manager
    - Timeout handling for all operations
    - Error translation from gRPC to Python exceptions
    - Type hints and documentation

    Example:
        async with AsyncClient("localhost:50051", timeout=5.0) as client:
            devices = await client.list_devices()
            info = await client.get_device_info("mock_stage")
            await client.move_absolute("mock_stage", 10.0)

    Note: Generated protobuf code is imported lazily to avoid import errors
    before running setup.py.
    """

    def __init__(
        self,
        address: str = "localhost:50051",
        timeout: float = 10.0,
        max_message_length: int = 100 * 1024 * 1024,  # 100MB for camera frames
    ):
        """
        Initialize AsyncClient.

        Args:
            address: Daemon address in "host:port" format
            timeout: Default timeout for operations in seconds
            max_message_length: Maximum gRPC message size in bytes
        """
        self.address = address
        self.timeout = timeout
        self._channel: Optional[Channel] = None
        self._hardware_stub = None
        self._control_stub = None
        self._max_message_length = max_message_length

    async def __aenter__(self):
        """Async context manager entry - connects to daemon."""
        await self.connect()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        """Async context manager exit - closes connection."""
        await self.close()
        return False

    async def connect(self) -> None:
        """
        Connect to the rust-daq daemon.

        Raises:
            CommunicationError: If connection fails
        """
        try:
            # Import generated code here to avoid errors before setup.py runs
            from .generated import daq_pb2_grpc

            # Create async gRPC channel with options
            options = [
                ("grpc.max_receive_message_length", self._max_message_length),
                ("grpc.max_send_message_length", self._max_message_length),
            ]

            self._channel = insecure_channel(self.address, options=options)
            self._hardware_stub = daq_pb2_grpc.HardwareServiceStub(self._channel)
            self._control_stub = daq_pb2_grpc.ControlServiceStub(self._channel)

            # Test connection by getting daemon info
            await self.get_daemon_info()

        except grpc.RpcError as e:
            raise translate_grpc_error(e, "Failed to connect to daemon")

    async def close(self) -> None:
        """Close the gRPC channel and clean up resources."""
        if self._channel:
            await self._channel.close()
            self._channel = None
            self._hardware_stub = None
            self._control_stub = None

    def _ensure_connected(self) -> None:
        """Raise error if not connected."""
        if not self._channel or not self._hardware_stub:
            raise DaqError("Not connected - use 'async with AsyncClient()' pattern")

    # =========================================================================
    # Control Service Methods
    # =========================================================================

    async def get_daemon_info(self) -> Dict[str, Any]:
        """
        Get daemon version and capabilities.

        Returns:
            Dictionary with:
                - version: Daemon version string
                - features: List of enabled features
                - available_hardware: List of available hardware types
                - uptime_seconds: Daemon uptime in seconds

        Raises:
            CommunicationError: If daemon is unreachable
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.DaemonInfoRequest()
            response = await self._control_stub.GetDaemonInfo(
                request, timeout=self.timeout
            )

            return {
                "version": response.version,
                "features": list(response.features),
                "available_hardware": list(response.available_hardware),
                "uptime_seconds": response.uptime_seconds,
            }
        except grpc.RpcError as e:
            raise translate_grpc_error(e, "Failed to get daemon info")

    # =========================================================================
    # Hardware Service Methods - Device Discovery
    # =========================================================================

    async def list_devices(
        self, capability_filter: Optional[str] = None
    ) -> List[Dict[str, Any]]:
        """
        List all available devices.

        Args:
            capability_filter: Optional filter by capability
                             ("movable", "readable", "triggerable", etc.)

        Returns:
            List of device info dictionaries, each containing:
                - id: Device ID
                - name: Device name
                - driver_type: Driver type (e.g., "mock_stage", "ell14")
                - capabilities: Dict of capability flags
                - metadata: Device-specific metadata

        Raises:
            CommunicationError: If request fails
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.ListDevicesRequest()
            if capability_filter:
                request.capability_filter = capability_filter

            response = await self._hardware_stub.ListDevices(
                request, timeout=self.timeout
            )

            devices = []
            for device in response.devices:
                devices.append(
                    {
                        "id": device.id,
                        "name": device.name,
                        "driver_type": device.driver_type,
                        "capabilities": {
                            "movable": device.is_movable,
                            "readable": device.is_readable,
                            "triggerable": device.is_triggerable,
                            "frame_producer": device.is_frame_producer,
                            "exposure_controllable": device.is_exposure_controllable,
                            "shutter_controllable": device.is_shutter_controllable,
                            "wavelength_tunable": device.is_wavelength_tunable,
                            "emission_controllable": device.is_emission_controllable,
                        },
                        "metadata": self._parse_device_metadata(device.metadata),
                    }
                )

            return devices

        except grpc.RpcError as e:
            raise translate_grpc_error(e, "Failed to list devices")

    def _parse_device_metadata(self, metadata) -> Dict[str, Any]:
        """Parse DeviceMetadata protobuf into a dict."""
        result = {}

        # Position info (for Movable)
        if metadata.HasField("position_units"):
            result["position_units"] = metadata.position_units
        if metadata.HasField("min_position"):
            result["min_position"] = metadata.min_position
        if metadata.HasField("max_position"):
            result["max_position"] = metadata.max_position

        # Reading info (for Readable)
        if metadata.HasField("reading_units"):
            result["reading_units"] = metadata.reading_units

        # Frame info (for FrameProducer)
        if metadata.HasField("frame_width"):
            result["frame_width"] = metadata.frame_width
        if metadata.HasField("frame_height"):
            result["frame_height"] = metadata.frame_height
        if metadata.HasField("bits_per_pixel"):
            result["bits_per_pixel"] = metadata.bits_per_pixel

        # Exposure info (for ExposureControl)
        if metadata.HasField("min_exposure_ms"):
            result["min_exposure_ms"] = metadata.min_exposure_ms
        if metadata.HasField("max_exposure_ms"):
            result["max_exposure_ms"] = metadata.max_exposure_ms

        # Wavelength info (for WavelengthTunable)
        if metadata.HasField("min_wavelength_nm"):
            result["min_wavelength_nm"] = metadata.min_wavelength_nm
        if metadata.HasField("max_wavelength_nm"):
            result["max_wavelength_nm"] = metadata.max_wavelength_nm

        return result

    async def get_device_state(self, device_id: str) -> Dict[str, Any]:
        """
        Get current state of a device.

        Args:
            device_id: Device identifier

        Returns:
            Dictionary with device state:
                - device_id: Device ID
                - online: Whether device is online
                - position: Current position (if Movable)
                - last_reading: Last reading (if Readable)
                - armed: Armed state (if Triggerable)
                - streaming: Streaming state (if FrameProducer)
                - exposure_ms: Current exposure (if ExposureControl)

        Raises:
            DeviceError: If device not found
            CommunicationError: If request fails
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.DeviceStateRequest(device_id=device_id)
            response = await self._hardware_stub.GetDeviceState(
                request, timeout=self.timeout
            )

            state = {
                "device_id": response.device_id,
                "online": response.online,
            }

            # Add optional fields if present
            if response.HasField("position"):
                state["position"] = response.position
            if response.HasField("last_reading"):
                state["last_reading"] = response.last_reading
            if response.HasField("armed"):
                state["armed"] = response.armed
            if response.HasField("streaming"):
                state["streaming"] = response.streaming
            if response.HasField("exposure_ms"):
                state["exposure_ms"] = response.exposure_ms

            return state

        except grpc.RpcError as e:
            raise translate_grpc_error(e, f"Failed to get state for device {device_id}")

    # =========================================================================
    # Hardware Service Methods - Motion Control
    # =========================================================================

    async def move_absolute(
        self,
        device_id: str,
        position: float,
        wait_for_completion: bool = False,
        timeout_ms: Optional[int] = None,
    ) -> Dict[str, Any]:
        """
        Move device to absolute position.

        Args:
            device_id: Device identifier
            position: Target position in device units
            wait_for_completion: If True, wait for motion to complete
            timeout_ms: Timeout for completion in milliseconds

        Returns:
            Dictionary with:
                - success: Whether command succeeded
                - final_position: Actual position after move
                - settled: Whether position is settled (if wait_for_completion)
                - error_message: Error message if any

        Raises:
            DeviceError: If device doesn't support motion or move fails
            TimeoutError: If wait times out
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.MoveRequest(
                device_id=device_id,
                value=position,
            )

            if wait_for_completion:
                request.wait_for_completion = True
                if timeout_ms is not None:
                    request.timeout_ms = timeout_ms

            response = await self._hardware_stub.MoveAbsolute(
                request, timeout=self.timeout
            )

            result = {
                "success": response.success,
                "final_position": response.final_position,
            }

            if response.error_message:
                result["error_message"] = response.error_message

            if response.HasField("settled"):
                result["settled"] = response.settled

            if not response.success:
                raise DeviceError(
                    f"Move failed: {response.error_message}",
                    device_id=device_id,
                )

            return result

        except grpc.RpcError as e:
            raise translate_grpc_error(
                e, f"Failed to move device {device_id} to {position}"
            )

    async def move_relative(
        self,
        device_id: str,
        distance: float,
        wait_for_completion: bool = False,
        timeout_ms: Optional[int] = None,
    ) -> Dict[str, Any]:
        """
        Move device by relative distance.

        Args:
            device_id: Device identifier
            distance: Distance to move in device units
            wait_for_completion: If True, wait for motion to complete
            timeout_ms: Timeout for completion in milliseconds

        Returns:
            Dictionary with move result (same as move_absolute)

        Raises:
            DeviceError: If device doesn't support motion or move fails
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.MoveRequest(
                device_id=device_id,
                value=distance,
            )

            if wait_for_completion:
                request.wait_for_completion = True
                if timeout_ms is not None:
                    request.timeout_ms = timeout_ms

            response = await self._hardware_stub.MoveRelative(
                request, timeout=self.timeout
            )

            result = {
                "success": response.success,
                "final_position": response.final_position,
            }

            if response.error_message:
                result["error_message"] = response.error_message

            if response.HasField("settled"):
                result["settled"] = response.settled

            if not response.success:
                raise DeviceError(
                    f"Move failed: {response.error_message}",
                    device_id=device_id,
                )

            return result

        except grpc.RpcError as e:
            raise translate_grpc_error(
                e, f"Failed to move device {device_id} by {distance}"
            )

    async def get_position(self, device_id: str) -> float:
        """
        Get current position of a movable device.

        Args:
            device_id: Device identifier

        Returns:
            Current position in device units

        Raises:
            DeviceError: If device doesn't support position reading
        """
        state = await self.get_device_state(device_id)
        if "position" not in state:
            raise DeviceError(
                f"Device {device_id} does not support position reading",
                device_id=device_id,
            )
        return state["position"]

    # =========================================================================
    # Hardware Service Methods - Parameter Control
    # =========================================================================

    async def set_parameter(
        self, device_id: str, parameter_name: str, value: str
    ) -> Dict[str, Any]:
        """
        Set a device parameter.

        Args:
            device_id: Device identifier
            parameter_name: Parameter name (e.g., "exposure_ms", "wavelength")
            value: Parameter value as string

        Returns:
            Dictionary with:
                - success: Whether set succeeded
                - actual_value: Actual value after setting
                - error_message: Error message if any

        Raises:
            DeviceError: If parameter doesn't exist or set fails
            ConfigurationError: If value is invalid
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.SetParameterRequest(
                device_id=device_id,
                parameter_name=parameter_name,
                value=value,
            )

            response = await self._hardware_stub.SetParameter(
                request, timeout=self.timeout
            )

            result = {
                "success": response.success,
                "actual_value": response.actual_value,
            }

            if response.error_message:
                result["error_message"] = response.error_message

            if not response.success:
                raise DeviceError(
                    f"Failed to set {parameter_name}: {response.error_message}",
                    device_id=device_id,
                )

            return result

        except grpc.RpcError as e:
            raise translate_grpc_error(
                e, f"Failed to set {parameter_name} on {device_id}"
            )

    async def get_parameter(self, device_id: str, parameter_name: str) -> Dict[str, Any]:
        """
        Get a device parameter value.

        Args:
            device_id: Device identifier
            parameter_name: Parameter name

        Returns:
            Dictionary with:
                - device_id: Device ID
                - name: Parameter name
                - value: Current value
                - units: Units string
                - timestamp_ns: Timestamp in nanoseconds

        Raises:
            DeviceError: If parameter doesn't exist
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.GetParameterRequest(
                device_id=device_id,
                parameter_name=parameter_name,
            )

            response = await self._hardware_stub.GetParameter(
                request, timeout=self.timeout
            )

            return {
                "device_id": response.device_id,
                "name": response.name,
                "value": response.value,
                "units": response.units,
                "timestamp_ns": response.timestamp_ns,
            }

        except grpc.RpcError as e:
            raise translate_grpc_error(
                e, f"Failed to get {parameter_name} from {device_id}"
            )

    # =========================================================================
    # Hardware Service Methods - Device State Streaming
    # =========================================================================

    async def stream_device_state(
        self,
        device_ids: Optional[List[str]] = None,
        max_rate_hz: int = 10,
        include_snapshot: bool = True,
    ) -> AsyncIterator[Dict[str, Any]]:
        """
        Stream device state updates in real-time.

        Args:
            device_ids: List of device IDs to monitor (None = all devices)
            max_rate_hz: Maximum update rate in Hz
            include_snapshot: Include full snapshot as first message

        Yields:
            Dictionary with state update:
                - device_id: Device ID
                - timestamp_ns: Update timestamp
                - version: Monotonic version number
                - is_snapshot: Whether this is a full snapshot
                - fields: Dictionary of changed fields

        Raises:
            CommunicationError: If streaming fails
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.DeviceStateSubscribeRequest(
                max_rate_hz=max_rate_hz,
                include_snapshot=include_snapshot,
            )

            if device_ids:
                request.device_ids.extend(device_ids)

            async for update in self._hardware_stub.SubscribeDeviceState(
                request, timeout=None  # No timeout for streaming
            ):
                # Parse fields_json into actual dict
                import json

                fields = {}
                for key, value_json in update.fields_json.items():
                    try:
                        fields[key] = json.loads(value_json)
                    except json.JSONDecodeError:
                        fields[key] = value_json  # Fallback to string

                yield {
                    "device_id": update.device_id,
                    "timestamp_ns": update.timestamp_ns,
                    "version": update.version,
                    "is_snapshot": update.is_snapshot,
                    "fields": fields,
                }

        except grpc.RpcError as e:
            raise translate_grpc_error(e, "Device state streaming failed")

    # =========================================================================
    # Hardware Service Methods - Frame Streaming
    # =========================================================================

    async def stream_frames(
        self,
        device_id: str,
        include_pixel_data: bool = True,
    ) -> AsyncIterator[Dict[str, Any]]:
        """
        Stream camera frames in real-time.

        Args:
            device_id: Camera device ID
            include_pixel_data: Whether to include raw pixel bytes (can be large)

        Yields:
            Dictionary with frame data:
                - device_id: Camera device ID
                - frame_number: Sequential frame number
                - width: Frame width in pixels
                - height: Frame height in pixels
                - timestamp_ns: Frame timestamp in nanoseconds
                - pixel_data: Raw pixel bytes (if include_pixel_data=True)
                - pixel_format: Pixel format string (e.g., "u16_le", "u8")

        Raises:
            CommunicationError: If streaming fails
            DeviceError: If device not found or not a camera

        Example:
            async with AsyncClient() as client:
                async for frame in client.stream_frames("camera_0"):
                    print(f"Frame {frame['frame_number']}: {frame['width']}x{frame['height']}")
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.StreamFramesRequest(
                device_id=device_id,
                include_pixel_data=include_pixel_data,
            )

            async for frame in self._hardware_stub.StreamFrames(
                request, timeout=None  # No timeout for streaming
            ):
                yield {
                    "device_id": frame.device_id,
                    "frame_number": frame.frame_number,
                    "width": frame.width,
                    "height": frame.height,
                    "timestamp_ns": frame.timestamp_ns,
                    "pixel_data": frame.pixel_data if include_pixel_data else None,
                    "pixel_format": frame.pixel_format,
                }

        except grpc.RpcError as e:
            raise translate_grpc_error(e, f"Frame streaming failed for {device_id}")

    # =========================================================================
    # Hardware Service Methods - Parameter Change Streaming
    # =========================================================================

    async def stream_parameter_changes(
        self,
        device_id: Optional[str] = None,
        parameter_names: Optional[List[str]] = None,
    ) -> AsyncIterator[Dict[str, Any]]:
        """
        Stream parameter changes in real-time.

        Args:
            device_id: Filter by device ID (None = all devices)
            parameter_names: Filter by parameter names (None = all parameters)

        Yields:
            Dictionary with parameter change:
                - device_id: Device ID
                - name: Parameter name
                - old_value: Previous value (as string)
                - new_value: New value (as string)
                - units: Parameter units

        Raises:
            CommunicationError: If streaming fails

        Example:
            async with AsyncClient() as client:
                async for change in client.stream_parameter_changes("laser"):
                    print(f"{change['name']}: {change['old_value']} -> {change['new_value']}")
        """
        from .generated import daq_pb2

        self._ensure_connected()

        try:
            request = daq_pb2.StreamParameterChangesRequest()
            if device_id:
                request.device_id = device_id
            if parameter_names:
                request.parameter_names.extend(parameter_names)

            async for change in self._hardware_stub.StreamParameterChanges(
                request, timeout=None  # No timeout for streaming
            ):
                yield {
                    "device_id": change.device_id,
                    "name": change.name,
                    "old_value": change.old_value,
                    "new_value": change.new_value,
                    "units": change.units,
                }

        except grpc.RpcError as e:
            raise translate_grpc_error(e, "Parameter change streaming failed")
