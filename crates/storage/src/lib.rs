// Allow clippy lints that require significant refactoring
// These are triggered by -D warnings in CI
#![allow(
    clippy::missing_fields_in_debug,
    clippy::unnecessary_debug_formatting,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr,
    clippy::inline_always,
    clippy::semicolon_if_nothing_returned,
    clippy::ignored_unit_patterns,
    clippy::manual_let_else,
    clippy::unused_async,
    clippy::doc_overindented_list_items,
    clippy::float_cmp
)]

//! # daq-storage
//!
//! High-throughput data storage and buffering infrastructure for rust-daq.
//!
//! This crate provides the storage layer handling:
//!
//! - **[`RingBuffer`]** - Memory-mapped circular buffers for high-speed streaming
//! - **[`HDF5Writer`]** - HDF5 file output with compression
//! - **[`DocumentWriter`]** - Bluesky document persistence
//! - **[`StorageConfig`]** - Configurable data paths (env var, platform defaults)
//! - **Cross-Process Access** - Python and Julia can read ring buffers via mmap
//!
//! ## Quick Example
//!
//! ```rust,ignore
//! use storage::ring_buffer::RingBuffer;
//! use std::path::Path;
//!
//! // Create a ring buffer with 100 frame slots
//! let buffer = RingBuffer::create(Path::new("/tmp/daq_ring"), 100)?;
//!
//! // Write frames (from camera callback)
//! buffer.write(&frame_bytes)?;
//!
//! // Read frames (for storage or visualization)
//! let frame = buffer.read_latest()?;
//! ```
//!
//! ## Feature Flags
//!
//! - `storage_hdf5` - HDF5 file output with compression
//! - `storage_arrow` - Arrow IPC format support
//! - `storage_parquet` - Parquet columnar format
//! - `storage_tiff` - TIFF image stacks
//! - `storage_zarr` - Zarr V3 chunked arrays
//!
//! [`RingBuffer`]: ring_buffer::RingBuffer
//! [`HDF5Writer`]: hdf5_writer::HDF5Writer
//! [`DocumentWriter`]: document_writer::DocumentWriter
//! [`StorageConfig`]: config::StorageConfig

// TODO: Fix doc comment generic types to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]

pub mod arrow_writer;
pub mod comedi_writer;
pub mod config;
pub mod document_writer;
#[cfg(feature = "storage_hdf5")]
pub mod hdf5_annotation;
pub mod hdf5_recovery;
pub mod hdf5_writer;
#[cfg(feature = "storage_parquet")]
pub mod parquet_writer;
pub mod ring_buffer;
pub mod ring_buffer_reader;
pub mod tap_registry;
#[cfg(feature = "storage_tiff")]
pub mod tiff_writer;
#[cfg(feature = "storage_zarr")]
pub mod zarr_writer;

pub use comedi_writer::{
    AcquisitionMetadata, ChannelConfig, ComediStreamWriter, ComediStreamWriterBuilder,
    CompressionType, ContinuousAcquisitionSession, StorageFormat, StreamStats,
};
pub use config::StorageConfig;
pub use document_writer::DocumentWriter;
#[cfg(feature = "storage_hdf5")]
pub use hdf5_annotation::{add_run_annotation, read_run_annotations, RunAnnotation};
pub use hdf5_recovery::{recover_hdf5, RecoveryError, RecoveryReport};
pub use hdf5_writer::{HDF5Writer, Hdf5Metrics};
pub use ring_buffer::{AsyncRingBuffer, RingBuffer};
pub use ring_buffer_reader::{ReaderStats, RingBufferReader};

#[cfg(feature = "storage_parquet")]
pub use arrow_writer::ParquetDocumentWriter;
#[cfg(feature = "storage_arrow")]
pub use arrow_writer::{
    read_tensor_shape, ArrowDocumentWriter, TENSOR_DIM_NAMES_KEY, TENSOR_SHAPE_KEY,
};
#[cfg(feature = "storage_parquet")]
pub use parquet_writer::{ParquetCompression, ParquetWriter, ParquetWriterConfig};
#[cfg(feature = "storage_tiff")]
pub use tiff_writer::{LoanedFrame, TiffWriter};
#[cfg(feature = "storage_zarr")]
pub use zarr_writer::{ZarrArrayBuilder, ZarrWriter};

#[cfg(feature = "storage_hdf5")]
pub(crate) fn map_hdf5_err(e: hdf5::Error) -> common::error::DaqError {
    common::error::DaqError::Hdf5(e)
}
