#![allow(unsafe_code)]
//! Memory-mapped ring buffer for lock-free, zero-copy data streaming.
//!
//! This module implements a high-performance ring buffer backed by memory-mapped files,
//! designed for concurrent access with a single writer and multiple readers.
//!
//! # Features
//! - Lock-free operations using atomic instructions
//! - Zero-copy data access via memory mapping (single-process writer/reader model)
//! - Cross-language compatibility with #[repr(C)] layout
//! - Apache Arrow IPC format support for structured data
//! - Designed for 10k+ writes/sec throughput
//! - Live data tapping for remote visualization without disrupting HDF5 writing
//!
//! # Architecture
//!
//! The ring buffer uses a memory-mapped file with a fixed header followed by a circular
//! data region. Writers append data at the `write_head` position, while readers consume
//! from the `read_tail` position. When the write head reaches capacity, it wraps back
//! to the beginning (circular behavior).
//!
//! ## Data Tapping
//!
//! The ring buffer supports "tap consumers" that receive every Nth frame for live
//! visualization without blocking the primary HDF5 writer. Taps use async channels
//! with backpressure handling - if a tap consumer is slow, frames are dropped rather
//! than blocking the writer.
//!
//! # Thread Safety
//!
//! - **Writes**: Serialized via internal mutex. Multiple writers are safe but sequential.
//! - **Reads**: Lock-free using atomic loads with Acquire ordering.
//! - **Concurrent read/write**: Safe via seqlock pattern with epoch counter validation.
//! - **Process model**: Designed for a single process; cross-process synchronization is
//!   not provided because the tap registry and mutexes are process-local.
//! - **Taps**: Non-blocking send with automatic frame dropping on backpressure.

use anyhow::Context;
use common::error::{DaqError, StorageError, StorageErrorKind};
use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{fence, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::mpsc;

// Unix-specific imports for file permissions (bd-jnfu.16)
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::tap_registry::{TapHealth, TapRegistry};

#[cfg(feature = "storage_arrow")]
use arrow::record_batch::RecordBatch;

/// Magic number for ring buffer header validation
const MAGIC: u64 = 0xDA_DA_DA_DA_00_00_00_01;

/// Size of the ring buffer header in bytes (128 bytes = 2 cache lines on most systems)
const HEADER_SIZE: usize = 128;

/// Ring buffer header with cache-line alignment.
///
/// This structure uses #[repr(C)] to ensure a predictable memory layout
/// that can be accessed from other languages (Python, C++).
///
/// Layout (128 bytes total):
/// - magic: u64 (8 bytes)
/// - capacity_bytes: u64 (8 bytes)
/// - write_head: AtomicU64 (8 bytes)
/// - read_tail: AtomicU64 (8 bytes)
/// - write_epoch: AtomicU64 (8 bytes)
/// - schema_len: u32 (4 bytes)
/// - _reserved: u32 (4 bytes) - alignment padding
/// - stream_id: AtomicU64 (8 bytes) - incremented on buffer re-init for cross-process readers
/// - _padding: [u8; 72] (72 bytes)
///
/// Total: 8 + 8 + 8 + 8 + 8 + 4 + 4 + 8 + 72 = 128 bytes
#[repr(C)]
struct RingBufferHeader {
    /// Magic number for validation (0xDADADADA00000001)
    magic: u64,

    /// Total size of the data region in bytes
    capacity_bytes: u64,

    /// Current write position (monotonically increasing)
    write_head: AtomicU64,

    /// Oldest valid data position (for circular buffer management)
    read_tail: AtomicU64,

    /// Write epoch counter for seqlock synchronization
    ///
    /// Incremented before and after each write operation.
    /// Readers check this before and after reading - if it changed
    /// (or is odd during read), the read may have seen partial data.
    write_epoch: AtomicU64,

    /// Length of the Arrow schema JSON (if using Arrow format)
    schema_len: u32,

    /// Reserved for alignment
    _reserved: u32,

    /// Stream identifier for cross-process readers.
    ///
    /// Incremented each time the buffer is re-initialized. Cross-process
    /// readers (Python/Julia via mmap) can detect buffer re-creation by
    /// comparing this value to their cached version.
    stream_id: AtomicU64,

    /// Padding to align header to 128 bytes (cache line boundary)
    /// Calculation: 128 - (8 + 8 + 8 + 8 + 8 + 4 + 4 + 8) = 128 - 56 = 72 bytes
    _padding: [u8; 72],
}

// Static assertion to ensure header size matches HEADER_SIZE constant
const _: () = assert!(
    std::mem::size_of::<RingBufferHeader>() == HEADER_SIZE,
    "RingBufferHeader size must equal HEADER_SIZE (128 bytes)"
);

/// High-performance memory-mapped ring buffer.
///
/// # Safety
/// This structure contains raw pointers to memory-mapped regions. It is safe to use
/// as long as:
/// - The memory-mapped file remains valid for the lifetime of RingBuffer
/// - Readers use appropriate atomic ordering (Acquire)
/// - Writers use appropriate atomic ordering (Release)
///
/// # Thread Safety
/// The buffer uses an internal write lock to serialize concurrent writes, making it
/// safe for multiple writers. Reads remain lock-free.
pub struct RingBuffer {
    /// Filesystem path to the backing file
    path: PathBuf,

    /// Memory-mapped file backing the ring buffer
    mmap: MmapMut,

    /// Pointer to the header structure
    /// SAFETY: Points to the start of mmap, valid as long as mmap exists
    header: *mut RingBufferHeader,

    /// Pointer to the data region (after header)
    /// SAFETY: Points to HEADER_SIZE bytes into mmap, valid as long as mmap exists
    data_ptr: *mut u8,

    /// Capacity of the data region in bytes
    capacity: u64,

    /// Data access lock for memory-safe concurrent access (bd-t17q).
    ///
    /// This RwLock ensures readers and writers don't access the same memory
    /// region concurrently, preventing data races (undefined behavior).
    ///
    /// - Writers take an exclusive write lock
    /// - Readers take a shared read lock
    /// - Multiple readers can read concurrently when no writer is active
    ///
    /// Note: The seqlock (write_epoch) is still used for optimistic read retries
    /// after data changes, but the RwLock prevents actual concurrent access.
    data_lock: RwLock<()>,

    /// Registry for live data taps
    taps: Arc<TapRegistry>,

    /// Arrow schema JSON for cross-process readers (bd-1il7).
    ///
    /// Stored in-memory when first Arrow batch is written. Cross-process
    /// readers can retrieve this via gRPC `get_ring_buffer_tap_info` to
    /// decode Arrow IPC data without parsing embedded schema.
    #[cfg(feature = "storage_arrow")]
    arrow_schema_json: RwLock<Option<String>>,
}

impl std::fmt::Debug for RingBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tap_count = self.taps.count();
        f.debug_struct("RingBuffer")
            .field("capacity", &self.capacity)
            .field("write_head", &self.write_head())
            .field("read_tail", &self.read_tail())
            .field("tap_count", &tap_count)
            .field("data_ptr", &format!("{:p}", self.data_ptr))
            .field("header", &format!("{:p}", self.header))
            .finish()
    }
}

// SAFETY: RingBuffer can be safely sent to other threads because:
// 1. It owns its mmap (MmapMut) which is itself Send
// 2. Raw pointers (header, data_ptr) point into the mmap and remain valid
//    as long as the mmap exists - they are derived from mmap.as_mut_ptr()
// 3. All pointer dereferences are guarded by:
//    - Bounds checks (capacity validation, debug_assert! in hot paths)
//    - Atomic ordering (Acquire/Release for cross-thread visibility)
// 4. The TapRegistry is wrapped in Arc and is itself Send+Sync
unsafe impl Send for RingBuffer {}

// SAFETY (bd-t17q, updated bd-qa36.6.8): RingBuffer can be safely shared across
// threads because:
// 1. **Primary safety mechanism:** `data_lock` (std::sync::RwLock) serializes all
//    data access:
//    - Writers take exclusive write lock, preventing concurrent reads/writes
//    - Readers take shared read lock, allowing concurrent reads but blocking writers
//    - This alone is sufficient to prevent data races on the mmap region.
// 2. Readers use Acquire ordering on atomic loads to see published writes
// 3. Writers use Release ordering after memcpy to publish data
// 4. **Defensive layer:** The seqlock (write_epoch) is a *consistency validator*,
//    not the primary concurrency mechanism. Since bd-t17q added data_lock, the
//    seqlock no longer prevents data races — the RwLock does. The seqlock now
//    serves two purposes:
//    a. Crash detection: an odd epoch indicates a write was in progress when the
//       process crashed (incomplete memcpy → potentially corrupt data).
//    b. Future optimization: if a lock-free read path is added later, the seqlock
//       provides the necessary retry mechanism for torn reads.
//    Writers still increment epoch (odd) before writing, (even) after.
// 5. Memory ordering: Release on write_head/epoch ensures prior memcpy is visible
//    to readers who Acquire load these values
// 6. The TapRegistry uses internal synchronization (RwLock)
//
// INVARIANTS that must be maintained:
// - data_lock MUST be held (write mode for writers, read mode for readers)
// - write_epoch MUST be incremented before and after memcpy with proper ordering
// - Capacity bounds MUST be validated before any pointer arithmetic
unsafe impl Sync for RingBuffer {}

