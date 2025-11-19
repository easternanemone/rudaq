# Hardware Validation Testing Infrastructure

This document describes the comprehensive test result reporting and collection system for the V4 DAQ hardware validation suite.

## Overview

The testing infrastructure provides:

- **Test Result Collection**: Real-time gathering of test results with timestamped events
- **Result Aggregation**: Organization of results by test suite/category
- **Multi-Format Export**: JSON, CSV, and markdown report generation
- **Hardware Reporting**: Detailed hardware metrics, safety incidents, and environmental conditions
- **Regression Detection**: Baseline comparison for identifying test regressions
- **Error Categorization**: Automatic classification of failure types (timeout, hardware error, safety violation, etc.)

## Architecture

### Core Modules

#### `src/testing/mod.rs` - Core Testing Module
Provides test result structures and report generation:

- **`TestResult`** - Individual test execution with metrics and performance data
- **`TestStatus`** - Pass/fail/timeout/hardware-error/safety-violation status
- **`TestSuite`** - Aggregated results for a category (SCPI, Newport1830C, etc.)
- **`TestReport`** - Complete test run report with statistics
- **`PerformanceMetrics`** - Execution time, memory, CPU, throughput, latency

```rust
// Creating a test result
let result = TestResult::new(
    "test_001".to_string(),
    "Basic Connection".to_string(),
    "SCPI".to_string(),
)
.with_status(TestStatus::Passed)
.with_performance(PerformanceMetrics {
    execution_time_ms: 45.5,
    memory_usage_mb: Some(2.3),
    cpu_usage_percent: Some(12.5),
    throughput: Some(220.0),
    latency_measurements: Some(vec![40.0, 42.0, 45.0]),
})
.mark_completed(Utc::now());
```

#### `src/testing/hardware_report.rs` - Hardware-Specific Reporting
Detailed hardware testing capabilities:

- **`HardwareReport`** - Device-specific test report
- **`HardwareStatus`** - Healthy/Degraded/Faulty/Unresponsive/Testing/Ready
- **`EnvironmentalMetrics`** - Temperature, humidity, pressure, vibration
- **`HardwarePerformance`** - Response time, success rate, power, repeatability
- **`SafetyIncident`** - Safety violations with severity levels
- **`MeasurementData`** - Statistical tracking of sensor readings

```rust
// Creating hardware report
let mut report = HardwareReport::new("DEV001", "MaiTai")
    .with_firmware("3.2.1")
    .with_status(HardwareStatus::Healthy);

report.environment = EnvironmentalMetrics {
    ambient_temperature_c: Some(22.5),
    humidity_percent: Some(45.0),
    pressure_hpa: Some(1013.2),
    dust_level: None,
    vibration_level: None,
};

// Add measurements
report.add_measurement("Temperature".to_string(), "C".to_string(),
    vec![22.5, 22.6, 22.4, 22.5, 22.7]);

// Add safety incident
report.add_safety_incident(SafetyIncident {
    timestamp: Utc::now(),
    severity: IncidentSeverity::Warning,
    incident_type: IncidentType::TemperatureExceeded,
    description: "Temperature exceeded threshold".to_string(),
    recovery_action: Some("Reduced power output".to_string()),
});
```

#### `src/testing/result_collector.rs` - Real-Time Collection
Asynchronous result collection with progress tracking:

- **`ResultCollector`** - Thread-safe result accumulation
- **`ErrorCategory`** - Timeout, HardwareError, SafetyViolation, ConfigError, etc.
- **`TestEvent`** - Timestamped events for audit trail
- **`ProgressInfo`** - Real-time test execution progress with ETA

```rust
// Creating collector
let collector = ResultCollector::new();

// Adding results asynchronously
for result in results {
    collector.add_result(result).await;
}

// Tracking progress
let progress = collector.get_progress().await;
println!("Progress: {}/{} ({:.1}%)",
    progress.completed_tests,
    progress.total_tests,
    progress.progress_percent);

// Generate final report
let report = collector.generate_report("system-id".to_string()).await;
```

## Usage Examples

### Basic Test Reporting

```rust
use v4_daq::testing::{TestResult, TestStatus, TestSuite, TestReport};

// Create test results
let result1 = TestResult::new("test_001".to_string(), "Test 1".to_string(), "SCPI".to_string())
    .with_status(TestStatus::Passed);

let result2 = TestResult::new("test_002".to_string(), "Test 2".to_string(), "SCPI".to_string())
    .with_status(TestStatus::Failed)
    .with_error("Assertion failed".to_string());

// Aggregate into suite
let mut suite = TestSuite::new("SCPI".to_string());
suite.add_result(result1);
suite.add_result(result2);

// Create report
let mut report = TestReport::new("maitai-eos".to_string());
report.add_suite(suite);

// Export
println!("{}", report.to_markdown());
let json = report.to_json()?;
let csv = report.to_csv();
```

