#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
//! Data pipeline integration tests
//!
//! Tests for HDF5, Arrow, and ring buffer data flow.
//!
//! # Test Coverage
//!
//! - Ring buffer operations (verified working in unit tests)
//! - HDF5 writer integration (when storage_hdf5 enabled)
//! - Arrow writer integration (when storage_arrow enabled)
//! - Ring buffer to HDF5 background writer flow
//! - High-throughput pipeline stress tests
//!
//! # Feature Gates
//!
//! Tests are conditionally compiled based on enabled features:
//! - `storage_hdf5` - HDF5 file format tests
//! - `storage_arrow` - Apache Arrow IPC format tests
//!
//! # Running Tests
//!
//! ```bash
//! # Test with Arrow support (HDF5 tests will be skipped if library not installed)
//! cargo test data_pipeline --features storage_arrow
//!
//! # Test with HDF5 support (requires HDF5 library: brew install hdf5)
//! cargo test data_pipeline --features storage_hdf5,storage_arrow
//!
//! # Test with both (if HDF5 available)
//! cargo test data_pipeline --features storage_hdf5,storage_arrow
//! ```
//!
//! **Note**: HDF5 tests require the HDF5 library to be installed:
//! - macOS: `brew install hdf5`
//! - Ubuntu: `sudo apt-get install libhdf5-dev`
//! - If HDF5 is not available, those tests will be skipped automatically.

mod helpers;

#[cfg(feature = "storage_hdf5")]
mod hdf5_tests;

#[cfg(feature = "storage_arrow")]
mod arrow_tests;

mod ringbuffer_tests;

#[cfg(feature = "storage_hdf5")]
mod ringbuffer_hdf5_tests;

#[cfg(all(test, feature = "storage_arrow"))]
mod performance_tests;