impl RingBuffer {
    /// Create a new ring buffer backed by a memory-mapped file.
    ///
    /// # Arguments
    /// * `path` - Path to the backing file (typically in /dev/shm or /tmp)
    /// * `capacity_mb` - Size of the data region in megabytes
    ///
    /// # Returns
    /// A new `RingBuffer` instance with initialized header
    ///
    /// # Example
    /// ```no_run
    /// use std::path::Path;
    /// use storage::ring_buffer::RingBuffer;
    ///
    /// let rb = RingBuffer::create(Path::new("/tmp/my_ring_buffer"), 100).unwrap();
    /// ```
    pub fn create(path: &Path, capacity_mb: usize) -> Result<Self, DaqError> {
        // Validate capacity_mb is not zero
        if capacity_mb == 0 {
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                "Ring buffer capacity must be at least 1 MB, got 0 MB",
            )));
        }

        // Upper bounds check: maximum 16 GB to prevent allocation failures
        const MAX_CAPACITY_MB: usize = 16 * 1024; // 16 GB
        if capacity_mb > MAX_CAPACITY_MB {
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                format!(
                    "Ring buffer capacity exceeds maximum ({}), got {} MB",
                    MAX_CAPACITY_MB, capacity_mb
                ),
            )));
        }

        // Safely compute capacity_bytes with overflow detection
        let capacity_bytes = capacity_mb
            .checked_mul(1024)
            .and_then(|v| v.checked_mul(1024))
            .ok_or_else(|| {
                DaqError::Storage(StorageError::new(
                    StorageErrorKind::Configuration,
                    format!(
                        "Capacity calculation overflowed: {} MB exceeds maximum allocatable size",
                        capacity_mb
                    ),
                ))
            })?;

        let total_size = capacity_bytes.checked_add(HEADER_SIZE).ok_or_else(|| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                format!(
                    "Total size calculation overflowed after adding header ({} bytes)",
                    HEADER_SIZE
                ),
            ))
        })?;

        // Create or open the backing file
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true);

        // Set restrictive permissions on Unix to prevent data leakage (bd-jnfu.16)
        // Ring buffer files in /tmp or /dev/shm would otherwise be world-readable
        #[cfg(unix)]
        opts.mode(0o600); // Owner read/write only

        let is_new_file = !path.exists();

        // Only truncate when creating a brand-new buffer; preserve existing data otherwise.
        if is_new_file {
            opts.truncate(true);
        }

        let file = opts.open(path).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Io,
                format!("Failed to create/open ring buffer file {:?}: {}", path, e),
            ))
        })?;

        // Validate existing file size or set for new file
        let existing_size = file.metadata().map_err(DaqError::Io)?.len();

        if is_new_file || existing_size == 0 {
            // Set file size for new buffer or empty file
            file.set_len(total_size as u64).map_err(DaqError::Io)?;
        } else if existing_size != total_size as u64 {
            // Existing buffer with data has different capacity - this would corrupt data
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                format!(
                    "Ring buffer capacity mismatch: file has {} bytes but requested {} bytes. \
                 Delete the existing file or use matching capacity.",
                    existing_size, total_size
                ),
            )));
        }

        // Create memory mapping
        // SAFETY: We just created the file and set its size, so mapping is safe
        let mut mmap = unsafe {
            MmapOptions::new().map_mut(&file).map_err(|e| {
                DaqError::Storage(StorageError::new(
                    StorageErrorKind::Io,
                    format!("Failed to create memory mapping: {}", e),
                ))
            })?
        };
        debug_assert!(mmap.len() >= total_size, "mmap shorter than requested size");

        // Initialize header
        debug_assert!(mmap.len() >= HEADER_SIZE, "mmap shorter than header");
        // SAFETY: mmap is at least HEADER_SIZE bytes (total_size includes HEADER_SIZE)
        let header = mmap.as_mut_ptr() as *mut RingBufferHeader;
        unsafe {
            // Write header fields
            (*header).magic = MAGIC;
            (*header).capacity_bytes = capacity_bytes as u64;
            (*header).write_head = AtomicU64::new(0);
            (*header).read_tail = AtomicU64::new(0);
            (*header).write_epoch = AtomicU64::new(0);
            (*header).schema_len = 0;
            (*header)._reserved = 0;

            // Generate stream_id from timestamp for cross-process reader detection.
            // Using system time ensures uniqueness across buffer re-creations.
            let stream_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1);
            (*header).stream_id = AtomicU64::new(stream_id);

            // Zero out padding for deterministic behavior
            (*header)._padding.fill(0);
        }

        // Calculate data region pointer
        debug_assert!(HEADER_SIZE <= mmap.len());
        // SAFETY: mmap is total_size bytes, so offset HEADER_SIZE is within bounds
        let data_ptr = unsafe { mmap.as_mut_ptr().add(HEADER_SIZE) };

        Ok(Self {
            path: path.to_path_buf(),
            mmap,
            header,
            data_ptr,
            capacity: capacity_bytes as u64,
            data_lock: RwLock::new(()),
            taps: Arc::new(TapRegistry::new()),
            #[cfg(feature = "storage_arrow")]
            arrow_schema_json: RwLock::new(None),
        })
    }

    /// Open an existing ring buffer from a memory-mapped file.
    ///
    /// # Arguments
    /// * `path` - Path to the existing backing file
    ///
    /// # Returns
    /// A `RingBuffer` instance attached to the existing buffer
    ///
    /// # Safety
    /// This function validates the file size and magic number BEFORE accessing
    /// header fields to prevent undefined behavior from corrupted files.
    pub fn open(path: &Path) -> Result<Self, DaqError> {
        let file = OpenOptions::new().read(true).write(true).open(path).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Io,
                format!("Failed to open ring buffer file {:?}: {}", path, e),
            ))
        })?;

        // CRITICAL: Validate file size BEFORE memory mapping to prevent
        // accessing invalid memory if the file was truncated or corrupted
        let file_size = file.metadata().map_err(DaqError::Io)?.len() as usize;

        if file_size < HEADER_SIZE {
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                format!(
                    "Ring buffer file too small: {} bytes, minimum {} bytes required for header",
                    file_size, HEADER_SIZE
                ),
            )));
        }

        // Create memory mapping
        // SAFETY: We verified file_size >= HEADER_SIZE, so the mmap will be at least
        // HEADER_SIZE bytes and header access will be within bounds
        let mut mmap = unsafe {
            MmapOptions::new().map_mut(&file).map_err(|e| {
                DaqError::Storage(StorageError::new(
                    StorageErrorKind::Io,
                    format!("Failed to map ring buffer file: {}", e),
                ))
            })?
        };

        // Double-check mmap size matches file size (defensive programming)
        if mmap.len() < HEADER_SIZE {
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                format!(
                    "Memory map too small: {} bytes, expected at least {} bytes",
                    mmap.len(),
                    HEADER_SIZE
                ),
            )));
        }

        // SAFETY: We verified mmap.len() >= HEADER_SIZE above, so casting to
        // RingBufferHeader pointer and accessing magic/capacity is safe
        let header = mmap.as_mut_ptr() as *mut RingBufferHeader;
        let (magic, capacity) = unsafe { ((*header).magic, (*header).capacity_bytes) };

        // Validate magic number to detect corrupted files
        if magic != MAGIC {
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                format!(
                    "Invalid ring buffer magic number: expected 0x{:016X}, got 0x{:016X}. \
                 File may be corrupted or not a valid ring buffer.",
                    MAGIC, magic
                ),
            )));
        }

        // Validate that file size matches expected size (header + capacity)
        let expected_size = HEADER_SIZE + capacity as usize;
        if file_size != expected_size {
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                format!(
                    "Ring buffer file size mismatch: file is {} bytes but header indicates \
                 {} bytes capacity (expected {} total bytes). File may be truncated or corrupted.",
                    file_size, capacity, expected_size
                ),
            )));
        }

        // SAFETY: We verified mmap.len() == expected_size == HEADER_SIZE + capacity,
        // so data_ptr = mmap + HEADER_SIZE is within bounds and points to a region
        // of exactly 'capacity' bytes
        let data_ptr = unsafe { mmap.as_mut_ptr().add(HEADER_SIZE) };

        Ok(Self {
            path: path.to_path_buf(),
            mmap,
            header,
            data_ptr,
            capacity,
            data_lock: RwLock::new(()),
            taps: Arc::new(TapRegistry::new()),
            #[cfg(feature = "storage_arrow")]
            arrow_schema_json: RwLock::new(None),
        })
    }

    /// Write data to the ring buffer.
    ///
    /// This operation takes an exclusive write lock on the data region,
    /// preventing concurrent reads/writes and ensuring memory safety (bd-t17q).
    ///
    /// After writing, all registered tap consumers are notified. Taps that
    /// can't keep up will have frames dropped (non-blocking send).
    ///
    /// # Arguments
    /// * `data` - Byte slice to write
    ///
    /// # Returns
    /// Ok(()) on success, Err if data is too large for buffer
    ///
    /// # Note
    /// If the buffer is full, this will overwrite the oldest data (circular behavior)
    ///
    /// # Thread Safety
    /// Multiple concurrent writers are safe - the internal lock serializes writes.
    pub fn write(&self, data: &[u8]) -> Result<(), DaqError> {
        // Acquire exclusive write lock to prevent concurrent reads/writes (bd-t17q)
        let _guard = self
            .data_lock
            .write()
            .map_err(|_| DaqError::Storage(StorageError::new(StorageErrorKind::Other, "Data lock poisoned")))?;

        let len = data.len() as u64;

        if len > self.capacity {
            return Err(DaqError::Storage(StorageError::new(
                StorageErrorKind::RingBuffer,
                format!(
                    "Data size ({} bytes) exceeds ring buffer capacity ({} bytes)",
                    len, self.capacity
                ),
            )));
        }

        // SAFETY: header is valid for the lifetime of self, and data_ptr points to a
        // valid mmap region of size self.capacity bytes
        unsafe {
            debug_assert!(HEADER_SIZE + self.capacity as usize <= self.mmap.len());
            debug_assert!(data.len() <= self.capacity as usize);

            // Increment epoch BEFORE write (odd = write in progress)
            // Use AcqRel to prevent the memcpy from floating up before this increment
            (*self.header).write_epoch.fetch_add(1, Ordering::AcqRel);

            // Load current write position with Acquire ordering to see previous writes
            let head = (*self.header).write_head.load(Ordering::Acquire);

            // Calculate circular offset
            let write_offset = (head % self.capacity) as usize;
            debug_assert!(write_offset < self.capacity as usize);

            // Handle wrap-around: if data doesn't fit before end, split the write
            if write_offset + data.len() > self.capacity as usize {
                // SAFETY: write_offset < capacity (due to modulo), and first_part_len
                // is bounded by capacity - write_offset, so data_ptr.add(write_offset)
                // is within the mmap region [data_ptr, data_ptr + capacity)
                let first_part_len = self.capacity as usize - write_offset;
                let dest = self.data_ptr.add(write_offset);
                std::ptr::copy_nonoverlapping(data.as_ptr(), dest, first_part_len);

                // SAFETY: second_part_len = data.len() - first_part_len, and we already
                // checked data.len() <= capacity, so second_part_len <= capacity.
                // Writing to data_ptr (start of region) is always valid.
                let second_part_len = data.len() - first_part_len;
                std::ptr::copy_nonoverlapping(
                    data.as_ptr().add(first_part_len),
                    self.data_ptr,
                    second_part_len,
                );
            } else {
                // SAFETY: write_offset + data.len() <= capacity (checked above),
                // so the entire write range is within the mmap data region
                let dest = self.data_ptr.add(write_offset);
                std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
            }

            // Update write head with Release ordering to publish the write
            (*self.header).write_head.fetch_add(len, Ordering::Release);

            // Increment epoch AFTER write (even = write complete)
            (*self.header).write_epoch.fetch_add(1, Ordering::Release);
        }

        // Notify tap consumers (non-blocking)
        // We do this AFTER the write is complete to avoid data races
        self.notify_taps(data);

        Ok(())
    }

    /// Notify all tap consumers about a new frame.
    ///
    /// This method delegates to the TapRegistry to handle notification.
    ///
    /// # Arguments
    /// * `data` - The frame data to send to taps
    fn notify_taps(&self, data: &[u8]) {
        self.taps.notify_all(data);
    }

    /// Register a new tap consumer to receive every Nth frame.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this tap
    /// * `nth_frame` - Deliver every nth frame (1 = every frame, 10 = every 10th)
    ///
    /// # Returns
    /// A receiver that will receive frame data, or an error if tap already exists
    ///
    /// # Example
    /// ```no_run
    /// # use std::path::Path;
    /// # use storage::ring_buffer::RingBuffer;
    /// # async fn example() -> anyhow::Result<()> {
    /// let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 10)?;
    ///
    /// // Register a tap to receive every 10th frame
    /// let mut rx = rb.register_tap("preview".to_string(), 10)?;
    ///
    /// // Receive frames in a separate task
    /// tokio::spawn(async move {
    ///     while let Some(frame) = rx.recv().await {
    ///         // Process frame for live preview
    ///         println!("Received frame: {} bytes", frame.len());
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_tap(
        &self,
        id: String,
        nth_frame: usize,
    ) -> Result<mpsc::Receiver<Vec<u8>>, DaqError> {
        let rx = self.taps.register(id.clone(), nth_frame).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::RingBuffer,
                format!("Failed to register tap '{}': {}", id, e),
            ))
        })?;

        tracing::info!("Registered tap '{}' (every {}th frame)", id, nth_frame);

        Ok(rx)
    }

    /// Update the tap channel capacity for future registrations.
    pub fn set_tap_channel_capacity(&self, capacity: usize) {
        self.taps.set_channel_capacity(capacity);
    }

    /// Snapshot tap health metrics for all active taps.
    pub fn tap_health(&self) -> Vec<TapHealth> {
        self.taps.tap_health()
    }

    /// Snapshot tap health for a specific tap.
    pub fn tap_health_for(&self, id: &str) -> Option<TapHealth> {
        self.taps.tap_health_for(id)
    }

    /// Unregister a tap consumer.
    ///
    /// # Arguments
    /// * `id` - The tap ID to remove
    ///
    /// # Returns
    /// Ok(true) if tap was found and removed, Ok(false) if tap didn't exist
    ///
    /// # Example
    /// ```no_run
    /// # use std::path::Path;
    /// # use storage::ring_buffer::RingBuffer;
    /// # fn example() -> anyhow::Result<()> {
    /// let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 10)?;
    /// let _rx = rb.register_tap("preview".to_string(), 10)?;
    ///
    /// // Later, when preview is no longer needed
    /// rb.unregister_tap("preview")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn unregister_tap(&self, id: &str) -> Result<bool, DaqError> {
        let removed = self.taps.unregister(id).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::RingBuffer,
                format!("Failed to unregister tap '{}': {}", id, e),
            ))
        })?;

        if removed {
            tracing::info!("Unregistered tap '{}'", id);
        }

        Ok(removed)
    }

    /// Get the number of currently registered taps.
    ///
    /// # Returns
    /// Number of active tap consumers
    pub fn tap_count(&self) -> usize {
        self.taps.count()
    }

    /// Get information about all registered taps.
    ///
    /// # Returns
    /// Vector of (tap_id, nth_frame) tuples
    pub fn list_taps(&self) -> Vec<(String, usize)> {
        self.taps.list()
    }

    /// Read a snapshot of current data in the ring buffer.
    ///
    /// This creates a copy of the available data from read_tail to write_head.
    /// Safe for concurrent use with write operations - uses seqlock validation
    /// to detect and retry if a write occurred during the read.
    ///
    /// # Returns
    /// A Vec containing a snapshot of the current buffer contents
    ///
    /// # Note
    /// This method will retry automatically if a concurrent write is detected.
    /// If the writer crashes mid-write (leaving epoch odd), this will timeout
    /// after 500ms (configurable via DAQ_RINGBUFFER_TIMEOUT_MS) and return an empty Vec.
    ///
    /// # Performance (bd-jnfu.8)
    /// Uses exponential backoff to reduce CPU usage during contention:
    /// - Retries 0-9: spin_loop() for sub-microsecond latency
    /// - Retries 10+: sleep with progressive backoff (10µs, 100µs, 1ms, 10ms)
    ///
    /// This significantly reduces CPU usage compared to pure spin-waiting while
    /// maintaining low latency for the common fast-path case.
    ///
    /// # Async Warning (bd-jnfu.14)
    /// This method uses `std::thread::sleep()` during backoff, which BLOCKS the
    /// calling thread. When calling from async contexts (Tokio tasks), wrap this
    /// call in `tokio::task::spawn_blocking()` to avoid stalling the runtime:
    ///
    /// ```ignore
    /// let ring_buffer = ring_buffer.clone();
    /// let snapshot = tokio::task::spawn_blocking(move || ring_buffer.read_snapshot())
    ///     .await
    ///     .expect("blocking task panicked");
    /// ```
    pub fn read_snapshot(&self) -> Vec<u8> {
        // Take shared read lock to prevent concurrent writes during byte copies.
        // Multiple readers can hold the lock concurrently, but writers get exclusive access.
        // This prevents undefined behavior from non-atomic byte copies racing with writes.
        let _guard = match self.data_lock.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("Data lock poisoned in read_snapshot, recovering");
                poisoned.into_inner()
            }
        };

        const MAX_RETRIES: usize = 500;
        // Configurable timeout via DAQ_RINGBUFFER_TIMEOUT_MS (default: 500ms)
        // Increased from 100ms to handle high-throughput camera streaming
        let timeout_ms: u128 = std::env::var("DAQ_RINGBUFFER_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);

        let start_time = Instant::now();

        for retry in 0..MAX_RETRIES {
            // Check for timeout (handles crashed writer scenario where epoch stays odd)
            if start_time.elapsed().as_millis() > timeout_ms {
                tracing::warn!(
                    "read_snapshot timed out after {}ms - high write contention or crashed writer (epoch stuck odd). \
                     Set DAQ_RINGBUFFER_TIMEOUT_MS to increase timeout for high-throughput streaming.",
                    timeout_ms
                );
                return Vec::new();
            }

            // Try to perform a consistent read (optimistic concurrency control)
            if let Some(buffer) = self.try_read_consistent() {
                return buffer;
            }

            // Backoff if consistent read failed due to concurrent write
            // bd-jnfu.8: Use progressive backoff to reduce CPU during contention
            Self::backoff_sleep(retry);
        }

        // After max retries, return empty (caller should handle high contention)
        tracing::warn!(
            "read_snapshot exceeded {} retries due to high write contention",
            MAX_RETRIES
        );
        Vec::new()
    }

    /// Attempt a single consistent read of the ring buffer.
    ///
    /// Returns `Some(Vec<u8>)` if successful, or `None` if a concurrent write occurred.
    /// This encapsulates the unsafe pointer logic and seqlock validation.
    #[inline(always)]
    fn try_read_consistent(&self) -> Option<Vec<u8>> {
        // SAFETY: header and data_ptr are valid for the lifetime of self
        unsafe {
            debug_assert!(HEADER_SIZE + self.capacity as usize <= self.mmap.len());

            // Load epoch BEFORE read (must be even = no write in progress)
            let epoch_before = (*self.header).write_epoch.load(Ordering::Acquire);

            // If epoch is odd, a write is in progress - abort immediately
            if !epoch_before.is_multiple_of(2) {
                return None;
            }

            // Load positions with Acquire ordering to see all previous writes
            let head = (*self.header).write_head.load(Ordering::Acquire);
            let tail = (*self.header).read_tail.load(Ordering::Acquire);

            // Calculate effective tail: if head is too far ahead, we must skip overwritten data
            let effective_tail = std::cmp::max(tail, head.saturating_sub(self.capacity));

            // Calculate available data
            let available = head.saturating_sub(effective_tail);

            if available == 0 {
                return Some(Vec::new());
            }

            // Validate available size against reasonable limits (defensive bounds check)
            if available > self.capacity {
                // This should be impossible due to saturating_sub logic above, but safety first
                tracing::error!("available bytes {} > capacity {}", available, self.capacity);
                return Some(Vec::new());
            }

            // Calculate read offset (circular) based on EFFECTIVE tail
            let read_offset = (effective_tail % self.capacity) as usize;

            let mut buffer = vec![0u8; available as usize];

            // Handle wrap-around copy
            if read_offset + available as usize > self.capacity as usize {
                // Split copy: end of buffer + start of buffer
                let first_part_len = self.capacity as usize - read_offset;

                // Bounds check for first part
                if first_part_len > 0 {
                    let src = self.data_ptr.add(read_offset);
                    std::ptr::copy_nonoverlapping(src, buffer.as_mut_ptr(), first_part_len);
                }

                // Bounds check for second part
                let second_part_len = available as usize - first_part_len;
                if second_part_len > 0 {
                    std::ptr::copy_nonoverlapping(
                        self.data_ptr,
                        buffer.as_mut_ptr().add(first_part_len),
                        second_part_len,
                    );
                }
            } else {
                // Contiguous copy
                let src = self.data_ptr.add(read_offset);
                std::ptr::copy_nonoverlapping(src, buffer.as_mut_ptr(), available as usize);
            }

            // Fence to ensure all data reads complete before loading epoch
            // Required for ARM/Apple Silicon where loads can be reordered
            fence(Ordering::SeqCst);

            // Load epoch AFTER read - must match epoch_before
            let epoch_after = (*self.header).write_epoch.load(Ordering::Acquire);

            if epoch_before == epoch_after {
                Some(buffer)
            } else {
                None
            }
        }
    }
    /// Progressive backoff sleep to reduce CPU usage during contention (bd-jnfu.8)
    ///
    /// Uses a tiered approach:
    /// - Retries 0-9: spin_loop() for sub-microsecond latency (fast path)
    /// - Retries 10-19: sleep 10µs (short waits for transient contention)
    /// - Retries 20-49: sleep 100µs (medium waits)
    /// - Retries 50-99: sleep 1ms (longer waits for sustained contention)
    /// - Retries 100+: sleep 10ms (yield heavily for pathological cases)
    ///
    /// This significantly reduces CPU usage compared to pure spin-waiting
    /// while maintaining low latency for the common case.
    #[inline]
    fn backoff_sleep(retry: usize) {
        use std::time::Duration;

        match retry {
            0..=9 => std::hint::spin_loop(),
            10..=19 => std::thread::sleep(Duration::from_micros(10)),
            20..=49 => std::thread::sleep(Duration::from_micros(100)),
            50..=99 => std::thread::sleep(Duration::from_millis(1)),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }

    /// Get the memory address of the data region for external mapping (Python/C++).
    ///
    /// This is useful for zero-copy access from other languages.
    ///
    /// # Returns
    /// The memory address of the data region as a usize
    pub fn data_address(&self) -> usize {
        self.data_ptr as usize
    }

    /// Get the capacity of the data region in bytes.
    ///
    /// This returns the maximum amount of data that can be stored in the ring buffer
    /// before wrap-around occurs. The actual file size is larger by [`HEADER_SIZE`]
    /// bytes (128 bytes for the header).
    ///
    /// # Returns
    ///
    /// The capacity in bytes as specified during creation.
    ///
    /// # Thread Safety
    ///
    /// This method is safe to call from multiple threads concurrently. The capacity
    /// is immutable after buffer creation.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use storage::ring_buffer::RingBuffer;
    /// # fn main() -> anyhow::Result<()> {
    /// let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 100)?;
    /// assert_eq!(rb.capacity(), 100 * 1024 * 1024); // 100 MB
    /// # Ok(())
    /// # }
    /// ```
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Get the filesystem path to the backing file.
    ///
    /// This path is needed for cross-process readers (Python/Julia) to mmap
    /// the same file and access the ring buffer data directly.
    ///
    /// # Returns
    ///
    /// The path to the memory-mapped backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the stream identifier for cross-process reader synchronization.
    ///
    /// The stream ID is a unique value generated when the buffer is created,
    /// based on system timestamp. Cross-process readers (Python/Julia via mmap)
    /// should cache this value and periodically check if it has changed to
    /// detect buffer re-initialization.
    ///
    /// # Returns
    ///
    /// A unique 64-bit identifier for this buffer instance.
    ///
    /// # Thread Safety
    ///
    /// Uses atomic load with Acquire ordering. Safe to call concurrently.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use storage::ring_buffer::RingBuffer;
    /// # fn main() -> anyhow::Result<()> {
    /// let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 100)?;
    /// let stream_id = rb.stream_id();
    /// // Python reader can compare this to detect buffer recreation
    /// assert!(stream_id > 0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn stream_id(&self) -> u64 {
        // SAFETY: header is valid for the lifetime of self
        unsafe { (*self.header).stream_id.load(Ordering::Acquire) }
    }

    /// Get the magic number for buffer validation.
    ///
    /// Returns the constant 0xDADADADA00000001. Cross-process readers should
    /// verify this value to ensure they're reading a valid ring buffer.
    pub fn magic(&self) -> u64 {
        // SAFETY: header is valid for the lifetime of self
        unsafe { (*self.header).magic }
    }

    /// Get the write epoch for seqlock synchronization.
    ///
    /// The write epoch is incremented before and after each write. Readers
    /// should check this before and after reading - if it changed (or is
    /// odd during read), the read may have seen partial data.
    pub fn write_epoch(&self) -> u64 {
        // SAFETY: header is valid for the lifetime of self
        unsafe { (*self.header).write_epoch.load(Ordering::Acquire) }
    }

    /// Get the current write head position (monotonically increasing).
    ///
    /// The write head tracks the total number of bytes written to the buffer since
    /// creation. It increases monotonically and never wraps. To get the actual offset
    /// in the circular buffer, compute `write_head % capacity`.
    ///
    /// # Returns
    ///
    /// Total bytes written since buffer creation.
    ///
    /// # Thread Safety
    ///
    /// Uses atomic load with Acquire ordering to ensure visibility of all writes
    /// that happened-before this load. Safe to call concurrently with writes.
    ///
    /// # Producer/Consumer Semantics
    ///
    /// - Available data = `write_head - read_tail` (capped at capacity)
    /// - If `write_head == read_tail`, the buffer is empty
    /// - If `write_head - read_tail > capacity`, old data has been overwritten
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use storage::ring_buffer::RingBuffer;
    /// # fn main() -> anyhow::Result<()> {
    /// let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 1)?;
    ///
    /// rb.write(b"test")?;
    /// assert_eq!(rb.write_head(), 4);
    ///
    /// rb.write(b"data")?;
    /// assert_eq!(rb.write_head(), 8); // Monotonically increasing
    /// # Ok(())
    /// # }
    /// ```
    pub fn write_head(&self) -> u64 {
        // SAFETY: header is valid for the lifetime of self
        unsafe { (*self.header).write_head.load(Ordering::Acquire) }
    }

    /// Get the current read tail position (marks oldest unconsumed data).
    ///
    /// The read tail tracks the oldest data position that has not yet been consumed
    /// by readers. Like the write head, it increases monotonically and never wraps.
    /// To get the actual offset in the circular buffer, compute `read_tail % capacity`.
    ///
    /// # Returns
    ///
    /// Position of the oldest unconsumed byte.
    ///
    /// # Thread Safety
    ///
    /// Uses atomic load with Acquire ordering. Safe to call concurrently with
    /// `update_read_tail()` and `advance_tail()` operations.
    ///
    /// # Producer/Consumer Semantics
    ///
    /// - Consumers should call `update_read_tail()` or `advance_tail()` after
    ///   processing data to free up buffer space
    /// - The tail is managed by consumers; the buffer itself only updates it
    ///   via explicit calls
    /// - If not updated, old data will eventually be overwritten when the
    ///   write head laps the tail
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use storage::ring_buffer::RingBuffer;
    /// # fn main() -> anyhow::Result<()> {
    /// let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 1)?;
    ///
    /// rb.write(b"test")?;
    /// assert_eq!(rb.read_tail(), 0); // No data consumed yet
    ///
    /// let snapshot = rb.read_snapshot();
    /// rb.advance_tail(snapshot.len() as u64);
    /// assert_eq!(rb.read_tail(), 4); // Tail advanced after consumption
    /// # Ok(())
    /// # }
    /// ```
    pub fn read_tail(&self) -> u64 {
        // SAFETY: header is valid for the lifetime of self
        unsafe { (*self.header).read_tail.load(Ordering::Acquire) }
    }

    /// Update the read tail position (mark data as consumed).
    ///
    /// This should be called by consumers after processing data to free up space.
    pub fn update_read_tail(&self, new_tail: u64) {
        // SAFETY: header is valid for the lifetime of self
        unsafe {
            (*self.header).read_tail.store(new_tail, Ordering::Release);
        }
    }

    /// Advance the read tail by a number of bytes (convenience wrapper).
    ///
    /// This is a helper for consumers who want to mark data as consumed
    /// by advancing the tail relative to its current position.
    pub fn advance_tail(&self, bytes: u64) {
        // SAFETY: header is valid for the lifetime of self
        unsafe {
            (*self.header).read_tail.fetch_add(bytes, Ordering::Release);
        }
    }

    /// Get the Arrow schema JSON for cross-process readers (bd-1il7).
    ///
    /// Returns the schema JSON if Arrow data has been written to this buffer.
    /// Cross-process readers (Python/Julia via mmap) can use this to decode
    /// Arrow IPC data without parsing the embedded schema.
    ///
    /// # Returns
    /// `Some(schema_json)` if Arrow data has been written, `None` otherwise.
    #[cfg(feature = "storage_arrow")]
    pub fn arrow_schema_json(&self) -> Option<String> {
        self.arrow_schema_json
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Get the schema length stored in the header.
    ///
    /// This indicates whether an Arrow schema is associated with this buffer.
    /// A non-zero value means `arrow_schema_json()` should return Some.
    pub fn schema_len(&self) -> u32 {
        // SAFETY: header is valid for the lifetime of self
        unsafe { (*self.header).schema_len }
    }
}

