//! Serial Port Abstractions for Driver Crates
//!
//! This module provides shared types and utilities for async serial communication
//! that can be used by driver crates without duplicating definitions.
//!
//! # Feature Flag
//!
//! This module requires the `serial` feature to be enabled:
//!
//! ```toml
//! [dependencies]
//! daq-core = { path = "../daq-core", features = ["serial"] }
//! ```
//!
//! # Types
//!
//! - [`SerialPortIO`]: Trait alias combining AsyncRead + AsyncWrite for serial ports
//! - [`DynSerial`]: Type-erased boxed serial port
//! - [`SharedPort`]: Thread-safe shared serial port with buffered reading
//! - [`SharedPortUnbuffered`]: Thread-safe shared serial port without buffering
//!
//! # Utilities
//!
//! - [`open_serial_async`]: Open a serial port with spawn_blocking
//! - [`drain_serial_buffer`]: Drain stale data from a serial port
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_core::serial::{open_serial_async, drain_serial_buffer, SharedPort};
//!
//! // Open a serial port
//! let port = open_serial_async("/dev/ttyUSB0", 9600, "My Device").await?;
//! let shared = wrap_shared(Box::new(port));
//!
//! // Drain any stale data before communication
//! let mut guard = shared.lock().await;
//! let discarded = drain_serial_buffer(guard.get_mut(), 50).await;
//! ```

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, BufReader};
use tokio::sync::Mutex;

// =============================================================================
// Serial Port Trait
// =============================================================================

/// Trait alias for async serial port I/O.
///
/// Any type implementing `AsyncRead + AsyncWrite + Unpin + Send` can be used
/// as a serial port. This includes:
/// - `tokio_serial::SerialStream` (real hardware)
/// - `tokio::io::DuplexStream` (testing)
/// - Any mock implementing the async I/O traits
pub trait SerialPortIO: AsyncRead + AsyncWrite + Unpin + Send {}

// Blanket implementation for all types meeting the requirements
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SerialPortIO for T {}

// =============================================================================
// Type Aliases
// =============================================================================

/// Type-erased boxed serial port.
///
/// Use this when you need to store a serial port without knowing its concrete type.
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::serial::DynSerial;
///
/// fn create_port(path: &str) -> anyhow::Result<DynSerial> {
///     let port = tokio_serial::new(path, 9600).open_native_async()?;
///     Ok(Box::new(port))
/// }
/// ```
pub type DynSerial = Box<dyn SerialPortIO>;

/// Thread-safe shared serial port with buffered reading.
///
/// This is the primary type for sharing a serial port between multiple async tasks.
/// The `BufReader` wrapper enables efficient line-by-line reading.
///
/// # Why BufReader?
///
/// Many serial protocols (RS-232, ASCII command/response) use line-delimited messages.
/// `BufReader` enables `read_line()` and `read_until()` methods that accumulate data
/// until a delimiter is found.
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::serial::SharedPort;
/// use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
///
/// async fn send_query(port: &SharedPort, command: &str) -> anyhow::Result<String> {
///     let mut guard = port.lock().await;
///
///     // Write command
///     guard.get_mut().write_all(format!("{}\r\n", command).as_bytes()).await?;
///
///     // Read response
///     let mut response = String::new();
///     guard.read_line(&mut response).await?;
///
///     Ok(response.trim().to_string())
/// }
/// ```
pub type SharedPort = Arc<Mutex<BufReader<DynSerial>>>;

/// Thread-safe shared serial port without buffering.
///
/// Use this when you need direct byte-level access without `BufReader` overhead.
/// Suitable for binary protocols or when managing buffering manually.
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::serial::SharedPortUnbuffered;
/// use tokio::io::{AsyncReadExt, AsyncWriteExt};
///
/// async fn read_bytes(port: &SharedPortUnbuffered, buf: &mut [u8]) -> anyhow::Result<usize> {
///     let mut guard = port.lock().await;
///     let n = guard.read(buf).await?;
///     Ok(n)
/// }
/// ```
pub type SharedPortUnbuffered = Arc<Mutex<DynSerial>>;

// =============================================================================
// Helper Functions
// =============================================================================

/// Create a SharedPort from a type-erased serial port.
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::serial::{DynSerial, SharedPort, wrap_shared};
///
/// let port: DynSerial = Box::new(tokio_serial::new("/dev/ttyUSB0", 9600).open_native_async()?);
/// let shared: SharedPort = wrap_shared(port);
/// ```
pub fn wrap_shared(port: DynSerial) -> SharedPort {
    Arc::new(Mutex::new(BufReader::new(port)))
}

/// Create a SharedPortUnbuffered from a type-erased serial port.
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::serial::{DynSerial, SharedPortUnbuffered, wrap_shared_unbuffered};
///
/// let port: DynSerial = Box::new(tokio_serial::new("/dev/ttyUSB0", 9600).open_native_async()?);
/// let shared: SharedPortUnbuffered = wrap_shared_unbuffered(port);
/// ```
pub fn wrap_shared_unbuffered(port: DynSerial) -> SharedPortUnbuffered {
    Arc::new(Mutex::new(port))
}

// =============================================================================
// Serial Port Utilities
// =============================================================================

