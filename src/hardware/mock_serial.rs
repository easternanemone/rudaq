//! Mock serial port implementation for testing async serial communication
//!
//! This module provides `MockSerialPort` which implements `AsyncRead` and `AsyncWrite`,
//! and a corresponding `MockDeviceHarness` to control the mock from within tests.
//! This allows for simulating device interactions, including command/response sequences,
//! delays, and timeouts.
//!
//! # Architecture
//!
//! The mock uses a pair of unbounded channels to simulate bidirectional communication:
//! - `MockSerialPort` (given to application): implements AsyncRead/AsyncWrite
//! - `MockDeviceHarness` (kept in test): scripts device behavior
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::hardware::mock_serial;
//! use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
//!
//! #[tokio::test]
//! async fn test_laser_query() {
//!     let (port, mut harness) = mock_serial::new();
//!     let mut reader = BufReader::new(port);
//!
//!     // Application sends command
//!     let app_task = tokio::spawn(async move {
//!         reader.write_all(b"POWER?\r").await.unwrap();
//!         let mut response = String::new();
//!         reader.read_line(&mut response).await.unwrap();
//!         response
//!     });
//!
//!     // Test harness simulates device
//!     harness.expect_write(b"POWER?\r").await;
//!     harness.send_response(b"POWER:2.5\r\n").unwrap();
//!
//!     assert_eq!(app_task.await.unwrap(), "POWER:2.5\r\n");
//! }
//! ```

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// The client-facing side of the mock serial port
///
/// This struct implements `AsyncRead` and `AsyncWrite` and is intended to be
/// passed to the application code under test as a drop-in replacement for a
/// real `serial2_tokio::SerialPort`.
#[derive(Debug)]
pub struct MockSerialPort {
    /// Channel to send written data to the harness
    writes_tx: UnboundedSender<Vec<u8>>,
    /// Channel to receive data from the harness to be read
    reads_rx: UnboundedReceiver<Vec<u8>>,
    /// Buffer for data received from the harness but not yet read by the client
    read_buffer: VecDeque<u8>,
}

/// The test-facing side for controlling the mock serial port
///
/// This harness allows a test to assert on data written by the client
/// and to send data back as if it were a real device.
#[derive(Debug)]
pub struct MockDeviceHarness {
    /// Channel to receive data written by the client
    writes_rx: UnboundedReceiver<Vec<u8>>,
    /// Channel to send data to the client for it to read
    reads_tx: UnboundedSender<Vec<u8>>,
    /// Buffer for data received from the client but not yet asserted by the test
    write_buffer: Vec<u8>,
}

/// Creates a new connected pair of `MockSerialPort` and `MockDeviceHarness`
pub fn new() -> (MockSerialPort, MockDeviceHarness) {
    let (client_to_harness_tx, client_to_harness_rx) = mpsc::unbounded_channel();
    let (harness_to_client_tx, harness_to_client_rx) = mpsc::unbounded_channel();

    let port = MockSerialPort {
        writes_tx: client_to_harness_tx,
        reads_rx: harness_to_client_rx,
        read_buffer: VecDeque::new(),
    };

    let harness = MockDeviceHarness {
        writes_rx: client_to_harness_rx,
        reads_tx: harness_to_client_tx,
        write_buffer: Vec::new(),
    };

    (port, harness)
}

// =============================================================================
// MockSerialPort Implementations
// =============================================================================

impl AsyncRead for MockSerialPort {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // If we have data in our internal buffer, use that first
        if !self.read_buffer.is_empty() {
            let available = self.read_buffer.len();
            let to_read = std::cmp::min(buf.remaining(), available);
            let chunk: Vec<u8> = self.read_buffer.drain(..to_read).collect();
            buf.put_slice(&chunk);
            return Poll::Ready(Ok(()));
        }

        // Otherwise, poll the channel for a new chunk of data from the harness
        match self.reads_rx.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => {
                // The harness sent data. Buffer it and then fill the user's buffer.
                self.read_buffer.extend(chunk);
                let available = self.read_buffer.len();
                let to_read = std::cmp::min(buf.remaining(), available);
                let data_to_put: Vec<u8> = self.read_buffer.drain(..to_read).collect();
                buf.put_slice(&data_to_put);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => {
                // Channel closed, which means end-of-file
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for MockSerialPort {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.writes_tx.send(buf.to_vec()) {
            Ok(_) => Poll::Ready(Ok(buf.len())),
            Err(_) => {
                // The receiving end (harness) was dropped
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "mock device harness disconnected",
                )))
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // No-op for this mock
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // No-op for this mock
        Poll::Ready(Ok(()))
    }
}

// =============================================================================
// MockDeviceHarness Implementations
// =============================================================================

impl MockDeviceHarness {
    /// Sends a response to the client
    ///
    /// This simulates the device sending data over the serial port.
    ///
    /// # Errors
    /// Returns error if the client port has been disconnected
    pub fn send_response(&self, data: &[u8]) -> Result<(), &'static str> {
        self.reads_tx
            .send(data.to_vec())
            .map_err(|_| "Failed to send response: client port disconnected")
    }

