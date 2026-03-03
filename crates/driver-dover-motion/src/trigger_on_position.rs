//! Trigger-on-Position (TOP) configuration types

use serde::{Deserialize, Serialize};

/// Configuration for Trigger-on-Position mode.
///
/// Defines the parameters for generating hardware triggers at fixed position intervals.
/// Used in LIBS experiments and other synchronized scanning applications.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TriggerOnPositionConfig {
    /// Position where triggering starts (in axis units, e.g., meters)
    pub start_position: f64,

    /// Position where triggering ends
    pub end_position: f64,

    /// Distance between trigger pulses (e.g., 0.0001 for 100 µm)
    pub increment: f64,

    /// If true, triggers fire in both forward and reverse motion
    pub bidirectional: bool,

    /// Trigger pulse duration in nanoseconds (50-204,800, in 50ns increments)
    pub pulse_width_ns: u32,
}

impl TriggerOnPositionConfig {
    /// Create a new TOP configuration.
    ///
    /// # Arguments
    ///
    /// * `start_position` - Where to begin generating triggers (axis units)
    /// * `end_position` - Where to stop generating triggers
    /// * `increment` - Position interval between triggers
    /// * `bidirectional` - Generate triggers in both directions
    /// * `pulse_width_ns` - GPIO pulse duration (50ns - 204,800ns, 50ns steps)
    pub fn new(
        start_position: f64,
        end_position: f64,
        increment: f64,
        bidirectional: bool,
        pulse_width_ns: u32,
    ) -> Self {
        Self {
            start_position,
            end_position,
            increment,
            bidirectional,
            pulse_width_ns,
        }
    }

    /// Validate the configuration.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Increment is not positive
    /// - Pulse width is outside valid range (50-204,800 ns)
    /// - Pulse width is not a multiple of 50ns
    pub fn validate(&self) -> Result<(), String> {
        if self.increment <= 0.0 {
            return Err(format!(
                "Trigger increment must be positive, got {}",
                self.increment
            ));
        }

        if self.pulse_width_ns < 50 || self.pulse_width_ns > 204_800 {
            return Err(format!(
                "Pulse width must be 50-204,800 ns, got {}",
                self.pulse_width_ns
            ));
        }

        if self.pulse_width_ns % 50 != 0 {
            return Err(format!(
                "Pulse width must be a multiple of 50ns, got {}",
                self.pulse_width_ns
            ));
        }

        Ok(())
    }

    /// Calculate the number of triggers that will be generated.
    pub fn num_triggers(&self) -> usize {
        let distance = (self.end_position - self.start_position).abs();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // SAFETY: trigger count is non-negative and bounded by physical travel range
        {
            (distance / self.increment).floor() as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        // Valid config
        let config = TriggerOnPositionConfig::new(0.0, 10.0, 0.1, true, 1000);
        assert!(config.validate().is_ok());

        // Invalid increment
        let config = TriggerOnPositionConfig::new(0.0, 10.0, -0.1, true, 1000);
        assert!(config.validate().is_err());

        // Invalid pulse width (too small)
        let config = TriggerOnPositionConfig::new(0.0, 10.0, 0.1, true, 25);
        assert!(config.validate().is_err());

        // Invalid pulse width (not multiple of 50)
        let config = TriggerOnPositionConfig::new(0.0, 10.0, 0.1, true, 175);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_num_triggers() {
        let config = TriggerOnPositionConfig::new(0.0, 10.0, 0.1, true, 1000);
        assert_eq!(config.num_triggers(), 100);

        let config = TriggerOnPositionConfig::new(0.0, 1.0, 0.01, true, 1000);
        assert_eq!(config.num_triggers(), 100);
    }
}
