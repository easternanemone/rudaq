"""
Tests for AsyncClient.

These tests cover:
- Exception translation (unit tests)
- Connection management (unit tests)
- Integration tests with real daemon (marked as integration)

Note: Most AsyncClient methods require a running daemon to test properly.
Unit tests with mocking are complex due to lazy protobuf imports.
Integration tests provide better coverage.
"""

import pytest
from unittest.mock import AsyncMock, MagicMock
import grpc

from rust_daq import AsyncClient
from rust_daq.exceptions import (
    DaqError,
    DeviceError,
    CommunicationError,
    TimeoutError,
    translate_grpc_error,
)


# ============================================================================
# Exception Translation Tests
# ============================================================================


def test_translate_grpc_unavailable():
    """Test translation of UNAVAILABLE status to CommunicationError."""
    mock_error = MagicMock(spec=grpc.RpcError)
    # Fix: Set return value for callable methods
    mock_error.code = MagicMock(return_value=grpc.StatusCode.UNAVAILABLE)
    mock_error.details = MagicMock(return_value="Connection refused")

    exc = translate_grpc_error(mock_error, "Test context")
    assert isinstance(exc, CommunicationError)
    assert "unavailable" in exc.message.lower()


def test_translate_grpc_deadline_exceeded():
    """Test translation of DEADLINE_EXCEEDED to TimeoutError."""
    mock_error = MagicMock(spec=grpc.RpcError)
    mock_error.code = MagicMock(return_value=grpc.StatusCode.DEADLINE_EXCEEDED)
    mock_error.details = MagicMock(return_value="Timeout")

    exc = translate_grpc_error(mock_error, "Test context")
    assert isinstance(exc, TimeoutError)


def test_translate_grpc_not_found():
    """Test translation of NOT_FOUND to DeviceError."""
    mock_error = MagicMock(spec=grpc.RpcError)
    mock_error.code = MagicMock(return_value=grpc.StatusCode.NOT_FOUND)
    mock_error.details = MagicMock(return_value="Device not found")

    exc = translate_grpc_error(mock_error, "Test context")
    assert isinstance(exc, DeviceError)


# ============================================================================
# AsyncClient Tests - Connection Management
# ============================================================================


@pytest.mark.asyncio
async def test_client_context_manager():
    """Test AsyncClient as async context manager."""
    client = AsyncClient("localhost:50051")

    # Mock the connect and close methods
    client.connect = AsyncMock()
    client.close = AsyncMock()

    async with client:
        client.connect.assert_called_once()
        assert client.connect.called

    client.close.assert_called_once()


@pytest.mark.asyncio
async def test_client_not_connected_error():
    """Test that operations fail when not connected."""
    client = AsyncClient("localhost:50051")

    with pytest.raises(DaqError, match="Not connected"):
        await client.list_devices()


def test_client_initialization():
    """Test client initialization with custom parameters."""
    client = AsyncClient(
        address="10.0.0.1:8080",
        timeout=30.0,
        max_message_length=200 * 1024 * 1024,
    )
    
    assert client.address == "10.0.0.1:8080"
    assert client.timeout == 30.0
    assert client._max_message_length == 200 * 1024 * 1024


# ============================================================================
# Integration Test Markers (require running daemon)
# ============================================================================


@pytest.mark.integration
@pytest.mark.asyncio
async def test_real_daemon_connection():
    """
    Integration test - requires rust-daq daemon running on localhost:50051.

    Run with: pytest -m integration
    
    This test verifies:
    - Connection establishment
    - GetDaemonInfo RPC call
    - Proper async context manager cleanup
    """
    async with AsyncClient("localhost:50051", timeout=5.0) as client:
        info = await client.get_daemon_info()
        assert "version" in info
        assert isinstance(info["features"], list)
        assert isinstance(info["available_hardware"], list)
        assert info["uptime_seconds"] >= 0


@pytest.mark.integration
@pytest.mark.asyncio
async def test_real_daemon_list_devices():
    """
    Integration test - requires rust-daq daemon running.

    Run with: pytest -m integration
    
    This test verifies:
    - ListDevices RPC call
    - Device info structure
    - Capability flags parsing
    """
    async with AsyncClient("localhost:50051", timeout=5.0) as client:
        devices = await client.list_devices()
        assert isinstance(devices, list)
        
        # Verify device structure if any devices present
        if len(devices) > 0:
            device = devices[0]
            assert "id" in device
            assert "name" in device
            assert "driver_type" in device
            assert "capabilities" in device
            assert isinstance(device["capabilities"], dict)


@pytest.mark.integration
@pytest.mark.asyncio
async def test_real_daemon_device_state():
    """
    Integration test - requires rust-daq daemon with at least one device.

    Run with: pytest -m integration
    
    This test verifies:
    - GetDeviceState RPC call
    - State structure parsing
    """
    async with AsyncClient("localhost:50051", timeout=5.0) as client:
        devices = await client.list_devices()
        
        if len(devices) > 0:
            device_id = devices[0]["id"]
            state = await client.get_device_state(device_id)
            
            assert "device_id" in state
            assert state["device_id"] == device_id
            assert "online" in state
            assert isinstance(state["online"], bool)


@pytest.mark.integration
@pytest.mark.asyncio
async def test_real_daemon_move_absolute():
    """
    Integration test - requires rust-daq daemon with a movable device.

    Run with: pytest -m integration
    
    This test verifies:
    - MoveAbsolute RPC call
    - Motion command execution
    - Response structure
    """
    async with AsyncClient("localhost:50051", timeout=5.0) as client:
        devices = await client.list_devices()
        
        # Find a movable device
        movable_device = None
        for device in devices:
            if device["capabilities"].get("movable"):
                movable_device = device
                break
        
        if movable_device:
            device_id = movable_device["id"]
            
            # Test absolute move
            result = await client.move_absolute(device_id, 5.0)
            
            assert "success" in result
            assert "final_position" in result
            
            # If move succeeded, verify position
            if result["success"]:
                assert isinstance(result["final_position"], float)
