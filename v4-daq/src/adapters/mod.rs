// V4 Adapters Module

use anyhow::Result;
use async_trait::async_trait;

// V4 serial adapter
pub mod serial_adapter;
pub use serial_adapter::SerialAdapter;

// Supporting modules
pub mod command_batch;
pub mod mock;
pub mod mock_adapter;
pub use mock_adapter::MockAdapter;

#[cfg(feature = "instrument_serial")]
pub mod serial;

/// Generic async adapter trait for hardware communication
#[async_trait]
pub trait Adapter: Send + Sync {
    async fn write(&mut self, command: Vec<u8>) -> Result<()>;
    async fn read(&mut self, buffer: &mut Vec<u8>) -> Result<usize>;
    async fn write_and_read(&mut self, command: Vec<u8>, buffer: &mut Vec<u8>) -> Result<usize> {
        self.write(command).await?;
        self.read(buffer).await
    }
}
