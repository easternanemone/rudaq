//! Real-time test result collection and aggregation.
//!
//! This module provides efficient collection and tracking of test results as they are
//! generated, with support for progress tracking and error categorization.

use super::{TestResult, TestStatus, TestSuite, TestReport};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

/// Error classification for test failures
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCategory {
    /// Test timed out waiting for response
    Timeout,
    /// Hardware communication error
    HardwareError,
    /// Safety violation detected
    SafetyViolation,
    /// Configuration/setup error
    ConfigError,
    /// Unexpected exception/panic
    Exception,
    /// Assertion failure
    AssertionFailure,
    /// Unknown error
    Unknown,
}

impl ErrorCategory {
    /// Detect error category from error message
    pub fn from_error(error: &str) -> Self {
        let lower = error.to_lowercase();

        if lower.contains("timeout") || lower.contains("timed out") {
            ErrorCategory::Timeout
        } else if lower.contains("hardware") || lower.contains("communication") || lower.contains("serial") {
            ErrorCategory::HardwareError
        } else if lower.contains("safety") || lower.contains("limit") || lower.contains("exceed") {
            ErrorCategory::SafetyViolation
        } else if lower.contains("config") || lower.contains("configuration") {
            ErrorCategory::ConfigError
        } else if lower.contains("panic") || lower.contains("exception") {
            ErrorCategory::Exception
        } else if lower.contains("assert") {
            ErrorCategory::AssertionFailure
        } else {
            ErrorCategory::Unknown
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCategory::Timeout => "TIMEOUT",
            ErrorCategory::HardwareError => "HARDWARE_ERROR",
            ErrorCategory::SafetyViolation => "SAFETY_VIOLATION",
            ErrorCategory::ConfigError => "CONFIG_ERROR",
            ErrorCategory::Exception => "EXCEPTION",
            ErrorCategory::AssertionFailure => "ASSERTION_FAILURE",
            ErrorCategory::Unknown => "UNKNOWN",
        }
    }
}

/// Test execution event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestEvent {
    /// Timestamp of event
    pub timestamp: DateTime<Utc>,
    /// Event type
    pub event_type: TestEventType,
    /// Test ID if applicable
    pub test_id: Option<String>,
    /// Associated message
    pub message: String,
    /// Error category if applicable
    pub error_category: Option<ErrorCategory>,
}

/// Type of test event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TestEventType {
    /// Test started
    TestStarted,
    /// Test passed
    TestPassed,
    /// Test failed
    TestFailed,
    /// Test skipped
    TestSkipped,
    /// Suite started
    SuiteStarted,
    /// Suite completed
    SuiteCompleted,
    /// Progress update
    ProgressUpdate,
    /// Warning issued
    Warning,
    /// Error occurred
    Error,
}

/// Progress tracking information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressInfo {
    /// Total number of tests
    pub total_tests: usize,
    /// Number of completed tests
    pub completed_tests: usize,
    /// Number of passed tests
    pub passed_tests: usize,
    /// Number of failed tests
    pub failed_tests: usize,
    /// Number of skipped tests
    pub skipped_tests: usize,
    /// Overall progress percentage
    pub progress_percent: f64,
    /// Start time
    pub start_time: DateTime<Utc>,
    /// Current time
    pub current_time: DateTime<Utc>,
}

impl ProgressInfo {
    /// Calculate estimated completion time
    pub fn estimated_completion(&self) -> Option<DateTime<Utc>> {
        if self.completed_tests == 0 {
            return None;
        }

        let elapsed_secs = (self.current_time.timestamp() - self.start_time.timestamp()) as f64;
        let avg_secs_per_test = elapsed_secs / self.completed_tests as f64;
        let remaining_tests = self.total_tests - self.completed_tests;
        let remaining_secs = avg_secs_per_test * remaining_tests as f64;

        Some(self.current_time + chrono::Duration::seconds(remaining_secs as i64))
    }

    /// Get time remaining as string
    pub fn time_remaining_str(&self) -> String {
        match self.estimated_completion() {
            Some(eta) => {
                let remaining = eta.timestamp() - self.current_time.timestamp();
                if remaining <= 0 {
                    "< 1 second".to_string()
                } else if remaining < 60 {
                    format!("{} seconds", remaining)
                } else if remaining < 3600 {
                    format!("{} minutes", remaining / 60)
                } else {
                    format!("{} hours", remaining / 3600)
                }
            }
            None => "Calculating...".to_string(),
        }
    }
}