### Real-Time Collection with Async

```rust
use v4_daq::testing::ResultCollector;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let collector = ResultCollector::new();

    // Collect results as tests run
    for test in tests {
        let result = run_test(&test).await;
        collector.add_result(result).await;

        // Track progress
        let progress = collector.get_progress().await;
        println!("Progress: {:.1}% - ETA: {}",
            progress.progress_percent,
            progress.time_remaining_str());
    }

    // Generate final report
    let report = collector.generate_report("system-id".to_string()).await;
    println!("{}", report.to_markdown());

    Ok(())
}
```

### Hardware-Specific Reporting

```rust
use v4_daq::testing::HardwareReport;

let mut report = HardwareReport::new("DEV_MAITAI_001", "MaiTai")
    .with_firmware("3.2.1");

// Log test events
report.add_log(LogLevel::Info, "Hardware initialized".to_string());

// Add performance metrics
report.performance.response_time_ms = Some(12.5);
report.performance.command_success_rate = Some(99.8);

// Add measurements with statistics
report.add_measurement(
    "Temperature".to_string(),
    "C".to_string(),
    vec![25.0, 25.1, 24.9, 25.2, 25.0]
);

// Export
fs::write("hardware_report.md", report.to_markdown())?;
fs::write("hardware_report.json", serde_json::to_string_pretty(&report)?)?;
```

### Regression Testing

```rust
// Load baseline
let baseline: TestReport = serde_json::from_str(&baseline_json)?;

// Load current results
let current: TestReport = serde_json::from_str(&current_json)?;

// Compare
if current.total_failed() > baseline.total_failed() {
    eprintln!("REGRESSION: {} new failures",
        current.total_failed() - baseline.total_failed());
    std::process::exit(1);
}

println!("PASS: Test results match or exceed baseline");
```

## Report Generation Example

The `examples/generate_test_report.rs` example demonstrates complete workflow:

```bash
# Generate test report
cargo run --example generate_test_report -- \
    --system-id maitai-eos \
    --output test-results
```

Output files:
- `report.md` - Human-readable markdown report
- `report.json` - Machine-readable JSON for automation
- `report.csv` - Spreadsheet-compatible CSV
- `hardware_*.json` - Per-device hardware reports
- `hardware_*.md` - Per-device hardware markdown

### Report Structure

The generated markdown report includes:

```markdown
# Hardware Validation Report
**Date**: YYYY-MM-DD HH:MM:SS
**System**: system-id

## Executive Summary
- Total Tests: 94
- Passed: 90 (95.7%)
- Failed: 4
- Duration: 1h 23m 45s

## Results by Suite
### SCPI (17 tests)
- Passed: 15 (88.2%)
- Failed: 2
[Individual test results with status indicators]

### Newport1830C (14 tests)
[...]

## Failures
### SCPI - Test Name
**Status**: FAILED
**Error**: Detailed error message
**Output**: Test log output

## Notes
[Summary of error categories and recommendations]
```

## Baseline Management

### Creating Baseline

```bash
# Create/update baseline from successful test run
./scripts/hardware_validation/create_baseline.sh \
    --system-id maitai-eos \
    --output-dir test-results
```

The script:
1. Builds and runs test suite
2. Generates report with timestamp
3. Captures results as baseline JSON
4. Stores for future regression detection

### Comparing with Baseline

```bash
# Compare current run with baseline
./scripts/hardware_validation/create_baseline.sh \
    --system-id maitai-eos \
    --output-dir test-results \
    --compare
```

Output includes:
- Baseline vs. Current test counts
- New failures detected
- New tests passing
- Regression alerts

## Error Categorization

Tests failures are automatically categorized:

- **Timeout** - Test exceeded time limit
- **HardwareError** - Communication or device error
- **SafetyViolation** - Safety threshold exceeded
- **ConfigError** - Configuration/setup failure
- **Exception** - Unexpected panic/exception
- **AssertionFailure** - Test assertion failed
- **Unknown** - Uncategorized error

Error categories are tracked in report notes for analysis.

## Export Formats

### JSON Export
```json
{
  "generated_at": "2025-11-18T13:12:53Z",
  "system_id": "maitai-eos",
  "total_tests": 94,
  "total_passed": 90,
  "total_failed": 4,
  "overall_pass_rate": 95.7,
  "suites": [
    {
      "name": "SCPI",
      "results": [
        {
          "test_id": "scpi_001",
          "test_name": "Basic Connection",
          "status": "PASSED",
          "duration": 0.045,
          "metrics": {...},
          "performance": {...}
        }
      ]
    }
  ]
}
```

