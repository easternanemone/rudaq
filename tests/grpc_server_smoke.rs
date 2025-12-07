#![cfg(all(
    feature = "networking",
    feature = "storage_hdf5",
    feature = "storage_arrow"
))]

use std::sync::Arc;

use rust_daq::data::ring_buffer::RingBuffer;
use rust_daq::grpc::DaqServer;

#[tokio::test]
async fn daq_server_new_smoke() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ring_path = temp_dir.path().join("ring.buf");
    let ring_buffer = Arc::new(RingBuffer::create(&ring_path, 4).unwrap());

    // Should construct without panicking
    let _server = DaqServer::new(Some(ring_buffer));
}
