//! Mock power meter implementation matching Newport 1830-C behaviors.
//!
//! Provides realistic simulation of optical power meters with:
//! - Unit modes (W, mW, dBm, dB, REL)
//! - Wavelength-dependent response curve
//! - Noise model (shot + thermal components)
//! - Filter/integration time simulation
//! - Attenuator simulation (10/20/30 dB)
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_driver_mock::MockPowerMeter;
//!
//! // Simple usage (backward compatible)
//! let meter = MockPowerMeter::new(2.5);
//! let reading = meter.read().await?;
//!
//! // Advanced usage with builder
//! let meter = MockPowerMeter::builder()
//!     .base_power(1.0e-3)  // 1mW
//!     .unit(PowerUnit::Milliwatts)
//!     .wavelength_nm(800.0)
//!     .filter(FilterSetting::Medium)
//!     .attenuator(Attenuator::Db10)
//!     .build();
//! ```

use crate::common::{ErrorConfig, MockMode, MockRng};
use anyhow::Result;
use async_trait::async_trait;
use daq_core::capabilities::{Parameterized, Readable};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;

// =============================================================================
// MockPowerMeterFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MockPowerMeter driver
#[derive(Debug, Clone, Deserialize)]
pub struct MockPowerMeterConfig {
    /// Base power reading in Watts (default: 1.0)
    #[serde(default = "default_base_power")]
    pub base_power: f64,
}

fn default_base_power() -> f64 {
    1.0
}

impl Default for MockPowerMeterConfig {
    fn default() -> Self {
        Self { base_power: 1.0 }
    }
}

/// Factory for creating MockPowerMeter instances.
pub struct MockPowerMeterFactory;

/// Static capabilities for MockPowerMeter
static MOCK_POWER_METER_CAPABILITIES: &[Capability] =
    &[Capability::Readable, Capability::Parameterized];

impl DriverFactory for MockPowerMeterFactory {
    fn driver_type(&self) -> &'static str {
        "mock_power_meter"
    }

    fn name(&self) -> &'static str {
        "Mock Power Meter"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MOCK_POWER_METER_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let _: MockPowerMeterConfig = config.clone().try_into()?;
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MockPowerMeterConfig = config.try_into().unwrap_or_default();

            let meter = Arc::new(MockPowerMeter::new(cfg.base_power));

            Ok(DeviceComponents {
                readable: Some(meter.clone()),
                parameterized: Some(meter),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// Power Meter Enums
// =============================================================================

/// Power unit modes (matching Newport 1830-C)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PowerUnit {
    /// Watts (scientific notation)
    Watts,
    /// Milliwatts
    Milliwatts,
    /// dBm (power relative to 1mW)
    Dbm,
    /// dB (relative to reference)
    Db,
    /// REL mode (relative linear)
    Relative,
}

impl Default for PowerUnit {
    fn default() -> Self {
        PowerUnit::Watts
    }
}

/// Filter settings (integration time)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterSetting {
    /// No averaging
    None,
    /// Short integration (~10ms)
    Fast,
    /// Medium integration (~100ms)
    Medium,
    /// Long integration (~1000ms, most stable)
    Slow,
}

impl FilterSetting {
    /// Get integration time in milliseconds
    fn integration_time_ms(&self) -> u64 {
        match self {
            FilterSetting::None => 0,
            FilterSetting::Fast => 10,
            FilterSetting::Medium => 100,
            FilterSetting::Slow => 1000,
        }
    }
}

impl Default for FilterSetting {
    fn default() -> Self {
        FilterSetting::None
    }
}

/// Attenuator settings
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Attenuator {
    /// No attenuation (0 dB)
    None,
    /// 10 dB attenuation
    Db10,
    /// 20 dB attenuation
    Db20,
    /// 30 dB attenuation
    Db30,
}

impl Attenuator {
    /// Get attenuation factor (multiplicative)
    fn factor(&self) -> f64 {
        match self {
            Attenuator::None => 1.0,
            Attenuator::Db10 => 0.1,
            Attenuator::Db20 => 0.01,
            Attenuator::Db30 => 0.001,
        }
    }
}

