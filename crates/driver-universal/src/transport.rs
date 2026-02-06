//! Transport abstraction for serial, TCP, and UDP communication.
//!
//! Provides [`MockTransport`] for testing. Real serial/TCP/UDP transports
//! will be added in a future phase.

use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
}
