//! Shared hard limits to prevent unbounded allocations or payload growth.

use crate::error::DaqError;

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
