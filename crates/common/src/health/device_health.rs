//! Per-device health tracking for the device registry (bd-qa36.4.2).
//!
//! Provides health state tracking for individual hardware devices, enabling
//! the device supervisor to detect faulted devices and attempt restart with
//! exponential backoff.
//!
//! ## State Machine
//!
//! Valid transitions:
//! - `Healthy -> Degraded` (error threshold reached)
//! - `Healthy -> Faulted` (critical/immediate fault)
//! - `Degraded -> Healthy` (consecutive successes)
//! - `Degraded -> Faulted` (fault threshold exceeded)
//! - `Faulted -> Recovering` (recovery initiated)
//! - `Recovering -> Healthy` (recovery + verification succeeded)
//! - `Recovering -> Faulted` (recovery failed)
//! - `Any -> Faulted` (critical fault -- hardware disconnect, kernel error)

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Maximum number of transition records to retain in history.
const MAX_TRANSITION_HISTORY: usize = 20;

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

/// Record of a single state transition for audit/debugging.
#[derive(Debug, Clone)]
pub struct TransitionRecord {
    /// When the transition occurred.
    pub timestamp: Instant,
    /// State before the transition.
    pub from: DeviceHealth,
    /// State after the transition.
    pub to: DeviceHealth,
    /// Human-readable reason for the transition.
    pub reason: String,
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
    /// Number of consecutive failures required before transitioning from
    /// `Healthy` to `Degraded`. Default is 1 (preserving legacy behavior).
    pub degradation_threshold: u32,
    /// Bounded history of state transitions (most recent last).
    transition_history: VecDeque<TransitionRecord>,
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
            degradation_threshold: 1,
            transition_history: VecDeque::new(),
        }
    }

    /// Create a new healthy device state with a custom degradation threshold.
    ///
    /// The degradation threshold controls how many consecutive failures are
    /// required before transitioning from `Healthy` to `Degraded`.
    pub fn with_degradation_threshold(mut self, threshold: u32) -> Self {
        self.degradation_threshold = threshold;
        self
    }

    /// Validate and execute a state transition.
    ///
    /// Returns the new state. Invalid transitions are logged as warnings but
    /// still allowed for backward compatibility.
    fn try_transition(&mut self, to: DeviceHealth, reason: &str) -> DeviceHealth {
        let from = self.health;

        // No-op if already in the target state.
        if from == to {
            return to;
        }

        let valid = matches!(
            (from, to),
            (DeviceHealth::Healthy, DeviceHealth::Degraded)
                | (DeviceHealth::Healthy, DeviceHealth::Faulted)
                | (DeviceHealth::Degraded, DeviceHealth::Healthy)
                | (DeviceHealth::Degraded, DeviceHealth::Faulted)
                | (DeviceHealth::Faulted, DeviceHealth::Recovering)
                | (DeviceHealth::Recovering, DeviceHealth::Healthy)
                | (DeviceHealth::Recovering, DeviceHealth::Faulted)
                // Any -> Faulted is always valid (critical fault)
                | (_, DeviceHealth::Faulted)
        );

        if !valid {
            tracing::warn!(
                from = %from,
                to = %to,
                reason = reason,
                "invalid health state transition (allowing for backward compatibility)"
            );
        }

        self.health = to;
        self.state_since = Instant::now();

        // Record the transition in history.
        self.transition_history.push_back(TransitionRecord {
            timestamp: Instant::now(),
            from,
            to,
            reason: reason.to_string(),
        });
        // Keep history bounded.
        while self.transition_history.len() > MAX_TRANSITION_HISTORY {
            self.transition_history.pop_front();
        }

        to
    }

    /// Record a device failure, transitioning to Faulted if threshold exceeded.
    pub fn record_failure(&mut self, error: impl Into<String>, fault_threshold: u32) {
        self.consecutive_failures += 1;
        self.last_failure = Some(Instant::now());
        let error_msg = error.into();
        self.last_error = Some(error_msg.clone());

        if self.consecutive_failures >= fault_threshold && self.health != DeviceHealth::Faulted {
            self.try_transition(
                DeviceHealth::Faulted,
                &format!("fault threshold ({fault_threshold}) exceeded: {error_msg}"),
            );
        } else if self.health == DeviceHealth::Healthy
            && self.consecutive_failures >= self.degradation_threshold
        {
            self.try_transition(
                DeviceHealth::Degraded,
                &format!(
                    "degradation threshold ({}) reached: {error_msg}",
                    self.degradation_threshold
                ),
            );
        }
    }

    /// Record a successful operation, transitioning back to Healthy.
    pub fn record_success(&mut self) {
        if self.health != DeviceHealth::Healthy {
            self.try_transition(DeviceHealth::Healthy, "consecutive success");
        }
        self.consecutive_failures = 0;
        self.last_error = None;
    }

    /// Mark the device as recovering (restart in progress).
    pub fn mark_recovering(&mut self) {
        self.try_transition(DeviceHealth::Recovering, "restart initiated");
        self.restart_attempts += 1;
    }

    /// Immediately transition to `Faulted` from any state.
    ///
    /// Use for hardware disconnects, kernel errors, and other critical faults
    /// that require immediate state change regardless of current state.
    pub fn record_critical_fault(&mut self, reason: &str) {
        self.last_failure = Some(Instant::now());
        self.last_error = Some(reason.to_string());
        self.try_transition(DeviceHealth::Faulted, &format!("critical fault: {reason}"));
    }

    /// Transition from `Recovering` to `Healthy` after successful recovery
    /// and capability verification.
    pub fn recovery_complete(&mut self) {
        self.try_transition(
            DeviceHealth::Healthy,
            "recovery complete: verification succeeded",
        );
        self.consecutive_failures = 0;
        self.last_error = None;
    }

    /// Transition from `Recovering` to `Faulted` when a recovery attempt fails.
    pub fn recovery_failed(&mut self, reason: &str) {
        self.last_failure = Some(Instant::now());
        self.last_error = Some(reason.to_string());
        self.try_transition(DeviceHealth::Faulted, &format!("recovery failed: {reason}"));
    }

    /// Read-only access to the transition history (most recent last).
    pub fn transition_history(&self) -> &VecDeque<TransitionRecord> {
        &self.transition_history
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

    // --- New tests for state machine formalization (bd-vgrj Phase 1) ---

    #[test]
    fn test_valid_transition_full_cycle() {
        // Healthy -> Degraded -> Faulted -> Recovering -> Healthy
        let mut state = DeviceHealthState::new();
        assert_eq!(state.health, DeviceHealth::Healthy);

        // Healthy -> Degraded
        state.record_failure("error", 3);
        assert_eq!(state.health, DeviceHealth::Degraded);

        // Degraded -> Faulted (via threshold)
        state.record_failure("error", 3);
        state.record_failure("error", 3);
        assert_eq!(state.health, DeviceHealth::Faulted);

        // Faulted -> Recovering
        state.mark_recovering();
        assert_eq!(state.health, DeviceHealth::Recovering);

        // Recovering -> Healthy
        state.recovery_complete();
        assert_eq!(state.health, DeviceHealth::Healthy);
        assert_eq!(state.consecutive_failures, 0);
        assert!(state.last_error.is_none());
    }

    #[test]
    fn test_invalid_transition_still_works_backward_compat() {
        // Healthy -> Recovering is not a valid transition, but should work
        // for backward compatibility (with a warning log).
        let mut state = DeviceHealthState::new();
        assert_eq!(state.health, DeviceHealth::Healthy);

        state.mark_recovering();
        // Should still transition despite being invalid
        assert_eq!(state.health, DeviceHealth::Recovering);
    }

    #[test]
    fn test_record_critical_fault_from_healthy() {
        let mut state = DeviceHealthState::new();
        state.record_critical_fault("hardware disconnect");
        assert_eq!(state.health, DeviceHealth::Faulted);
        assert_eq!(state.last_error.as_deref(), Some("hardware disconnect"));
        assert!(state.last_failure.is_some());
    }

    #[test]
    fn test_record_critical_fault_from_degraded() {
        let mut state = DeviceHealthState::new();
        state.record_failure("minor error", 5);
        assert_eq!(state.health, DeviceHealth::Degraded);

        state.record_critical_fault("kernel error");
        assert_eq!(state.health, DeviceHealth::Faulted);
    }

    #[test]
    fn test_record_critical_fault_from_recovering() {
        let mut state = DeviceHealthState::new();
        state.record_critical_fault("initial fault");
        state.mark_recovering();
        assert_eq!(state.health, DeviceHealth::Recovering);

        state.record_critical_fault("hardware vanished during recovery");
        assert_eq!(state.health, DeviceHealth::Faulted);
    }

    #[test]
    fn test_record_critical_fault_from_faulted_is_noop() {
        let mut state = DeviceHealthState::new();
        state.record_critical_fault("first fault");
        assert_eq!(state.health, DeviceHealth::Faulted);
        let history_len = state.transition_history().len();

        // Already faulted: should be a no-op (same state)
        state.record_critical_fault("second fault");
        assert_eq!(state.health, DeviceHealth::Faulted);
        // No new transition record since state didn't change
        assert_eq!(state.transition_history().len(), history_len);
    }

    #[test]
    fn test_recovery_complete() {
        let mut state = DeviceHealthState::new();
        state.record_critical_fault("fault");
        state.mark_recovering();
        assert_eq!(state.health, DeviceHealth::Recovering);

        state.recovery_complete();
        assert_eq!(state.health, DeviceHealth::Healthy);
        assert_eq!(state.consecutive_failures, 0);
        assert!(state.last_error.is_none());
    }

    #[test]
    fn test_recovery_failed() {
        let mut state = DeviceHealthState::new();
        state.record_critical_fault("fault");
        state.mark_recovering();
        assert_eq!(state.health, DeviceHealth::Recovering);

        state.recovery_failed("device did not respond");
        assert_eq!(state.health, DeviceHealth::Faulted);
        assert_eq!(state.last_error.as_deref(), Some("device did not respond"));
    }

    #[test]
    fn test_transition_history_recorded() {
        let mut state = DeviceHealthState::new();
        assert!(state.transition_history().is_empty());

        state.record_failure("err", 3); // Healthy -> Degraded
        assert_eq!(state.transition_history().len(), 1);

        let record = &state.transition_history()[0];
        assert_eq!(record.from, DeviceHealth::Healthy);
        assert_eq!(record.to, DeviceHealth::Degraded);
        assert!(record.reason.contains("err"));
    }

    #[test]
    fn test_transition_history_bounded_at_20() {
        let mut state = DeviceHealthState::new();

        // Generate more than 20 transitions by cycling through states
        for i in 0..25 {
            state.record_critical_fault(&format!("fault {i}"));
            state.mark_recovering();
            state.recovery_complete();
        }

        assert!(
            state.transition_history().len() <= MAX_TRANSITION_HISTORY,
            "history should be bounded at {MAX_TRANSITION_HISTORY}, got {}",
            state.transition_history().len()
        );
        assert_eq!(state.transition_history().len(), MAX_TRANSITION_HISTORY);
    }

    #[test]
    fn test_degradation_threshold() {
        let mut state = DeviceHealthState::new().with_degradation_threshold(3);
        assert_eq!(state.degradation_threshold, 3);

        // First two failures: should stay Healthy
        state.record_failure("err 1", 10);
        assert_eq!(state.health, DeviceHealth::Healthy);
        assert_eq!(state.consecutive_failures, 1);

        state.record_failure("err 2", 10);
        assert_eq!(state.health, DeviceHealth::Healthy);
        assert_eq!(state.consecutive_failures, 2);

        // Third failure: should transition to Degraded
        state.record_failure("err 3", 10);
        assert_eq!(state.health, DeviceHealth::Degraded);
        assert_eq!(state.consecutive_failures, 3);
    }

    #[test]
    fn test_degradation_threshold_default_preserves_behavior() {
        // Default threshold of 1 means first failure degrades (legacy behavior)
        let mut state = DeviceHealthState::new();
        assert_eq!(state.degradation_threshold, 1);

        state.record_failure("err", 5);
        assert_eq!(state.health, DeviceHealth::Degraded);
    }
}
