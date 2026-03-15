//! DLPack descriptor stub for future zero-copy tensor interop.
//!
//! This module is gated behind the `dlpack` feature flag and provides
//! the shape/strides/dtype descriptor that DLPack consumers need.
//! No actual DLPack dependency is introduced yet -- this is a structural
//! seam for future integration.

use crate::FrameData;

/// DLPack-compatible tensor descriptor.
///
/// Mirrors the subset of `DLManagedTensor` fields needed for
/// frame data exchange with NumPy/PyTorch/JAX.
#[derive(Debug, Clone)]
pub struct DlPackDescriptor {
    /// Data pointer offset (always 0 for contiguous frames).
    pub byte_offset: u64,
    /// Number of dimensions (always 2 for images: height x width).
    pub ndim: usize,
    /// Shape: \[height, width\] in elements.
    pub shape: Vec<i64>,
    /// Strides in elements (row-major): \[width, 1\].
    pub strides: Vec<i64>,
    /// DLPack data type code (`kDLUInt`=1, `kDLFloat`=2).
    pub dtype_code: u8,
    /// Bits per element.
    pub dtype_bits: u8,
    /// Number of lanes (always 1 for scalar types).
    pub dtype_lanes: u16,
}

impl FrameData {
    /// Create a DLPack-compatible descriptor for this frame.
    ///
    /// Returns shape, strides, and dtype information suitable for
    /// constructing a `DLManagedTensor`. The caller is responsible
    /// for ensuring the frame data outlives any tensor created from
    /// this descriptor (use `BorrowGuard` for lifetime management).
    #[must_use]
    pub fn to_dlpack_descriptor(&self) -> DlPackDescriptor {
        let width = i64::from(self.width);
        let height = i64::from(self.height);
        let (dtype_code, dtype_bits) = match self.bit_depth {
            8 => (1_u8, 8_u8), // kDLUInt, 8-bit
            16 => (1, 16),     // kDLUInt, 16-bit
            32 => (2, 32),     // kDLFloat, 32-bit
            _ => (1, 8),       // default to u8
        };

        DlPackDescriptor {
            byte_offset: 0,
            ndim: 2,
            shape: vec![height, width],
            strides: vec![width, 1],
            dtype_code,
            dtype_bits,
            dtype_lanes: 1,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_16bit() {
        let mut frame = FrameData::with_capacity(1024);
        frame.width = 32;
        frame.height = 24;
        frame.bit_depth = 16;

        let desc = frame.to_dlpack_descriptor();
        assert_eq!(desc.ndim, 2);
        assert_eq!(desc.shape, vec![24, 32]);
        assert_eq!(desc.strides, vec![32, 1]);
        assert_eq!(desc.dtype_code, 1); // kDLUInt
        assert_eq!(desc.dtype_bits, 16);
        assert_eq!(desc.dtype_lanes, 1);
        assert_eq!(desc.byte_offset, 0);
    }

    #[test]
    fn descriptor_8bit() {
        let mut frame = FrameData::with_capacity(512);
        frame.width = 16;
        frame.height = 16;
        frame.bit_depth = 8;

        let desc = frame.to_dlpack_descriptor();
        assert_eq!(desc.shape, vec![16, 16]);
        assert_eq!(desc.strides, vec![16, 1]);
        assert_eq!(desc.dtype_code, 1); // kDLUInt
        assert_eq!(desc.dtype_bits, 8);
    }

    #[test]
    fn descriptor_32bit_float() {
        let mut frame = FrameData::with_capacity(4096);
        frame.width = 64;
        frame.height = 48;
        frame.bit_depth = 32;

        let desc = frame.to_dlpack_descriptor();
        assert_eq!(desc.shape, vec![48, 64]);
        assert_eq!(desc.strides, vec![64, 1]);
        assert_eq!(desc.dtype_code, 2); // kDLFloat
        assert_eq!(desc.dtype_bits, 32);
    }

    #[test]
    fn descriptor_unknown_depth_defaults_to_u8() {
        let mut frame = FrameData::with_capacity(256);
        frame.width = 8;
        frame.height = 8;
        frame.bit_depth = 12; // non-standard

        let desc = frame.to_dlpack_descriptor();
        assert_eq!(desc.dtype_code, 1); // kDLUInt
        assert_eq!(desc.dtype_bits, 8); // fallback
    }
}
