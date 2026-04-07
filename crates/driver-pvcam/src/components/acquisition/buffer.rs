//! Frame metadata and page-aligned buffer types for PVCAM acquisition.

#[cfg(feature = "pvcam_sdk")]
use anyhow::{Result, anyhow, bail};
#[cfg(feature = "pvcam_sdk")]
use std::alloc::{Layout, alloc_zeroed, dealloc};

/// Hardware frame metadata decoded from PVCAM embedded metadata (Gemini SDK review).
///
/// When `PARAM_METADATA_ENABLED` is true, PVCAM embeds timing information
/// directly in the frame buffer. This struct holds the decoded values
/// which provide microsecond-precision hardware timestamps from the FPGA.
///
/// # Timestamps
///
/// - `timestamp_bof_ns`: Beginning of frame timestamp in nanoseconds
/// - `timestamp_eof_ns`: End of frame timestamp in nanoseconds
/// - `exposure_time_ns`: Actual exposure time in nanoseconds
#[derive(Debug, Clone)]
pub struct FrameMetadata {
    pub frame_nr: i32,
    pub timestamp_bof_ns: u64,
    pub timestamp_eof_ns: u64,
    pub exposure_time_ns: u64,
    pub bit_depth: u16,
    pub roi_count: u16,
}

/// Page-aligned buffer for DMA performance (Gemini SDK review).
/// PVCAM DMA requires 4KB page alignment to avoid internal driver copies.
#[cfg(feature = "pvcam_sdk")]
pub struct PageAlignedBuffer {
    ptr: *mut u8,
    layout: Layout,
    len: usize,
}

// SAFETY: PageAlignedBuffer is Send because:
// - The raw pointer points to heap-allocated memory with no thread-local state
// - The buffer is only accessed through &mut self methods or via Arc<Mutex<>>
// - The underlying memory has no thread affinity
#[cfg(feature = "pvcam_sdk")]
unsafe impl Send for PageAlignedBuffer {}

// SAFETY: PageAlignedBuffer is Sync because:
// - All access is protected by Mutex<Option<PageAlignedBuffer>> in PvcamAcquisition
// - No &self methods expose the raw pointer for concurrent access
#[cfg(feature = "pvcam_sdk")]
unsafe impl Sync for PageAlignedBuffer {}

#[cfg(feature = "pvcam_sdk")]
impl PageAlignedBuffer {
    const PAGE_SIZE: usize = 4096;

    /// Allocate a page-aligned buffer of the given size.
    ///
    /// # Errors
    /// Returns an error if:
    /// - The layout is invalid (size/alignment combination rejected by allocator)
    /// - The allocation fails (out of memory)
    pub fn new(size: usize) -> Result<Self> {
        let layout = Layout::from_size_align(size, Self::PAGE_SIZE).map_err(|e| {
            anyhow!(
                "Invalid layout for page-aligned buffer (size={}, align={}): {}",
                size,
                Self::PAGE_SIZE,
                e
            )
        })?;
        // SAFETY: alloc_zeroed requirements are satisfied:
        // 1. `layout` has non-zero size (validated by from_size_align above)
        // 2. `layout.align()` is a power of two (PAGE_SIZE = 4096 = 2^12)
        // 3. Rounding up `layout.size()` to a multiple of alignment won't overflow
        //    (ensured by from_size_align validation)
        // The returned pointer is checked for null before use.
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            bail!(
                "Failed to allocate page-aligned buffer of {} bytes - out of memory",
                size
            );
        }
        Ok(Self {
            ptr,
            layout,
            len: size,
        })
    }

    /// Get a mutable pointer to the buffer for passing to PVCAM SDK.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    /// Get the buffer length in bytes.
    #[expect(
        dead_code,
        reason = "public API for buffer introspection; used in SDK acquisition paths"
    )]
    pub fn len(&self) -> usize {
        self.len
    }
}

#[cfg(feature = "pvcam_sdk")]
impl Drop for PageAlignedBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: dealloc requirements are satisfied:
            // 1. `self.ptr` was allocated by `alloc_zeroed` with the same allocator
            // 2. `self.layout` is the same Layout used for the original allocation
            // 3. The null check above ensures we don't dealloc a null pointer
            // 4. This is called only once (in Drop) and the struct is consumed
            unsafe {
                dealloc(self.ptr, self.layout);
            }
        }
    }
}
