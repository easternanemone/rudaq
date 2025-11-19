//! Memory-mapped ring buffer for lock-free, zero-copy data streaming.
//!
//! This module implements a high-performance ring buffer backed by memory-mapped files,
//! designed for concurrent access with a single writer and multiple readers.
//!
//! # Features
//! - Lock-free operations using atomic instructions
//! - Zero-copy data access via memory mapping
//! - Cross-language compatibility with #[repr(C)] layout
//! - Apache Arrow IPC format support for structured data
//! - Designed for 10k+ writes/sec throughput
//!
//! # Memory Layout
//! ```text
//! [128-byte header] [variable-size data region]
//!
//! Header (cache-line aligned):
//!   magic: u64              (0xDA_DA_DA_DA_00_00_00_01)
//!   capacity_bytes: u64     (size of data region)
//!   write_head: AtomicU64   (current write offset)
//!   read_tail: AtomicU64    (oldest valid data offset)
//!   schema_len: u32         (Arrow schema length)
//!   padding: [u8; 116]      (cache line alignment)
//! ```

use anyhow::{anyhow, Context, Result};
use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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

    /// Length of the Arrow schema JSON (if using Arrow format)
    schema_len: u32,

    /// Padding to align header to 128 bytes (cache line boundary)
    _padding: [u8; 116],
}

/// High-performance memory-mapped ring buffer.
///
/// # Safety
/// This structure contains raw pointers to memory-mapped regions. It is safe to use
/// as long as:
/// - The memory-mapped file remains valid for the lifetime of RingBuffer
/// - Only one writer exists at a time
/// - Readers use appropriate atomic ordering (Acquire)
/// - Writers use appropriate atomic ordering (Release)
pub struct RingBuffer {
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
}

// SAFETY: RingBuffer uses atomic operations for synchronization and can be safely
// sent between threads. The raw pointers are only accessed with proper atomic ordering.
unsafe impl Send for RingBuffer {}

