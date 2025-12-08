"""
Custom exceptions for rust-daq client library.

Provides a hierarchy of exceptions for different error scenarios,
translating low-level gRPC errors into meaningful Python exceptions.
"""

from typing import Optional
import grpc


class DaqError(Exception):
    """Base exception for all rust-daq client errors."""

    def __init__(self, message: str, details: Optional[str] = None):
        """
        Initialize a DaqError.

        Args:
            message: Human-readable error message
            details: Optional additional technical details
        """
        super().__init__(message)
        self.message = message
        self.details = details

    def __str__(self) -> str:
        if self.details:
            return f"{self.message} (details: {self.details})"
        return self.message


class DeviceError(DaqError):
    """
    Error related to device operations.

    Raised when:
    - Device is not found
    - Device operation fails
    - Device is in invalid state
    - Device capability mismatch
    """

    def __init__(
        self,
        device_id: str,
        message: str | None = None,
    ):
        """
        Initialize a DeviceError.

        Args:
            device_id: ID of the device that caused the error
            message: Optional human-readable error message
        """
        msg = f"Device error for '{device_id}': {message}" if message else f"Device error for '{device_id}'"
        super().__init__(msg)
        self.device_id = device_id


class CommunicationError(DaqError):
    """
    Error related to gRPC communication with daemon.

    Raised when:
    - Connection to daemon fails
    - Network timeout occurs
    - gRPC channel error
    - Daemon is unreachable
    """

    def __init__(
        self,
        grpc_code: str | None = None,
        message: str | None = None,
    ):
        """
        Initialize a CommunicationError.

        Args:
            grpc_code: Optional gRPC status code
            message: Optional human-readable error message
        """
        msg = f"Communication error: {message} (gRPC code: {grpc_code})" if message else f"Communication error (gRPC code: {grpc_code})"
        super().__init__(msg)
        self.grpc_code = grpc_code


class TimeoutError(CommunicationError):
    """
    Operation timed out.

    Raised when:
    - gRPC call exceeds timeout
    - Device operation takes too long
    - Streaming operation stalls
    """

    def __init__(
        self,
        timeout_seconds: float | None = None,
        message: str | None = None,
    ):
        """
        Initialize a TimeoutError.

        Args:
            timeout_seconds: Optional timeout value in seconds
            message: Optional human-readable error message
        """
        msg = (
            f"Operation timed out after {timeout_seconds}s: {message}"
            if message and timeout_seconds
            else f"Operation timed out after {timeout_seconds}s"
            if timeout_seconds
            else f"Operation timed out: {message}"
            if message
            else "Operation timed out"
        )
        super().__init__(msg)
        self.timeout_seconds = timeout_seconds


class ConfigurationError(DaqError):
    """
    Error related to configuration or parameters.

    Raised when:
    - Invalid parameter value
    - Configuration validation fails
    - Incompatible settings
    """

    def __init__(
        self,
        message: str,
        parameter_name: Optional[str] = None,
        details: Optional[str] = None,
    ):
        """
        Initialize a ConfigurationError.

        Args:
            message: Human-readable error message
            parameter_name: Name of the invalid parameter
            details: Optional additional technical details
        """
        super().__init__(message, details)
        self.parameter_name = parameter_name


def translate_grpc_error(error: grpc.RpcError, context: str = "") -> DaqError:
    """
    Translate a gRPC error into an appropriate DaqError subclass.

    Args:
        error: The gRPC error to translate
        context: Optional context string for better error messages

    Returns:
        Appropriate DaqError subclass instance
    """
    status_code = error.code() if hasattr(error, "code") else None
    details = error.details() if hasattr(error, "details") else str(error)

    # Build contextual message
    if context:
        base_message = f"{context}: {details}"
    else:
        base_message = details

    # Map gRPC status codes to exception types
    if status_code == grpc.StatusCode.UNAVAILABLE:
        return CommunicationError(
            "Daemon unavailable - is the daemon running?",
            grpc_code=status_code,
            details=details,
        )
    elif status_code == grpc.StatusCode.DEADLINE_EXCEEDED:
        return TimeoutError(
            "Operation timed out",
        )
    elif status_code == grpc.StatusCode.NOT_FOUND:
        return DeviceError(
            base_message,
            details=details,
        )
    elif status_code == grpc.StatusCode.INVALID_ARGUMENT:
        return ConfigurationError(
            base_message,
            details=details,
        )
    elif status_code in (
        grpc.StatusCode.UNAUTHENTICATED,
        grpc.StatusCode.PERMISSION_DENIED,
    ):
        return CommunicationError(
            f"Authentication error: {details}",
            grpc_code=status_code,
            details=details,
        )
    else:
        # Generic communication error for other gRPC errors
        return CommunicationError(
            base_message,
            grpc_code=status_code,
            details=details,
        )
