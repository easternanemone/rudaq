//! Transport abstraction for serial, TCP, and UDP communication.
//!
//! Provides:
//! - [`SerialTransport`] — real serial via `common::serial`
//! - [`TcpTransport`] — real TCP via `tokio::net`
//! - [`MockTransport`] — pre-programmed responses for testing

use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Read a line terminated by `\r`, `\n`, or `\r\n`.
///
/// Unlike [`AsyncBufReadExt::read_line`] which only stops on `\n`, this
/// handles devices (e.g., IPG lasers) that terminate responses with `\r`
/// only.  Stops at the first `\r` or `\n` encountered.  Any trailing
/// `\n` after a `\r` (i.e., a CRLF pair) is left in the buffer and
/// drained by the caller's stale-data check on the next operation.
async fn read_line_any_eol<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<String> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            break; // EOF
        }

        if let Some(pos) = buffer.iter().position(|&b| b == b'\n' || b == b'\r') {
            bytes.extend_from_slice(&buffer[..pos]);
            reader.consume(pos + 1);
            break;
        }

        // No delimiter in this chunk — consume all and continue
        bytes.extend_from_slice(buffer);
        let len = buffer.len();
        reader.consume(len);
    }
    Ok(String::from_utf8(bytes)?.trim().to_string())
}

/// Transport trait abstracting the communication layer.
///
/// Implementations handle the low-level sending and receiving of data
/// to/from a device over serial, TCP, or UDP.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Send raw bytes to the device.
    async fn send(&self, data: &[u8]) -> Result<()>;

    /// Receive a response from the device with a timeout.
    async fn receive(&self, timeout: Duration) -> Result<String>;

    /// Send a command and receive a response (convenience method).
    async fn query(&self, data: &[u8], timeout: Duration) -> Result<String> {
        self.send(data).await?;
        self.receive(timeout).await
    }
}

// ---------------------------------------------------------------------------
// SerialTransport
// ---------------------------------------------------------------------------

/// Real serial transport backed by `common::serial`.
///
/// Uses a `SharedPort` (buffered) for line-delimited protocols and appends
/// an optional terminator (e.g., `\r\n`, `\n`) after each send.
pub struct SerialTransport {
    port: common::serial::SharedPort,
    terminator: String,
}

impl SerialTransport {
    /// Open a serial port and wrap it for use as a `Transport`.
    ///
    /// When `serial_config` fields are `None`, defaults to 8N1 with no flow control.
    pub async fn open(
        port_path: &str,
        baud_rate: u32,
        terminator: Option<&str>,
        serial_config: &crate::config::validated::SerialConfig,
    ) -> Result<Self> {
        let has_custom_config = serial_config.data_bits.is_some()
            || serial_config.parity.is_some()
            || serial_config.stop_bits.is_some()
            || serial_config.flow_control.is_some();

        let port: common::serial::DynSerial = if has_custom_config {
            // Use serial2_tokio directly for custom serial config
            #[cfg(feature = "serial")]
            {
                use anyhow::Context;
                use serial2_tokio::{CharSize, FlowControl, Parity, StopBits};
                use tokio::task::spawn_blocking;

                let port_path_owned = port_path.to_string();
                let sc = serial_config.clone();
                let port = spawn_blocking(move || {
                    let mut port = serial2_tokio::SerialPort::open(&port_path_owned, baud_rate)
                        .context(format!("Failed to open serial port: {port_path_owned}"))?;

                    // Read current settings and apply overrides
                    let mut settings = port
                        .get_configuration()
                        .context("Failed to read serial port settings")?;
                    if let Some(db) = sc.data_bits {
                        settings.set_char_size(match db {
                            5 => CharSize::Bits5,
                            6 => CharSize::Bits6,
                            7 => CharSize::Bits7,
                            _ => CharSize::Bits8,
                        });
                    }
                    if let Some(p) = sc.parity {
                        use crate::config::validated::SerialParity;
                        settings.set_parity(match p {
                            SerialParity::Odd => Parity::Odd,
                            SerialParity::Even => Parity::Even,
                            SerialParity::None => Parity::None,
                        });
                    }
                    if let Some(sb) = sc.stop_bits {
                        settings.set_stop_bits(match sb {
                            2 => StopBits::Two,
                            _ => StopBits::One,
                        });
                    }
                    if let Some(fc) = sc.flow_control {
                        use crate::config::validated::SerialFlowControl;
                        settings.set_flow_control(match fc {
                            SerialFlowControl::Software => FlowControl::XonXoff,
                            SerialFlowControl::Hardware => FlowControl::RtsCts,
                            SerialFlowControl::None => FlowControl::None,
                        });
                    }
                    port.set_configuration(&settings)
                        .context("Failed to apply serial port settings")?;

                    Ok::<_, anyhow::Error>(port)
                })
                .await
                .context("spawn_blocking for serial port opening failed")??;
                Box::new(port)
            }
            #[cfg(not(feature = "serial"))]
            {
                anyhow::bail!(
                    "serial feature not enabled; cannot open serial port with custom config"
                )
            }
        } else {
            // Default 8N1 — use common::serial
            common::serial::open_serial_async(port_path, baud_rate, "Universal").await?
        };

        let shared = common::serial::wrap_shared(port);
        Ok(Self {
            port: shared,
            terminator: terminator.unwrap_or("").to_string(),
        })
    }
}