// =============================================================================
// Async Wrapper for Tokio Runtime Safety (bd-cmcp)
// =============================================================================

/// Async-safe wrapper for RingBuffer that enforces spawn_blocking usage.
///
/// This wrapper prevents accidentally calling blocking RingBuffer methods
/// directly from async contexts. All operations that may block
/// (like `read_snapshot`) are automatically wrapped in `tokio::task::spawn_blocking`.
///
/// # Why This Wrapper Exists
///
/// RingBuffer uses `std::thread::sleep()` in its backoff logic during contention.
/// When called directly from async contexts, this blocks the entire tokio worker thread,
/// which can stall the entire async runtime. This wrapper enforces the use of
/// `spawn_blocking` at compile time, making it impossible to accidentally block
/// the async runtime.
///
/// # Example
///
/// ```no_run
/// # use std::path::Path;
/// # use std::sync::Arc;
/// # use storage::ring_buffer::{RingBuffer, AsyncRingBuffer};
/// # async fn example() -> anyhow::Result<()> {
/// let rb = Arc::new(RingBuffer::create(Path::new("/tmp/test.buf"), 10)?);
/// let async_rb = AsyncRingBuffer::new(rb);
///
/// // This automatically uses spawn_blocking internally
/// let snapshot = async_rb.read_snapshot().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AsyncRingBuffer {
    inner: Arc<RingBuffer>,
}

