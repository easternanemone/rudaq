//! Operational modes for mock devices.
//!
//! Mock devices can operate in different modes optimized for specific testing scenarios:
//!
//! - **Instant**: Zero delays, deterministic behavior for unit tests
//! - **Realistic**: Hardware-like timing for integration tests
//! - **Chaos**: Configurable failures for resilience testing

/// Operational modes for mock devices
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockMode {
    /// Zero delays, deterministic - for unit tests
    Instant,
    /// Hardware-like timing - for integration tests
    Realistic,
    /// Configurable failures - for resilience testing
    Chaos,
}

impl Default for MockMode {
    fn default() -> Self {
        MockMode::Instant
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_mode() {
        assert_eq!(MockMode::default(), MockMode::Instant);
    }

    #[test]
    fn test_mode_equality() {
        assert_eq!(MockMode::Instant, MockMode::Instant);
        assert_ne!(MockMode::Instant, MockMode::Realistic);
        assert_ne!(MockMode::Realistic, MockMode::Chaos);
    }

    #[test]
    fn test_mode_clone() {
        let mode = MockMode::Realistic;
        let cloned = mode.clone();
        assert_eq!(mode, cloned);
    }
}
