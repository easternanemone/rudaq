//! SDK3 buffer management for frame acquisition.
//!
//! Provides aligned buffer allocation and a buffer set for the
//! AT_QueueBuffer / AT_WaitBuffer acquisition cycle.
//!
//! # Design
//!
//! SDK3 requires buffers to be 8-byte aligned. This module provides:
//! - `AlignedBuffer`: A single 8-byte aligned heap allocation
//! - `SdkBufferSet`: A set of N buffers for circular queue usage
//!
//! Buffers are allocated once and reused across acquisitions.
//! The SDK takes ownership of buffer pointers during acquisition
//! (between QueueBuffer and WaitBuffer), so we must ensure they
//! are not dropped while in use.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ptr::NonNull;

/// SDK3-required alignment for frame buffers.
const SDK3_BUFFER_ALIGNMENT: usize = 8;

/// Default number of SDK3 buffers to allocate.
pub const DEFAULT_BUFFER_COUNT: usize = 10;

/// An 8-byte aligned heap buffer for SDK3 frame data.
///
/// The buffer is zeroed on allocation and maintains a fixed size.
/// It is NOT resizable — create a new buffer if size changes.
///
/// # Safety
///
/// This type uses raw allocation. The buffer pointer is valid for the
/// lifetime of this struct. `Send` and `Sync` are implemented because
/// the buffer is owned and not aliased (SDK borrows during QueueBuffer
/// but we track that externally).
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
    size: usize,
}

impl AlignedBuffer {
    /// Allocate a new 8-byte aligned buffer of the given size.
    ///
    /// # Panics
    ///
    /// Panics if size is 0 or if allocation fails (OOM).
    pub fn new(size: usize) -> Self {
        assert!(size > 0, "buffer size must be > 0");

        let layout =
            Layout::from_size_align(size, SDK3_BUFFER_ALIGNMENT).expect("invalid buffer layout");

        // SAFETY: layout has non-zero size (asserted above)
        let ptr = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(ptr).expect("buffer allocation failed (OOM)");

        Self { ptr, layout, size }
    }

    /// Get the raw pointer to the buffer. Used for AT_QueueBuffer.
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Get the buffer size in bytes.
    #[inline]
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get the buffer contents as a slice.
    ///
    /// # Safety
    ///
    /// Caller must ensure the SDK is not currently writing to this buffer
    /// (i.e., this buffer is not queued or has been returned by WaitBuffer).
    #[inline]
    pub unsafe fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr is valid for `size` bytes (allocated in new()), properly aligned,
        // and caller ensures SDK is not writing to this buffer.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.size) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: ptr was allocated with this layout in new()
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

// SAFETY: The buffer is a contiguous allocation owned by this struct.
// It is not aliased (SDK borrows are tracked externally via queued state).
unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

/// A set of aligned buffers for SDK3 acquisition.
///
/// Manages N buffers that are rotated through the SDK's queue.
/// Call `queue_all()` before starting acquisition, then use
/// `buffer_for_ptr()` to find which buffer the SDK returned.
pub struct SdkBufferSet {
    buffers: Vec<AlignedBuffer>,
    image_size: usize,
}

impl SdkBufferSet {
    /// Create a new buffer set with `count` buffers of `image_size` bytes each.
    ///
    /// # Panics
    ///
    /// Panics if `count` is 0 or `image_size` is 0.
    pub fn new(count: usize, image_size: usize) -> Self {
        assert!(count > 0, "buffer count must be > 0");
        assert!(image_size > 0, "image size must be > 0");

        let buffers = (0..count).map(|_| AlignedBuffer::new(image_size)).collect();

        Self {
            buffers,
            image_size,
        }
    }

    /// Get the number of buffers in this set.
    #[inline]
    pub fn count(&self) -> usize {
        self.buffers.len()
    }

    /// Get the image size each buffer was allocated for.
    #[inline]
    pub fn image_size(&self) -> usize {
        self.image_size
    }

    /// Get a reference to a buffer by index.
    #[inline]
    pub fn get(&self, index: usize) -> Option<&AlignedBuffer> {
        self.buffers.get(index)
    }

    /// Iterator over all buffers (for queuing all at acquisition start).
    pub fn iter(&self) -> impl Iterator<Item = &AlignedBuffer> {
        self.buffers.iter()
    }

    /// Find the buffer index that matches a pointer returned by AT_WaitBuffer.
    ///
    /// SDK3 returns the same pointer that was queued, so we can identify
    /// which buffer was filled by comparing pointers.
    ///
    /// Returns `None` if the pointer doesn't match any buffer.
    pub fn index_for_ptr(&self, ptr: *const u8) -> Option<usize> {
        self.buffers
            .iter()
            .position(|buf| std::ptr::eq(buf.as_ptr(), ptr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aligned_buffer_allocation() {
        let buf = AlignedBuffer::new(4096);
        assert_eq!(buf.size(), 4096);
        assert!((buf.as_ptr() as usize).is_multiple_of(SDK3_BUFFER_ALIGNMENT));
    }

    #[test]
    fn test_aligned_buffer_zeroed() {
        let buf = AlignedBuffer::new(1024);
        unsafe {
            let slice = buf.as_slice();
            assert!(slice.iter().all(|&b| b == 0));
        }
    }

    #[test]
    #[should_panic(expected = "buffer size must be > 0")]
    fn test_zero_size_panics() {
        let _ = AlignedBuffer::new(0);
    }

    #[test]
    fn test_buffer_set_creation() {
        let set = SdkBufferSet::new(10, 8_388_608); // 8MB
        assert_eq!(set.count(), 10);
        assert_eq!(set.image_size(), 8_388_608);
    }

    #[test]
    fn test_buffer_set_ptr_lookup() {
        let set = SdkBufferSet::new(3, 4096);

        // Verify each buffer's pointer is findable
        for i in 0..3 {
            let ptr = set.get(i).unwrap().as_ptr();
            assert_eq!(set.index_for_ptr(ptr), Some(i));
        }

        // Unknown pointer returns None
        let fake_ptr = 0xDEAD as *const u8;
        assert_eq!(set.index_for_ptr(fake_ptr), None);
    }

    #[test]
    fn test_buffer_alignment() {
        let set = SdkBufferSet::new(5, 1024);
        for buf in set.iter() {
            assert!(
                (buf.as_ptr() as usize).is_multiple_of(SDK3_BUFFER_ALIGNMENT),
                "buffer not 8-byte aligned"
            );
        }
    }
}
