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
#![cfg(all(
    feature = "networking",
    feature = "storage_hdf5",
    feature = "storage_arrow"
))]

use std::sync::Arc;

use daq_server::grpc::DaqServer;
use daq_storage::ring_buffer::RingBuffer;

#[tokio::test]
async fn daq_server_new_smoke() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ring_path = temp_dir.path().join("ring.buf");
    let ring_buffer = Arc::new(RingBuffer::create(&ring_path, 4).unwrap());

    // Should construct without panicking
    let _server = DaqServer::new(Some(ring_buffer));
}