impl Default for Attenuator {
    fn default() -> Self {
        Attenuator::None
    }
}

// =============================================================================
// Noise and Response Models
// =============================================================================

/// Noise model for power meter simulation
#[derive(Clone, Debug)]
pub struct NoiseModel {
    /// Shot noise coefficient (proportional to sqrt(power))
    pub shot_noise_coefficient: f64,
    /// Thermal noise floor (watts)
    pub thermal_noise_floor: f64,
}

impl NoiseModel {
    /// Create default noise model (1% shot noise, 1nW thermal floor)
    pub fn default_noise() -> Self {
        Self {
            shot_noise_coefficient: 0.01,
            thermal_noise_floor: 1e-9,
        }
    }

    /// Create noise-free model
    pub fn none() -> Self {
        Self {
            shot_noise_coefficient: 0.0,
            thermal_noise_floor: 0.0,
        }
    }

    /// Apply noise to base power reading
    fn apply_noise(&self, base_power: f64, rng: &MockRng) -> f64 {
        if base_power == 0.0 {
            return 0.0;
        }

        // Shot noise: proportional to sqrt(power)
        let shot_stddev = self.shot_noise_coefficient * base_power.abs().sqrt();
        let shot_noise = if shot_stddev > 0.0 {
            rng.gen_range(-shot_stddev..shot_stddev)
        } else {
            0.0
        };

        // Thermal noise: constant floor
        let thermal_stddev = self.thermal_noise_floor;
        let thermal_noise = if thermal_stddev > 0.0 {
            rng.gen_range(-thermal_stddev..thermal_stddev)
        } else {
            0.0
        };

        base_power + shot_noise + thermal_noise
    }
}

impl Default for NoiseModel {
    fn default() -> Self {
        Self::default_noise()
    }
}

/// Spectral response curve R(λ) for wavelength-dependent correction
#[derive(Clone, Debug)]
pub struct SpectralResponse {
    /// Wavelength points (nm)
    wavelength_nm: Vec<f64>,
    /// Responsivity at each wavelength (A/W)
    responsivity: Vec<f64>,
}

impl SpectralResponse {
    /// Create flat response (no wavelength dependence)
    pub fn flat() -> Self {
        Self {
            wavelength_nm: vec![300.0, 1100.0],
            responsivity: vec![1.0, 1.0],
        }
    }

    /// Create realistic silicon photodiode response
    pub fn silicon_photodiode() -> Self {
        // Typical Si photodiode response peaks around 900nm
        Self {
            wavelength_nm: vec![300.0, 400.0, 500.0, 600.0, 700.0, 800.0, 900.0, 1000.0, 1100.0],
            responsivity: vec![0.15, 0.25, 0.35, 0.45, 0.55, 0.65, 0.70, 0.65, 0.50],
        }
    }

    /// Get correction factor for given wavelength (linear interpolation)
    fn correction_factor(&self, wavelength_nm: f64) -> f64 {
        if self.wavelength_nm.is_empty() {
            return 1.0;
        }

        // Clamp to valid range
        let wl = wavelength_nm.clamp(
            self.wavelength_nm[0],
            self.wavelength_nm[self.wavelength_nm.len() - 1],
        );

        // Find interpolation points
        for i in 0..self.wavelength_nm.len() - 1 {
            if wl >= self.wavelength_nm[i] && wl <= self.wavelength_nm[i + 1] {
                // Linear interpolation
                let t = (wl - self.wavelength_nm[i])
                    / (self.wavelength_nm[i + 1] - self.wavelength_nm[i]);
                return self.responsivity[i] + t * (self.responsivity[i + 1] - self.responsivity[i]);
            }
        }

        // Fallback to last value
        self.responsivity[self.responsivity.len() - 1]
    }
}

impl Default for SpectralResponse {
    fn default() -> Self {
        Self::flat()
    }
}

// =============================================================================
// MockPowerMeter - Simulated Power Meter
// =============================================================================

