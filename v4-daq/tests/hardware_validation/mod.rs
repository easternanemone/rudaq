//! Hardware Validation Framework for V4 Rust-DAQ
//!
//! Comprehensive automated test framework for executing 94 hardware test scenarios
//! across 5 V4 actors on real hardware via SSH.
//!
//! ## Test Structure
//!
//! - 17 SCPI generic instrument tests
//! - 14 Newport 1830-C optical power meter tests
//! - 16 ESP300 motion controller tests
//! - 28 PVCAM camera tests
//! - 19 MaiTai laser tests
//!
//! ## Safety Critical Tests
//!
//! All tests marked with `#[ignore]` - run explicitly with `cargo test -- --ignored`.
//! Safety verification is prioritized for critical hardware operations.
//!
//! ## Hardware Requirements
//!
//! Tests assume the following SSH target:
//! - Host: maitai@maitai-eos
//! - Real hardware connected and operational
//! - Serial ports, VISA resources, USB cameras available

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Test result with detailed information
#[derive(Debug, Clone)]
pub struct TestResult {
    pub test_name: String,
    pub passed: bool,
    pub duration_ms: u64,
    pub error_message: Option<String>,
    pub timestamp_ns: i64,
}

impl TestResult {
    /// Create a new passing test result
    pub fn passed(test_name: &str, duration_ms: u64) -> Self {
        Self {
            test_name: test_name.to_string(),
            passed: true,
            duration_ms,
            error_message: None,
            timestamp_ns: current_timestamp_ns(),
        }
    }

    /// Create a new failing test result
    pub fn failed(test_name: &str, duration_ms: u64, error: &str) -> Self {
        Self {
            test_name: test_name.to_string(),
            passed: false,
            duration_ms,
            error_message: Some(error.to_string()),
            timestamp_ns: current_timestamp_ns(),
        }
    }
}

impl fmt::Display for TestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.passed { "PASS" } else { "FAIL" };
        write!(f, "[{}] {} ({}ms)", status, self.test_name, self.duration_ms)?;
        if let Some(error) = &self.error_message {
            write!(f, ": {}", error)?;
        }
        Ok(())
    }
}

/// Hardware test harness for collecting and managing test results
pub struct HardwareTestHarness {
    results: Vec<TestResult>,
    setup_errors: Vec<String>,
    teardown_errors: Vec<String>,
}

impl HardwareTestHarness {
    /// Create a new test harness
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
            setup_errors: Vec::new(),
            teardown_errors: Vec::new(),
        }
    }

    /// Add a test result
    pub fn add_result(&mut self, result: TestResult) {
        self.results.push(result);
    }

    /// Record a setup error
    pub fn add_setup_error(&mut self, error: &str) {
        self.setup_errors.push(error.to_string());
    }

    /// Record a teardown error
    pub fn add_teardown_error(&mut self, error: &str) {
        self.teardown_errors.push(error.to_string());
    }

    /// Get all results
    pub fn results(&self) -> &[TestResult] {
        &self.results
    }

    /// Get summary statistics
    pub fn summary(&self) -> TestSummary {
        let total = self.results.len();
        let passed = self.results.iter().filter(|r| r.passed).count();
        let failed = total - passed;
        let total_duration_ms: u64 = self.results.iter().map(|r| r.duration_ms).sum();

        TestSummary {
            total,
            passed,
            failed,
            total_duration_ms,
            setup_errors: self.setup_errors.len(),
            teardown_errors: self.teardown_errors.len(),
        }
    }

    /// Print report to stdout
    pub fn print_report(&self) {
        println!("\n=== HARDWARE VALIDATION REPORT ===\n");

        if !self.setup_errors.is_empty() {
            println!("Setup Errors:");
            for error in &self.setup_errors {
                println!("  - {}", error);
            }
            println!();
        }

        println!("Test Results:");
        for result in &self.results {
            println!("  {}", result);
        }

        if !self.teardown_errors.is_empty() {
            println!("\nTeardown Errors:");
            for error in &self.teardown_errors {
                println!("  - {}", error);
            }
        }

        let summary = self.summary();
        println!("\n=== SUMMARY ===");
        println!("Total:   {}", summary.total);
        println!("Passed:  {}", summary.passed);
        println!("Failed:  {}", summary.failed);
        println!("Duration: {}ms", summary.total_duration_ms);

        if summary.setup_errors > 0 {
            println!("Setup Errors: {}", summary.setup_errors);
        }
        if summary.teardown_errors > 0 {
            println!("Teardown Errors: {}", summary.teardown_errors);
        }

        if summary.failed == 0 && summary.setup_errors == 0 && summary.teardown_errors == 0 {
            println!("\nAll tests passed!");
        }
    }
}

impl Default for HardwareTestHarness {
    fn default() -> Self {
        Self::new()
    }
}

/// Test summary statistics
#[derive(Debug, Clone)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub total_duration_ms: u64,
    pub setup_errors: usize,
    pub teardown_errors: usize,
}

/// Common test utilities
pub mod utils {
    use super::*;
    use std::time::Instant;

    /// Timeout for hardware operations (hardware can be slow)
    pub const HARDWARE_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

    /// Timeout for communication operations
    pub const COMMUNICATION_TIMEOUT: Duration = Duration::from_secs(2);

    /// Timeout for measurement operations
    pub const MEASUREMENT_TIMEOUT: Duration = Duration::from_secs(10);

    /// Safety verification check
    pub trait SafetyCheck {
        /// Verify hardware is in safe state before operation
        fn verify_safe_for_operation(&self) -> Result<(), String>;

        /// Return hardware to safe state after operation
        fn return_to_safe_state(&self) -> Result<(), String>;
    }

    /// Helper function to measure test execution time
    pub fn measure_test_execution<F, T>(mut test_fn: F) -> (T, u64)
    where
        F: FnMut() -> T,
    {
        let start = Instant::now();
        let result = test_fn();
        let duration_ms = start.elapsed().as_millis() as u64;
        (result, duration_ms)
    }

    /// Helper function to verify hardware response within timeout
    pub async fn verify_hardware_response_timeout<F, T>(
        operation: F,
        timeout: Duration,
    ) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
    {
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| format!("Operation timed out after {}ms", timeout.as_millis()))?
    }

    /// Safety wrapper for critical operations
    pub async fn safe_operation<F, T>(
        pre_check: impl Fn() -> Result<(), String>,
        operation: F,
        post_check: impl Fn() -> Result<(), String>,
    ) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
    {
        // Verify safe state before operation
        pre_check()?;

        // Execute operation
        let result = operation.await?;

        // Return to safe state after operation
        post_check()?;

        Ok(result)
    }
}

/// Get current timestamp in nanoseconds since UNIX epoch
fn current_timestamp_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}

// Sub-module declarations
pub mod scpi_hardware_tests;
pub mod newport_hardware_tests;
pub mod esp300_hardware_tests;
pub mod pvcam_hardware_tests;
pub mod maitai_hardware_tests;
