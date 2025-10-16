//! Type-safe configuration value objects for instruments.
//!
//! This module provides strongly-typed configuration structs that replace
//! manual TOML parsing in instrument `connect()` methods. Benefits include:
//!
//! - Compile-time type safety
//! - Centralized validation logic
//! - Self-documenting configuration requirements
//! - Better error messages

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Configuration for the mock instrument.
///
/// # Examples
///
/// ```toml
/// [instruments.mock]
/// type = "mock"
/// sample_rate_hz = 1000.0
/// num_samples = 10000
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MockInstrumentConfig {
    /// Sample rate in Hz (must be positive)
    pub sample_rate_hz: f64,
    /// Number of samples to generate (must be > 0)
    pub num_samples: usize,
}

impl MockInstrumentConfig {
    /// Creates a configuration from a TOML value.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - TOML structure doesn't match expected fields
    /// - Field types are incorrect
    pub fn from_toml(config: &toml::Value) -> Result<Self> {
        toml::from_str(&toml::to_string(config)?)
            .context("Failed to parse mock instrument configuration")
    }

    /// Validates the configuration parameters.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - `sample_rate_hz` is not positive
    /// - `num_samples` is zero
    pub fn validate(&self) -> Result<()> {
        if self.sample_rate_hz <= 0.0 {
            anyhow::bail!("sample_rate_hz must be positive, got {}", self.sample_rate_hz);
        }
        if self.num_samples == 0 {
            anyhow::bail!("num_samples must be greater than 0");
        }
        Ok(())
    }

    /// Creates a validated configuration from TOML.
    ///
    /// Combines `from_toml()` and `validate()` in one call.
    pub fn from_toml_validated(config: &toml::Value) -> Result<Self> {
        let config = Self::from_toml(config)?;
        config.validate()?;
        Ok(config)
    }
}

impl Default for MockInstrumentConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 1000.0,
            num_samples: 10000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_config_default_is_valid() {
        let config = MockInstrumentConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_mock_config_validation_rejects_zero_sample_rate() {
        let config = MockInstrumentConfig {
            sample_rate_hz: 0.0,
            num_samples: 100,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_mock_config_validation_rejects_zero_samples() {
        let config = MockInstrumentConfig {
            sample_rate_hz: 1000.0,
            num_samples: 0,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_mock_config_from_toml() {
        let toml_str = r#"
            sample_rate_hz = 500.0
            num_samples = 5000
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let config = MockInstrumentConfig::from_toml(&value).unwrap();

        assert_eq!(config.sample_rate_hz, 500.0);
        assert_eq!(config.num_samples, 5000);
    }
}