/// Mock power meter with realistic simulation.
///
/// Simulates a power meter matching Newport 1830-C behaviors:
/// - Configurable base power value
/// - Unit conversion (W, mW, dBm, dB, REL)
/// - Wavelength-dependent response
/// - Noise model (shot + thermal)
/// - Filter/integration time
/// - Attenuator simulation
///
/// # Backward Compatibility
///
/// `MockPowerMeter::new(power)` provides simple behavior:
/// - Watts output
/// - Simple 1% noise
/// - No wavelength correction
/// - No attenuation
///
/// For advanced features, use the builder:
///
/// ```rust,ignore
/// let meter = MockPowerMeter::builder()
///     .base_power(1.0e-3)
///     .unit(PowerUnit::Milliwatts)
///     .wavelength_nm(800.0)
///     .filter(FilterSetting::Medium)
///     .build();
/// ```
pub struct MockPowerMeter {
    base_power: Parameter<f64>,
    params: ParameterSet,

    // Advanced features
    unit: PowerUnit,
    wavelength_nm: f64,
    noise_model: NoiseModel,
    filter: FilterSetting,
    attenuator: Attenuator,
    spectral_response: SpectralResponse,
    rng: Arc<MockRng>,
    mode: MockMode,
    error_config: ErrorConfig,
}

impl MockPowerMeter {
    /// Create new mock power meter with specified base power (Watts).
    ///
    /// Provides simple backward-compatible behavior:
    /// - Watts output
    /// - 1% noise
    /// - No wavelength correction
    ///
    /// # Arguments
    /// * `base_power` - Base power reading in Watts
    pub fn new(base_power: f64) -> Self {
        let mut params = ParameterSet::new();
        let power_param = Parameter::new("base_power", base_power)
            .with_description("Base power reading for simulated measurements")
            .with_unit("W")
            .with_range(0.0, 10.0); // 0 to 10W range

        params.register(power_param.clone());

        Self {
            base_power: power_param,
            params,
            unit: PowerUnit::Watts,
            wavelength_nm: 800.0,
            noise_model: NoiseModel::default_noise(),
            filter: FilterSetting::None,
            attenuator: Attenuator::None,
            spectral_response: SpectralResponse::flat(),
            rng: Arc::new(MockRng::new(None)),
            mode: MockMode::default(),
            error_config: ErrorConfig::default(),
        }
    }

    /// Create builder for advanced configuration
    pub fn builder() -> MockPowerMeterBuilder {
        MockPowerMeterBuilder::default()
    }

    /// Set the base power reading.
    pub async fn set_base_power(&self, power: f64) -> Result<()> {
        self.base_power.set(power).await
    }

    /// Get the current base power setting.
    pub fn get_base_power(&self) -> f64 {
        self.base_power.get()
    }

    /// Set unit mode
    pub fn set_unit(&mut self, unit: PowerUnit) {
        self.unit = unit;
    }

    /// Get current unit mode
    pub fn get_unit(&self) -> PowerUnit {
        self.unit
    }

    /// Set wavelength calibration
    pub fn set_wavelength(&mut self, wavelength_nm: f64) {
        self.wavelength_nm = wavelength_nm.clamp(300.0, 1100.0);
    }

    /// Get current wavelength
    pub fn get_wavelength(&self) -> f64 {
        self.wavelength_nm
    }

    /// Set filter setting
    pub fn set_filter(&mut self, filter: FilterSetting) {
        self.filter = filter;
    }

    /// Get current filter
    pub fn get_filter(&self) -> FilterSetting {
        self.filter
    }

    /// Set attenuator
    pub fn set_attenuator(&mut self, attenuator: Attenuator) {
        self.attenuator = attenuator;
    }

    /// Get current attenuator
    pub fn get_attenuator(&self) -> Attenuator {
        self.attenuator
    }

