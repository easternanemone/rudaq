Strategy: High-Throughput Data Storage

Data acquisition requires dumping gigabytes of data to disk safely, concurrently, and quickly. Legacy formats like HDF5 rely on C libraries with global locks, which can choke highly concurrent Rust applications.

1. Zarrs (Cloud-Native Chunked Storage)

The Need: Storing massive N-dimensional arrays (camera streams, hyper-spectral data) at high speeds.
The Solution: zarrs, a pure Rust implementation of the Zarr V3 storage format.

Benefits:

Lock-Free Concurrency: Unlike HDF5, Zarr stores data as independent compressed chunks (e.g., one file per 100x100x100 block). This allows the Rust async runtime to write chunks to disk (or S3) entirely in parallel.

Cloud-Ready: Zarr maps perfectly to key-value stores. You can seamlessly stream DAQ data directly to AWS S3 or local NVMe drives using the same API.

Implementation Strategy: Expand crates/storage/src/zarr_writer.rs. Configure the zarrs writer to use Blosc or LZ4 compression. Have your camera drivers push chunks of frames directly to a thread pool that compresses and flushes them to the Zarr store without blocking the main hardware loop.

2. SurrealDB (Metadata & Configuration)

The Need: Tracking complex, highly relational data: which hardware generated which data, calibration profiles (e.g., mechelle_5000.toml), and experiment states.
The Solution: Maximize the use of surrealdb (which is currently under test in surrealdb_e2e).

Benefits: It acts as both a document store (like MongoDB) and a Graph database. It runs purely in Rust (embedded via RocksDB) or as a remote cluster.

Implementation Strategy: Use SurrealDB's graph relations to link data. Instead of flat metadata files, insert a Camera node, an Experiment node, and a ZarrDataset node. Create graph edges: Experiment -> GENERATED -> ZarrDataset and ZarrDataset -> CAPTURED_BY -> Camera. This allows rich querying like "Find all datasets captured by PVCAM serial #1234 where temperature > 20C."
