//! Data processing and storage modules.
//!
//! # HDF5 Storage - The Mullet Strategy
//!
//! **CANONICAL IMPLEMENTATION**: [`hdf5_writer`] is the ONLY HDF5 implementation in this codebase.
//!
//! ## Architecture
//!
//! The storage system follows "The Mullet Strategy":
//! - **Party in front**: [`ring_buffer`] provides fast Arrow-based writes (10k+ writes/sec)
//! - **Business in back**: [`hdf5_writer`] persists to HDF5 files for Python/MATLAB/Igor compatibility
//!
//! ## Key Components
//!
//! - [`ring_buffer::RingBuffer`] - Memory-mapped ring buffer for high-throughput Arrow IPC
//! - [`hdf5_writer::HDF5Writer`] - Background writer that flushes ring buffer to HDF5 (1 Hz, non-blocking)
//! - [`ring_buffer_reader::RingBufferReader`] - Helper for clients to decode frames from ring buffer taps
//!
//! ## Legacy Code (Deprecated)
//!
//! The following modules have been removed or are being phased out:
//! - `storage.rs` - Duplicate HDF5 implementation (DELETED)
//! - `fft`, `iir_filter`, `processor`, `registry`, `storage_factory`, `trigger` - V1 legacy (commented out)
//!
//! See: JULES_FLEET_STATUS_2025-11-20.md Phase 1 for migration details.

// V1 legacy modules commented out due to removed DataProcessor/StorageWriter traits
// These need to be migrated to V3 architecture or removed
// See: JULES_FLEET_STATUS_2025-11-20.md Phase 1
// pub mod fft;
// pub mod iir_filter;
// pub mod processor;
// pub mod registry;
// pub mod storage_factory;
// pub mod trigger;

/// Background HDF5 writer - The Mullet Strategy backend
///
/// **CANONICAL HDF5 IMPLEMENTATION** - This is the ONLY HDF5 writer in the codebase.
///
/// Flushes [`ring_buffer::RingBuffer`] data to HDF5 files at 1 Hz without blocking
/// hardware loops. Converts Arrow IPC to HDF5 format for compatibility with
/// Python/MATLAB/Igor analysis tools.
pub mod hdf5_writer;

/// Memory-mapped ring buffer for high-throughput data
///
/// Stores Arrow IPC format data in `/dev/shm` for fast writes (10k+ writes/sec).
/// Works in tandem with [`hdf5_writer`] to implement The Mullet Strategy.
pub mod ring_buffer;

/// Helper utility for reading frames from ring buffer taps
///
/// Provides convenient API for clients to receive and deserialize frames sent
/// via [`ring_buffer::RingBuffer::register_tap()`]. Useful for live data
/// visualization and remote monitoring.
pub mod ring_buffer_reader;

/// Live data tap registry and backpressure handling
pub mod tap_registry;

// Re-export main types for convenience
pub use ring_buffer::RingBuffer;
pub use ring_buffer_reader::{ReaderStats, RingBufferReader};