    /// Convert watts to selected unit
    fn convert_to_unit(&self, watts: f64) -> f64 {
        match self.unit {
            PowerUnit::Watts => watts,
            PowerUnit::Milliwatts => watts * 1000.0,
            PowerUnit::Dbm => {
                if watts <= 0.0 {
                    -120.0 // Floor for zero/negative power
                } else {
                    10.0 * (watts / 1e-3).log10() // dBm = 10 * log10(P / 1mW)
                }
            }
            PowerUnit::Db => {
                // Relative to 1W reference
                if watts <= 0.0 {
                    -120.0
                } else {
                    10.0 * watts.log10()
                }
            }
            PowerUnit::Relative => {
                // Normalized to base power
                let base = self.base_power.get();
                if base == 0.0 {
                    0.0
                } else {
                    watts / base
                }
            }
        }
    }
}

impl Parameterized for MockPowerMeter {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

impl Default for MockPowerMeter {
    fn default() -> Self {
        Self::new(1.0)
    }
}

#[async_trait]
impl Readable for MockPowerMeter {
    async fn read(&self) -> Result<f64> {
        // Check for injected errors
        self.error_config
            .check_operation("mock_power_meter", "read")?;

        // Simulate integration time delay
        if self.filter.integration_time_ms() > 0 && self.mode == MockMode::Realistic {
            tokio::time::sleep(tokio::time::Duration::from_millis(
                self.filter.integration_time_ms(),
            ))
            .await;
        }

        // Get base power
        let base = self.base_power.get();

        // Apply wavelength-dependent correction
        let correction = self.spectral_response.correction_factor(self.wavelength_nm);
        let corrected = base * correction;

        // Apply noise
        let noisy = self.noise_model.apply_noise(corrected, &self.rng);

        // Apply attenuation
        let attenuated = noisy * self.attenuator.factor();

        // Convert to selected unit
        let reading = self.convert_to_unit(attenuated);

        Ok(reading)
    }
}

// =============================================================================
// Builder Pattern
// =============================================================================

/// Builder for MockPowerMeter with advanced configuration
#[derive(Debug, Clone)]
pub struct MockPowerMeterBuilder {
    base_power: f64,
    unit: PowerUnit,
    wavelength_nm: f64,
    noise_model: NoiseModel,
    filter: FilterSetting,
    attenuator: Attenuator,
    spectral_response: SpectralResponse,
    mode: MockMode,
    error_config: ErrorConfig,
    rng_seed: Option<u64>,
}

impl Default for MockPowerMeterBuilder {
    fn default() -> Self {
        Self {
            base_power: 1.0,
            unit: PowerUnit::Watts,
            wavelength_nm: 800.0,
            noise_model: NoiseModel::default_noise(),
            filter: FilterSetting::None,
            attenuator: Attenuator::None,
            spectral_response: SpectralResponse::flat(),
            mode: MockMode::default(),
            error_config: ErrorConfig::default(),
            rng_seed: None,
        }
    }
}

impl MockPowerMeterBuilder {
    /// Set base power (Watts)
    pub fn base_power(mut self, power: f64) -> Self {
        self.base_power = power;
        self
    }