#[async_trait::async_trait]
impl Transport for SerialTransport {
    async fn send(&self, data: &[u8]) -> Result<()> {
        let mut guard = self.port.lock().await;
        let writer = guard.get_mut();
        writer.write_all(data).await?;
        if !self.terminator.is_empty() {
            writer.write_all(self.terminator.as_bytes()).await?;
        }
        writer.flush().await?;
        Ok(())
    }

    async fn receive(&self, timeout: Duration) -> Result<String> {
        let mut guard = self.port.lock().await;
        tokio::time::timeout(timeout, read_line_any_eol(&mut *guard))
            .await
            .map_err(|_| anyhow::anyhow!("serial receive timed out"))?
    }

    /// Override query to drain stale BufReader data before each command-response cycle.
    ///
    /// After init_sequence commands (which use send() without reading), stale
    /// echo data may sit in the BufReader's internal buffer. Consuming it before
    /// sending ensures read_line() receives the actual response.
    async fn query(&self, data: &[u8], timeout: Duration) -> Result<String> {
        let mut guard = self.port.lock().await;

        // Drain BufReader's internal buffer (stale echo/response data)
        let stale = guard.buffer().len();
        if stale > 0 {
            guard.consume(stale);
        }

        // Send command
        let writer = guard.get_mut();
        writer.write_all(data).await?;
        if !self.terminator.is_empty() {
            writer.write_all(self.terminator.as_bytes()).await?;
        }
        writer.flush().await?;

        // Read response — loop to skip empty lines and echoes
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline - tokio::time::Instant::now();
            if remaining.is_zero() {
                anyhow::bail!("serial receive timed out");
            }

            let response = tokio::time::timeout(remaining, read_line_any_eol(&mut *guard))
                .await
                .map_err(|_| anyhow::anyhow!("serial receive timed out"))??;

            // Skip empty lines
            if response.is_empty() {
                continue;
            }

            // Skip echoes of the command we just sent
            let cmd_str = String::from_utf8_lossy(data);
            if response == cmd_str.as_ref() {
                continue;
            }

            return Ok(response);
        }
    }
}

// ---------------------------------------------------------------------------
// TcpTransport
// ---------------------------------------------------------------------------

/// Real TCP transport using `tokio::net::TcpStream`.
///
/// Wraps the stream in a `BufReader` for line-delimited reads. Disables
/// Nagle's algorithm for low-latency command-response exchanges.
pub struct TcpTransport {
    stream: tokio::sync::Mutex<BufReader<tokio::net::TcpStream>>,
    terminator: String,
}

impl TcpTransport {
    /// Connect to a TCP endpoint.
    pub async fn connect(
        host: &str,
        port: u16,
        connect_timeout: Duration,
        terminator: Option<&str>,
    ) -> Result<Self> {
        let addr: std::net::SocketAddr = format!("{host}:{port}").parse()?;
        let stream = tokio::time::timeout(connect_timeout, tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| anyhow::anyhow!("TCP connect to {addr} timed out"))??;
        stream.set_nodelay(true)?;
        let term = terminator.unwrap_or("");
        tracing::debug!(
            addr = %addr,
            terminator_bytes = ?term.as_bytes(),
            terminator_len = term.len(),
            "TcpTransport connected"
        );
        Ok(Self {
            stream: tokio::sync::Mutex::new(BufReader::new(stream)),
            terminator: term.to_string(),
        })
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    async fn send(&self, data: &[u8]) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let writer = guard.get_mut();
        writer.write_all(data).await?;
        if !self.terminator.is_empty() {
            writer.write_all(self.terminator.as_bytes()).await?;
        }
        writer.flush().await?;
        Ok(())
    }

    async fn receive(&self, timeout: Duration) -> Result<String> {
        let mut guard = self.stream.lock().await;
        // Discard stale buffered data before reading fresh response
        let stale = guard.buffer().len();
        if stale > 0 {
            guard.consume(stale);
        }
        tokio::time::timeout(timeout, read_line_any_eol(&mut *guard))
            .await
            .map_err(|_| anyhow::anyhow!("TCP receive timed out"))?
    }