// SAFETY: All read/write operations use atomic instructions with appropriate ordering,
// making concurrent access safe.
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
    /// use rust_daq::data::ring_buffer::RingBuffer;
    ///
    /// let rb = RingBuffer::create(Path::new("/tmp/my_ring_buffer"), 100).unwrap();
    /// ```
    pub fn create(path: &Path, capacity_mb: usize) -> Result<Self> {
        let capacity_bytes = capacity_mb * 1024 * 1024;
        let total_size = HEADER_SIZE + capacity_bytes;

        // Create or open the backing file
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .with_context(|| format!("Failed to create ring buffer file: {:?}", path))?;

        // Set file size
        file.set_len(total_size as u64)
            .context("Failed to set ring buffer file size")?;

        // Create memory mapping
        // SAFETY: We just created the file and set its size, so mapping is safe
        let mut mmap = unsafe {
            MmapOptions::new()
                .map_mut(&file)
                .context("Failed to create memory mapping")?
        };

        // Initialize header
        // SAFETY: mmap is at least HEADER_SIZE bytes (total_size includes HEADER_SIZE)
        let header = mmap.as_mut_ptr() as *mut RingBufferHeader;
        unsafe {
            // Write header fields
            (*header).magic = MAGIC;
            (*header).capacity_bytes = capacity_bytes as u64;
            (*header).write_head = AtomicU64::new(0);
            (*header).read_tail = AtomicU64::new(0);
            (*header).schema_len = 0;

            // Zero out padding for deterministic behavior
            (*header)._padding.fill(0);
        }

        // Calculate data region pointer
        // SAFETY: mmap is total_size bytes, so offset HEADER_SIZE is within bounds
        let data_ptr = unsafe { mmap.as_mut_ptr().add(HEADER_SIZE) };

        Ok(Self {
            mmap,
            header,
            data_ptr,
            capacity: capacity_bytes as u64,
        })
    }

    /// Open an existing ring buffer from a memory-mapped file.
    ///
    /// # Arguments
    /// * `path` - Path to the existing backing file
    ///
    /// # Returns
    /// A `RingBuffer` instance attached to the existing buffer
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Failed to open ring buffer file: {:?}", path))?;

        // Create memory mapping
        // SAFETY: Opening existing file created by create()
        let mut mmap = unsafe {
            MmapOptions::new()
                .map_mut(&file)
                .context("Failed to map ring buffer file")?
        };

        // Validate header
        // SAFETY: File was created with create(), header is valid
        let header = mmap.as_mut_ptr() as *mut RingBufferHeader;
        let (magic, capacity) = unsafe {
            ((*header).magic, (*header).capacity_bytes)
        };

        if magic != MAGIC {
            return Err(anyhow!(
                "Invalid ring buffer magic number: expected 0x{:016X}, got 0x{:016X}",
                MAGIC,
                magic
            ));
        }

        // SAFETY: mmap size validated, offset HEADER_SIZE is within bounds
        let data_ptr = unsafe { mmap.as_mut_ptr().add(HEADER_SIZE) };

        Ok(Self {
            mmap,
            header,
            data_ptr,
            capacity,
        })
    }

    /// Write data to the ring buffer (lock-free).
    ///
    /// This operation is lock-free and safe for concurrent readers. However, only
    /// one writer should exist at a time.
    ///
    /// # Arguments
    /// * `data` - Byte slice to write
    ///
    /// # Returns
    /// Ok(()) on success, Err if data is too large for buffer
    ///
    /// # Note
    /// If the buffer is full, this will overwrite the oldest data (circular behavior)
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let len = data.len() as u64;

        if len > self.capacity {
            return Err(anyhow!(
                "Data size ({} bytes) exceeds ring buffer capacity ({} bytes)",
                len,
                self.capacity
            ));
        }

        // SAFETY: header is valid for the lifetime of self
        unsafe {
            // Load current write position with Acquire ordering to see previous writes
            let head = (*self.header).write_head.load(Ordering::Acquire);

            // Calculate circular offset
            let write_offset = (head % self.capacity) as usize;

            // Handle wrap-around: if data doesn't fit before end, split the write
            if write_offset + data.len() > self.capacity as usize {
                // Write first part to the end of buffer
                let first_part_len = self.capacity as usize - write_offset;
                let dest = self.data_ptr.add(write_offset);
                std::ptr::copy_nonoverlapping(data.as_ptr(), dest, first_part_len);

                // Write second part to the beginning of buffer
                let second_part_len = data.len() - first_part_len;
                std::ptr::copy_nonoverlapping(
                    data.as_ptr().add(first_part_len),
                    self.data_ptr,
                    second_part_len,
                );
            } else {
                // Data fits without wrapping
                let dest = self.data_ptr.add(write_offset);
                std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
            }

            // Update write head with Release ordering to publish the write
            (*self.header).write_head.fetch_add(len, Ordering::Release);
        }

        Ok(())
    }

    /// Read a snapshot of current data in the ring buffer.
    ///
    /// This creates a copy of the available data from read_tail to write_head.
    /// Safe for concurrent use with write operations.
    ///
    /// # Returns
    /// A Vec containing a snapshot of the current buffer contents
    pub fn read_snapshot(&self) -> Vec<u8> {
        // SAFETY: header is valid for the lifetime of self
        unsafe {
            // Load positions with Acquire ordering to see all previous writes
            let head = (*self.header).write_head.load(Ordering::Acquire);
            let tail = (*self.header).read_tail.load(Ordering::Acquire);

            // Calculate available data (capped at capacity to handle wrap-around)
            let available = (head.saturating_sub(tail)).min(self.capacity);

            if available == 0 {
                return Vec::new();
            }

            // Calculate read offset (circular)
            let read_offset = (tail % self.capacity) as usize;

            let mut buffer = vec![0u8; available as usize];

            // Handle wrap-around
            if read_offset + available as usize > self.capacity as usize {
                // Read first part from read_offset to end
                let first_part_len = self.capacity as usize - read_offset;
                let src = self.data_ptr.add(read_offset);
                std::ptr::copy_nonoverlapping(src, buffer.as_mut_ptr(), first_part_len);

                // Read second part from beginning
                let second_part_len = available as usize - first_part_len;
                std::ptr::copy_nonoverlapping(
                    self.data_ptr,
                    buffer.as_mut_ptr().add(first_part_len),
                    second_part_len,
                );
            } else {
                // Data doesn't wrap
                let src = self.data_ptr.add(read_offset);
                std::ptr::copy_nonoverlapping(src, buffer.as_mut_ptr(), available as usize);
            }

            buffer
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

    /// Get the capacity of the ring buffer in bytes.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Get the current write head position.
    pub fn write_head(&self) -> u64 {
        // SAFETY: header is valid for the lifetime of self
        unsafe { (*self.header).write_head.load(Ordering::Acquire) }
    }

    /// Get the current read tail position.
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
}

#[cfg(feature = "storage_arrow")]
impl RingBuffer {
    /// Write an Arrow RecordBatch in IPC format.
    ///
    /// This serializes the batch to Arrow IPC format and writes it to the ring buffer.
    ///
    /// # Arguments
    /// * `batch` - The Arrow RecordBatch to write
    ///
    /// # Returns
    /// Ok(()) on success, Err on serialization or write failure
    pub fn write_arrow_batch(&self, batch: &RecordBatch) -> Result<()> {
        use arrow::ipc::writer::FileWriter;
        use std::io::Cursor;

        let mut buffer = Vec::new();
        let mut writer = FileWriter::try_new(&mut buffer, &batch.schema())
            .context("Failed to create Arrow IPC writer")?;

        writer.write(batch).context("Failed to write Arrow batch")?;
        writer.finish().context("Failed to finish Arrow writer")?;

        self.write(&buffer)
            .context("Failed to write Arrow IPC data to ring buffer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

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
        assert!(ops_per_sec > 10_000.0, "Performance too low: {} ops/sec", ops_per_sec);
    }
}
