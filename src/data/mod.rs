//! Data processing and storage modules.

// V1 legacy modules commented out due to removed DataProcessor/StorageWriter traits
// These need to be migrated to V3 architecture or removed
// See: JULES_FLEET_STATUS_2025-11-20.md Phase 1
// pub mod fft;
// pub mod iir_filter;
// pub mod processor;
// pub mod registry;
// pub mod storage;
// pub mod storage_factory;
// pub mod trigger;

pub mod hdf5_writer;
pub mod ring_buffer;
