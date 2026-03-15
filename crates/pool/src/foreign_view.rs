//! Zero-copy foreign access to frame data.
//!
//! `ForeignView` provides a minimal, read-only interface for external
//! consumers (Python via PyO3, C/C++ via FFI, GPU via DLPack) to access
//! frame buffer memory without copying.

/// Read-only view into frame buffer memory for foreign consumers.
///
/// # Safety
///
/// Callers using `as_ptr()` must ensure:
/// - The `ForeignView` implementor outlives any use of the returned pointer
/// - No mutable access occurs while the pointer is in use
/// - Pointer arithmetic respects `len()` bounds
pub trait ForeignView {
    /// Raw pointer to the start of the frame buffer.
    fn as_ptr(&self) -> *const u8;

    /// Length of the buffer in bytes.
    fn len(&self) -> usize;

    /// Whether the buffer is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Frame dimensions as (width, height).
    fn shape(&self) -> (u32, u32);

    /// Data type string (e.g., "u16", "f32").
    fn dtype(&self) -> &str;

    /// Bits per pixel.
    fn bit_depth(&self) -> u32;
}
