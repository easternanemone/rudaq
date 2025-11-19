//! Testing utilities and infrastructure for hardware validation.
//!
//! This module provides comprehensive test result collection, aggregation, and reporting
//! for the hardware validation test suite. It includes:
//!
//! - `TestResult`: Individual test execution results with detailed metrics
//! - `TestSuite`: Aggregated results for a group of related tests
//! - `TestReport`: Complete test run report with statistics and analysis
//! - Serialization support (JSON, CSV, markdown)
//! - Statistics calculation (pass rates, timing analysis, error categorization)
//! - Baseline comparison for regression detection

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use chrono::{DateTime, Utc};

pub mod hardware_report;
pub mod result_collector;

pub use hardware_report::{
    HardwareReport, HardwareStatus, EnvironmentalMetrics, HardwarePerformance,
    SafetyIncident, IncidentSeverity, IncidentType, TestLog, LogLevel, MeasurementData,
    MeasurementStats,
};
pub use result_collector::{ResultCollector, ErrorCategory, TestEvent, TestEventType, ProgressInfo};

/// Individual test result with detailed metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Unique test identifier
    pub test_id: String,
    /// Test name for display
    pub test_name: String,
    /// Category/actor name (e.g., "SCPI", "Newport1830C")
    pub category: String,
    /// Test execution status
    pub status: TestStatus,
    /// Test execution start time
    pub started_at: DateTime<Utc>,
    /// Test execution end time
    pub ended_at: DateTime<Utc>,
    /// Total execution duration
    pub duration: Duration,
    /// Test output/logs
    pub output: String,
    /// Error message if test failed
    pub error: Option<String>,
    /// Custom metrics as key-value pairs
    pub metrics: HashMap<String, f64>,
    /// Performance measurements
    pub performance: Option<PerformanceMetrics>,
    /// Safety-related observations
    pub safety_notes: Option<String>,
}

/// Test execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TestStatus {
    /// Test passed successfully
    Passed,
    /// Test failed
    Failed,
    /// Test was skipped
    Skipped,
    /// Test timed out
    Timeout,
    /// Test encountered a hardware error
    HardwareError,
    /// Safety violation detected
    SafetyViolation,
}

impl TestStatus {
    /// Check if status indicates success
    pub fn is_passed(&self) -> bool {
        matches!(self, TestStatus::Passed)
    }

    /// Check if status indicates failure
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            TestStatus::Failed
                | TestStatus::Timeout
                | TestStatus::HardwareError
                | TestStatus::SafetyViolation
        )
    }

    /// Get human-readable status string
    pub fn as_str(&self) -> &'static str {
        match self {
            TestStatus::Passed => "PASSED",
            TestStatus::Failed => "FAILED",
            TestStatus::Skipped => "SKIPPED",
            TestStatus::Timeout => "TIMEOUT",
            TestStatus::HardwareError => "HARDWARE_ERROR",
            TestStatus::SafetyViolation => "SAFETY_VIOLATION",
        }
    }
}

/// Performance metrics for a test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Execution time in milliseconds
    pub execution_time_ms: f64,
    /// Memory usage in MB
    pub memory_usage_mb: Option<f64>,
    /// CPU usage percentage
    pub cpu_usage_percent: Option<f64>,
    /// Throughput (items/sec)
    pub throughput: Option<f64>,
    /// Latency measurements
    pub latency_measurements: Option<Vec<f64>>,
}

impl TestResult {
    /// Create a new test result
    pub fn new(test_id: String, test_name: String, category: String) -> Self {
        let now = Utc::now();
        Self {
            test_id,
            test_name,
            category,
            status: TestStatus::Passed,
            started_at: now,
            ended_at: now,
            duration: Duration::ZERO,
            output: String::new(),
            error: None,
            metrics: HashMap::new(),
            performance: None,
            safety_notes: None,
        }
    }

    /// Set test status
    pub fn with_status(mut self, status: TestStatus) -> Self {
        self.status = status;
        self
    }

    /// Set test output
    pub fn with_output(mut self, output: String) -> Self {
        self.output = output;
        self
    }

    /// Set error message
    pub fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }

    /// Add a metric
    pub fn with_metric(mut self, key: String, value: f64) -> Self {
        self.metrics.insert(key, value);
        self
    }

    /// Set performance metrics
    pub fn with_performance(mut self, metrics: PerformanceMetrics) -> Self {
        self.performance = Some(metrics);
        self
    }

    /// Add safety notes
    pub fn with_safety_notes(mut self, notes: String) -> Self {
        self.safety_notes = Some(notes);
        self
    }

    /// Mark test as completed with given end time
    pub fn mark_completed(mut self, ended_at: DateTime<Utc>) -> Self {
        self.ended_at = ended_at;
        self.duration = ended_at
            .signed_duration_since(self.started_at)
            .to_std()
            .unwrap_or(Duration::ZERO);
        self
    }
}

/// Aggregated test results for a category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuite {
    /// Suite name (category/actor)
    pub name: String,
    /// All test results in this suite
    pub results: Vec<TestResult>,
    /// Suite start time
    pub started_at: DateTime<Utc>,
    /// Suite end time
    pub ended_at: DateTime<Utc>,
}

