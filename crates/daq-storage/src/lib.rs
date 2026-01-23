// TODO: Fix doc comment generic types to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]

pub mod arrow_writer;
pub mod comedi_writer;
pub mod document_writer;
pub mod hdf5_writer;
#[cfg(feature = "storage_hdf5")]
pub mod hdf5_annotation;
pub mod ring_buffer;
pub mod ring_buffer_reader;
pub mod tap_registry;
#[cfg(feature = "storage_tiff")]
pub mod tiff_writer;

pub use comedi_writer::{
    AcquisitionMetadata, ChannelConfig, ComediStreamWriter, ComediStreamWriterBuilder,
    CompressionType, ContinuousAcquisitionSession, StorageFormat, StreamStats,
};
pub use document_writer::DocumentWriter;
pub use hdf5_writer::HDF5Writer;
#[cfg(feature = "storage_hdf5")]
pub use hdf5_annotation::{add_run_annotation, read_run_annotations, RunAnnotation};
pub use ring_buffer::{AsyncRingBuffer, RingBuffer};
pub use ring_buffer_reader::{ReaderStats, RingBufferReader};

#[cfg(feature = "storage_arrow")]
pub use arrow_writer::ArrowDocumentWriter;
#[cfg(feature = "storage_parquet")]
pub use arrow_writer::ParquetDocumentWriter;
#[cfg(feature = "storage_tiff")]
pub use tiff_writer::{LoanedFrame, TiffWriter};
