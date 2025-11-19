//! Hardware Validation Framework Tests
//!
//! Main test harness for executing 94 hardware test scenarios across 5 V4 actors
//! on real hardware via SSH to maitai@maitai-eos.
//!
//! ## Test Execution
//!
//! Run all tests with:
//!   cargo test --test hardware_validation_test -- --ignored
//!
//! Run specific test suite:
//!   cargo test --test hardware_validation_test -- --ignored scpi
//!   cargo test --test hardware_validation_test -- --ignored esp300
//!   cargo test --test hardware_validation_test -- --ignored newport
//!   cargo test --test hardware_validation_test -- --ignored pvcam
//!   cargo test --test hardware_validation_test -- --ignored maitai
//!
//! ## Test Structure
//!
//! - 17 SCPI generic instrument tests
//! - 14 Newport 1830-C optical power meter tests
//! - 16 ESP300 motion controller tests
//! - 28 PVCAM camera tests
//! - 19 MaiTai laser tests (with critical safety checks)
//!
//! Total: 94 hardware test scenarios
//!
//! ## Safety Critical
//!
//! MaiTai tests include forced safety checks to ensure shutter remains closed.
//! All tests can be run on mock hardware for CI/CD.

mod hardware_validation;

use hardware_validation::*;

// Re-export test modules for discovery
pub use hardware_validation::scpi_hardware_tests;
pub use hardware_validation::newport_hardware_tests;
pub use hardware_validation::esp300_hardware_tests;
pub use hardware_validation::pvcam_hardware_tests;
pub use hardware_validation::maitai_hardware_tests;

/// Integration test: Verify test framework loads correctly
#[test]
fn test_framework_initialization() {
    let harness = HardwareTestHarness::new();
    assert_eq!(harness.results().len(), 0);

    let summary = harness.summary();
    assert_eq!(summary.total, 0);
    assert_eq!(summary.passed, 0);
    assert_eq!(summary.failed, 0);
}

/// Integration test: Test result creation
#[test]
fn test_result_creation() {
    let result = TestResult::passed("test_name", 100);
    assert!(result.passed);
    assert_eq!(result.duration_ms, 100);
    assert_eq!(result.test_name, "test_name");
    assert!(result.error_message.is_none());

    let failed = TestResult::failed("test_name", 50, "error message");
    assert!(!failed.passed);
    assert_eq!(failed.duration_ms, 50);
    assert!(failed.error_message.is_some());
}

/// Integration test: Test harness result collection
#[test]
fn test_harness_collection() {
    let mut harness = HardwareTestHarness::new();

    harness.add_result(TestResult::passed("test_1", 100));
    harness.add_result(TestResult::passed("test_2", 150));
    harness.add_result(TestResult::failed("test_3", 50, "error"));

    let summary = harness.summary();
    assert_eq!(summary.total, 3);
    assert_eq!(summary.passed, 2);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.total_duration_ms, 300);
}

/// Integration test: Test utilities
#[tokio::test]
async fn test_utilities_measure_execution() {
    let (result, duration_ms) = utils::measure_test_execution(|| {
        std::thread::sleep(std::time::Duration::from_millis(10));
        42
    });

    assert_eq!(result, 42);
    assert!(duration_ms >= 10);
}

/// Integration test: Timeout utility
#[tokio::test]
async fn test_utilities_timeout() {
    let result = utils::verify_hardware_response_timeout(
        async {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            Ok::<i32, String>(42)
        },
        std::time::Duration::from_secs(1),
    )
    .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

/// Integration test: Safe operation wrapper
#[tokio::test]
async fn test_utilities_safe_operation() {
    let result = utils::safe_operation(
        || Ok(()),
        async { Ok::<i32, String>(42) },
        || Ok(()),
    )
    .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// Test all test suites are available
    #[test]
    fn test_all_suites_available() {
        // This test verifies that all test modules compile and are discoverable
        // The actual hardware tests are marked with #[ignore] and run with --ignored flag
        println!("SCPI tests: 17 scenarios");
        println!("Newport 1830-C tests: 14 scenarios");
        println!("ESP300 tests: 16 scenarios");
        println!("PVCAM tests: 28 scenarios");
        println!("MaiTai tests: 19 scenarios");
        println!("Total: 94 hardware test scenarios");
    }

    /// Documentation test
    #[test]
    fn test_framework_documentation() {
        println!("\n=== Hardware Validation Framework ===");
        println!("\nUsage:");
        println!("  cargo test --test hardware_validation_test -- --ignored");
        println!("\nBy suite:");
        println!("  cargo test --test hardware_validation_test -- --ignored scpi");
        println!("  cargo test --test hardware_validation_test -- --ignored newport");
        println!("  cargo test --test hardware_validation_test -- --ignored esp300");
        println!("  cargo test --test hardware_validation_test -- --ignored pvcam");
        println!("  cargo test --test hardware_validation_test -- --ignored maitai");
        println!("\nSafety Features:");
        println!("  - MaiTai shutter safety verification");
        println!("  - ESP300 axis safe return procedures");
        println!("  - Timeout handling for slow hardware");
        println!("  - Error recovery mechanisms");
        println!("  - Detailed result reporting");
    }
}
