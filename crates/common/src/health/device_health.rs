//! Per-device health tracking for the device registry (bd-qa36.4.2).
//!
//! Provides health state tracking for individual hardware devices, enabling
//! the device supervisor to detect faulted devices and attempt restart with
//! exponential backoff.

use std::time::{Duration, Instant};

/// Health status of a single registered device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceHealth {
    /// Device is operating normally.
    Healthy,
    /// Device has experienced errors but is still partially functional.
    Degraded,
    /// Device has failed and is not operational.
    Faulted,
    /// Device is being restarted by the supervisor.
    Recovering,
}

impl std::fmt::Display for DeviceHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceHealth::Healthy => write!(f, "healthy"),
            DeviceHealth::Degraded => write!(f, "degraded"),
            DeviceHealth::Faulted => write!(f, "faulted"),
            DeviceHealth::Recovering => write!(f, "recovering"),
        }
    }
}

/// Tracks the health state history and restart attempts for a device.
#[derive(Debug, Clone)]
pub struct DeviceHealthState {
    /// Current health status.
    pub health: DeviceHealth,
    /// Number of consecutive failures since last healthy state.
    pub consecutive_failures: u32,
    /// Total restart attempts since registration.
    pub restart_attempts: u32,
    /// When the current health state was entered.
    pub state_since: Instant,
    /// When the last failure occurred, if any.
    pub last_failure: Option<Instant>,
    /// Last error message, if any.
    pub last_error: Option<String>,
}

impl DeviceHealthState {
    /// Create a new healthy device state.
    pub fn new() -> Self {
        Self {
            health: DeviceHealth::Healthy,
            consecutive_failures: 0,
            restart_attempts: 0,
            state_since: Instant::now(),
            last_failure: None,
            last_error: None,
        }
    }

    /// Record a device failure, transitioning to Faulted if threshold exceeded.
    pub fn record_failure(&mut self, error: impl Into<String>, fault_threshold: u32) {
        self.consecutive_failures += 1;
        self.last_failure = Some(Instant::now());
        self.last_error = Some(error.into());

        if self.consecutive_failures >= fault_threshold && self.health != DeviceHealth::Faulted {
            self.health = DeviceHealth::Faulted;
            self.state_since = Instant::now();
        } else if self.health == DeviceHealth::Healthy {
            self.health = DeviceHealth::Degraded;
            self.state_since = Instant::now();
        }
    }

    /// Record a successful operation, transitioning back to Healthy.
    pub fn record_success(&mut self) {
        if self.health != DeviceHealth::Healthy {
            self.health = DeviceHealth::Healthy;
            self.state_since = Instant::now();
        }
        self.consecutive_failures = 0;
        self.last_error = None;
    }

    /// Mark the device as recovering (restart in progress).
    pub fn mark_recovering(&mut self) {
        self.health = DeviceHealth::Recovering;
        self.state_since = Instant::now();
        self.restart_attempts += 1;
    }

    /// Calculate the backoff delay for the next restart attempt.
    ///
    /// Uses exponential backoff: base_delay * 2^(attempts-1), capped at max_delay.
    pub fn backoff_delay(&self, base_delay: Duration, max_delay: Duration) -> Duration {
        if self.restart_attempts == 0 {
            return base_delay;
        }
        let exp = (self.restart_attempts - 1).min(10); // cap exponent to avoid overflow
        let multiplier = 1u32.checked_shl(exp).unwrap_or(1024);
        let delay = base_delay.saturating_mul(multiplier);
        delay.min(max_delay)
    }
}

impl Default for DeviceHealthState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_device_is_healthy() {
        let state = DeviceHealthState::new();
        assert_eq!(state.health, DeviceHealth::Healthy);
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.restart_attempts, 0);
    }

    #[test]
    fn test_single_failure_degrades() {
        let mut state = DeviceHealthState::new();
        state.record_failure("test error", 3);
        assert_eq!(state.health, DeviceHealth::Degraded);
        assert_eq!(state.consecutive_failures, 1);
        assert!(state.last_error.is_some());
    }

    #[test]
    fn test_threshold_failures_fault() {
        let mut state = DeviceHealthState::new();
        state.record_failure("error 1", 3);
        state.record_failure("error 2", 3);
        assert_eq!(state.health, DeviceHealth::Degraded);
        state.record_failure("error 3", 3);
        assert_eq!(state.health, DeviceHealth::Faulted);
        assert_eq!(state.consecutive_failures, 3);
    }

    #[test]
    fn test_success_resets_to_healthy() {
        let mut state = DeviceHealthState::new();
        state.record_failure("error", 3);
        state.record_failure("error", 3);
        assert_eq!(state.health, DeviceHealth::Degraded);

        state.record_success();
        assert_eq!(state.health, DeviceHealth::Healthy);
        assert_eq!(state.consecutive_failures, 0);
        assert!(state.last_error.is_none());
    }

    #[test]
    fn test_backoff_delay_exponential() {
        let mut state = DeviceHealthState::new();
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(60);

        // First attempt: base delay
        assert_eq!(state.backoff_delay(base, max), base);

        state.mark_recovering();
        // After 1 attempt: 1 * 2^0 = 1s
        assert_eq!(state.backoff_delay(base, max), Duration::from_secs(1));

        state.mark_recovering();
        // After 2 attempts: 1 * 2^1 = 2s
        assert_eq!(state.backoff_delay(base, max), Duration::from_secs(2));

        state.mark_recovering();
        // After 3 attempts: 1 * 2^2 = 4s
        assert_eq!(state.backoff_delay(base, max), Duration::from_secs(4));
    }

    #[test]
    fn test_backoff_capped_at_max() {
        let mut state = DeviceHealthState::new();
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(10);

        for _ in 0..20 {
            state.mark_recovering();
        }
        assert!(state.backoff_delay(base, max) <= max);
    }

    #[test]
    fn test_device_health_display() {
        assert_eq!(DeviceHealth::Healthy.to_string(), "healthy");
        assert_eq!(DeviceHealth::Degraded.to_string(), "degraded");
        assert_eq!(DeviceHealth::Faulted.to_string(), "faulted");
        assert_eq!(DeviceHealth::Recovering.to_string(), "recovering");
    }
}