impl AsyncRingBuffer {
    /// Create a new AsyncRingBuffer from a RingBuffer.
    ///
    /// # Arguments
    /// * `ring_buffer` - The underlying RingBuffer to wrap
    ///
    /// # Example
    /// ```no_run
    /// # use std::path::Path;
    /// # use std::sync::Arc;
    /// # use storage::ring_buffer::{RingBuffer, AsyncRingBuffer};
    /// # fn example() -> anyhow::Result<()> {
    /// let rb = Arc::new(RingBuffer::create(Path::new("/tmp/test.buf"), 10)?);
    /// let async_rb = AsyncRingBuffer::new(rb);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(ring_buffer: Arc<RingBuffer>) -> Self {
        Self { inner: ring_buffer }
    }

    /// Read a snapshot of current data in the ring buffer (async-safe).
    ///
    /// This method automatically uses `tokio::task::spawn_blocking` to ensure
    /// the blocking backoff logic doesn't stall the async runtime.
    ///
    /// # Returns
    /// Ok(Vec) containing a snapshot of the current buffer contents,
    /// or Err if the blocking task panicked or was cancelled.
    ///
    /// # Example
    /// ```no_run
    /// # use std::path::Path;
    /// # use std::sync::Arc;
    /// # use storage::ring_buffer::{RingBuffer, AsyncRingBuffer};
    /// # async fn example() -> anyhow::Result<()> {
    /// let rb = Arc::new(RingBuffer::create(Path::new("/tmp/test.buf"), 10)?);
    /// let async_rb = AsyncRingBuffer::new(rb);
    ///
    /// let snapshot = async_rb.read_snapshot().await?;
    /// println!("Read {} bytes", snapshot.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn read_snapshot(&self) -> Result<Vec<u8>, DaqError> {
        let rb = self.inner.clone();
        tokio::task::spawn_blocking(move || rb.read_snapshot())
            .await
            .map_err(|e| {
                DaqError::Storage(StorageError::new(
                    StorageErrorKind::RingBuffer,
                    format!("spawn_blocking failed during read_snapshot: {e}"),
                ))
            })
    }

    /// Write data to the ring buffer (async-safe).
    ///
    /// While writes themselves don't block on backoff, wrapping them in spawn_blocking
    /// ensures consistent behavior and prevents potential lock contention from
    /// blocking the async runtime.
    ///
    /// # Arguments
    /// * `data` - Byte slice to write
    ///
    /// # Returns
    /// Ok(()) on success, Err if data is too large for buffer
    ///
    /// # Example
    /// ```no_run
    /// # use std::path::Path;
    /// # use std::sync::Arc;
    /// # use storage::ring_buffer::{RingBuffer, AsyncRingBuffer};
    /// # async fn example() -> anyhow::Result<()> {
    /// let rb = Arc::new(RingBuffer::create(Path::new("/tmp/test.buf"), 10)?);
    /// let async_rb = AsyncRingBuffer::new(rb);
    ///
    /// async_rb.write(b"test data").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn write(&self, data: &[u8]) -> Result<(), DaqError> {
        let rb = self.inner.clone();
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || rb.write(&data))
            .await
            .map_err(|e| {
                DaqError::Storage(StorageError::new(
                    StorageErrorKind::RingBuffer,
                    format!("spawn_blocking failed during write: {e}"),
                ))
            })??;
        Ok(())
    }

    /// Register a new tap consumer to receive every Nth frame.
    ///
    /// This method doesn't block, so it doesn't need spawn_blocking.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this tap
    /// * `nth_frame` - Deliver every nth frame (1 = every frame, 10 = every 10th)
    ///
    /// # Returns
    /// A receiver that will receive frame data, or an error if tap already exists
    pub fn register_tap(
        &self,
        id: String,
        nth_frame: usize,
    ) -> Result<mpsc::Receiver<Vec<u8>>, DaqError> {
        self.inner.register_tap(id, nth_frame)
    }

    /// Unregister a tap consumer.
    ///
    /// # Arguments
    /// * `id` - The tap ID to remove
    ///
    /// # Returns
    /// Ok(true) if tap was found and removed, Ok(false) if tap didn't exist
    pub fn unregister_tap(&self, id: &str) -> Result<bool, DaqError> {
        self.inner.unregister_tap(id)
    }

    /// Get the number of currently registered taps.
    pub fn tap_count(&self) -> usize {
        self.inner.tap_count()
    }

    /// Get information about all registered taps.
    pub fn list_taps(&self) -> Vec<(String, usize)> {
        self.inner.list_taps()
    }

    /// Get the capacity of the data region in bytes.
    pub fn capacity(&self) -> u64 {
        self.inner.capacity()
    }

    /// Get the filesystem path to the backing file.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    /// Get the stream identifier for cross-process reader synchronization.
    pub fn stream_id(&self) -> u64 {
        self.inner.stream_id()
    }

    /// Get the magic number for buffer validation.
    pub fn magic(&self) -> u64 {
        self.inner.magic()
    }

    /// Get the write epoch for seqlock synchronization.
    pub fn write_epoch(&self) -> u64 {
        self.inner.write_epoch()
    }

    /// Get the current write head position (monotonically increasing).
    pub fn write_head(&self) -> u64 {
        self.inner.write_head()
    }

    /// Get the current read tail position (marks oldest unconsumed data).
    pub fn read_tail(&self) -> u64 {
        self.inner.read_tail()
    }

    /// Update the read tail position (mark data as consumed).
    pub fn update_read_tail(&self, new_tail: u64) {
        self.inner.update_read_tail(new_tail)
    }

    /// Advance the read tail by a number of bytes (convenience wrapper).
    pub fn advance_tail(&self, bytes: u64) {
        self.inner.advance_tail(bytes)
    }

    /// Get the Arrow schema JSON for cross-process readers (bd-1il7).
    #[cfg(feature = "storage_arrow")]
    pub fn arrow_schema_json(&self) -> Option<String> {
        self.inner.arrow_schema_json()
    }

    /// Get the schema length stored in the header.
    pub fn schema_len(&self) -> u32 {
        self.inner.schema_len()
    }

    /// Get access to the inner RingBuffer.
    ///
    /// # Warning
    /// This exposes the underlying blocking RingBuffer. Only use this if you
    /// know you're in a non-async context or will properly wrap calls in
    /// spawn_blocking yourself.
    pub fn inner(&self) -> &Arc<RingBuffer> {
        &self.inner
    }
}