### CSV Export
```csv
Suite,TestID,TestName,Status,DurationMS,Output,Error
SCPI,scpi_001,Basic Connection,PASSED,45.5,,
SCPI,scpi_002,Device Identification,PASSED,42.3,,
Newport,newport_001,Serial Connection,FAILED,120.1,,Error message
```

### Markdown Export
Human-readable format with:
- Executive summary
- Per-category results
- Failure details
- Performance analysis
- Error categorization
- Recommendations

## Integration with CI/CD

### GitHub Actions Example

```yaml
- name: Run Hardware Tests
  run: cargo run --example generate_test_report

- name: Check Baseline
  run: |
    ./scripts/hardware_validation/create_baseline.sh \
      --compare \
      --system-id maitai-eos
  continue-on-error: true

- name: Upload Results
  uses: actions/upload-artifact@v3
  with:
    name: test-results
    path: test-results/
```

## Key Capabilities

### Real-Time Progress Tracking
- Current completion percentage
- Estimated time to completion
- Tests passed/failed/skipped count
- Active test category

### Comprehensive Metrics
- Execution time per test
- Memory and CPU usage
- Throughput measurements
- Latency distributions
- Hardware-specific measurements

### Safety Monitoring
- Incident logging with severity
- Recovery action tracking
- Environmental condition recording
- Temperature/power anomalies
- Motion limit violations

### Failure Analysis
- Automatic error categorization
- Detailed error messages
- Test output preservation
- Hardware state snapshots

## Performance

- **Async Collection**: Non-blocking result collection using Tokio
- **Memory Efficient**: Streaming JSON generation for large result sets
- **CSV Export**: O(n) streaming for spreadsheet compatibility
- **Fast Markdown**: Quick human-readable report generation

## Testing

The testing infrastructure includes comprehensive unit tests:

```bash
# Run testing module tests
cargo test --lib testing

# Run example test report generation
cargo run --example generate_test_report -- \
    --system-id test-system \
    --output /tmp/test-results
```

## API Reference

### TestResult

```rust
pub fn new(test_id: String, test_name: String, category: String) -> Self
pub fn with_status(self, status: TestStatus) -> Self
pub fn with_error(self, error: String) -> Self
pub fn with_performance(self, metrics: PerformanceMetrics) -> Self
pub fn with_safety_notes(self, notes: String) -> Self
pub fn mark_completed(self, ended_at: DateTime<Utc>) -> Self
```

### TestSuite

```rust
pub fn new(name: String) -> Self
pub fn add_result(&mut self, result: TestResult)
pub fn passed_count(&self) -> usize
pub fn failed_count(&self) -> usize
pub fn pass_rate(&self) -> f64
pub fn failures(&self) -> Vec<&TestResult>
pub fn mark_completed(&mut self)
```

### TestReport

```rust
pub fn new(system_id: String) -> Self
pub fn add_suite(&mut self, suite: TestSuite)
pub fn total_tests(&self) -> usize
pub fn total_passed(&self) -> usize
pub fn total_failed(&self) -> usize
pub fn overall_pass_rate(&self) -> f64
pub fn to_json(&self) -> Result<String>
pub fn to_csv(&self) -> String
pub fn to_markdown(&self) -> String
pub fn with_notes(self, notes: String) -> Self
```

### ResultCollector

```rust
pub async fn add_result(&self, result: TestResult)
pub async fn log_event(&self, event: TestEvent)
pub async fn get_progress(&self) -> ProgressInfo
pub async fn generate_report(&self, system_id: String) -> TestReport
pub async fn to_json(&self, system_id: String) -> Result<String>
pub async fn to_csv(&self, system_id: String) -> String
pub async fn to_markdown(&self, system_id: String) -> String
```

### HardwareReport

```rust
pub fn new(device_id: String, device_type: String) -> Self
pub fn add_safety_incident(&mut self, incident: SafetyIncident)
pub fn add_log(&mut self, level: LogLevel, message: String)
pub fn add_measurement(&mut self, name: String, unit: String, values: Vec<f64>)
pub fn critical_incidents(&self) -> Vec<&SafetyIncident>
pub fn error_logs(&self) -> Vec<&TestLog>
pub fn to_markdown(&self) -> String
```

## Future Enhancements

Planned improvements:
- Database storage for historical tracking
- Real-time web dashboard for test monitoring
- Advanced statistics and trend analysis
- Comparative analysis across system versions
- Machine learning-based anomaly detection
- Integration with performance monitoring tools
- Parallel test execution coordination

## Support and Contributing

For issues, improvements, or questions about the testing infrastructure:
1. Check existing issues in the repository
2. Review test documentation and examples
3. Run example test report generation
4. Examine test output for error details

## License

See main project LICENSE file.
