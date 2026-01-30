//! Shared port management for RS-485 multidrop bus devices.
//!
//! Multiple ELL14 devices can share a single serial port. This module
//! provides a static registry to track and reuse open ports.

use common::serial::open_serial_async;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::OnceLock;

// Re-export SharedPort type for backward compatibility
pub use common::serial::SharedPortUnbuffered as SharedPort;

/// Module-local registry for shared serial ports.
static SHARED_PORTS: OnceLock<RwLock<HashMap<String, SharedPort>>> = OnceLock::new();

fn port_registry() -> &'static RwLock<HashMap<String, SharedPort>> {
    SHARED_PORTS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Get an existing shared port if one is already open for the given path.
pub fn get_existing_port(port_path: &str) -> Option<SharedPort> {
    let registry = port_registry().read();
    registry.get(port_path).cloned()
}

/// Register a newly opened port in the shared registry.
pub fn register_port(port_path: &str, port: SharedPort) {
    let mut registry = port_registry().write();
    registry.insert(port_path.to_string(), port);
    tracing::info!(port = port_path, "Registered new ELL14 shared port");
}

/// Remove a port from the registry (e.g., when it becomes stale).
pub fn remove_port(port_path: &str) -> bool {
    let mut registry = port_registry().write();
    let removed = registry.remove(port_path).is_some();
    if removed {
        tracing::info!(
            port = port_path,
            "Removed stale ELL14 shared port from registry"
        );
    }
    removed
}

/// Get or create a shared port for the given path with default timeout (500ms).
///
/// If a port is already open for this path, returns the existing connection.
/// Otherwise, opens a new port and registers it.
///
/// Note: Uses 9600 baud (legacy ELL14 default). For newer ELL14 units at 115200 baud,
/// use `get_or_open_port_115200` instead.
pub async fn get_or_open_port(port_path: &str) -> anyhow::Result<SharedPort> {
    get_or_open_port_with_timeout(port_path, std::time::Duration::from_millis(500)).await
}

/// Get or create a shared port at 115200 baud (common ELL14 configuration).
///
/// Many ELL14 units are factory-configured to 115200 baud instead of the
/// default 9600. Use this function for those devices.
pub async fn get_or_open_port_115200(port_path: &str) -> anyhow::Result<SharedPort> {
    get_or_open_port_with_baud(port_path, 115200, std::time::Duration::from_millis(500)).await
}

/// Get or create a shared port with custom baud rate.
pub async fn get_or_open_port_with_baud(
    port_path: &str,
    baud_rate: u32,
    _timeout: std::time::Duration,
) -> anyhow::Result<SharedPort> {
    use tokio::io::AsyncWriteExt;

    // Check if already open
    if let Some(port) = get_existing_port(port_path) {
        // Health check: try to flush the port to verify it's still connected
        let health_check = async {
            let mut guard = port.lock().await;
            guard.flush().await
        };

        match tokio::time::timeout(std::time::Duration::from_millis(100), health_check).await {
            Ok(Ok(())) => {
                tracing::debug!(port = port_path, "Reusing healthy ELL14 shared port");
                return Ok(port);
            }
            Ok(Err(e)) => {
                tracing::warn!(port = port_path, error = %e, "ELL14 shared port health check failed, reopening");
                remove_port(port_path);
            }
            Err(_) => {
                tracing::warn!(
                    port = port_path,
                    "ELL14 shared port health check timed out, reopening"
                );
                remove_port(port_path);
            }
        }
    }

    // Open new port using shared utility
    let stream = open_serial_async(port_path, baud_rate, "ELL14").await?;
    let shared = common::serial::wrap_shared_unbuffered(Box::new(stream));

    // Store in registry
    register_port(port_path, shared.clone());

    Ok(shared)
}

/// Get or create a shared port for the given path with custom timeout.
///
/// If a port is already open for this path, performs a health check and returns
/// the existing connection if healthy. Otherwise, opens a new port and registers it.
pub async fn get_or_open_port_with_timeout(
    port_path: &str,
    timeout: std::time::Duration,
) -> anyhow::Result<SharedPort> {
    get_or_open_port_with_baud(port_path, 9600, timeout).await
}

/// Close all shared ports (for cleanup/testing).
pub fn close_all_ports() {
    if let Some(registry) = SHARED_PORTS.get() {
        let mut guard = registry.write();
        let count = guard.len();
        guard.clear();
        tracing::info!(count, "Closed all shared ELL14 ports");
    }
}

/// Get the number of currently open shared ports.
pub fn port_count() -> usize {
    SHARED_PORTS.get().map(|r| r.read().len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_registry_initializes() {
        // Just verify we can access the registry without panicking
        // port_count() returns usize, so it's always >= 0
        let _count = port_count();
    }

    #[test]
    fn test_close_all_ports() {
        // Should not panic even if no ports are open
        close_all_ports();
    }
}
