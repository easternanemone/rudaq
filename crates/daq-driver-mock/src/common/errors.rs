//! Error injection framework for mock devices.
//!
//! Enables configurable failures and error scenarios for resilience testing.
//! Integrates with daq-core's DriverError infrastructure.

use super::rng::MockRng;
use daq_core::error::{DriverError, DriverErrorKind};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Error injection configuration for mock devices
#[derive(Clone, Debug)]
pub struct ErrorConfig {
    /// Per-operation failure rate (0.0 to 1.0)
    failure_rates: Arc<HashMap<&'static str, f64>>,
    /// Specific failure scenarios
    scenarios: Arc<Vec<ErrorScenario>>,
    /// RNG for failure decisions
    rng: Arc<MockRng>,
    /// State tracking for scenarios
    state: Arc<Mutex<ErrorState>>,
}

#[derive(Debug, Clone)]
pub enum ErrorScenario {
    /// Fail after N successful operations
    FailAfterN {
        operation: &'static str,
        count: u32,
    },
    /// Timeout on specific operation
    Timeout {
        operation: &'static str,
    },
    /// Simulate communication loss
    CommunicationLoss,
    /// Hardware fault with specific code
    HardwareFault {
        code: u32,
    },
}

#[derive(Default, Debug)]
struct ErrorState {
    /// Operation counters for FailAfterN scenarios
    operation_counts: HashMap<&'static str, u32>,
    /// Whether communication is lost
    communication_lost: bool,
    /// Hardware fault code (0 = no fault)
    hardware_fault_code: u32,
}

impl ErrorConfig {
    /// Create error config with no errors (default)
    pub fn none() -> Self {
        Self {
            failure_rates: Arc::new(HashMap::new()),
            scenarios: Arc::new(Vec::new()),
            rng: Arc::new(MockRng::new(None)),
            state: Arc::new(Mutex::new(ErrorState::default())),
        }
    }

    /// Create error config with uniform random failures
    pub fn random_failures(rate: f64) -> Self {
        Self::random_failures_seeded(rate, None)
    }

    /// Create error config with uniform random failures and specific seed
    pub fn random_failures_seeded(rate: f64, seed: Option<u64>) -> Self {
        let mut rates = HashMap::new();
        rates.insert("*", rate); // Wildcard for all operations
        Self {
            failure_rates: Arc::new(rates),
            scenarios: Arc::new(Vec::new()),
            rng: Arc::new(MockRng::new(seed)),
            state: Arc::new(Mutex::new(ErrorState::default())),
        }
    }

    /// Create error config with a single scenario
    pub fn scenario(scenario: ErrorScenario) -> Self {
        Self::scenarios(vec![scenario])
    }

    /// Create error config with multiple scenarios
    pub fn scenarios(scenarios: Vec<ErrorScenario>) -> Self {
        Self {
            failure_rates: Arc::new(HashMap::new()),
            scenarios: Arc::new(scenarios),
            rng: Arc::new(MockRng::new(None)),
            state: Arc::new(Mutex::new(ErrorState::default())),
        }
    }

    /// Create error config with custom failure rates per operation
    pub fn with_rates(rates: HashMap<&'static str, f64>) -> Self {
        Self {
            failure_rates: Arc::new(rates),
            scenarios: Arc::new(Vec::new()),
            rng: Arc::new(MockRng::new(None)),
            state: Arc::new(Mutex::new(ErrorState::default())),
        }
    }

    /// Check if an operation should fail and return appropriate error
    pub fn check_operation(
        &self,
        driver_type: &str,
        operation: &'static str,
    ) -> Result<(), DriverError> {
        let mut state = self.state.lock().unwrap();

        // Check communication loss scenario
        if state.communication_lost {
            return Err(DriverError::new(
                driver_type,
                DriverErrorKind::Communication,
                "Communication lost",
            ));
        }

        // Check hardware fault scenario
        if state.hardware_fault_code != 0 {
            return Err(DriverError::new(
                driver_type,
                DriverErrorKind::Hardware,
                format!("Hardware fault: {}", state.hardware_fault_code),
            ));
        }

        // Check scenarios
        for scenario in self.scenarios.iter() {
            match scenario {
                ErrorScenario::FailAfterN {
                    operation: op,
                    count,
                } if *op == operation => {
                    let current = state.operation_counts.entry(operation).or_insert(0);
                    *current += 1;
                    if *current > *count {
                        return Err(DriverError::new(
                            driver_type,
                            DriverErrorKind::Hardware,
                            format!("Injected failure after {} operations", count),
                        ));
                    }
                }
                ErrorScenario::Timeout {
                    operation: op,
                } if *op == operation => {
                    return Err(DriverError::new(
                        driver_type,
                        DriverErrorKind::Timeout,
                        format!("Operation '{}' timed out", operation),
                    ));
                }
                ErrorScenario::CommunicationLoss => {
                    // Only trigger on first occurrence
                    if !state.communication_lost {
                        state.communication_lost = true;
                        return Err(DriverError::new(
                            driver_type,
                            DriverErrorKind::Communication,
                            "Communication lost",
                        ));
                    }
                }
                ErrorScenario::HardwareFault { code } => {
                    // Only trigger on first occurrence
                    if state.hardware_fault_code == 0 {
                        state.hardware_fault_code = *code;
                        return Err(DriverError::new(
                            driver_type,
                            DriverErrorKind::Hardware,
                            format!("Hardware fault: {}", code),
                        ));
                    }
                }
                _ => {}
            }
        }

        // Check failure rates
        let rate = self
            .failure_rates
            .get(operation)
            .or_else(|| self.failure_rates.get("*"))
            .copied()
            .unwrap_or(0.0);

        if self.rng.should_fail(rate) {
            return Err(DriverError::new(
                driver_type,
                DriverErrorKind::Hardware,
                format!("Random failure on operation '{}'", operation),
            ));
        }

        // Increment operation counter for FailAfterN tracking
        if self.scenarios.iter().any(|s| matches!(s, ErrorScenario::FailAfterN { operation: op, .. } if *op == operation)) {
            state.operation_counts.entry(operation).or_insert(0);
        }

        Ok(())
    }

