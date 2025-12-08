//! Shared resource pool for serial connections.
//!
//! Keeps a single `tokio_serial::SerialStream` per port path and reuses it across
//! drivers that need the same physical connection (e.g., multidrop buses).

#[cfg(feature = "tokio_serial")]
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Pool of shared serial connections keyed by port path.
#[cfg(feature = "tokio_serial")]
#[derive(Default)]
pub struct SerialPool {
    connections: HashMap<String, Arc<Mutex<SerialStream>>>,
}

#[cfg(feature = "tokio_serial")]
impl SerialPool {
    /// Create an empty pool.
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    /// Get an existing connection for `port`, or open a new one at `baud`.
    pub async fn get_or_create(
        &mut self,
        port: &str,
        baud: u32,
    ) -> Result<Arc<Mutex<SerialStream>>> {
        if let Some(existing) = self.connections.get(port) {
            return Ok(existing.clone());
        }

        let stream = tokio_serial::new(port, baud).open_native_async()?;
        let arc = Arc::new(Mutex::new(stream));
        self.connections.insert(port.to_string(), arc.clone());
        Ok(arc)
    }
}
