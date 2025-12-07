//! HDF5 storage example (V5 headless-first data path)
//!
//! This placeholder shows how to enable the HDF5 backend when running the daemon.
//!
//! Build & run:
//!   cargo run --example hdf5_storage_example --features storage_hdf5
//!
//! The full HDF5 writer is integrated via `src/data/hdf5_writer.rs` and the
//! ring buffer. For a working sample, see docs/perf/v5_bench.md and
//! docs/architecture/V5_ARCHITECTURE.md.

#[cfg(feature = "storage_hdf5")]
fn main() {
    println!("HDF5 backend is enabled. Integrate `HDF5Writer` with the ring buffer in your application.");
    println!("See docs/architecture/V5_ARCHITECTURE.md for the data path and\nexamples of configuring the writer in the daemon.");
}

#[cfg(not(feature = "storage_hdf5"))]
fn main() {
    println!("Enable the 'storage_hdf5' feature to run this example:");
    println!("  cargo run --example hdf5_storage_example --features storage_hdf5");
}
