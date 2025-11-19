//! Test report generation tool
//!
//! Parses test results and generates comprehensive markdown reports with
//! executive summary, per-category results, failure analysis, and recommendations.
//!
//! Usage:
//!   cargo run --example generate_test_report -- [--system-id <id>] [--output <dir>]

use chrono::Utc;
use std::fs;
use std::path::PathBuf;
use v4_daq::testing::{
    ResultCollector, TestResult, TestStatus, PerformanceMetrics, HardwareReport,
    HardwareStatus, EnvironmentalMetrics, HardwarePerformance,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let mut system_id = "maitai-eos".to_string();
    let mut output_dir = PathBuf::from("test-results");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--system-id" => {
                if i + 1 < args.len() {
                    system_id = args[i + 1].clone();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--output" => {
                if i + 1 < args.len() {
                    output_dir = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }

    println!("Generating test report for system: {}", system_id);

    // Create result collector and populate with sample data
    let collector = ResultCollector::new();

    // Simulate test results for different categories
    simulate_scpi_tests(&collector).await;
    simulate_newport_tests(&collector).await;
    simulate_esp300_tests(&collector).await;
    simulate_pvcam_tests(&collector).await;
    simulate_maitai_tests(&collector).await;

    // Get progress
    let progress = collector.get_progress().await;
    println!(
        "\nTest Summary: {}/{} passed ({:.1}%)",
        progress.passed_tests, progress.total_tests, progress.progress_percent
    );

    // Generate report
    let report = collector.generate_report(system_id.clone()).await;

    // Create output directory with timestamp
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S");
    let report_dir = output_dir.join(timestamp.to_string());
    fs::create_dir_all(&report_dir)?;

    // Export as markdown
    let markdown_path = report_dir.join("report.md");
    let markdown_content = report.to_markdown();
    fs::write(&markdown_path, &markdown_content)?;
    println!("\nMarkdown report: {}", markdown_path.display());

    // Export as JSON
    let json_path = report_dir.join("report.json");
    let json_content = report.to_json()?;
    fs::write(&json_path, &json_content)?;
    println!("JSON report: {}", json_path.display());

    // Export as CSV
    let csv_path = report_dir.join("report.csv");
    let csv_content = report.to_csv();
    fs::write(&csv_path, &csv_content)?;
    println!("CSV report: {}", csv_path.display());

    // Generate hardware reports for each device
    generate_hardware_reports(&report_dir)?;

    // Generate baseline if requested
    println!("\nGenerating baseline for regression testing...");
    let baseline_path = output_dir.join("baseline.json");
    fs::write(&baseline_path, &json_content)?;
    println!("Baseline saved: {}", baseline_path.display());

    // Compare against baseline if it exists
    if baseline_path.exists() {
        compare_with_baseline(&baseline_path, &json_content)?;
    }

    println!("\nReport generation complete!");
    println!("Results location: {}", report_dir.display());

    Ok(())
}

/// Simulate SCPI test results (17 tests)
async fn simulate_scpi_tests(collector: &ResultCollector) {
    let tests = vec![
        ("scpi_001", "Basic Connection"),
        ("scpi_002", "Device Identification"),
        ("scpi_003", "Command Queuing"),
        ("scpi_004", "Status Register Read"),
        ("scpi_005", "Error Queue Access"),
        ("scpi_006", "Clear Status Command"),
        ("scpi_007", "Event Handling"),
        ("scpi_008", "Command Timeout"),
        ("scpi_009", "Parameter Validation"),
        ("scpi_010", "Long Command Sequence"),
        ("scpi_011", "Concurrent Commands"),
        ("scpi_012", "Error Recovery"),
        ("scpi_013", "Status Query Response Time"),
        ("scpi_014", "Memory Allocation"),
        ("scpi_015", "Subsystem Interaction"),
        ("scpi_016", "Reset Command"),
        ("scpi_017", "Calibration Commands"),
    ];

    for (id, name) in tests {
        let status = if id == "scpi_008" {
            TestStatus::Timeout
        } else if id == "scpi_015" {
            TestStatus::Failed
        } else {
            TestStatus::Passed
        };

        let mut result = TestResult::new(
            id.to_string(),
            name.to_string(),
            "SCPI".to_string(),
        )
        .with_status(status);

        if status == TestStatus::Timeout {
            result = result.with_error("Command did not respond within 5000ms".to_string());
        } else if status == TestStatus::Failed {
            result = result.with_error("Subsystem 16 not responding to queries".to_string());
        }

        let now = Utc::now();
        result = result.mark_completed(now);
        result = result.with_performance(PerformanceMetrics {
            execution_time_ms: 45.5,
            memory_usage_mb: Some(2.3),
            cpu_usage_percent: Some(12.5),
            throughput: Some(220.0),
            latency_measurements: Some(vec![40.0, 42.0, 45.0, 48.0, 47.0]),
        });

        collector.add_result(result).await;
    }
}

/// Simulate Newport 1830-C test results (14 tests)
async fn simulate_newport_tests(collector: &ResultCollector) {
    let tests = vec![
        ("newport_001", "Serial Port Connection"),
        ("newport_002", "Motor Initialization"),
        ("newport_003", "Homing Sequence"),
        ("newport_004", "Position Readback"),
        ("newport_005", "Move Absolute"),
        ("newport_006", "Move Relative"),
        ("newport_007", "Velocity Setting"),
        ("newport_008", "Acceleration Control"),
        ("newport_009", "Limit Switch Detection"),
        ("newport_010", "Emergency Stop"),
        ("newport_011", "Temperature Monitoring"),
        ("newport_012", "Error State Recovery"),
        ("newport_013", "Repeatability Test"),
        ("newport_014", "Long Duration Test"),
    ];

    for (id, name) in tests {
        let status = if id == "newport_013" {
            TestStatus::Failed
        } else {
            TestStatus::Passed
        };

        let mut result = TestResult::new(
            id.to_string(),
            name.to_string(),
            "Newport1830C".to_string(),
        )
        .with_status(status);

        if status == TestStatus::Failed {
            result = result.with_error("Position repeatability ±15µm exceeded threshold of ±10µm".to_string());
        }

        let now = Utc::now();
        result = result.mark_completed(now);
        result = result.with_performance(PerformanceMetrics {
            execution_time_ms: 120.3,
            memory_usage_mb: Some(4.1),
            cpu_usage_percent: Some(8.2),
            throughput: Some(8.3),
            latency_measurements: None,
        });

        collector.add_result(result).await;
    }
}

/// Simulate ESP300 test results (16 tests)
async fn simulate_esp300_tests(collector: &ResultCollector) {
    let tests = vec![
        ("esp300_001", "Power Supply Check"),
        ("esp300_002", "Firmware Version"),
        ("esp300_003", "Memory Test"),
        ("esp300_004", "Axis Configuration"),
        ("esp300_005", "Enable Axis"),
        ("esp300_006", "Position Encoding"),
        ("esp300_007", "Speed Control"),
        ("esp300_008", "Jog Commands"),
        ("esp300_009", "Home Search"),
        ("esp300_010", "Absolute Positioning"),
        ("esp300_011", "Relative Positioning"),
        ("esp300_012", "Backlash Compensation"),
        ("esp300_013", "Abort Command"),
        ("esp300_014", "Status Polling"),
        ("esp300_015", "Event Logging"),
        ("esp300_016", "Shutdown Sequence"),
    ];

    for (id, name) in tests {
        let result = TestResult::new(
            id.to_string(),
            name.to_string(),
            "ESP300".to_string(),
        )
        .with_status(TestStatus::Passed);

        let now = Utc::now();
        let result = result.mark_completed(now);
        let result = result.with_performance(PerformanceMetrics {
            execution_time_ms: 85.7,
            memory_usage_mb: Some(3.5),
            cpu_usage_percent: Some(9.1),
            throughput: Some(11.7),
            latency_measurements: None,
        });

        collector.add_result(result).await;
    }
}

/// Simulate PVCAM test results (28 tests)
async fn simulate_pvcam_tests(collector: &ResultCollector) {
    let test_names = vec![
        "Camera Initialization",
        "Frame Grabbing",
        "Exposure Control",
        "Gain Adjustment",
        "Binning Modes",
        "Region of Interest",
        "Trigger Configuration",
        "Speed Table Setup",
        "CMS Control",
        "Port Control",
        "Output Amplifier",
        "ADC Offset",
        "ADC Gain",
        "Black Level",
        "Temperature Monitoring",
        "Readout Speed",
        "Shutter Control",
        "Sequential Readout",
        "Long Exposure",
        "Multiple Frames",
        "Frame Rate Test",
        "Error Handling",
        "Timeout Recovery",
        "Hardware Reset",
        "Firmware Query",
        "Memory Access",
        "Sensor Health",
        "Thermal Stability",
    ];

    for (idx, name) in test_names.iter().enumerate() {
        let id = format!("pvcam_{:03}", idx + 1);
        let result = TestResult::new(
            id.to_string(),
            name.to_string(),
            "PVCAM".to_string(),
        )
        .with_status(TestStatus::Passed);

        let now = Utc::now();
        let result = result.mark_completed(now);
        let result = result.with_performance(PerformanceMetrics {
            execution_time_ms: 250.5,
            memory_usage_mb: Some(15.2),
            cpu_usage_percent: Some(35.4),
            throughput: Some(4.0),
            latency_measurements: None,
        });

        collector.add_result(result).await;
    }
}

/// Simulate MaiTai test results (19 tests)
async fn simulate_maitai_tests(collector: &ResultCollector) {
    let tests = vec![
        ("maitai_001", "Power Supply Test"),
        ("maitai_002", "Wavelength Range"),
        ("maitai_003", "Shutter Control"),
        ("maitai_004", "Power Level"),
        ("maitai_005", "Frequency Stabilization"),
        ("maitai_006", "Temperature Control"),
        ("maitai_007", "Interlock Status"),
        ("maitai_008", "Safety Interlocks"),
        ("maitai_009", "Command Response Time"),
        ("maitai_010", "Serial Communication"),
        ("maitai_011", "Error Recovery"),
        ("maitai_012", "Status Register"),
        ("maitai_013", "Long Term Stability"),
        ("maitai_014", "Thermal Cycling"),
        ("maitai_015", "Mode Hopping"),
        ("maitai_016", "Power Ramp"),
        ("maitai_017", "Wavelength Tuning"),
        ("maitai_018", "Harmonics Control"),
        ("maitai_019", "Calibration Verification"),
    ];

    for (id, name) in tests {
        let status = if id == "maitai_013" {
            TestStatus::Failed
        } else {
            TestStatus::Passed
        };

        let mut result = TestResult::new(
            id.to_string(),
            name.to_string(),
            "MaiTai".to_string(),
        )
        .with_status(status);

        if status == TestStatus::Failed {
            result = result.with_error("Power drift exceeded 2% over 1 hour test".to_string());
        }

        let now = Utc::now();
        result = result.mark_completed(now);
        result = result.with_performance(PerformanceMetrics {
            execution_time_ms: 180.2,
            memory_usage_mb: Some(5.8),
            cpu_usage_percent: Some(15.3),
            throughput: Some(5.6),
            latency_measurements: None,
        });

        result = result.with_safety_notes("Operating within safety parameters".to_string());

        collector.add_result(result).await;
    }
}

/// Generate hardware-specific reports
fn generate_hardware_reports(report_dir: &PathBuf) -> anyhow::Result<()> {
    // MaiTai Hardware Report
    let mut maitai_report = HardwareReport::new("DEV_MAITAI_001".to_string(), "MaiTai".to_string())
        .with_firmware("3.2.1".to_string())
        .with_status(HardwareStatus::Healthy);

    maitai_report.environment = EnvironmentalMetrics {
        ambient_temperature_c: Some(22.5),
        humidity_percent: Some(45.0),
        pressure_hpa: Some(1013.2),
        dust_level: None,
        vibration_level: None,
    };

    maitai_report.performance = HardwarePerformance {
        response_time_ms: Some(12.5),
        command_success_rate: Some(99.8),
        power_consumption_w: Some(85.3),
        temperature_stability: Some(0.3),
        position_repeatability_um: None,
        throughput_cps: Some(80.0),
        error_rate_per_1k: Some(0.2),
    };

    maitai_report.add_log(v4_daq::testing::hardware_report::LogLevel::Info, "Hardware initialized successfully".to_string());
    maitai_report.add_measurement("Temperature".to_string(), "C".to_string(), vec![22.5, 22.6, 22.4, 22.5, 22.7]);
    maitai_report.add_measurement("Power".to_string(), "W".to_string(), vec![85.0, 85.3, 85.2, 85.5, 85.1]);

    let maitai_json = serde_json::to_string_pretty(&maitai_report)?;
    fs::write(report_dir.join("hardware_maitai.json"), maitai_json)?;
    fs::write(report_dir.join("hardware_maitai.md"), maitai_report.to_markdown())?;

    println!("Generated MaiTai hardware reports");

    Ok(())
}

/// Compare current results with baseline
fn compare_with_baseline(baseline_path: &PathBuf, current_json: &str) -> anyhow::Result<()> {
    let baseline_content = fs::read_to_string(baseline_path)?;

    let baseline: serde_json::Value = serde_json::from_str(&baseline_content)?;
    let current: serde_json::Value = serde_json::from_str(current_json)?;

    let baseline_passed = baseline["total_passed"].as_u64().unwrap_or(0);
    let current_passed = current["total_passed"].as_u64().unwrap_or(0);

    let baseline_failed = baseline["total_failed"].as_u64().unwrap_or(0);
    let current_failed = current["total_failed"].as_u64().unwrap_or(0);

    println!("\nRegression Analysis:");
    println!("  Baseline - Passed: {}, Failed: {}", baseline_passed, baseline_failed);
    println!("  Current  - Passed: {}, Failed: {}", current_passed, current_failed);

    if current_failed > baseline_failed {
        println!("  WARNING: {} new failures detected!", current_failed - baseline_failed);
    } else if current_passed > baseline_passed {
        println!("  SUCCESS: {} new tests passing!", current_passed - baseline_passed);
    } else {
        println!("  Status unchanged from baseline");
    }

    Ok(())
}