    /// Reset error state (clear counters, faults)
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        *state = ErrorState::default();
    }
}

impl Default for ErrorConfig {
    fn default() -> Self {
        Self::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_errors() {
        let config = ErrorConfig::none();
        for _ in 0..100 {
            assert!(config.check_operation("test_driver", "read").is_ok());
        }
    }

    #[test]
    fn test_random_failures() {
        let config = ErrorConfig::random_failures_seeded(0.5, Some(42));
        let mut failures = 0;
        for _ in 0..1000 {
            if config.check_operation("test_driver", "read").is_err() {
                failures += 1;
            }
        }
        // Expect roughly 50% failures
        assert!(failures > 400 && failures < 600, "Got {} failures", failures);
    }

    #[test]
    fn test_fail_after_n() {
        let config = ErrorConfig::scenario(ErrorScenario::FailAfterN {
            operation: "read",
            count: 5,
        });

        // First 5 should succeed
        for i in 0..5 {
            assert!(
                config.check_operation("test_driver", "read").is_ok(),
                "Operation {} should succeed",
                i + 1
            );
        }

        // 6th and beyond should fail
        for i in 5..10 {
            assert!(
                config.check_operation("test_driver", "read").is_err(),
                "Operation {} should fail",
                i + 1
            );
        }
    }

    #[test]
    fn test_timeout_scenario() {
        let config = ErrorConfig::scenario(ErrorScenario::Timeout {
            operation: "move",
        });

        let result = config.check_operation("test_driver", "move");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind, DriverErrorKind::Timeout);
        assert!(err.message.contains("timed out"));
    }

    #[test]
    fn test_communication_loss() {
        let config = ErrorConfig::scenario(ErrorScenario::CommunicationLoss);

        // First call triggers communication loss
        let result = config.check_operation("test_driver", "read");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind, DriverErrorKind::Communication);

        // Subsequent calls should also fail
        let result2 = config.check_operation("test_driver", "write");
        assert!(result2.is_err());
    }

    #[test]
    fn test_hardware_fault() {
        let config = ErrorConfig::scenario(ErrorScenario::HardwareFault { code: 0x42 });

        let result = config.check_operation("test_driver", "read");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind, DriverErrorKind::Hardware);
        assert!(err.message.contains("66")); // 0x42 = 66
    }

    #[test]
    fn test_reset() {
        // Use FailAfterN which is stateful and respects reset
        let config = ErrorConfig::scenario(ErrorScenario::FailAfterN {
            operation: "read",
            count: 2,
        });

        // First two should succeed
        assert!(config.check_operation("test_driver", "read").is_ok());
        assert!(config.check_operation("test_driver", "read").is_ok());

        // Third should fail
        assert!(config.check_operation("test_driver", "read").is_err());

        // Reset
        config.reset();

        // Should work again (counter reset)
        assert!(config.check_operation("test_driver", "read").is_ok());
        assert!(config.check_operation("test_driver", "read").is_ok());

        // And fail again after 2
        assert!(config.check_operation("test_driver", "read").is_err());
    }

    #[test]
    fn test_multiple_scenarios() {
        let config = ErrorConfig::scenarios(vec![
            ErrorScenario::FailAfterN {
                operation: "read",
                count: 2,
            },
            ErrorScenario::Timeout {
                operation: "move",
            },
        ]);

        // Read should work twice
        assert!(config.check_operation("test_driver", "read").is_ok());
        assert!(config.check_operation("test_driver", "read").is_ok());
        // Then fail
        assert!(config.check_operation("test_driver", "read").is_err());

        // Move should always timeout
        assert!(config.check_operation("test_driver", "move").is_err());
    }

    #[test]
    fn test_custom_rates() {
        let mut rates = HashMap::new();
        rates.insert("read", 1.0); // Always fail
        rates.insert("write", 0.0); // Never fail

        let config = ErrorConfig::with_rates(rates);

        // Read should always fail
        for _ in 0..10 {
            assert!(config.check_operation("test_driver", "read").is_err());
        }

        // Write should never fail
        for _ in 0..10 {
            assert!(config.check_operation("test_driver", "write").is_ok());
        }
    }

    #[test]
    fn test_default_is_none() {
        let config = ErrorConfig::default();
        for _ in 0..100 {
            assert!(config.check_operation("test_driver", "any").is_ok());
        }
    }
}
