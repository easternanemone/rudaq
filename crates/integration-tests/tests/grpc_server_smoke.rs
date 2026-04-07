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
    feature = "server",
    feature = "storage_hdf5",
    feature = "storage_arrow"
))]

use std::sync::Arc;

use experiment::RunEngine;
use hardware::registry::DeviceRegistry;
use server::grpc::DaqServer;
use storage::ring_buffer::RingBuffer;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn daq_server_new_smoke() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ring_path = temp_dir.path().join("ring.buf");
    let ring_buffer = Arc::new(RingBuffer::create(&ring_path, 4).unwrap());

    // Should construct without panicking
    #[cfg(feature = "scripting")]
    {
        let registry = DeviceRegistry::new();
        let run_engine = Arc::new(RunEngine::new(registry));
        let token = CancellationToken::new();
        let _server = DaqServer::new(Some(ring_buffer), run_engine, token)
            .expect("Failed to create DaqServer");
    }
    #[cfg(not(feature = "scripting"))]
    {
        let _server = DaqServer::new(Some(ring_buffer)).expect("Failed to create DaqServer");
    }
}