impl std::fmt::Debug for AsyncRingBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncRingBuffer")
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(feature = "storage_arrow")]
impl AsyncRingBuffer {
    /// Write an Arrow RecordBatch in IPC format with length prefix (async-safe).
    ///
    /// This serializes the batch to Arrow IPC format and writes it to the ring buffer
    /// with a 4-byte little-endian length prefix.
    ///
    /// # Arguments
    /// * `batch` - The Arrow RecordBatch to write
    ///
    /// # Returns
    /// Ok(()) on success, Err on serialization or write failure
    pub async fn write_arrow_batch(&self, batch: &RecordBatch) -> Result<(), DaqError> {
        let rb = self.inner.clone();
        let batch = batch.clone();
        tokio::task::spawn_blocking(move || rb.write_arrow_batch(&batch))
            .await
            .map_err(|e| {
                DaqError::Storage(StorageError::new(
                    StorageErrorKind::RingBuffer,
                    format!("spawn_blocking failed during write_arrow_batch: {e}"),
                ))
            })??;
        Ok(())
    }
}

#[cfg(feature = "storage_arrow")]
impl RingBuffer {
    /// Write an Arrow RecordBatch in IPC format with length prefix.
    ///
    /// This serializes the batch to Arrow IPC format and writes it to the ring buffer
    /// with a 4-byte little-endian length prefix. This enables cross-process readers
    /// (Python/Julia via mmap) to easily determine record boundaries.
    ///
    /// On the first write, the Arrow schema is extracted and stored for retrieval
    /// via `arrow_schema_json()` (bd-1il7).
    ///
    /// Wire format:
    /// ```text
    /// +----------------+------------------+
    /// | length (4 LE)  | Arrow IPC data   |
    /// +----------------+------------------+
    /// ```
    ///
    /// # Arguments
    /// * `batch` - The Arrow RecordBatch to write
    ///
    /// # Returns
    /// Ok(()) on success, Err on serialization or write failure
    pub fn write_arrow_batch(&self, batch: &RecordBatch) -> Result<(), DaqError> {
        use arrow::ipc::writer::FileWriter;

        // Store schema on first write (bd-1il7)
        self.store_schema_if_needed(&batch.schema());

        let mut buffer = Vec::new();
        let mut writer = FileWriter::try_new(&mut buffer, &batch.schema()).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Arrow,
                format!("Failed to create Arrow IPC writer: {}", e),
            ))
        })?;

        writer.write(batch).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Arrow,
                format!("Failed to write Arrow batch: {}", e),
            ))
        })?;
        writer.finish().map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Arrow,
                format!("Failed to finish Arrow writer: {}", e),
            ))
        })?;

        // Prepend 4-byte little-endian length for cross-process readers
        let len = buffer.len() as u32;
        let mut framed = Vec::with_capacity(4 + buffer.len());
        framed.extend_from_slice(&len.to_le_bytes());
        framed.extend_from_slice(&buffer);

        self.write(&framed).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::RingBuffer,
                format!("Failed to write Arrow IPC data to ring buffer: {}", e),
            ))
        })
    }

    /// Store Arrow schema JSON on first write (bd-1il7).
    ///
    /// This extracts the schema from the first batch and stores it for
    /// cross-process readers. The schema is serialized to JSON format
    /// which can be parsed by Python/Julia clients.
    fn store_schema_if_needed(&self, schema: &arrow::datatypes::SchemaRef) {
        // Check if schema already stored (fast path)
        if self.schema_len() > 0 {
            return;
        }

        // Try to acquire write lock and store schema
        if let Ok(mut guard) = self.arrow_schema_json.write() {
            if guard.is_none() {
                // Serialize schema to JSON
                let schema_json = serde_json::json!({
                    "fields": schema.fields().iter().map(|f| {
                        serde_json::json!({
                            "name": f.name(),
                            "data_type": format!("{:?}", f.data_type()),
                            "nullable": f.is_nullable(),
                        })
                    }).collect::<Vec<_>>(),
                    "metadata": schema.metadata().clone(),
                });

                if let Ok(json_str) = serde_json::to_string(&schema_json) {
                    let len = json_str.len();
                    *guard = Some(json_str);

                    // Update header schema_len (capped at u32::MAX)
                    let schema_len = std::cmp::min(len, u32::MAX as usize) as u32;
                    // SAFETY: header is valid for the lifetime of self
                    unsafe {
                        (*self.header).schema_len = schema_len;
                    }

                    tracing::debug!(
                        schema_len = len,
                        "Stored Arrow schema JSON in ring buffer (bd-1il7)"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    /// Test that create() rejects zero capacity.
    /// Regression test for bd-sep2 (capacity overflow validation).
    #[test]
    fn test_create_rejects_zero_capacity() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("zero_capacity.buf");

        let result = RingBuffer::create(&path, 0);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("must be at least 1 MB") || err_msg.contains("capacity"),
            "Error should mention zero capacity is invalid: {}",
            err_msg
        );
    }

    /// Test that create() rejects capacity exceeding maximum (16 GB).
    /// Regression test for bd-sep2 (capacity overflow validation).
    #[test]
    fn test_create_rejects_excessive_capacity() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("excessive_capacity.buf");

        // Try to create with 17 GB (exceeds 16 GB limit)
        let result = RingBuffer::create(&path, 17 * 1024);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("exceeds maximum") || err_msg.contains("capacity"),
            "Error should mention capacity exceeds maximum: {}",
            err_msg
        );
    }

    /// Test that create() handles overflow in multiplication safely.
    /// This would overflow on 32-bit systems or with pathologically large values.
    /// Regression test for bd-sep2 (capacity overflow validation).
    #[test]
    fn test_create_handles_arithmetic_overflow() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("arithmetic_overflow.buf");

        // Use a value that would overflow if unchecked
        // On 64-bit systems, max usize is ~9 exabytes (9,223,372 TB)
        // So we can't easily trigger overflow without hitting the 16GB limit first
        // But we can test that the function doesn't panic on large values
        let result = RingBuffer::create(&path, 16 * 1024);
        // This should fail due to the 16GB upper limit, not a panic from overflow
        match result {
            Err(e) => {
                let msg = e.to_string();
                // Either exceeds limit or allocation fails, both are acceptable
                assert!(
                    msg.contains("exceeds") || msg.contains("Failed") || msg.contains("allocation"),
                    "Error should be about exceeds limit or allocation: {}",
                    msg
                );
            }
            Ok(_) => {
                // 16GB exactly is at the limit, so it might succeed depending on available RAM
                // This is still a valid outcome for this test
            }
        }
    }

    #[test]
    fn test_create_ring_buffer() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 1).unwrap(); // 1 MB
        assert_eq!(rb.capacity(), 1024 * 1024);
        assert_eq!(rb.write_head(), 0);
        assert_eq!(rb.read_tail(), 0);
    }

    #[test]
    fn test_open_existing_ring_buffer() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        // Create and write some data
        {
            let rb = RingBuffer::create(&path, 1).unwrap();
            rb.write(b"test data").unwrap();
        }

        // Open existing buffer
        let rb = RingBuffer::open(&path).unwrap();
        assert_eq!(rb.capacity(), 1024 * 1024);
        assert_eq!(rb.write_head(), 9); // "test data" = 9 bytes
    }

    #[test]
    fn test_write_and_read() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 1).unwrap();

        // Write data
        let test_data = b"Hello, ring buffer!";
        rb.write(test_data).unwrap();

        // Read snapshot
        let snapshot = rb.read_snapshot();
        assert_eq!(snapshot, test_data);
    }

    #[test]
    fn test_circular_wrap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        // Create small buffer (1KB)
        let rb = RingBuffer::create(&path, 1).unwrap();
        let capacity = rb.capacity() as usize;

        // Write data that will wrap around
        let test_data = vec![0xAA; 512];

        // Fill buffer past capacity to test wrap
        for _ in 0..3 {
            rb.write(&test_data).unwrap();
        }

        // Verify data wraps correctly
        let snapshot = rb.read_snapshot();
        assert!(snapshot.len() <= capacity);
    }

    #[test]
    fn test_wrap_preserves_latest_bytes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("wrap_latest.buf");

        let rb = RingBuffer::create(&path, 1).unwrap(); // 1 MB
        let capacity = rb.capacity() as usize;

        let first = vec![0x11u8; capacity - 16];
        let second = vec![0x22u8; 32]; // pushes past capacity to force wrap

        rb.write(&first).unwrap();
        rb.write(&second).unwrap();

        let snapshot = rb.read_snapshot();
        assert_eq!(snapshot.len(), capacity);

        let mut combined = first;
        combined.extend_from_slice(&second);
        let expected_tail = combined[combined.len() - capacity..].to_vec();

        assert_eq!(snapshot, expected_tail);
    }

    #[test]
    fn test_concurrent_write_read() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap()); // 10 MB

        // Spawn writer thread
        let rb_writer = Arc::clone(&rb);
        let writer = thread::spawn(move || {
            for i in 0..1000 {
                let data = format!("Message {}", i);
                rb_writer.write(data.as_bytes()).unwrap();
            }
        });

        // Spawn reader thread
        let rb_reader = Arc::clone(&rb);
        let reader = thread::spawn(move || {
            let mut read_count = 0;
            while read_count < 100 {
                let snapshot = rb_reader.read_snapshot();
                if !snapshot.is_empty() {
                    read_count += 1;
                }
                thread::sleep(std::time::Duration::from_micros(100));
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn test_read_snapshot_times_out_on_stuck_epoch() {
        // Use a short timeout for testing (100ms instead of default 500ms)
        std::env::set_var("DAQ_RINGBUFFER_TIMEOUT_MS", "100");

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("stuck_epoch.buf");

        let rb = RingBuffer::create(&path, 1).unwrap();

        // Simulate a writer crash that left the epoch odd
        unsafe { (*rb.header).write_epoch.store(1, Ordering::Release) };

        let start = Instant::now();
        let snapshot = rb.read_snapshot();
        let elapsed = start.elapsed();

        // Restore default timeout
        std::env::remove_var("DAQ_RINGBUFFER_TIMEOUT_MS");

        assert!(snapshot.is_empty());
        assert!(elapsed < Duration::from_millis(200));
    }

    #[test]
    fn test_data_too_large() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 1).unwrap(); // 1 MB

        // Try to write more than capacity
        let large_data = vec![0u8; 2 * 1024 * 1024]; // 2 MB
        let result = rb.write(&large_data);

        assert!(result.is_err());
    }

    #[test]
    fn test_write_performance() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 100).unwrap(); // 100 MB

        let test_data = vec![0u8; 1024]; // 1 KB per write
        let iterations = 10_000;

        let start = std::time::Instant::now();
        for _ in 0..iterations {
            rb.write(&test_data).unwrap();
        }
        let elapsed = start.elapsed();

        let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
        println!("Write performance: {:.0} ops/sec", ops_per_sec);

        // Should achieve 10k+ writes/sec
        assert!(
            ops_per_sec > 10_000.0,
            "Performance too low: {} ops/sec",
            ops_per_sec
        );
    }

    #[tokio::test]
    async fn test_tap_registration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 10).unwrap();

        // Register a tap
        let mut rx = rb.register_tap("test_tap".to_string(), 1).unwrap();

        // Verify tap is registered
        assert_eq!(rb.tap_count(), 1);
        assert_eq!(rb.list_taps(), vec![("test_tap".to_string(), 1)]);

        // Write data
        let test_data = b"test frame";
        rb.write(test_data).unwrap();

        // Should receive the frame
        let received = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .unwrap();

        assert_eq!(received.as_ref(), Some(&test_data.to_vec()));

        // Unregister tap
        assert!(rb.unregister_tap("test_tap").unwrap());
        assert_eq!(rb.tap_count(), 0);
    }

    #[tokio::test]
    async fn test_tap_nth_frame() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 10).unwrap();

        // Register tap to receive every 3rd frame
        let mut rx = rb.register_tap("test_tap".to_string(), 3).unwrap();

        // Write 10 frames
        for i in 0..10 {
            let data = format!("frame_{}", i);
            rb.write(data.as_bytes()).unwrap();
        }

        // Should receive frames 0, 3, 6, 9 (4 frames total)
        let mut received_count = 0;
        while let Ok(Some(_)) =
            tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await
        {
            received_count += 1;
        }

        assert_eq!(received_count, 4);
    }

    #[tokio::test]
    async fn test_tap_backpressure() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 10).unwrap();

        // Register tap but don't consume from it
        let _rx = rb.register_tap("slow_tap".to_string(), 1).unwrap();

        // Write more frames than the channel can hold
        // Channel size is DEFAULT_TAP_CHANNEL_SIZE (16)
        for i in 0..50 {
            let data = format!("frame_{:03}", i);
            rb.write(data.as_bytes()).unwrap();
        }

        // Write should complete without blocking (frames dropped)
        // If this test completes, backpressure handling is working
        assert_eq!(rb.tap_count(), 1);
    }

    #[tokio::test]
    async fn test_multiple_taps() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 10).unwrap();

        // Register multiple taps with different rates
        let mut rx1 = rb.register_tap("tap1".to_string(), 1).unwrap();
        let mut rx2 = rb.register_tap("tap2".to_string(), 2).unwrap();
        let mut rx3 = rb.register_tap("tap3".to_string(), 5).unwrap();

        assert_eq!(rb.tap_count(), 3);

        // Write 10 frames
        for i in 0..10 {
            let data = format!("frame_{}", i);
            rb.write(data.as_bytes()).unwrap();
        }

        // Count received frames for each tap
        let mut count1 = 0;
        let mut count2 = 0;
        let mut count3 = 0;

        while let Ok(Some(_)) =
            tokio::time::timeout(std::time::Duration::from_millis(10), rx1.recv()).await
        {
            count1 += 1;
        }

        while let Ok(Some(_)) =
            tokio::time::timeout(std::time::Duration::from_millis(10), rx2.recv()).await
        {
            count2 += 1;
        }

        while let Ok(Some(_)) =
            tokio::time::timeout(std::time::Duration::from_millis(10), rx3.recv()).await
        {
            count3 += 1;
        }

        // Tap1 gets every frame (10)
        assert_eq!(count1, 10);
        // Tap2 gets every 2nd frame (5)
        assert_eq!(count2, 5);
        // Tap3 gets every 5th frame (2)
        assert_eq!(count3, 2);
    }

    #[test]
    fn test_tap_duplicate_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 10).unwrap();

        // Register first tap
        let _rx1 = rb.register_tap("tap1".to_string(), 1).unwrap();

        // Try to register with same ID
        let result = rb.register_tap("tap1".to_string(), 2);
        assert!(result.is_err());
    }

    #[test]
    fn test_unregister_nonexistent_tap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_ring.buf");

        let rb = RingBuffer::create(&path, 10).unwrap();

        // Try to unregister tap that doesn't exist
        let result = rb.unregister_tap("nonexistent");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    /// Test that open() rejects files that are too small to contain a header.
    /// Regression test for bd-jnfu.1 (ring buffer safety audit).
    #[test]
    fn test_open_rejects_truncated_file() {
        use std::fs::File;
        use std::io::Write;

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("truncated.buf");

        // Create a file that's smaller than HEADER_SIZE (128 bytes)
        let mut file = File::create(&path).unwrap();
        file.write_all(&[0u8; 64]).unwrap(); // Only 64 bytes
        drop(file);

        // open() should fail with a descriptive error
        let result = RingBuffer::open(&path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("too small") || err_msg.contains("minimum"),
            "Error should mention file is too small: {}",
            err_msg
        );
    }

    /// Test that open() rejects files with invalid magic number.
    /// Regression test for bd-jnfu.1 (ring buffer safety audit).
    #[test]
    fn test_open_rejects_invalid_magic() {
        use std::fs::File;
        use std::io::Write;

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bad_magic.buf");

        // Create a file with enough bytes but wrong magic
        let mut file = File::create(&path).unwrap();
        let bad_data = vec![0xFFu8; 256]; // 256 bytes of 0xFF
        file.write_all(&bad_data).unwrap();
        drop(file);

        // open() should fail with magic number error
        let result = RingBuffer::open(&path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("magic") || err_msg.contains("Invalid"),
            "Error should mention invalid magic: {}",
            err_msg
        );
    }

    /// Test that open() rejects files where size doesn't match header capacity.
    /// Regression test for bd-jnfu.1 (ring buffer safety audit).
    #[test]
    fn test_open_rejects_size_mismatch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("size_mismatch.buf");

        // Create a valid ring buffer
        {
            let _rb = RingBuffer::create(&path, 1).unwrap(); // 1 MB
        }

        // Truncate the file to simulate corruption
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        // Truncate to header + 512KB (should be header + 1MB)
        file.set_len((HEADER_SIZE + 512 * 1024) as u64).unwrap();
        drop(file);

        // open() should fail with size mismatch error
        let result = RingBuffer::open(&path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("mismatch") || err_msg.contains("truncated"),
            "Error should mention size mismatch: {}",
            err_msg
        );
    }

    // =============================================================================
    // AsyncRingBuffer Tests (bd-cmcp)
    // =============================================================================

    #[tokio::test]
    async fn test_async_ring_buffer_basic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_test.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Write data
        let test_data = b"Hello, async ring buffer!";
        async_rb.write(test_data).await.unwrap();

        // Read snapshot
        let snapshot = async_rb.read_snapshot().await.unwrap();
        assert_eq!(snapshot, test_data);
    }

    #[tokio::test]
    async fn test_async_ring_buffer_concurrent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_concurrent.buf");

        let rb = Arc::new(RingBuffer::create(&path, 100).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Spawn multiple writer tasks
        let mut writer_handles = Vec::new();
        for i in 0..10 {
            let arb = async_rb.clone();
            let handle = tokio::spawn(async move {
                for j in 0..100 {
                    let data = format!("Writer {} Message {}", i, j);
                    arb.write(data.as_bytes()).await.unwrap();
                }
            });
            writer_handles.push(handle);
        }

        // Spawn multiple reader tasks
        let mut reader_handles = Vec::new();
        for _ in 0..5 {
            let arb = async_rb.clone();
            let handle = tokio::spawn(async move {
                for _ in 0..50 {
                    let _snapshot = arb.read_snapshot().await.unwrap();
                    tokio::time::sleep(tokio::time::Duration::from_micros(100)).await;
                }
            });
            reader_handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in writer_handles {
            handle.await.unwrap();
        }
        for handle in reader_handles {
            handle.await.unwrap();
        }

        // Final read should succeed
        let snapshot = async_rb.read_snapshot().await.unwrap();
        assert!(!snapshot.is_empty());
    }

    #[tokio::test]
    async fn test_async_ring_buffer_tap_registration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_tap.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Register a tap
        let mut rx = async_rb.register_tap("test_tap".to_string(), 1).unwrap();

        // Verify tap is registered
        assert_eq!(async_rb.tap_count(), 1);
        assert_eq!(async_rb.list_taps(), vec![("test_tap".to_string(), 1)]);

        // Write data
        let test_data = b"test frame";
        async_rb.write(test_data).await.unwrap();

        // Should receive the frame
        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap();

        assert_eq!(received.as_ref(), Some(&test_data.to_vec()));

        // Unregister tap
        assert!(async_rb.unregister_tap("test_tap").unwrap());
        assert_eq!(async_rb.tap_count(), 0);
    }

    #[tokio::test]
    async fn test_async_ring_buffer_metadata_methods() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_metadata.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Test metadata accessors
        assert_eq!(async_rb.capacity(), 10 * 1024 * 1024);
        assert!(async_rb.magic() == MAGIC);
        assert_eq!(async_rb.write_head(), 0);
        assert_eq!(async_rb.read_tail(), 0);
        assert!(async_rb.stream_id() > 0);

        // Write some data
        async_rb.write(b"test data").await.unwrap();

        // Check that write_head advanced
        assert_eq!(async_rb.write_head(), 9);

        // Test tail operations
        async_rb.advance_tail(5);
        assert_eq!(async_rb.read_tail(), 5);

        async_rb.update_read_tail(9);
        assert_eq!(async_rb.read_tail(), 9);
    }

    #[tokio::test]
    async fn test_async_ring_buffer_wrap_around() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_wrap.buf");

        let rb = Arc::new(RingBuffer::create(&path, 1).unwrap()); // 1 MB
        let async_rb = AsyncRingBuffer::new(rb);

        let capacity = async_rb.capacity() as usize;

        // Write data that will wrap around
        let test_data = vec![0xAA; 512];

        // Fill buffer past capacity to test wrap
        for _ in 0..3 {
            async_rb.write(&test_data).await.unwrap();
        }

        // Verify data wraps correctly
        let snapshot = async_rb.read_snapshot().await.unwrap();
        assert!(snapshot.len() <= capacity);
    }

    #[tokio::test]
    async fn test_async_ring_buffer_debug_impl() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_debug.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Test Debug implementation
        let debug_str = format!("{:?}", async_rb);
        assert!(debug_str.contains("AsyncRingBuffer"));
    }

    #[tokio::test]
    async fn test_async_ring_buffer_clone() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_clone.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Clone the async ring buffer
        let async_rb_clone = async_rb.clone();

        // Write with original
        async_rb.write(b"original").await.unwrap();

        // Read with clone
        let snapshot = async_rb_clone.read_snapshot().await.unwrap();
        assert_eq!(snapshot, b"original");

        // Write with clone
        async_rb_clone.write(b"clone").await.unwrap();

        // Read with original
        let snapshot = async_rb.read_snapshot().await.unwrap();
        assert!(!snapshot.is_empty());
    }

    #[tokio::test]
    async fn test_async_ring_buffer_inner_access() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_inner.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb.clone());

        // Access inner should return the same Arc
        let inner = async_rb.inner();
        assert!(Arc::ptr_eq(inner, &rb));
    }

    #[cfg(feature = "storage_arrow")]
    #[tokio::test]
    async fn test_async_ring_buffer_arrow_batch() {
        use arrow::array::{Float64Array, Int32Array};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc as ArrowArc;

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("async_arrow.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Create a simple Arrow RecordBatch
        let schema = ArrowArc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("value", DataType::Float64, false),
        ]));

        let id_array = Int32Array::from(vec![1, 2, 3]);
        let value_array = Float64Array::from(vec![1.0, 2.0, 3.0]);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![ArrowArc::new(id_array), ArrowArc::new(value_array)],
        )
        .unwrap();

        // Write Arrow batch
        async_rb.write_arrow_batch(&batch).await.unwrap();

        // Read snapshot
        let snapshot = async_rb.read_snapshot().await.unwrap();
        assert!(!snapshot.is_empty());

        // The snapshot should contain the Arrow IPC data with length prefix
        assert!(snapshot.len() > 4); // At least 4 bytes for length prefix
    }

    /// Test Arrow schema storage (bd-1il7)
    #[cfg(feature = "storage_arrow")]
    #[tokio::test]
    async fn test_arrow_schema_storage() {
        use arrow::array::{Float64Array, Int32Array};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc as ArrowArc;

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("schema_test.buf");

        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        // Initially no schema
        assert!(async_rb.arrow_schema_json().is_none());
        assert_eq!(async_rb.schema_len(), 0);

        // Create and write an Arrow batch
        let schema = ArrowArc::new(Schema::new(vec![
            Field::new("temperature", DataType::Float64, false),
            Field::new("pressure", DataType::Int32, true),
        ]));

        let temp_array = Float64Array::from(vec![20.5, 21.0, 22.5]);
        let press_array = Int32Array::from(vec![Some(101325), None, Some(101400)]);

        let batch = RecordBatch::try_new(
            schema,
            vec![ArrowArc::new(temp_array), ArrowArc::new(press_array)],
        )
        .unwrap();

        async_rb.write_arrow_batch(&batch).await.unwrap();

        // After writing, schema should be stored
        let schema_json = async_rb.arrow_schema_json();
        assert!(schema_json.is_some());
        let json = schema_json.unwrap();
        assert!(json.contains("temperature"));
        assert!(json.contains("pressure"));
        assert!(json.contains("Float64"));
        assert!(json.contains("Int32"));

        // schema_len should be non-zero
        assert!(async_rb.schema_len() > 0);

        // Writing again should not change the schema (first write wins)
        let schema2 = ArrowArc::new(Schema::new(vec![Field::new(
            "different",
            DataType::Float64,
            false,
        )]));
        let arr2 = Float64Array::from(vec![1.0]);
        let batch2 = RecordBatch::try_new(schema2, vec![ArrowArc::new(arr2)]).unwrap();
        async_rb.write_arrow_batch(&batch2).await.unwrap();

        // Schema should still be the original one
        let schema_json2 = async_rb.arrow_schema_json().unwrap();
        assert!(schema_json2.contains("temperature"));
        assert!(!schema_json2.contains("different"));
    }

    // =============================================================================
    // Property-Based Tests (proptest) — bd-m5fh.7.1
    // =============================================================================

    mod property_tests {
        use super::*;
        use proptest::prelude::*;
        use std::sync::Arc;

        /// Strategy: generate a write payload between 1 byte and 64 KB.
        fn write_payload() -> impl Strategy<Value = Vec<u8>> {
            proptest::collection::vec(any::<u8>(), 1..=65_536)
        }

        /// Strategy: generate a sequence of write payloads (1..=20 writes, each 1..=4 KB).
        fn write_sequence() -> impl Strategy<Value = Vec<Vec<u8>>> {
            proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..=4_096), 1..=20)
        }

        // Reduce cases for faster CI runs; increase locally with PROPTEST_CASES env var
        proptest! {
            #![proptest_config(ProptestConfig::with_cases(20))]

            /// Property: write_head advances by exactly data.len() after each write.
            #[test]
            fn write_head_advances_by_data_len(data in write_payload()) {
                let temp_dir = tempfile::tempdir().unwrap();
                let path = temp_dir.path().join("prop_head.buf");
                let rb = RingBuffer::create(&path, 1).unwrap(); // 1 MB

                let head_before = rb.write_head();
                rb.write(&data).unwrap();
                let head_after = rb.write_head();

                prop_assert_eq!(head_after, head_before + data.len() as u64);
            }

            /// Property: write_epoch is always even after a completed write.
            #[test]
            fn epoch_even_after_write(data in write_payload()) {
                let temp_dir = tempfile::tempdir().unwrap();
                let path = temp_dir.path().join("prop_epoch.buf");
                let rb = RingBuffer::create(&path, 1).unwrap();

                rb.write(&data).unwrap();
                let epoch = rb.write_epoch();

                prop_assert!(epoch.is_multiple_of(2), "epoch {} should be even after write", epoch);
            }

            /// Property: read_snapshot length never exceeds capacity.
            #[test]
            fn snapshot_bounded_by_capacity(writes in write_sequence()) {
                let temp_dir = tempfile::tempdir().unwrap();
                let path = temp_dir.path().join("prop_bounded.buf");
                let rb = RingBuffer::create(&path, 1).unwrap();
                let capacity = rb.capacity() as usize;

                for data in &writes {
                    rb.write(data).unwrap();
                }

                let snapshot = rb.read_snapshot();
                prop_assert!(
                    snapshot.len() <= capacity,
                    "snapshot {} > capacity {}",
                    snapshot.len(),
                    capacity
                );
            }

            /// Property: after wrap-around, snapshot contains the tail of concatenated writes.
            #[test]
            fn wrap_preserves_latest_data(writes in write_sequence()) {
                let temp_dir = tempfile::tempdir().unwrap();
                let path = temp_dir.path().join("prop_wrap.buf");
                let rb = RingBuffer::create(&path, 1).unwrap();
                let capacity = rb.capacity() as usize;

                let mut all_data = Vec::new();
                for data in &writes {
                    rb.write(data).unwrap();
                    all_data.extend_from_slice(data);
                }

                let snapshot = rb.read_snapshot();

                if all_data.len() <= capacity {
                    prop_assert_eq!(&snapshot, &all_data);
                } else {
                    let expected = &all_data[all_data.len() - capacity..];
                    prop_assert_eq!(&snapshot, expected);
                }
            }

            /// Property: write_head is monotonically increasing across a sequence of writes.
            #[test]
            fn write_head_monotonic(writes in write_sequence()) {
                let temp_dir = tempfile::tempdir().unwrap();
                let path = temp_dir.path().join("prop_mono.buf");
                let rb = RingBuffer::create(&path, 1).unwrap();

                let mut prev_head = rb.write_head();
                for data in &writes {
                    rb.write(data).unwrap();
                    let head = rb.write_head();
                    prop_assert!(head > prev_head, "head {} <= prev {}", head, prev_head);
                    prev_head = head;
                }
            }

            /// Property: write rejects data larger than capacity.
            #[test]
            fn write_rejects_oversized(extra in 1..=1024usize) {
                let temp_dir = tempfile::tempdir().unwrap();
                let path = temp_dir.path().join("prop_oversize.buf");
                let rb = RingBuffer::create(&path, 1).unwrap();
                let capacity = rb.capacity() as usize;

                let oversized = vec![0u8; capacity + extra];
                let result = rb.write(&oversized);
                prop_assert!(result.is_err(), "should reject data exceeding capacity");
            }
        }
    }

    // =============================================================================
    // Stress Tests — bd-m5fh.7.1
    // =============================================================================

    /// Stress test: many concurrent writers with a reader verifying no panics or deadlocks.
    #[test]
    #[ignore = "stress test; run with: cargo test -- --ignored"]
    fn stress_concurrent_multi_writer() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("stress_multi.buf");
        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());

        let num_writers = 8;
        let writes_per_writer = 500;
        let payload_size = 256;

        let mut handles = Vec::new();

        for writer_id in 0..num_writers {
            let rb = Arc::clone(&rb);
            handles.push(thread::spawn(move || {
                for i in 0..writes_per_writer {
                    let mut data = vec![writer_id as u8; payload_size];
                    let seq_bytes = (i as u32).to_le_bytes();
                    data[..4].copy_from_slice(&seq_bytes);
                    rb.write(&data).unwrap();
                }
            }));
        }

        let rb_reader = Arc::clone(&rb);
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        let reader = thread::spawn(move || {
            let mut reads = 0u64;
            while !done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                let snapshot = rb_reader.read_snapshot();
                assert!(snapshot.len() <= rb_reader.capacity() as usize);
                reads += 1;
                if reads.is_multiple_of(100) {
                    thread::yield_now();
                }
            }
            reads
        });

        for h in handles {
            h.join().unwrap();
        }

        done.store(true, std::sync::atomic::Ordering::Relaxed);
        let read_count = reader.join().unwrap();

        let expected_total_bytes = (num_writers * writes_per_writer * payload_size) as u64;
        assert_eq!(rb.write_head(), expected_total_bytes);
        assert!(
            rb.write_epoch().is_multiple_of(2),
            "epoch should be even after all writes"
        );
        assert!(
            read_count > 0,
            "reader should have completed at least one read"
        );
    }

    /// Stress test: rapid tap registration and unregistration during writes.
    #[tokio::test]
    #[ignore = "stress test; run with: cargo test -- --ignored"]
    async fn stress_tap_register_unregister_during_writes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("stress_tap_reg.buf");
        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        let num_writes = 200;
        let num_tap_cycles = 20;

        let writer_rb = async_rb.clone();
        let writer = tokio::spawn(async move {
            for i in 0..num_writes {
                let data = vec![i as u8; 1024];
                writer_rb.write(&data).await.unwrap();
                if i % 10 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });

        let tap_rb = async_rb.clone();
        let tap_task = tokio::spawn(async move {
            for cycle in 0..num_tap_cycles {
                let tap_id = format!("stress_tap_{}", cycle);
                if let Ok(mut rx) = tap_rb.register_tap(tap_id.clone(), 1) {
                    for _ in 0..5 {
                        match tokio::time::timeout(Duration::from_millis(10), rx.recv()).await {
                            Ok(Some(_)) => {}
                            _ => break,
                        }
                    }
                    let _ = tap_rb.unregister_tap(&tap_id);
                }
            }
        });

        writer.await.unwrap();
        tap_task.await.unwrap();

        assert!(async_rb.write_head() > 0);
        assert_eq!(async_rb.tap_count(), 0, "all taps should be unregistered");
    }

    /// Stress test: tap backpressure under saturation.
    #[tokio::test]
    #[ignore = "stress test; run with: cargo test -- --ignored"]
    async fn stress_tap_backpressure_saturation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("stress_bp.buf");
        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb.clone());

        let mut rx = async_rb
            .register_tap("slow_consumer".to_string(), 1)
            .unwrap();

        let writer_rb = async_rb.clone();
        let writer = tokio::spawn(async move {
            for i in 0..500u32 {
                let data = vec![0xFFu8; 4096];
                writer_rb.write(&data).await.unwrap();
                if i % 50 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });

        let consumer = tokio::spawn(async move {
            let mut received = 0u64;
            loop {
                match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                    Ok(Some(_)) => received += 1,
                    _ => break,
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            received
        });

        writer.await.unwrap();
        let received = consumer.await.unwrap();

        assert!(
            received > 0,
            "consumer should have received at least 1 frame"
        );
        // Slow consumer with 5ms delay per frame cannot keep up with 500 writes.
        // On fast CI runners the consumer may receive many frames before the writer
        // fills the channel, so only assert backpressure occurred via dropped_frames
        // OR that the consumer received fewer frames than were written.
        let health = rb.tap_health_for("slow_consumer");
        let dropped = health.map_or(0, |h| h.dropped_frames);
        assert!(
            dropped > 0 || received < 500,
            "expected backpressure evidence: dropped_frames={}, received={}",
            dropped,
            received
        );
    }

    /// Stress test: concurrent async writers and readers (tokio tasks).
    #[tokio::test]
    #[ignore = "stress test; run with: cargo test -- --ignored"]
    async fn stress_async_concurrent_writers_readers() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("stress_async.buf");
        let rb = Arc::new(RingBuffer::create(&path, 10).unwrap());
        let async_rb = AsyncRingBuffer::new(rb);

        let num_writers = 10;
        let writes_per = 200;
        let num_readers = 5;
        let reads_per = 100;

        let mut handles = Vec::new();

        for w in 0..num_writers {
            let arb = async_rb.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..writes_per {
                    let data = format!("W{}:{}", w, i);
                    arb.write(data.as_bytes()).await.unwrap();
                }
            }));
        }

        for _ in 0..num_readers {
            let arb = async_rb.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..reads_per {
                    let snapshot = arb.read_snapshot().await.unwrap();
                    assert!(snapshot.len() <= arb.capacity() as usize);
                    tokio::task::yield_now().await;
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert!(async_rb.write_head() > 0);
        assert!(async_rb.inner().write_epoch().is_multiple_of(2));
    }

    /// Stress test: rapid create/write/read/drop cycles to check resource cleanup.
    #[test]
    #[ignore = "stress test; run with: cargo test -- --ignored"]
    fn stress_buffer_lifecycle_cycles() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cycles = 50;

        for i in 0..cycles {
            let path = temp_dir.path().join(format!("lifecycle_{}.buf", i));
            let rb = RingBuffer::create(&path, 1).unwrap();

            let data = vec![i as u8; 4096];
            rb.write(&data).unwrap();

            let snapshot = rb.read_snapshot();
            assert_eq!(snapshot, data);

            let _rx = rb.register_tap(format!("tap_{}", i), 1).unwrap();
            rb.unregister_tap(&format!("tap_{}", i)).unwrap();
        }
    }
}
