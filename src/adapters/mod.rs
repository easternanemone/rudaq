// src/adapters/mod.rs

use anyhow::Result;
use async_trait::async_trait;

// V1 adapter (simple Adapter trait) - use full path: crate::adapters::serial::SerialAdapter
pub mod serial;

// V2 adapters (HardwareAdapter trait for daq-core)
pub mod serial_adapter;
pub use serial_adapter::SerialAdapter;

pub mod mock_adapter;
pub use mock_adapter::MockAdapter;

#[async_trait]
pub trait Adapter: Send + Sync {
    async fn write(&mut self, command: Vec<u8>) -> Result<()>;
    async fn read(&mut self, buffer: &mut Vec<u8>) -> Result<usize>;
    async fn write_and_read(&mut self, command: Vec<u8>, buffer: &mut Vec<u8>) -> Result<usize> {
        self.write(command).await?;
        self.read(buffer).await
    }
}