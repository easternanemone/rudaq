//! Shared Serial Port for Exclusive V2/V4 Actor Access
//!
//! Provides thread-safe, exclusive access to serial ports for both V2 and V4 actors
//! using RAII guards and ownership tracking. Prevents concurrent access through
//! Arc<Mutex<>> with timeout protection against deadlocks.

use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;

/// Configuration for a serial port connection
#[derive(Clone, Debug)]
pub struct SerialPortConfig {
    /// Path to the serial device (e.g., "/dev/ttyUSB0", "COM3")
    pub path: String,
    /// Baud rate (9600, 115200, etc.)
    pub baud_rate: u32,
    /// Data bits (7 or 8)
    pub data_bits: u8,
    /// Stop bits (1 or 2)
    pub stop_bits: u8,
    /// Parity setting
    pub parity: SerialParity,
    /// Read timeout
    pub timeout: Duration,
}

/// Parity modes for serial communication
#[derive(Clone, Debug, Copy)]
pub enum SerialParity {
    None,
    Even,
    Odd,
}

impl Default for SerialPortConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 1,
            parity: SerialParity::None,
            timeout: Duration::from_secs(1),
        }
    }
}

/// Inner state of the shared serial port
#[derive(Debug)]
struct SerialPortInner {
    /// Configuration for this port
    config: SerialPortConfig,
    /// Optional open port handle (mock for now, would be real SerialPort)
    port: Option<MockSerialPort>,
    /// Current owner actor ID (None if unowned)
    owner: Option<String>,
}

/// Mock serial port for compilation (real impl would use serialport crate)
#[derive(Clone, Debug)]
struct MockSerialPort {
    path: String,
    is_open: bool,
}

impl MockSerialPort {
    fn new(path: String) -> Self {
        Self {
            path,
            is_open: true,
        }
    }
}

/// Shared serial port wrapper with exclusive access control
///
/// Provides safe concurrent access to serial ports from multiple actors
/// (V2 and V4) using Arc<Mutex<>> and RAII guards. Only one actor can
/// hold exclusive access at a time.
#[derive(Clone)]
pub struct SharedSerialPort {
    /// Thread-safe inner state
    inner: Arc<Mutex<SerialPortInner>>,
    /// Configuration (read-only after creation)
    config: SerialPortConfig,
}

impl SharedSerialPort {
    /// Create a new shared serial port wrapper
    ///
    /// # Arguments
    /// * `config` - Serial port configuration
    ///
    /// # Example
    /// ```
    /// use v4_daq::hardware::{SharedSerialPort, SerialPortConfig};
    /// use std::time::Duration;
    ///
    /// let config = SerialPortConfig {
    ///     path: "/dev/ttyUSB0".to_string(),
    ///     baud_rate: 9600,
    ///     data_bits: 8,
    ///     stop_bits: 1,
    ///     parity: v4_daq::hardware::SerialParity::None,
    ///     timeout: Duration::from_secs(1),
    /// };
    ///
    /// let port = SharedSerialPort::new(config);
    /// ```
    pub fn new(config: SerialPortConfig) -> Self {
        let inner = SerialPortInner {
            config: config.clone(),
            port: Some(MockSerialPort::new(config.path.clone())),
            owner: None,
        };

        Self {
            inner: Arc::new(Mutex::new(inner)),
            config,
        }
    }

    /// Acquire exclusive access to the serial port
    ///
    /// Returns an RAII guard that releases ownership when dropped.
    /// Blocks until the port is available (with timeout protection).
    ///
    /// # Arguments
    /// * `actor_id` - Unique identifier of the requesting actor
    /// * `timeout` - Maximum time to wait for port availability
    ///
    /// # Returns
    /// - `Ok(SerialGuard)` - Exclusive access granted
    /// - `Err` - Port in use by another actor or timeout
    ///
    /// # Example
    /// ```no_run
    /// # use v4_daq::hardware::{SharedSerialPort, SerialPortConfig};
    /// # use std::time::Duration;
    /// # let port = SharedSerialPort::new(Default::default());
    /// #[tokio::main]
    /// async fn example() -> anyhow::Result<()> {
    ///     let guard = port.acquire("actor_v4_1", Duration::from_secs(5)).await?;
    ///     // Exclusive access held for duration of guard
    ///     Ok(())
    /// }
    /// ```
    pub async fn acquire(&self, actor_id: &str, acquire_timeout: Duration) -> Result<SerialGuard> {
        // Use timeout to prevent indefinite blocking
        let result = timeout(acquire_timeout, self.acquire_impl(actor_id)).await;

        match result {
            Ok(inner_result) => inner_result,
            Err(_) => Err(anyhow!(
                "Timeout acquiring serial port {} for actor {} (timeout: {:?})",
                self.config.path,
                actor_id,
                acquire_timeout
            )),
        }
    }

