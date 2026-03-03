//! Timestamp utilities for the DAQ system.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current timestamp in nanoseconds since Unix epoch.
///
/// Returns 0 if system clock is before Unix epoch (bd-21yj).
#[allow(clippy::cast_possible_truncation)]
// SAFETY: value is bounded and fits in target type
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
#[allow(clippy::cast_possible_truncation)]
// SAFETY: value is bounded and fits in target type
pub fn now_ns_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}
