//! Timestamp utilities for the DAQ system.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current timestamp in nanoseconds since Unix epoch.
///
/// Returns 0 if system clock is before Unix epoch (bd-21yj).
#[expect(
    clippy::cast_possible_truncation,
    reason = "nanos since epoch fits in u64 until year 2554"
)]
pub fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Current timestamp in nanoseconds since Unix epoch, as `i64`.
///
/// Useful for contexts where signed timestamps are expected (e.g., state cache).
/// Returns 0 if system clock is before Unix epoch.
#[expect(
    clippy::cast_possible_truncation,
    reason = "nanos since epoch fits in i64 until year 2262"
)]
pub fn now_ns_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}
