use super::Adapter;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SerialAdapter {
    port: Arc<Mutex<Box<dyn SerialPort>>>,
}

impl SerialAdapter {
    pub fn new(port: Box<dyn SerialPort>) -> Self {
        Self {
            port: Arc::new(Mutex::new(port)),
        }
    }
}

#[async_trait]
impl Adapter for SerialAdapter {
    async fn write(&mut self, command: Vec<u8>) -> Result<()> {
        let port = self.port.clone();
        tokio::task::spawn_blocking(move || port.blocking_lock().write_all(&command))
            .await
            .context("Task panicked")??;
        Ok(())
    }

    async fn read(&mut self, buffer: &mut Vec<u8>) -> Result<usize> {
        let port = self.port.clone();
        let bytes_read = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, usize)> {
            let mut temp_buffer = vec![0; 1024];
            let bytes_read = port.blocking_lock().read(&mut temp_buffer)?;
            Ok((temp_buffer, bytes_read))
        })
        .await
        .context("Task panicked")??;
        buffer.clear();
        buffer.extend_from_slice(&bytes_read.0[..bytes_read.1]);
        Ok(bytes_read.1)
    }
}
