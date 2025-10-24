//! Mock hardware adapter for testing
//!
//! This adapter implements the HardwareAdapter trait for testing instruments
//! without requiring physical hardware. It provides:
//! - Simulated connection latency
//! - Controllable failure injection
//! - Call logging for test verification

use async_trait::async_trait;
use daq_core::{AdapterConfig, HardwareAdapter, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Mock hardware adapter for testing
///
/// # Example
///
/// ```
/// use rust_daq::adapters::MockAdapter;
/// use daq_core::HardwareAdapter;
///
/// # tokio_test::block_on(async {
/// let mut adapter = MockAdapter::new();
/// adapter.connect(&Default::default()).await.unwrap();
/// assert!(adapter.is_connected());
/// # })
/// ```
pub struct MockAdapter {
    connected: AtomicBool,
    latency_ms: u64,
    should_fail_next: AtomicBool,
    call_log: Arc<Mutex<Vec<String>>>,
}

impl MockAdapter {
    /// Create a new mock adapter with default settings
    pub fn new() -> Self {
        Self {
            connected: AtomicBool::new(false),
            latency_ms: 10,
            should_fail_next: AtomicBool::new(false),
            call_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Set simulated latency in milliseconds
    pub fn with_latency(mut self, ms: u64) -> Self {
        self.latency_ms = ms;
        self
    }

    /// Trigger a failure on the next operation
    pub fn trigger_failure(&self) {
        self.should_fail_next.store(true, Ordering::SeqCst);
    }

    /// Get a copy of the call log for verification
    pub fn get_call_log(&self) -> Vec<String> {
        self.call_log.lock().unwrap().clone()
    }

    /// Clear the call log
    pub fn clear_call_log(&self) {
        self.call_log.lock().unwrap().clear();
    }

    fn log_call(&self, method: &str) {
        self.call_log.lock().unwrap().push(method.to_string());
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HardwareAdapter for MockAdapter {
    async fn connect(&mut self, _config: &AdapterConfig) -> Result<()> {
        self.log_call("connect");

        if self.should_fail_next.swap(false, Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Mock connection failure"));
        }

        // Simulate connection latency
        tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;

        self.connected.store(true, Ordering::SeqCst);
        log::info!("MockAdapter connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.log_call("disconnect");

        self.connected.store(false, Ordering::SeqCst);
        log::info!("MockAdapter disconnected");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn adapter_type(&self) -> &str {
        "mock"
    }

    fn info(&self) -> String {
        format!("MockAdapter (latency: {}ms)", self.latency_ms)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_adapter_connection() {
        let mut adapter = MockAdapter::new();

        assert!(!adapter.is_connected());

        adapter.connect(&AdapterConfig::default()).await.unwrap();
        assert!(adapter.is_connected());

        adapter.disconnect().await.unwrap();
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_adapter_failure() {
        let mut adapter = MockAdapter::new();

        adapter.trigger_failure();
        let result = adapter.connect(&AdapterConfig::default()).await;

        assert!(result.is_err());
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_adapter_call_log() {
        let mut adapter = MockAdapter::new();

        adapter.connect(&AdapterConfig::default()).await.unwrap();
        adapter.disconnect().await.unwrap();

        let log = adapter.get_call_log();
        assert_eq!(log[0], "connect");
        assert_eq!(log[1], "disconnect");
    }

    #[tokio::test]
    async fn test_adapter_latency() {
        let mut adapter = MockAdapter::new().with_latency(50);

        let start = std::time::Instant::now();
        adapter.connect(&AdapterConfig::default()).await.unwrap();
        let elapsed = start.elapsed();

        // Should take at least 50ms due to simulated latency
        assert!(elapsed.as_millis() >= 50);
    }

    #[tokio::test]
    async fn test_failure_is_one_shot() {
        // Verifies that `trigger_failure` only affects the very next operation
        // and then resets itself automatically.
        let mut adapter = MockAdapter::new();

        // First connection attempt should fail
        adapter.trigger_failure();
        assert!(adapter.connect(&AdapterConfig::default()).await.is_err());
        assert!(
            !adapter.is_connected(),
            "Adapter should not be connected after a failure"
        );

        // The failure flag should have been consumed. The next attempt should succeed.
        assert!(adapter.connect(&AdapterConfig::default()).await.is_ok());
        assert!(
            adapter.is_connected(),
            "Adapter should be connected on the second attempt"
        );
    }

    #[tokio::test]
    async fn test_clear_call_log() {
        let mut adapter = MockAdapter::new();
        adapter.connect(&AdapterConfig::default()).await.unwrap();
        assert_eq!(adapter.get_call_log().len(), 1);

        adapter.clear_call_log();
        assert!(adapter.get_call_log().is_empty());

        adapter.disconnect().await.unwrap();
        assert_eq!(adapter.get_call_log().as_slice(), &["disconnect"]);
    }

    #[tokio::test]
    async fn test_zero_latency() {
        let mut adapter = MockAdapter::new().with_latency(0);

        let start = std::time::Instant::now();
        adapter.connect(&AdapterConfig::default()).await.unwrap();
        let elapsed = start.elapsed();

        // With zero latency, this should be very fast.
        // Allow a small margin for OS scheduling and function call overhead.
        assert!(
            elapsed.as_millis() < 5,
            "Connection with zero latency took too long: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_concurrent_access_is_safe() {
        // This test checks for race conditions or deadlocks on the internal state (Mutex, Atomics).
        // It spawns multiple tasks that interact with the same adapter instance concurrently.
        let adapter = Arc::new(MockAdapter::new());
        let mut tasks = vec![];

        for i in 0..10 {
            let adapter_clone = Arc::clone(&adapter);
            tasks.push(tokio::spawn(async move {
                // Mix of read/write operations to stress the locks.
                if i % 3 == 0 {
                    adapter_clone.trigger_failure();
                }
                let _ = adapter_clone.is_connected();
                adapter_clone.log_call("concurrent_op");
                let _ = adapter_clone.get_call_log();
            }));
        }

        // Wait for all tasks to complete.
        for task in tasks {
            task.await.unwrap();
        }

        // The primary goal is to ensure this code completes without panicking (e.g., from a poisoned mutex).
        // The final state is non-deterministic, but its stability proves thread-safety.
        assert_eq!(adapter.get_call_log().len(), 10);
    }
}