/// Open a serial port asynchronously using spawn_blocking.
///
/// This function wraps the serial port opening in `spawn_blocking` to avoid
/// blocking the async runtime during port initialization. Standard settings
/// are applied: 8N1, no flow control.
///
/// # Parameters
///
/// - `port_path`: Path to the serial port (e.g., "/dev/ttyUSB0")
/// - `baud_rate`: Baud rate (e.g., 9600, 115200)
/// - `device_name`: Human-readable device name for error messages
///
/// # Returns
///
/// A `tokio_serial::SerialStream` ready for async I/O.
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::serial::open_serial_async;
///
/// let port = open_serial_async("/dev/ttyUSB0", 9600, "ESP300").await?;
/// ```
///
/// # Errors
///
/// Returns an error if the port cannot be opened or spawn_blocking fails.
pub async fn open_serial_async(
    port_path: &str,
    baud_rate: u32,
    device_name: &str,
) -> anyhow::Result<tokio_serial::SerialStream> {
    use anyhow::Context;
    use tokio::task::spawn_blocking;
    use tokio_serial::SerialPortBuilderExt;

    let port_path_owned = port_path.to_string();
    let device_name_owned = device_name.to_string();

    spawn_blocking(move || {
        tokio_serial::new(&port_path_owned, baud_rate)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!(
                "Failed to open {} serial port: {}",
                device_name_owned, port_path_owned
            ))
    })
    .await
    .context("spawn_blocking for serial port opening failed")?
}

/// Drain stale data from a serial port buffer.
///
/// This function aggressively reads and discards data from the serial port until
/// no more data is immediately available. Useful for clearing buffers before
/// sending commands, especially on RS-485 multidrop buses where other devices
/// may have sent data.
///
/// # Parameters
///
/// - `port`: Mutable reference to the serial port (or any AsyncRead)
/// - `timeout_ms`: Timeout in milliseconds for the drain operation
///
/// # Returns
///
/// Total number of bytes discarded.
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::serial::{drain_serial_buffer, SharedPort};
///
/// async fn clear_and_send(port: &SharedPort, command: &str) -> anyhow::Result<()> {
///     let mut guard = port.lock().await;
///     
///     // Clear stale data
///     let discarded = drain_serial_buffer(guard.get_mut(), 50).await;
///     if discarded > 0 {
///         tracing::debug!("Discarded {} stale bytes", discarded);
///     }
///     
///     // Send command
///     guard.get_mut().write_all(command.as_bytes()).await?;
///     Ok(())
/// }
/// ```
pub async fn drain_serial_buffer<R: AsyncRead + Unpin>(
    port: &mut R,
    timeout_ms: u64,
) -> usize {
    let mut discard = [0u8; 256];
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let mut total_discarded = 0usize;

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, port.read(&mut discard)).await {
            Ok(Ok(0)) => break, // EOF or no more data
            Ok(Ok(n)) => {
                total_discarded += n;
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No data available, done
                break;
            }
            Ok(Err(_)) => break, // Real I/O error, abort drain
            Err(_) => break,     // Timeout, no more immediate data
        }
    }

    total_discarded
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_shared_port_with_duplex() {
        // Create a duplex stream for testing
        let (mut host, device) = tokio::io::duplex(64);
        let port: SharedPort = wrap_shared(Box::new(device));

        // Write from host side
        host.write_all(b"Hello\n").await.unwrap();

        // Read from shared port side
        let mut guard = port.lock().await;
        let mut line = String::new();
        guard.read_line(&mut line).await.unwrap();

        assert_eq!(line.trim(), "Hello");
    }

    #[tokio::test]
    async fn test_shared_port_unbuffered_with_duplex() {
        // Create a duplex stream for testing
        let (mut host, device) = tokio::io::duplex(64);
        let port: SharedPortUnbuffered = wrap_shared_unbuffered(Box::new(device));

        // Write from host side
        host.write_all(b"test").await.unwrap();

        // Read from shared port side
        let mut guard = port.lock().await;
        let mut buf = [0u8; 4];
        let n = guard.read(&mut buf).await.unwrap();

        assert_eq!(n, 4);
        assert_eq!(&buf, b"test");
    }

    #[tokio::test]
    async fn test_shared_port_clone() {
        // Verify that SharedPort can be cloned for use in multiple tasks
        let (mut host, device) = tokio::io::duplex(64);
        let port: SharedPort = wrap_shared(Box::new(device));

        // Clone for another task
        let port_clone = port.clone();

        // Write from host
        host.write_all(b"data\n").await.unwrap();

        // Read from clone - should work since it's the same underlying port
        let mut guard = port_clone.lock().await;
        let mut line = String::new();
        guard.read_line(&mut line).await.unwrap();

        assert_eq!(line.trim(), "data");
    }

    #[tokio::test]
    async fn test_drain_serial_buffer() {
        // Create a duplex stream with some data
        let (mut host, mut device) = tokio::io::duplex(64);

        // Write some stale data from host
        host.write_all(b"stale data 12345").await.unwrap();

        // Wait a bit for data to be available
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Drain the buffer
        let discarded = drain_serial_buffer(&mut device, 50).await;

        // Should have read all 16 bytes
        assert_eq!(discarded, 16);

        // Buffer should now be empty
        let mut buf = [0u8; 1];
        match tokio::time::timeout(Duration::from_millis(10), device.read(&mut buf)).await {
            Ok(Ok(0)) => {}, // EOF is ok
            Ok(Ok(_)) => panic!("Expected no data, but read some"),
            Err(_) => {}, // Timeout is expected
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}, // No data available
            Ok(Err(e)) => panic!("Unexpected error: {}", e),
        }
    }
}
