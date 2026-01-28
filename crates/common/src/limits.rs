//! Shared hard limits to prevent unbounded allocations or payload growth.
//!
//! This module centralizes:
//! - Payload size limits (frames, responses, scripts)
//! - Timeout durations for gRPC and health checks
//!
//! Using centralized constants ensures consistency across services and
//! makes tuning easier.

use crate::error::DaqError;
use std::time::Duration;

// =============================================================================
// Timeout Constants
// =============================================================================

/// Default timeout for gRPC RPC calls (15 seconds).
///
/// Used by hardware_service, scan_service, and other gRPC handlers
/// to prevent hung operations from blocking indefinitely.
pub const RPC_TIMEOUT: Duration = Duration::from_secs(15);

/// Interval between health check probes (5 seconds).
///
/// Used by health services and system monitors to periodically
/// check service/system status.
pub const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(5);

/// Duration window for FPS calculation (1 second).
///
/// Frame timestamps older than this are discarded when computing
/// the current frames-per-second rate.
pub const FPS_WINDOW: Duration = Duration::from_secs(1);

/// Timeout for graceful shutdown operations (2 seconds).
///
/// Used when stopping background tasks to allow cleanup before
/// forcing termination.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

// =============================================================================
// Rate Limiting
// =============================================================================

/// Maximum concurrent frame streams per client IP (default: 3).
///
/// Prevents a single client from consuming all server bandwidth by opening
/// too many simultaneous frame streams. Returns `ResourceExhausted` when exceeded.
pub const MAX_STREAMS_PER_CLIENT: usize = 3;

// =============================================================================
// Size Limits
// =============================================================================

/// Maximum allowed frame payload in bytes (default: 100MB).
pub const MAX_FRAME_BYTES: usize = 100 * 1024 * 1024;
/// Maximum allowed response payload in bytes (default: 1MB).
pub const MAX_RESPONSE_SIZE: usize = 1024 * 1024;
/// Maximum allowed script upload size in bytes (default: 1MB).
pub const MAX_SCRIPT_SIZE: usize = 1024 * 1024;
/// Maximum supported width/height for frames.
pub const MAX_FRAME_DIMENSION: u32 = 65_536;

/// Validated frame sizing information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSize {
    pub pixels: usize,
    pub bytes: usize,
}

/// Validate frame dimensions and calculate pixel/byte sizes safely.
pub fn validate_frame_size(
    width: u32,
    height: u32,
    bytes_per_pixel: usize,
) -> Result<FrameSize, DaqError> {
    if width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION {
        return Err(DaqError::FrameDimensionsTooLarge {
            width,
            height,
            max_dimension: MAX_FRAME_DIMENSION,
        });
    }

    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(DaqError::SizeOverflow {
            context: "frame pixel count",
        })?;

    let bytes = pixels
        .checked_mul(bytes_per_pixel)
        .ok_or(DaqError::SizeOverflow {
            context: "frame byte size",
        })?;

    if bytes > MAX_FRAME_BYTES {
        return Err(DaqError::FrameTooLarge {
            bytes,
            max_bytes: MAX_FRAME_BYTES,
        });
    }

    Ok(FrameSize { pixels, bytes })
}