/// Result collector for gathering test results in real-time
pub struct ResultCollector {
    suites: Arc<RwLock<HashMap<String, TestSuite>>>,
    events: Arc<RwLock<Vec<TestEvent>>>,
    error_categories: Arc<RwLock<HashMap<ErrorCategory, usize>>>,
    start_time: Instant,
    started_at: DateTime<Utc>,
}

impl ResultCollector {
    /// Create a new result collector
    pub fn new() -> Self {
        Self {
            suites: Arc::new(RwLock::new(HashMap::new())),
            events: Arc::new(RwLock::new(Vec::new())),
            error_categories: Arc::new(RwLock::new(HashMap::new())),
            start_time: Instant::now(),
            started_at: Utc::now(),
        }
    }

    /// Register or get a test suite
    pub async fn get_or_create_suite(&self, name: String) -> TestSuite {
        let mut suites = self.suites.write().await;
        if let Some(suite) = suites.get(&name) {
            suite.clone()
        } else {
            let suite = TestSuite::new(name.clone());
            suites.insert(name, suite.clone());
            suite
        }
    }

    /// Add a test result to the appropriate suite
    pub async fn add_result(&self, result: TestResult) {
        let suite_name = result.category.clone();

        // Log event
        self.log_event(TestEvent {
            timestamp: Utc::now(),
            event_type: if result.status.is_passed() {
                TestEventType::TestPassed
            } else {
                TestEventType::TestFailed
            },
            test_id: Some(result.test_id.clone()),
            message: format!("{}: {}", result.test_name, result.status.as_str()),
            error_category: result.error.as_ref().map(|e| ErrorCategory::from_error(e)),
        })
        .await;

        // Categorize error if present
        if let Some(error) = &result.error {
            let category = ErrorCategory::from_error(error);
            let mut categories = self.error_categories.write().await;
            *categories.entry(category).or_insert(0) += 1;
        }

        // Add to suite
        let mut suites = self.suites.write().await;
        let suite = suites
            .entry(suite_name.clone())
            .or_insert_with(|| TestSuite::new(suite_name));
        suite.add_result(result);
    }

    /// Log a test event
    pub async fn log_event(&self, event: TestEvent) {
        let mut events = self.events.write().await;
        events.push(event);
    }

    /// Log a message
    pub async fn log_message(&self, event_type: TestEventType, message: String) {
        self.log_event(TestEvent {
            timestamp: Utc::now(),
            event_type,
            test_id: None,
            message,
            error_category: None,
        })
        .await;
    }

    /// Get current progress
    pub async fn get_progress(&self) -> ProgressInfo {
        let suites = self.suites.read().await;
        let mut total = 0;
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;

        for suite in suites.values() {
            total += suite.total_count();
            passed += suite.passed_count();
            failed += suite.failed_count();
            skipped += suite.results.iter().filter(|r| r.status == TestStatus::Skipped).count();
        }

        let completed = passed + failed + skipped;
        let progress_percent = if total == 0 {
            0.0
        } else {
            (completed as f64 / total as f64) * 100.0
        };

        ProgressInfo {
            total_tests: total,
            completed_tests: completed,
            passed_tests: passed,
            failed_tests: failed,
            skipped_tests: skipped,
            progress_percent,
            start_time: self.started_at,
            current_time: Utc::now(),
        }
    }

    /// Generate final report
    pub async fn generate_report(&self, system_id: String) -> TestReport {
        let mut suites = self.suites.write().await;

        // Mark all suites as completed
        for suite in suites.values_mut() {
            suite.mark_completed();
        }

        let mut report = TestReport::new(system_id);

        for suite in suites.values() {
            report.add_suite(suite.clone());
        }

        // Add summary note
        let duration = self.start_time.elapsed();
        let error_summary = self.error_categories.read().await;
        let mut error_text = String::new();

        if !error_summary.is_empty() {
            error_text.push_str("Error Categories:\n");
            for (category, count) in error_summary.iter() {
                error_text.push_str(&format!("- {}: {}\n", category.as_str(), count));
            }
        }

        report = report.with_notes(format!(
            "Total Duration: {:.2}s\n{}",
            duration.as_secs_f64(),
            error_text
        ));

        report
    }

    /// Get all events
    pub async fn get_events(&self) -> Vec<TestEvent> {
        self.events.read().await.clone()
    }

    /// Get error category breakdown
    pub async fn get_error_breakdown(&self) -> HashMap<ErrorCategory, usize> {
        self.error_categories.read().await.clone()
    }

    /// Get all suites
    pub async fn get_suites(&self) -> Vec<TestSuite> {
        let suites = self.suites.read().await;
        suites.values().cloned().collect()
    }

