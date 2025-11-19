//! Mock hardware adapter for testing
//!
//! This adapter provides a simulated hardware interface for testing instruments
//! without requiring physical hardware. It provides:
//! - Simulated connection latency
//! - Controllable failure injection
//! - Call logging for test verification

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Mock hardware adapter for testing
///
/// # Example
///
/// ```
/// use v4_daq::adapters::MockAdapter;
///
/// let mut adapter = MockAdapter::new();
/// adapter.set_connected(true);
/// assert!(adapter.is_connected());
/// ```
#[derive(Clone)]
pub struct MockAdapter {
    connected: Arc<AtomicBool>,
    latency_ms: Arc<Mutex<u64>>,
    should_fail_next: Arc<AtomicBool>,
    call_log: Arc<Mutex<Vec<String>>>,
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl MockAdapter {
    /// Create a new mock adapter with default settings
    pub fn new() -> Self {
        Self {
            connected: Arc::new(AtomicBool::new(false)),
            latency_ms: Arc::new(Mutex::new(10)),
            should_fail_next: Arc::new(AtomicBool::new(false)),
            call_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Set simulated latency in milliseconds
    pub fn with_latency(self, ms: u64) -> Self {
        *self.latency_ms.lock().unwrap() = ms;
        self
    }

    /// Inject a failure for the next operation
    pub fn inject_next_failure(&self) {
        self.should_fail_next.store(true, Ordering::SeqCst);
    }

    /// Check if a failure was injected
    fn check_failure(&self) -> bool {
        self.should_fail_next.swap(false, Ordering::SeqCst)
    }

    /// Set the connection state manually
    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::SeqCst);
    }

    /// Check if currently connected
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Get the call log
    pub fn call_log(&self) -> Vec<String> {
        self.call_log.lock().unwrap().clone()
    }

    /// Clear the call log
    pub fn clear_log(&self) {
        self.call_log.lock().unwrap().clear();
    }

    /// Log a call for testing verification
    fn log_call(&self, call: String) {
        self.call_log.lock().unwrap().push(call);
    }

    /// Simulate a delayed operation
    pub async fn simulate_latency(&self) {
        let latency = *self.latency_ms.lock().unwrap();
        if latency > 0 {
            tokio::time::sleep(Duration::from_millis(latency)).await;
        }
    }

    /// Simulate a connection operation
    pub async fn connect(&self) -> std::result::Result<(), String> {
        self.simulate_latency().await;
        self.log_call("connect".to_string());

        if self.check_failure() {
            return Err("Injected failure".to_string());
        }

        self.set_connected(true);
        Ok(())
    }

    /// Simulate a disconnection operation
    pub async fn disconnect(&self) -> std::result::Result<(), String> {
        self.simulate_latency().await;
        self.log_call("disconnect".to_string());

        if self.check_failure() {
            return Err("Injected failure".to_string());
        }

        self.set_connected(false);
        Ok(())
    }

    /// Simulate sending a command
    pub async fn send_command(&self, command: &str) -> std::result::Result<String, String> {
        self.simulate_latency().await;
        self.log_call(format!("send_command: {}", command));

        if self.check_failure() {
            return Err("Injected failure".to_string());
        }

        if !self.is_connected() {
            return Err("Not connected".to_string());
        }

        // Return a mock response
        Ok(format!("MOCK_RESPONSE: {}", command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_adapter_creation() {
        let adapter = MockAdapter::new();
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_connect() {
        let adapter = MockAdapter::new();
        adapter.connect().await.unwrap();
        assert!(adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_connect_with_latency() {
        let adapter = MockAdapter::new().with_latency(10);
        let start = std::time::Instant::now();
        adapter.connect().await.unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() >= 10);
    }

    #[tokio::test]
    async fn test_mock_adapter_disconnect() {
        let adapter = MockAdapter::new();
        adapter.connect().await.unwrap();
        adapter.disconnect().await.unwrap();
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_failure_injection() {
        let adapter = MockAdapter::new();
        adapter.inject_next_failure();
        let result = adapter.connect().await;
        assert!(result.is_err());
        // Failure should be consumed
        let result = adapter.connect().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_adapter_send_command_when_not_connected() {
        let adapter = MockAdapter::new();
        let result = adapter.send_command("TEST").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not connected");
    }

    #[tokio::test]
    async fn test_mock_adapter_send_command_when_connected() {
        let adapter = MockAdapter::new();
        adapter.connect().await.unwrap();
        let result = adapter.send_command("TEST").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "MOCK_RESPONSE: TEST");
    }

    #[tokio::test]
    async fn test_mock_adapter_call_logging() {
        let adapter = MockAdapter::new();
        adapter.connect().await.unwrap();
        adapter.send_command("CMD").await.unwrap();
        adapter.disconnect().await.unwrap();

        let log = adapter.call_log();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0], "connect");
        assert!(log[1].contains("send_command"));
        assert_eq!(log[2], "disconnect");
    }

    #[test]
    fn test_mock_adapter_clear_log() {
        let adapter = MockAdapter::new();
        adapter.log_call("test".to_string());
        assert_eq!(adapter.call_log().len(), 1);
        adapter.clear_log();
        assert_eq!(adapter.call_log().len(), 0);
    }
}