impl TestSuite {
    /// Create a new test suite
    pub fn new(name: String) -> Self {
        let now = Utc::now();
        Self {
            name,
            results: Vec::new(),
            started_at: now,
            ended_at: now,
        }
    }

    /// Add a test result
    pub fn add_result(&mut self, result: TestResult) {
        self.results.push(result);
    }

    /// Get total number of tests
    pub fn total_count(&self) -> usize {
        self.results.len()
    }

    /// Get number of passed tests
    pub fn passed_count(&self) -> usize {
        self.results.iter().filter(|r| r.status.is_passed()).count()
    }

    /// Get number of failed tests
    pub fn failed_count(&self) -> usize {
        self.results.iter().filter(|r| r.status.is_failure()).count()
    }

    /// Get pass rate as percentage
    pub fn pass_rate(&self) -> f64 {
        if self.results.is_empty() {
            return 100.0;
        }
        (self.passed_count() as f64 / self.total_count() as f64) * 100.0
    }

    /// Get total duration
    pub fn total_duration(&self) -> Duration {
        self.results.iter().map(|r| r.duration).sum()
    }

    /// Get average test duration
    pub fn average_duration(&self) -> Duration {
        if self.results.is_empty() {
            return Duration::ZERO;
        }
        let total: Duration = self.results.iter().map(|r| r.duration).sum();
        let secs = total.as_secs_f64() / self.results.len() as f64;
        Duration::from_secs_f64(secs)
    }

    /// Get failures with detailed information
    pub fn failures(&self) -> Vec<&TestResult> {
        self.results.iter().filter(|r| r.status.is_failure()).collect()
    }

    /// Mark suite as completed
    pub fn mark_completed(&mut self) {
        if !self.results.is_empty() {
            self.started_at = self.results.iter().map(|r| r.started_at).min().unwrap();
            self.ended_at = self.results.iter().map(|r| r.ended_at).max().unwrap();
        }
    }
}

/// Complete test report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    /// Report generation timestamp
    pub generated_at: DateTime<Utc>,
    /// System/configuration identifier
    pub system_id: String,
    /// All test suites
    pub suites: Vec<TestSuite>,
    /// Report notes
    pub notes: Option<String>,
}

impl TestReport {
    /// Create a new test report
    pub fn new(system_id: String) -> Self {
        Self {
            generated_at: Utc::now(),
            system_id,
            suites: Vec::new(),
            notes: None,
        }
    }

    /// Add a test suite
    pub fn add_suite(&mut self, suite: TestSuite) {
        self.suites.push(suite);
    }

    /// Get total number of tests
    pub fn total_tests(&self) -> usize {
        self.suites.iter().map(|s| s.total_count()).sum()
    }

    /// Get total passed tests
    pub fn total_passed(&self) -> usize {
        self.suites.iter().map(|s| s.passed_count()).sum()
    }

    /// Get total failed tests
    pub fn total_failed(&self) -> usize {
        self.suites.iter().map(|s| s.failed_count()).sum()
    }

    /// Get overall pass rate
    pub fn overall_pass_rate(&self) -> f64 {
        if self.total_tests() == 0 {
            return 100.0;
        }
        (self.total_passed() as f64 / self.total_tests() as f64) * 100.0
    }

    /// Get total duration across all suites
    pub fn total_duration(&self) -> Duration {
        self.suites.iter().map(|s| s.total_duration()).sum()
    }

    /// Get all failures across all suites
    pub fn all_failures(&self) -> Vec<(String, TestResult)> {
        let mut failures = Vec::new();
        for suite in &self.suites {
            for failure in suite.failures() {
                failures.push((suite.name.clone(), failure.clone()));
            }
        }
        failures
    }

    /// Add report notes
    pub fn with_notes(mut self, notes: String) -> Self {
        self.notes = Some(notes);
        self
    }

    /// Export report as JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Export report as CSV (flat format)
    pub fn to_csv(&self) -> String {
        let mut csv = String::from("Suite,TestID,TestName,Status,DurationMS,Output,Error\n");

        for suite in &self.suites {
            for result in &suite.results {
                let escaped_output = escape_csv_field(&result.output);
                let escaped_error = result
                    .error
                    .as_ref()
                    .map(|e| escape_csv_field(e))
                    .unwrap_or_default();

                csv.push_str(&format!(
                    "\"{}\",\"{}\",\"{}\",\"{}\",{},\"{}\",\"{}\"\n",
                    suite.name,
                    result.test_id,
                    result.test_name,
                    result.status.as_str(),
                    result.duration.as_millis(),
                    escaped_output,
                    escaped_error
                ));
            }
        }

        csv
    }

    /// Export report as markdown
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        // Header
        md.push_str(&format!(
            "# Hardware Validation Report\n\n**Date**: {}\n**System**: {}\n\n",
            self.generated_at.format("%Y-%m-%d %H:%M:%S"),
            self.system_id
        ));