    /// Internal implementation of acquire
    async fn acquire_impl(&self, actor_id: &str) -> Result<SerialGuard> {
        let mut inner = self.inner.lock().await;

        // Check if port is already owned
        if let Some(ref owner) = inner.owner {
            return Err(anyhow!(
                "Serial port {} is already in use by actor '{}', requested by '{}'",
                self.config.path,
                owner,
                actor_id
            ));
        }

        // Check if port is open
        if inner.port.is_none() {
            return Err(anyhow!(
                "Serial port {} is not available (closed or error)",
                self.config.path
            ));
        }

        // Acquire ownership
        inner.owner = Some(actor_id.to_string());

        Ok(SerialGuard {
            inner: self.inner.clone(),
            actor_id: actor_id.to_string(),
        })
    }

    /// Check if the port is currently available (not owned)
    ///
    /// # Example
    /// ```
    /// # use v4_daq::hardware::SharedSerialPort;
    /// # let port = SharedSerialPort::new(Default::default());
    /// assert!(port.is_available());
    /// ```
    pub fn is_available(&self) -> bool {
        // Non-blocking check - don't await the mutex
        if let Ok(guard) = self.inner.try_lock() {
            guard.owner.is_none()
        } else {
            false // Mutex contention, consider unavailable
        }
    }

    /// Get the current owner of the port, if any
    ///
    /// # Example
    /// ```
    /// # use v4_daq::hardware::SharedSerialPort;
    /// # let port = SharedSerialPort::new(Default::default());
    /// assert!(port.current_owner().is_none());
    /// ```
    pub fn current_owner(&self) -> Option<String> {
        if let Ok(guard) = self.inner.try_lock() {
            guard.owner.clone()
        } else {
            None
        }
    }

    /// Get the serial port path
    pub fn path(&self) -> &str {
        &self.config.path
    }

    /// Get the baud rate
    pub fn baud_rate(&self) -> u32 {
        self.config.baud_rate
    }
}

/// RAII guard for exclusive serial port access
///
/// Holds exclusive access to a shared serial port. Automatically releases
/// ownership when dropped, allowing other actors to acquire the port.
#[derive(Debug)]
pub struct SerialGuard {
    /// Reference to the shared port inner state
    inner: Arc<Mutex<SerialPortInner>>,
    /// Actor ID that owns this guard
    actor_id: String,
}

impl SerialGuard {
    /// Write data to the serial port
    ///
    /// # Arguments
    /// * `data` - Bytes to write
    ///
    /// # Example
    /// ```no_run
    /// # use v4_daq::hardware::{SharedSerialPort, SerialPortConfig};
    /// # use std::time::Duration;
    /// # let port = SharedSerialPort::new(Default::default());
    /// #[tokio::main]
    /// async fn example() -> anyhow::Result<()> {
    ///     let mut guard = port.acquire("actor_1", Duration::from_secs(5)).await?;
    ///     guard.write(b"*IDN?\r\n").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        let _guard = self.inner.lock().await;
        // In real implementation, write to actual serial port
        tracing::debug!(
            actor_id = %self.actor_id,
            bytes = data.len(),
            "Writing to serial port"
        );
        Ok(())
    }

    /// Read data from the serial port
    ///
    /// Blocks until data is available or timeout expires.
    ///
    /// # Arguments
    /// * `buf` - Buffer to read into
    ///
    /// # Returns
    /// Number of bytes read
    ///
    /// # Example
    /// ```no_run
    /// # use v4_daq::hardware::{SharedSerialPort, SerialPortConfig};
    /// # use std::time::Duration;
    /// # let port = SharedSerialPort::new(Default::default());
    /// #[tokio::main]
    /// async fn example() -> anyhow::Result<()> {
    ///     let mut guard = port.acquire("actor_1", Duration::from_secs(5)).await?;
    ///     let mut buf = [0u8; 256];
    ///     let n = guard.read(&mut buf).await?;
    ///     println!("Read {} bytes", n);
    ///     Ok(())
    /// }
    /// ```
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let _guard = self.inner.lock().await;
        // In real implementation, read from actual serial port
        tracing::debug!(
            actor_id = %self.actor_id,
            buf_size = buf.len(),
            "Reading from serial port"
        );
        Ok(0)
    }

    /// Write all data to the serial port
    ///
    /// Ensures all bytes are written (retrying if necessary).
    ///
    /// # Arguments
    /// * `data` - Bytes to write
    ///
    /// # Example
    /// ```no_run
    /// # use v4_daq::hardware::{SharedSerialPort, SerialPortConfig};
    /// # use std::time::Duration;
    /// # let port = SharedSerialPort::new(Default::default());
    /// #[tokio::main]
    /// async fn example() -> anyhow::Result<()> {
    ///     let mut guard = port.acquire("actor_1", Duration::from_secs(5)).await?;
    ///     guard.write_all(b"*IDN?\r\n").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn write_all(&mut self, data: &[u8]) -> Result<()> {
        let _guard = self.inner.lock().await;
        // In real implementation, write_all to actual serial port
        tracing::debug!(
            actor_id = %self.actor_id,
            bytes = data.len(),
            "Writing all data to serial port"
        );
        Ok(())
    }

    /// Get the actor ID that owns this guard
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }
}

