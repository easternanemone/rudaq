//! Shared resource pool for serial connections.
//!
//! Keeps a single shared serial port per port path and reuses it across
//! drivers that need the same physical connection (e.g., multidrop buses).
//! Uses `common::serial` (serial2-tokio) for async port opening.

#[cfg(feature = "serial")]
use common::serial::{SharedPortUnbuffered, open_serial_async, wrap_shared_unbuffered};

#[cfg(feature = "serial")]
use anyhow::Result;
#[cfg(feature = "serial")]
use std::collections::HashMap;

/// Pool of shared serial connections keyed by port path.
#[cfg(feature = "serial")]
#[derive(Default)]
pub struct SerialPool {
    connections: HashMap<String, SharedPortUnbuffered>,
}

#[cfg(feature = "serial")]
impl SerialPool {
    /// Create an empty pool.
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    /// Get an existing connection for `port`, or open a new one at `baud`.
    pub async fn get_or_create(&mut self, port: &str, baud: u32) -> Result<SharedPortUnbuffered> {
        if let Some(existing) = self.connections.get(port) {
            return Ok(existing.clone());
        }

        let dyn_port = open_serial_async(port, baud, "SerialPool").await?;
        let shared = wrap_shared_unbuffered(dyn_port);
        self.connections.insert(port.to_string(), shared.clone());
        Ok(shared)
    }
}