        // Executive Summary
        md.push_str("## Executive Summary\n\n");
        md.push_str(&format!(
            "- **Total Tests**: {}\n",
            self.total_tests()
        ));
        md.push_str(&format!(
            "- **Passed**: {} ({:.1}%)\n",
            self.total_passed(),
            self.overall_pass_rate()
        ));
        md.push_str(&format!(
            "- **Failed**: {}\n",
            self.total_failed()
        ));
        md.push_str(&format!(
            "- **Duration**: {}\n\n",
            format_duration(self.total_duration())
        ));

        // Results by Suite
        md.push_str("## Results by Suite\n\n");
        for suite in &self.suites {
            md.push_str(&format!("### {} ({} tests)\n\n", suite.name, suite.total_count()));
            md.push_str(&format!(
                "- **Passed**: {} ({:.1}%)\n",
                suite.passed_count(),
                suite.pass_rate()
            ));
            md.push_str(&format!(
                "- **Failed**: {}\n",
                suite.failed_count()
            ));
            md.push_str(&format!(
                "- **Duration**: {}\n",
                format_duration(suite.total_duration())
            ));
            md.push_str(&format!(
                "- **Average per test**: {}\n\n",
                format_duration(suite.average_duration())
            ));

            // Individual results
            for result in &suite.results {
                let status_indicator = if result.status.is_passed() {
                    "✓"
                } else {
                    "✗"
                };
                md.push_str(&format!(
                    "{} **{}** - {} ({:.2}s)\n",
                    status_indicator,
                    result.test_name,
                    result.status.as_str(),
                    result.duration.as_secs_f64()
                ));

                if let Some(error) = &result.error {
                    md.push_str(&format!("  - Error: {}\n", error));
                }
            }

            md.push_str("\n");
        }

        // Failures section
        let failures = self.all_failures();
        if !failures.is_empty() {
            md.push_str("## Failures\n\n");
            for (suite_name, result) in &failures {
                md.push_str(&format!("### {} - {}\n\n", suite_name, result.test_name));
                md.push_str(&format!("**Status**: {}\n\n", result.status.as_str()));
                if let Some(error) = &result.error {
                    md.push_str(&format!("**Error**: {}\n\n", error));
                }
                md.push_str(&format!("**Output**:\n```\n{}\n```\n\n", result.output));
            }
        }

        // Notes
        if let Some(notes) = &self.notes {
            md.push_str(&format!("## Notes\n\n{}\n\n", notes));
        }

        md
    }
}

/// Helper function to format duration
fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();

    if secs >= 3600 {
        let hours = secs / 3600;
        let minutes = (secs % 3600) / 60;
        let seconds = secs % 60;
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if secs >= 60 {
        let minutes = secs / 60;
        let seconds = secs % 60;
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}.{:03}s", secs, millis)
    }
}

/// Helper function to escape CSV fields
fn escape_csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_creation() {
        let result = TestResult::new(
            "test_001".to_string(),
            "Basic Connection".to_string(),
            "SCPI".to_string(),
        );

        assert_eq!(result.test_id, "test_001");
        assert_eq!(result.category, "SCPI");
        assert!(result.status.is_passed());
    }

    #[test]
    fn test_suite_statistics() {
        let mut suite = TestSuite::new("SCPI".to_string());

        for i in 0..10 {
            let result = TestResult::new(
                format!("test_{}", i),
                format!("Test {}", i),
                "SCPI".to_string(),
            );
            suite.add_result(result);
        }

        assert_eq!(suite.total_count(), 10);
        assert_eq!(suite.passed_count(), 10);
        assert_eq!(suite.failed_count(), 0);
        assert_eq!(suite.pass_rate(), 100.0);
    }

    #[test]
    fn test_report_generation() {
        let mut report = TestReport::new("test-system".to_string());
        let mut suite = TestSuite::new("SCPI".to_string());

        let result = TestResult::new(
            "test_001".to_string(),
            "Basic Connection".to_string(),
            "SCPI".to_string(),
        );
        suite.add_result(result);
        report.add_suite(suite);

        assert_eq!(report.total_tests(), 1);
        assert_eq!(report.total_passed(), 1);
        assert_eq!(report.overall_pass_rate(), 100.0);
    }

    #[test]
    fn test_csv_export() {
        let mut report = TestReport::new("test-system".to_string());
        let mut suite = TestSuite::new("SCPI".to_string());

        let result = TestResult::new(
            "test_001".to_string(),
            "Basic Connection".to_string(),
            "SCPI".to_string(),
        );
        suite.add_result(result);
        report.add_suite(suite);

        let csv = report.to_csv();
        assert!(csv.contains("test_001"));
        assert!(csv.contains("SCPI"));
    }

    #[test]
    fn test_markdown_export() {
        let mut report = TestReport::new("test-system".to_string());
        let mut suite = TestSuite::new("SCPI".to_string());

        let result = TestResult::new(
            "test_001".to_string(),
            "Basic Connection".to_string(),
            "SCPI".to_string(),
        );
        suite.add_result(result);
        report.add_suite(suite);

        let md = report.to_markdown();
        assert!(md.contains("Hardware Validation Report"));
        assert!(md.contains("test-system"));
        assert!(md.contains("SCPI"));
    }
}