    /// Get suite by name
    pub async fn get_suite(&self, name: &str) -> Option<TestSuite> {
        let suites = self.suites.read().await;
        suites.get(name).cloned()
    }

    /// Get total test count
    pub async fn total_tests(&self) -> usize {
        let suites = self.suites.read().await;
        suites.values().map(|s| s.total_count()).sum()
    }

    /// Get passed test count
    pub async fn passed_tests(&self) -> usize {
        let suites = self.suites.read().await;
        suites.values().map(|s| s.passed_count()).sum()
    }

    /// Get failed test count
    pub async fn failed_tests(&self) -> usize {
        let suites = self.suites.read().await;
        suites.values().map(|s| s.failed_count()).sum()
    }

    /// Export all results as JSON
    pub async fn to_json(&self, system_id: String) -> Result<String, serde_json::Error> {
        let report = self.generate_report(system_id).await;
        report.to_json()
    }

    /// Export all results as CSV
    pub async fn to_csv(&self, system_id: String) -> String {
        let report = self.generate_report(system_id).await;
        report.to_csv()
    }

    /// Export all results as markdown
    pub async fn to_markdown(&self, system_id: String) -> String {
        let report = self.generate_report(system_id).await;
        report.to_markdown()
    }
}

impl Default for ResultCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ResultCollector {
    fn clone(&self) -> Self {
        Self {
            suites: Arc::clone(&self.suites),
            events: Arc::clone(&self.events),
            error_categories: Arc::clone(&self.error_categories),
            start_time: Instant::now(),
            started_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_collector_creation() {
        let collector = ResultCollector::new();
        assert_eq!(collector.total_tests().await, 0);
    }

    #[tokio::test]
    async fn test_add_result() {
        let collector = ResultCollector::new();
        let result = TestResult::new(
            "test_001".to_string(),
            "Test 1".to_string(),
            "SCPI".to_string(),
        );

        collector.add_result(result).await;

        assert_eq!(collector.total_tests().await, 1);
        assert_eq!(collector.passed_tests().await, 1);
    }

    #[tokio::test]
    async fn test_error_categorization() {
        let collector = ResultCollector::new();
        let result = TestResult::new(
            "test_001".to_string(),
            "Test 1".to_string(),
            "SCPI".to_string(),
        )
        .with_status(TestStatus::Timeout)
        .with_error("Operation timed out after 5 seconds".to_string());

        collector.add_result(result).await;

        let breakdown = collector.get_error_breakdown().await;
        assert!(breakdown.contains_key(&ErrorCategory::Timeout));
    }

    #[tokio::test]
    async fn test_progress_tracking() {
        let collector = ResultCollector::new();

        for i in 0..10 {
            let result = TestResult::new(
                format!("test_{}", i),
                format!("Test {}", i),
                "SCPI".to_string(),
            );
            collector.add_result(result).await;
        }

        let progress = collector.get_progress().await;
        assert_eq!(progress.total_tests, 10);
        assert_eq!(progress.completed_tests, 10);
        assert_eq!(progress.progress_percent, 100.0);
    }

    #[tokio::test]
    async fn test_suite_management() {
        let collector = ResultCollector::new();

        let result1 = TestResult::new(
            "test_001".to_string(),
            "Test 1".to_string(),
            "SCPI".to_string(),
        );
        let result2 = TestResult::new(
            "test_002".to_string(),
            "Test 2".to_string(),
            "Newport".to_string(),
        );

        collector.add_result(result1).await;
        collector.add_result(result2).await;

        let scpi_suite = collector.get_suite("SCPI").await;
        assert!(scpi_suite.is_some());
        assert_eq!(scpi_suite.unwrap().total_count(), 1);

        let suites = collector.get_suites().await;
        assert_eq!(suites.len(), 2);
    }

    #[tokio::test]
    async fn test_event_logging() {
        let collector = ResultCollector::new();

        collector
            .log_message(
                TestEventType::ProgressUpdate,
                "Test run started".to_string(),
            )
            .await;

        let events = collector.get_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, TestEventType::ProgressUpdate);
    }

    #[test]
    fn test_error_category_detection() {
        assert_eq!(
            ErrorCategory::from_error("Operation timed out"),
            ErrorCategory::Timeout
        );
        assert_eq!(
            ErrorCategory::from_error("Hardware communication error"),
            ErrorCategory::HardwareError
        );
        assert_eq!(
            ErrorCategory::from_error("Safety limit exceeded"),
            ErrorCategory::SafetyViolation
        );
    }
}