    /// Override query to hold the stream lock for the entire send+receive cycle.
    ///
    /// This prevents interleaved commands from concurrent tasks and ensures
    /// the response we read corresponds to the command we just sent.
    async fn query(&self, data: &[u8], timeout: Duration) -> Result<String> {
        let mut guard = self.stream.lock().await;

        // Drain stale buffered data before sending
        let stale = guard.buffer().len();
        if stale > 0 {
            guard.consume(stale);
        }

        // Send command + terminator
        let writer = guard.get_mut();
        writer.write_all(data).await?;
        if !self.terminator.is_empty() {
            writer.write_all(self.terminator.as_bytes()).await?;
        }
        writer.flush().await?;

        // Read response
        tokio::time::timeout(timeout, read_line_any_eol(&mut *guard))
            .await
            .map_err(|_| anyhow::anyhow!("TCP receive timed out"))?
    }
}

// ---------------------------------------------------------------------------
// MockTransport
// ---------------------------------------------------------------------------

/// Shared inner state for `MockTransport`.
///
/// Using `Arc` internally allows cloning a `MockTransport` to share state
/// between the driver (which owns it as `Box<dyn Transport>`) and test code
/// (which needs to inspect sent data).
#[derive(Debug)]
struct MockTransportInner {
    /// Queue of responses to return (FIFO).
    responses: Mutex<Vec<String>>,
    /// Record of all sent data.
    sent: Mutex<Vec<Vec<u8>>>,
}

/// A mock transport for testing that records sent data and returns
/// pre-programmed responses.
///
/// Cloning a `MockTransport` shares the same internal state, so you can
/// clone before passing to the driver and then inspect sent data on the clone.
#[derive(Clone)]
pub struct MockTransport {
    inner: Arc<MockTransportInner>,
}

impl MockTransport {
    /// Create a new mock transport with pre-programmed responses.
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            inner: Arc::new(MockTransportInner {
                responses: Mutex::new(responses),
                sent: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Get all data that was sent through this transport.
    pub fn sent_data(&self) -> Vec<Vec<u8>> {
        self.inner.sent.lock().unwrap().clone()
    }

    /// Get sent data as UTF-8 strings (for convenience).
    pub fn sent_strings(&self) -> Vec<String> {
        self.inner
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect()
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send(&self, data: &[u8]) -> Result<()> {
        self.inner.sent.lock().unwrap().push(data.to_vec());
        Ok(())
    }

    async fn receive(&self, _timeout: Duration) -> Result<String> {
        let mut responses = self.inner.responses.lock().unwrap();
        if responses.is_empty() {
            anyhow::bail!("MockTransport: no more responses queued")
        }
        Ok(responses.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_transport_send_receive() {
        let transport = MockTransport::new(vec!["OK".to_string(), "42".to_string()]);

        transport.send(b"CMD1").await.unwrap();
        let resp1 = transport.receive(Duration::from_secs(1)).await.unwrap();
        assert_eq!(resp1, "OK");

        transport.send(b"CMD2").await.unwrap();
        let resp2 = transport.receive(Duration::from_secs(1)).await.unwrap();
        assert_eq!(resp2, "42");

        assert_eq!(transport.sent_strings(), vec!["CMD1", "CMD2"]);
    }

    #[tokio::test]
    async fn mock_transport_query() {
        let transport = MockTransport::new(vec!["RESPONSE".to_string()]);

        let result = transport
            .query(b"QUERY", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result, "RESPONSE");
        assert_eq!(transport.sent_strings(), vec!["QUERY"]);
    }

    #[tokio::test]
    async fn mock_transport_empty_queue() {
        let transport = MockTransport::new(vec![]);
        let result = transport.receive(Duration::from_secs(1)).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // read_line_any_eol tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn read_line_any_eol_cr_only() {
        let data = b"HELLO\rWORLD\r";
        let mut reader = &data[..];
        let line = read_line_any_eol(&mut reader).await.unwrap();
        assert_eq!(line, "HELLO");
        let line2 = read_line_any_eol(&mut reader).await.unwrap();
        assert_eq!(line2, "WORLD");
    }

    #[tokio::test]
    async fn read_line_any_eol_lf_only() {
        let data = b"HELLO\nWORLD\n";
        let mut reader = &data[..];
        let line = read_line_any_eol(&mut reader).await.unwrap();
        assert_eq!(line, "HELLO");
        let line2 = read_line_any_eol(&mut reader).await.unwrap();
        assert_eq!(line2, "WORLD");
    }

    #[tokio::test]
    async fn read_line_any_eol_crlf() {
        let data = b"HELLO\r\nWORLD\r\n";
        let mut reader = &data[..];
        // First line stops at \r; the trailing \n appears as an empty next read
        let line = read_line_any_eol(&mut reader).await.unwrap();
        assert_eq!(line, "HELLO");
    }

    #[tokio::test]
    async fn read_line_any_eol_eof_no_delimiter() {
        let data = b"NO_NEWLINE";
        let mut reader = &data[..];
        let line = read_line_any_eol(&mut reader).await.unwrap();
        assert_eq!(line, "NO_NEWLINE");
    }

    #[tokio::test]
    async fn read_line_any_eol_empty_input() {
        let data = b"";
        let mut reader = &data[..];
        let line = read_line_any_eol(&mut reader).await.unwrap();
        assert_eq!(line, "");
    }
}
