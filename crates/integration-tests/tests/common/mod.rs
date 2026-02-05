//! Common test utilities for rust-daq integration tests
//!
//! This module provides reusable test helpers for:
//! - Timing assertions with appropriate tolerances
//! - Environment-aware tolerance selection
//! - Test setup utilities

#![allow(dead_code)] // Utilities may not all be used immediately

use std::time::Duration;

/// Tolerance levels for real-time timing assertions.
///
/// Use these when `start_paused = true` is not appropriate (e.g., testing
/// actual hardware timing behavior rather than mock behavior).
#[derive(Debug, Clone, Copy)]
pub enum TimingTolerance {
    /// Exact match - only for simulated time with `start_paused = true`
    Exact,
    /// 5% tolerance - only for stable, controlled environments
    Tight,
    /// 20% tolerance - default for local development
    Normal,
    /// 50% tolerance - for CI environments with variable load
    Relaxed,
    /// 100% tolerance - for resource-constrained environments
    VeryRelaxed,
}

impl TimingTolerance {
    /// Get the tolerance factor as a fraction (0.0 to 1.0)
    pub fn factor(&self) -> f64 {
        match self {
            TimingTolerance::Exact => 0.0,
            TimingTolerance::Tight => 0.05,
            TimingTolerance::Normal => 0.20,
            TimingTolerance::Relaxed => 0.50,
            TimingTolerance::VeryRelaxed => 1.0,
        }
    }
}

/// Assert that a duration is within tolerance of an expected value.
///
/// # Arguments
/// * `actual` - The measured duration
/// * `expected` - The expected duration
/// * `tolerance` - The tolerance level to apply
/// * `context` - A description of what was being measured (for error messages)
///
/// # Panics
/// Panics if the actual duration is outside the tolerance range.
///
/// # Example
/// ```ignore
/// use std::time::Duration;
/// use common::{assert_duration_near, TimingTolerance};
///
/// let elapsed = Duration::from_millis(105);
/// assert_duration_near(
///     elapsed,
///     Duration::from_millis(100),
///     TimingTolerance::Normal,
///     "short operation"
/// );
/// ```
pub fn assert_duration_near(
    actual: Duration,
    expected: Duration,
    tolerance: TimingTolerance,
    context: &str,
) {
    let factor = tolerance.factor();
    let min = expected.mul_f64(1.0 - factor);
    let max = expected.mul_f64(1.0 + factor);

    assert!(
        actual >= min && actual <= max,
        "{}: expected {:?} ±{:.0}%, got {:?} (acceptable range: {:?} to {:?})",
        context,
        expected,
        factor * 100.0,
        actual,
        min,
        max
    );
}

/// Get appropriate timing tolerance based on environment.
///
/// Returns:
/// - `Relaxed` if running in CI (CI env var is set)
/// - `Tight` if TIMING_STRICT env var is set
/// - `Normal` otherwise
pub fn env_timing_tolerance() -> TimingTolerance {
    if std::env::var("CI").is_ok() {
        TimingTolerance::Relaxed
    } else if std::env::var("TIMING_STRICT").is_ok() {
        TimingTolerance::Tight
    } else {
        TimingTolerance::Normal
    }
}

/// Check if running in CI environment
pub fn is_ci() -> bool {
    std::env::var("CI").is_ok()
}

/// Skip test with message if condition is true
#[macro_export]
macro_rules! skip_if {
    ($condition:expr, $msg:expr) => {
        if $condition {
            eprintln!("Skipping test: {}", $msg);
            return;
        }
    };
}

/// Skip test if hardware is not available at the specified path
#[macro_export]
macro_rules! skip_without_hardware {
    ($port:expr) => {
        if !std::path::Path::new($port).exists() {
            eprintln!("Skipping test: hardware not available at {}", $port);
            return;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timing_tolerance_factors() {
        assert_eq!(TimingTolerance::Exact.factor(), 0.0);
        assert_eq!(TimingTolerance::Tight.factor(), 0.05);
        assert_eq!(TimingTolerance::Normal.factor(), 0.20);
        assert_eq!(TimingTolerance::Relaxed.factor(), 0.50);
        assert_eq!(TimingTolerance::VeryRelaxed.factor(), 1.0);
    }

    #[test]
    fn test_assert_duration_near_passes() {
        let expected = Duration::from_millis(100);

        // Exact match
        assert_duration_near(expected, expected, TimingTolerance::Exact, "exact");

        // Within 20% tolerance
        assert_duration_near(
            Duration::from_millis(110),
            expected,
            TimingTolerance::Normal,
            "10% over",
        );
        assert_duration_near(
            Duration::from_millis(85),
            expected,
            TimingTolerance::Normal,
            "15% under",
        );
    }

    #[test]
    #[should_panic(expected = "outside tolerance")]
    fn test_assert_duration_near_fails() {
        let expected = Duration::from_millis(100);
        // 30% over with 20% tolerance should fail
        assert_duration_near(
            Duration::from_millis(130),
            expected,
            TimingTolerance::Normal,
            "outside tolerance",
        );
    }
}