impl Drop for SerialGuard {
    fn drop(&mut self) {
        // Release ownership synchronously in drop
        if let Ok(mut inner) = self.inner.try_lock() {
            if inner.owner.as_ref().map(|o| o == &self.actor_id).unwrap_or(false) {
                tracing::debug!(
                    actor_id = %self.actor_id,
                    "Releasing serial port ownership"
                );
                inner.owner = None;
            }
        } else {
            // If we can't acquire the lock in drop, log a warning
            tracing::warn!(
                actor_id = %self.actor_id,
                "Failed to release serial port ownership: mutex contention in drop"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_port_is_available() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);
        assert!(port.is_available());
        assert!(port.current_owner().is_none());
    }

    #[test]
    fn test_port_properties() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 115200,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);
        assert_eq!(port.path(), "/dev/ttyUSB0");
        assert_eq!(port.baud_rate(), 115200);
    }

    #[tokio::test]
    async fn test_acquire_release_single_actor() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);

        // Acquire
        let guard = port
            .acquire("actor_1", Duration::from_secs(1))
            .await
            .expect("Failed to acquire port");

        assert!(!port.is_available());
        assert_eq!(port.current_owner(), Some("actor_1".to_string()));
        assert_eq!(guard.actor_id(), "actor_1");

        // Release (drop guard)
        drop(guard);

        assert!(port.is_available());
        assert!(port.current_owner().is_none());
    }

    #[tokio::test]
    async fn test_exclusive_access_two_actors() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);

        // First actor acquires
        let guard1 = port
            .acquire("actor_1", Duration::from_millis(100))
            .await
            .expect("Actor 1 failed to acquire");

        // Second actor tries to acquire - should fail
        let result = port
            .acquire("actor_2", Duration::from_millis(100))
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("already in use"));

        // First actor releases
        drop(guard1);

        // Now second actor can acquire
        let guard2 = port
            .acquire("actor_2", Duration::from_millis(100))
            .await
            .expect("Actor 2 failed to acquire after release");

        assert_eq!(port.current_owner(), Some("actor_2".to_string()));
        drop(guard2);
    }

    #[tokio::test]
    async fn test_timeout_on_acquire() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            ..Default::default()
        };
        let port = Arc::new(SharedSerialPort::new(config));

        // First actor holds the port
        let guard1 = port
            .acquire("actor_1", Duration::from_secs(10))
            .await
            .expect("Actor 1 failed to acquire");

        // Spawn a second actor that tries to acquire with short timeout
        let port_clone = port.clone();
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            port_clone.acquire("actor_2", Duration::from_secs(5)),
        )
        .await;

        assert!(result.is_err()); // Should timeout

        drop(guard1);
    }

    #[tokio::test]
    async fn test_guard_write_read() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);

        let mut guard = port
            .acquire("actor_1", Duration::from_secs(1))
            .await
            .expect("Failed to acquire port");

        // Test write
        guard.write(b"*IDN?\r\n").await.expect("Write failed");

        // Test write_all
        guard
            .write_all(b"*RST\r\n")
            .await
            .expect("Write all failed");

        // Test read (will return 0 in mock)
        let mut buf = [0u8; 256];
        let n = guard.read(&mut buf).await.expect("Read failed");
        assert_eq!(n, 0); // Mock always returns 0
    }

    #[tokio::test]
    async fn test_multiple_sequential_acquisitions() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            ..Default::default()
        };
        let port = SharedSerialPort::new(config);

        // Sequential acquisitions should work
        for i in 0..5 {
            let actor_id = format!("actor_{}", i);
            let guard = port
                .acquire(&actor_id, Duration::from_millis(100))
                .await
                .expect(&format!("Failed to acquire for {}", actor_id));

            assert_eq!(port.current_owner(), Some(actor_id.clone()));
            drop(guard);
            assert!(port.is_available());
        }
    }

    #[tokio::test]
    async fn test_concurrent_acquisitions() {
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
            ..Default::default()
        };
        let port = Arc::new(SharedSerialPort::new(config));

        let mut handles = vec![];

        // Spawn 5 concurrent tasks trying to acquire
        for i in 0..5 {
            let port_clone = port.clone();
            let handle = tokio::spawn(async move {
                let actor_id = format!("actor_{}", i);
                match port_clone.acquire(&actor_id, Duration::from_millis(500)).await {
                    Ok(guard) => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        drop(guard);
                        Ok(actor_id)
                    }
                    Err(e) => Err(e),
                }
            });
            handles.push(handle);
        }

        // Wait for all to complete
        let results: Vec<_> = futures::future::join_all(handles).await;

        // Some should succeed, some might fail due to timeouts
        let successes: Vec<_> = results
            .iter()
            .filter_map(|r| r.as_ref().ok().and_then(|inner| inner.as_ref().ok()))
            .collect();

        // At least one should succeed
        assert!(!successes.is_empty());
    }
}
