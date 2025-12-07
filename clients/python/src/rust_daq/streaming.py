"""
Layer 3: Async streaming classes for rust-daq.

Provides context-managed async iterators for:
- FrameStream: Real-time camera frame streaming
- ParameterSubscription: Real-time parameter change monitoring
- DeviceStateStream: Real-time device state updates

Example:
    async with AsyncClient() as client:
        async with FrameStream(client, "camera_0") as stream:
            async for frame in stream:
                process_frame(frame)

        async with ParameterSubscription(client, "laser") as sub:
            async for change in sub:
                print(f"{change['name']}: {change['new_value']}")
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import (
    TYPE_CHECKING,
    Any,
    AsyncIterator,
    Callable,
    Dict,
    List,
    Optional,
)

if TYPE_CHECKING:
    import numpy as np
    from .core import AsyncClient


@dataclass
class Frame:
    """
    Camera frame with metadata and optional numpy array.

    Attributes:
        device_id: Camera device ID
        frame_number: Sequential frame number
        width: Frame width in pixels
        height: Frame height in pixels
        timestamp_ns: Capture timestamp in nanoseconds
        pixel_data: Raw pixel bytes
        pixel_format: Format string (e.g., "u16_le", "u8")
    """
    device_id: str
    frame_number: int
    width: int
    height: int
    timestamp_ns: int
    pixel_data: bytes | None
    pixel_format: str

    def to_numpy(self) -> "np.ndarray":
        """
        Convert pixel data to numpy array.

        Returns:
            numpy.ndarray with shape (height, width) or (height, width, channels)

        Raises:
            ImportError: If numpy is not installed
            ValueError: If pixel_data is None or format is unknown
        """
        try:
            import numpy as np
        except ImportError:
            raise ImportError("numpy is required for to_numpy()")

        if self.pixel_data is None:
            raise ValueError("No pixel data available (include_pixel_data=False?)")

        # Parse pixel format
        format_map = {
            "u8": np.uint8,
            "u16_le": np.uint16,
            "u16_be": np.dtype(">u2"),
            "f32_le": np.float32,
        }

        dtype = format_map.get(self.pixel_format)
        if dtype is None:
            raise ValueError(f"Unknown pixel format: {self.pixel_format}")

        arr = np.frombuffer(self.pixel_data, dtype=dtype)
        return arr.reshape((self.height, self.width))


@dataclass
class ParameterChange:
    """
    Parameter change event.

    Attributes:
        device_id: Device ID
        name: Parameter name
        old_value: Previous value as string
        new_value: New value as string
        units: Parameter units
    """
    device_id: str
    name: str
    old_value: str
    new_value: str
    units: str

    def old_as_float(self) -> float:
        """Parse old_value as float."""
        return float(self.old_value)

    def new_as_float(self) -> float:
        """Parse new_value as float."""
        return float(self.new_value)


class FrameStream:
    """
    Async context manager for streaming camera frames.

    Provides a clean async iteration pattern with automatic cleanup.

    Example:
        async with AsyncClient() as client:
            async with FrameStream(client, "camera_0") as stream:
                async for frame in stream:
                    arr = frame.to_numpy()
                    print(f"Frame {frame.frame_number}: mean={arr.mean():.1f}")

            # With frame limit
            async with FrameStream(client, "camera_0", max_frames=100) as stream:
                async for frame in stream:
                    process(frame)
    """

    def __init__(
        self,
        client: "AsyncClient",
        device_id: str,
        include_pixel_data: bool = True,
        max_frames: Optional[int] = None,
        on_frame: Optional[Callable[[Frame], None]] = None,
    ):
        """
        Initialize FrameStream.

        Args:
            client: Connected AsyncClient instance
            device_id: Camera device ID to stream from
            include_pixel_data: Whether to include raw pixel bytes
            max_frames: Stop after this many frames (None = unlimited)
            on_frame: Optional callback called for each frame
        """
        self._client = client
        self._device_id = device_id
        self._include_pixel_data = include_pixel_data
        self._max_frames = max_frames
        self._on_frame = on_frame
        self._frame_count = 0
        self._active = False
        self._iterator: Optional[AsyncIterator[Dict[str, Any]]] = None

    async def __aenter__(self) -> "FrameStream":
        """Start streaming."""
        self._active = True
        self._frame_count = 0
        self._iterator = self._client.stream_frames(
            self._device_id,
            include_pixel_data=self._include_pixel_data,
        )
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> bool:
        """Stop streaming and cleanup."""
        self._active = False
        self._iterator = None
        return False

    def __aiter__(self) -> "FrameStream":
        return self

    async def __anext__(self) -> Frame:
        if not self._active or self._iterator is None:
            raise StopAsyncIteration

        if self._max_frames and self._frame_count >= self._max_frames:
            raise StopAsyncIteration

        try:
            raw = await self._iterator.__anext__()
        except StopAsyncIteration:
            self._active = False
            raise

        frame = Frame(
            device_id=raw["device_id"],
            frame_number=raw["frame_number"],
            width=raw["width"],
            height=raw["height"],
            timestamp_ns=raw["timestamp_ns"],
            pixel_data=raw["pixel_data"],
            pixel_format=raw["pixel_format"],
        )

        self._frame_count += 1

        if self._on_frame:
            self._on_frame(frame)

        return frame

    @property
    def frame_count(self) -> int:
        """Number of frames received so far."""
        return self._frame_count

    @property
    def is_active(self) -> bool:
        """Whether the stream is currently active."""
        return self._active


class ParameterSubscription:
    """
    Async context manager for subscribing to parameter changes.

    Example:
        async with AsyncClient() as client:
            # Subscribe to all parameter changes on a device
            async with ParameterSubscription(client, device_id="laser") as sub:
                async for change in sub:
                    print(f"{change.name}: {change.old_value} -> {change.new_value}")

            # Subscribe to specific parameters across all devices
            async with ParameterSubscription(
                client,
                parameter_names=["wavelength_nm", "power_mw"]
            ) as sub:
                async for change in sub:
                    if change.name == "wavelength_nm":
                        wavelength = change.new_as_float()
    """

    def __init__(
        self,
        client: "AsyncClient",
        device_id: Optional[str] = None,
        parameter_names: Optional[List[str]] = None,
        on_change: Optional[Callable[[ParameterChange], None]] = None,
    ):
        """
        Initialize ParameterSubscription.

        Args:
            client: Connected AsyncClient instance
            device_id: Filter by device ID (None = all devices)
            parameter_names: Filter by parameter names (None = all)
            on_change: Optional callback called for each change
        """
        self._client = client
        self._device_id = device_id
        self._parameter_names = parameter_names
        self._on_change = on_change
        self._change_count = 0
        self._active = False
        self._iterator: Optional[AsyncIterator[Dict[str, Any]]] = None

    async def __aenter__(self) -> "ParameterSubscription":
        """Start subscription."""
        self._active = True
        self._change_count = 0
        self._iterator = self._client.stream_parameter_changes(
            device_id=self._device_id,
            parameter_names=self._parameter_names,
        )
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> bool:
        """Stop subscription and cleanup."""
        self._active = False
        self._iterator = None
        return False

    def __aiter__(self) -> "ParameterSubscription":
        return self

    async def __anext__(self) -> ParameterChange:
        if not self._active or self._iterator is None:
            raise StopAsyncIteration

        try:
            raw = await self._iterator.__anext__()
        except StopAsyncIteration:
            self._active = False
            raise

        change = ParameterChange(
            device_id=raw["device_id"],
            name=raw["name"],
            old_value=raw["old_value"],
            new_value=raw["new_value"],
            units=raw["units"],
        )

        self._change_count += 1

        if self._on_change:
            self._on_change(change)

        return change

    @property
    def change_count(self) -> int:
        """Number of changes received so far."""
        return self._change_count

    @property
    def is_active(self) -> bool:
        """Whether the subscription is currently active."""
        return self._active


class DeviceStateStream:
    """
    Async context manager for streaming device state updates.

    Example:
        async with AsyncClient() as client:
            async with DeviceStateStream(client, ["motor_x", "motor_y"]) as stream:
                async for update in stream:
                    print(f"{update['device_id']}: {update['fields']}")
    """

    def __init__(
        self,
        client: "AsyncClient",
        device_ids: Optional[List[str]] = None,
        max_rate_hz: int = 10,
        include_snapshot: bool = True,
    ):
        """
        Initialize DeviceStateStream.

        Args:
            client: Connected AsyncClient instance
            device_ids: List of device IDs to monitor (None = all)
            max_rate_hz: Maximum update rate in Hz
            include_snapshot: Include full snapshot as first message
        """
        self._client = client
        self._device_ids = device_ids
        self._max_rate_hz = max_rate_hz
        self._include_snapshot = include_snapshot
        self._update_count = 0
        self._active = False
        self._iterator: Optional[AsyncIterator[Dict[str, Any]]] = None

    async def __aenter__(self) -> "DeviceStateStream":
        """Start streaming."""
        self._active = True
        self._update_count = 0
        self._iterator = self._client.stream_device_state(
            device_ids=self._device_ids,
            max_rate_hz=self._max_rate_hz,
            include_snapshot=self._include_snapshot,
        )
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> bool:
        """Stop streaming and cleanup."""
        self._active = False
        self._iterator = None
        return False

    def __aiter__(self) -> "DeviceStateStream":
        return self

    async def __anext__(self) -> Dict[str, Any]:
        if not self._active or self._iterator is None:
            raise StopAsyncIteration

        try:
            update = await self._iterator.__anext__()
        except StopAsyncIteration:
            self._active = False
            raise

        self._update_count += 1
        return update

    @property
    def update_count(self) -> int:
        """Number of updates received so far."""
        return self._update_count

    @property
    def is_active(self) -> bool:
        """Whether the stream is currently active."""
        return self._active