    /// Waits for the client to write specific data and asserts its correctness
    ///
    /// This will buffer incoming writes until the expected sequence is received.
    /// It includes a timeout to prevent tests from hanging indefinitely.
    ///
    /// # Panics
    /// Panics if the expected data is not received within 2 seconds or if
    /// the received data does not match the expected data.
    pub async fn expect_write(&mut self, expected: &[u8]) {
        use tokio::time::{timeout, Duration};

        let timeout_duration = Duration::from_secs(2); // Generous timeout for tests

        while self.write_buffer.len() < expected.len() {
            match timeout(timeout_duration, self.writes_rx.recv()).await {
                Ok(Some(chunk)) => self.write_buffer.extend_from_slice(&chunk),
                Ok(None) => panic!("Client-side port closed while expecting a write."),
                Err(_) => {
                    panic!(
                        "Timeout waiting for write. Expected `{:?}` ({} bytes), but only received `{:?}` ({} bytes).",
                        String::from_utf8_lossy(expected),
                        expected.len(),
                        String::from_utf8_lossy(&self.write_buffer),
                        self.write_buffer.len()
                    );
                }
            }
        }

        let actual = &self.write_buffer[..expected.len()];
        assert_eq!(
            actual,
            expected,
            "Mismatch in expected write. Expected `{:?}`, got `{:?}`.",
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(actual)
        );

        // Remove the asserted data from the buffer, keeping any excess for the next expectation
        self.write_buffer.drain(..expected.len());
    }

    /// Expects a write and sends a response in one operation
    ///
    /// Convenience method for common command/response patterns.
    pub async fn expect_and_respond(&mut self, expected: &[u8], response: &[u8]) {
        self.expect_write(expected).await;
        self.send_response(response)
            .expect("Failed to send response");
    }

    /// Drains any pending writes without asserting their content
    ///
    /// Useful for clearing the buffer in setup or teardown.
    pub async fn drain_writes(&mut self) {
        use tokio::time::{timeout, Duration};
        let short_timeout = Duration::from_millis(50);

        while let Ok(Some(chunk)) = timeout(short_timeout, self.writes_rx.recv()).await {
            self.write_buffer.extend_from_slice(&chunk);
        }
        self.write_buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_basic_write_read() {
        let (port, mut harness) = new();
        let mut port = BufReader::new(port);

        // Spawn task to write
        let write_task = tokio::spawn(async move {
            port.write_all(b"HELLO\n").await.unwrap();
            port
        });

        // Harness receives the write
        harness.expect_write(b"HELLO\n").await;

        write_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_command_response() {
        let (port, mut harness) = new();
        let mut port = BufReader::new(port);

        let app_task = tokio::spawn(async move {
            port.write_all(b"PING\n").await.unwrap();
            let mut response = String::new();
            port.read_line(&mut response).await.unwrap();
            response
        });

        harness.expect_write(b"PING\n").await;
        harness.send_response(b"PONG\n").unwrap();

        assert_eq!(app_task.await.unwrap(), "PONG\n");
    }

    #[tokio::test]
    async fn test_read_timeout() {
        let (port, mut harness) = new();
        let mut port = BufReader::new(port);

        let app_task = tokio::spawn(async move {
            port.write_all(b"QUERY\n").await.unwrap();
            let mut response = String::new();
            timeout(Duration::from_millis(100), port.read_line(&mut response)).await
        });

        // Expect the write but never send a response
        harness.expect_write(b"QUERY\n").await;
        // Don't send response - let it timeout

        let result = app_task.await.unwrap();
        assert!(result.is_err(), "Expected timeout error");
    }

    #[tokio::test]
    async fn test_multiple_commands() {
        let (port, mut harness) = new();
        let mut port = BufReader::new(port);

        let app_task = tokio::spawn(async move {
            port.write_all(b"CMD1\n").await.unwrap();
            let mut r1 = String::new();
            port.read_line(&mut r1).await.unwrap();

            port.write_all(b"CMD2\n").await.unwrap();
            let mut r2 = String::new();
            port.read_line(&mut r2).await.unwrap();

            (r1, r2)
        });

        harness.expect_and_respond(b"CMD1\n", b"ACK1\n").await;
        harness.expect_and_respond(b"CMD2\n", b"ACK2\n").await;

        let (r1, r2) = app_task.await.unwrap();
        assert_eq!(r1, "ACK1\n");
        assert_eq!(r2, "ACK2\n");
    }
}