    /// Set unit mode
    pub fn unit(mut self, unit: PowerUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Set wavelength calibration (nm)
    pub fn wavelength_nm(mut self, wavelength: f64) -> Self {
        self.wavelength_nm = wavelength.clamp(300.0, 1100.0);
        self
    }

    /// Set noise model
    pub fn noise_model(mut self, model: NoiseModel) -> Self {
        self.noise_model = model;
        self
    }

    /// Set filter/integration time
    pub fn filter(mut self, filter: FilterSetting) -> Self {
        self.filter = filter;
        self
    }

    /// Set attenuator
    pub fn attenuator(mut self, attenuator: Attenuator) -> Self {
        self.attenuator = attenuator;
        self
    }

    /// Set spectral response curve
    pub fn spectral_response(mut self, response: SpectralResponse) -> Self {
        self.spectral_response = response;
        self
    }

    /// Set operational mode
    pub fn mode(mut self, mode: MockMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set error injection configuration
    pub fn error_config(mut self, config: ErrorConfig) -> Self {
        self.error_config = config;
        self
    }

    /// Set RNG seed for deterministic behavior
    pub fn rng_seed(mut self, seed: u64) -> Self {
        self.rng_seed = Some(seed);
        self
    }

    /// Build the MockPowerMeter
    pub fn build(self) -> MockPowerMeter {
        let mut params = ParameterSet::new();
        let power_param = Parameter::new("base_power", self.base_power)
            .with_description("Base power reading for simulated measurements")
            .with_unit("W")
            .with_range(0.0, 10.0);

        params.register(power_param.clone());

        MockPowerMeter {
            base_power: power_param,
            params,
            unit: self.unit,
            wavelength_nm: self.wavelength_nm,
            noise_model: self.noise_model,
            filter: self.filter,
            attenuator: self.attenuator,
            spectral_response: self.spectral_response,
            rng: Arc::new(MockRng::new(self.rng_seed)),
            mode: self.mode,
            error_config: self.error_config,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_backward_compatibility() {
        // New simple constructor should work exactly as before
        let meter = MockPowerMeter::new(2.5);

        let reading = meter.read().await.unwrap();
        assert!(
            reading > 2.4 && reading < 2.6,
            "Reading {} not in expected range",
            reading
        );
    }

    #[tokio::test]
    async fn test_unit_conversion_watts_to_milliwatts() {
        let meter = MockPowerMeter::builder()
            .base_power(1.0)
            .unit(PowerUnit::Milliwatts)
            .noise_model(NoiseModel::none())
            .build();

        let reading = meter.read().await.unwrap();
        assert!(
            (reading - 1000.0).abs() < 1.0,
            "Expected ~1000 mW, got {}",
            reading
        );
    }

    #[tokio::test]
    async fn test_unit_conversion_watts_to_dbm() {
        let meter = MockPowerMeter::builder()
            .base_power(1e-3) // 1 mW
            .unit(PowerUnit::Dbm)
            .noise_model(NoiseModel::none())
            .build();

        let reading = meter.read().await.unwrap();
        assert!(
            (reading - 0.0).abs() < 0.1,
            "Expected ~0 dBm (1mW), got {}",
            reading
        );
    }

    #[tokio::test]
    async fn test_wavelength_correction_flat_response() {
        let meter = MockPowerMeter::builder()
            .base_power(1.0)
            .wavelength_nm(800.0)
            .spectral_response(SpectralResponse::flat())
            .noise_model(NoiseModel::none())
            .build();

        let reading = meter.read().await.unwrap();
        assert!(
            (reading - 1.0).abs() < 0.01,
            "Flat response should not change power"
        );
    }

    #[tokio::test]
    async fn test_wavelength_correction_silicon() {
        let meter = MockPowerMeter::builder()
            .base_power(1.0)
            .wavelength_nm(900.0) // Peak of silicon response (~0.70)
            .spectral_response(SpectralResponse::silicon_photodiode())
            .noise_model(NoiseModel::none())
            .build();

        let reading = meter.read().await.unwrap();
        assert!(
            reading > 0.65 && reading < 0.75,
            "Expected ~0.70 with Si response at 900nm, got {}",
            reading
        );
    }

    #[tokio::test]
    async fn test_noise_characteristics() {
        let meter = MockPowerMeter::builder()
            .base_power(1.0)
            .noise_model(NoiseModel {
                shot_noise_coefficient: 0.1,
                thermal_noise_floor: 0.01,
            })
            .rng_seed(42) // Deterministic
            .build();

        let readings: Vec<f64> = futures::future::join_all((0..100).map(|_| meter.read()))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()
            .unwrap();

        let mean = readings.iter().sum::<f64>() / readings.len() as f64;
        let variance = readings.iter().map(|r| (r - mean).powi(2)).sum::<f64>()
            / readings.len() as f64;

        // Mean should be close to base power
        assert!(
            (mean - 1.0).abs() < 0.1,
            "Mean {} should be near base power 1.0",
            mean
        );

        // Should have variance (not zero)
        assert!(variance > 0.0, "Should have noise variance");
    }

    #[tokio::test]
    async fn test_attenuator_factors() {
        let test_cases = vec![
            (Attenuator::None, 1.0),
            (Attenuator::Db10, 0.1),
            (Attenuator::Db20, 0.01),
            (Attenuator::Db30, 0.001),
        ];

        for (attenuator, expected_factor) in test_cases {
            let meter = MockPowerMeter::builder()
                .base_power(1.0)
                .attenuator(attenuator)
                .noise_model(NoiseModel::none())
                .build();

            let reading = meter.read().await.unwrap();
            assert!(
                (reading - expected_factor).abs() < 1e-6,
                "Attenuator {:?} should give {}, got {}",
                attenuator,
                expected_factor,
                reading
            );
        }
    }

    #[tokio::test]
    async fn test_filter_averaging() {
        // Filter doesn't average in current implementation, just adds delay
        // This test verifies the delay happens (in Realistic mode)
        let meter = MockPowerMeter::builder()
            .base_power(1.0)
            .filter(FilterSetting::Fast) // 10ms delay
            .mode(MockMode::Realistic)
            .noise_model(NoiseModel::none())
            .build();

        let start = std::time::Instant::now();
        let _reading = meter.read().await.unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() >= 10,
            "Fast filter should delay ~10ms, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_builder_pattern() {
        let meter = MockPowerMeter::builder()
            .base_power(1.0e-3)
            .unit(PowerUnit::Milliwatts)
            .wavelength_nm(800.0)
            .filter(FilterSetting::Medium)
            .attenuator(Attenuator::Db10)
            .noise_model(NoiseModel::none())
            .build();

        // Base: 1e-3 W
        // Attenuator: 0.1x -> 1e-4 W
        // Unit conversion: 1e-4 W * 1000 = 0.1 mW
        let reading = meter.read().await.unwrap();
        assert!(
            (reading - 0.1).abs() < 0.01,
            "Expected ~0.1 mW, got {}",
            reading
        );
    }

    #[tokio::test]
    async fn test_set_unit() {
        let mut meter = MockPowerMeter::new(1.0);
        assert_eq!(meter.get_unit(), PowerUnit::Watts);

        meter.set_unit(PowerUnit::Milliwatts);
        assert_eq!(meter.get_unit(), PowerUnit::Milliwatts);
    }

    #[tokio::test]
    async fn test_set_wavelength() {
        let mut meter = MockPowerMeter::new(1.0);
        meter.set_wavelength(900.0);
        assert_eq!(meter.get_wavelength(), 900.0);

        // Should clamp to valid range
        meter.set_wavelength(2000.0);
        assert_eq!(meter.get_wavelength(), 1100.0);

        meter.set_wavelength(100.0);
        assert_eq!(meter.get_wavelength(), 300.0);
    }

    #[tokio::test]
    async fn test_factory_creates_power_meter() {
        let factory = MockPowerMeterFactory;

        assert_eq!(factory.driver_type(), "mock_power_meter");

        let config = toml::Value::Table(toml::map::Map::new());
        let components = factory.build(config).await.unwrap();

        assert!(components.readable.is_some());
        assert!(components.parameterized.is_some());
    }

    #[test]
    fn test_power_unit_conversions() {
        let meter = MockPowerMeter::builder()
            .base_power(1e-3)
            .noise_model(NoiseModel::none())
            .build();

        // Watts
        assert!((meter.convert_to_unit(1e-3) - 1e-3).abs() < 1e-9);

        // Milliwatts
        let mut meter_mw = meter.clone();
        meter_mw.set_unit(PowerUnit::Milliwatts);
        assert!((meter_mw.convert_to_unit(1e-3) - 1.0).abs() < 1e-6);

        // dBm (1mW = 0 dBm)
        let mut meter_dbm = meter.clone();
        meter_dbm.set_unit(PowerUnit::Dbm);
        assert!((meter_dbm.convert_to_unit(1e-3) - 0.0).abs() < 0.001);
    }

    impl Clone for MockPowerMeter {
        fn clone(&self) -> Self {
            MockPowerMeter::builder()
                .base_power(self.base_power.get())
                .unit(self.unit)
                .wavelength_nm(self.wavelength_nm)
                .noise_model(self.noise_model.clone())
                .filter(self.filter)
                .attenuator(self.attenuator)
                .spectral_response(self.spectral_response.clone())
                .mode(self.mode)
                .error_config(self.error_config.clone())
                .build()
        }
    }
}
