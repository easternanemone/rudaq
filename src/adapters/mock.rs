
use super::Adapter;
use anyhow::Result;
use async_trait::async_trait;


pub struct MockAdapter;

#[async_trait]
impl Adapter for MockAdapter {
    async fn write(&mut self, _command: &[u8]) -> Result<()> {
        Ok(())
    }

    async fn read(&mut self, _buffer: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}
