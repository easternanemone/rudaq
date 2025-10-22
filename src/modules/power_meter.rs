//! PowerMeterModule - Proof-of-concept module implementation
//!
//! This module demonstrates the Module + ModuleWithInstrument trait system
//! for creating hardware-agnostic experiment logic with runtime instrument assignment.
//!
//! # Features
//!
//! - **Threshold Monitoring:** Configurable low/high power thresholds with alert generation
//! - **Statistical Analysis:** Real-time mean, std dev, min/max over sliding window
//! - **Type Safety:** Only accepts instruments implementing `Instrument<Measure = M>`
//!   where M::Data can be interpreted as power measurements
//! - **Runtime Assignment:** Instruments can be swapped without recompiling
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::modules::{Module, ModuleWithInstrument, ModuleConfig};
//! use rust_daq::modules::power_meter::PowerMeterModule;
//!
//! // Create module
//! let mut module = PowerMeterModule::new("laser_monitor".to_string());
//!
//! // Configure thresholds
//! let mut config = ModuleConfig::new();
//! config.set("low_threshold".to_string(), serde_json::json!(50.0));
//! config.set("high_threshold".to_string(), serde_json::json!(150.0));
//! module.init(config)?;
//!
//! // Assign power meter instrument
//! let instrument = Arc::new(power_meter);
//! module.assign_instrument("main".to_string(), instrument)?;
//!
//! // Module is now ready to use
//! assert_eq!(module.status(), ModuleStatus::Initialized);
//! ```

use crate::core::Instrument;
use crate::measurement::Measure;
use crate::modules::{Module, ModuleConfig, ModuleStatus, ModuleWithInstrument};
use anyhow::{anyhow, Result};
use std::collections::VecDeque;
use std::sync::Arc;

/// Configuration for power monitoring thresholds and analysis
#[derive(Clone, Debug)]
struct PowerMonitorConfig {
    /// Low power threshold in watts (alert if power drops below)
    low_threshold: f64,
    /// High power threshold in watts (alert if power exceeds)
    high_threshold: f64,
    /// Window duration for statistical analysis in seconds
    window_duration_s: f64,
    /// Alert callback name (for future alert system integration)
    alert_callback: Option<String>,
}

impl Default for PowerMonitorConfig {
    fn default() -> Self {
        Self {
            low_threshold: 50.0,
            high_threshold: 150.0,
            window_duration_s: 60.0,
            alert_callback: None,
        }
    }
}

/// Statistical summary over a time window
#[derive(Clone, Debug)]
struct PowerStatistics {
    /// Mean power in watts
    mean: f64,
    /// Standard deviation in watts
    std_dev: f64,
    /// Minimum power in watts
    min: f64,
    /// Maximum power in watts
    max: f64,
    /// Number of samples in window
    sample_count: usize,
}

/// Power monitoring module with threshold alerts and statistics
///
/// This module demonstrates a complete implementation of the Module system:
/// - Generic over measurement type `M` for flexibility
/// - Type-safe instrument assignment via `ModuleWithInstrument<M>`
/// - Configurable thresholds and analysis windows
/// - Real-time statistical analysis
/// - Alert generation on threshold violations
///
/// # Type Parameter
///
/// * `M: Measure` - The measurement type for the assigned instrument.
///   Typically `PowerMeasure` for power meters, but can work with any
///   scalar measurement that can be interpreted as power.
pub struct PowerMeterModule<M: Measure> {
    /// Module instance name (e.g., "laser_monitor", "pump_power")
    name: String,

    /// Assigned power meter instrument (None until assigned)
    power_meter: Option<Arc<dyn Instrument<Measure = M>>>,

    /// Current module status
    status: ModuleStatus,

    /// Configuration for thresholds and analysis
    config: PowerMonitorConfig,

    /// Sliding window of recent power readings (value, timestamp)
    power_history: VecDeque<(f64, chrono::DateTime<chrono::Utc>)>,

    /// Current statistical summary (updated on each measurement)
    current_stats: Option<PowerStatistics>,

    /// Count of threshold violations (for debugging/logging)
    low_threshold_violations: u64,
    high_threshold_violations: u64,

    /// Phantom data for generic type parameter
    _phantom: std::marker::PhantomData<M>,
}

impl<M: Measure + 'static> PowerMeterModule<M> {
    /// Creates a new PowerMeterModule with default configuration
    ///
    /// The module starts in `Idle` status and requires:
    /// 1. Initialization via `init(config)`
    /// 2. Instrument assignment via `assign_instrument()`
    ///
    /// before it can be started.
    pub fn new(name: String) -> Self {
        Self {
            name,
            power_meter: None,
            status: ModuleStatus::Idle,
            config: PowerMonitorConfig::default(),
            power_history: VecDeque::new(),
            current_stats: None,
            low_threshold_violations: 0,
            high_threshold_violations: 0,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Processes a new power measurement
    ///
    /// This method:
    /// 1. Adds measurement to sliding window
    /// 2. Removes old measurements outside window
    /// 3. Recalculates statistics
    /// 4. Checks thresholds and generates alerts
    fn process_power_measurement(&mut self, power_watts: f64) {
        let now = chrono::Utc::now();

        // Add to history
        self.power_history.push_back((power_watts, now));

        // Remove old measurements outside window
        let cutoff = now - chrono::Duration::seconds(self.config.window_duration_s as i64);
        while let Some((_, timestamp)) = self.power_history.front() {
            if timestamp < &cutoff {
                self.power_history.pop_front();
            } else {
                break;
            }
        }

        // Recalculate statistics
        if !self.power_history.is_empty() {
            self.update_statistics();
        }

        // Check thresholds
        self.check_thresholds(power_watts);
    }

    /// Updates statistical summary from current power history window
    fn update_statistics(&mut self) {
        if self.power_history.is_empty() {
            self.current_stats = None;
            return;
        }

        let values: Vec<f64> = self.power_history.iter().map(|(v, _)| *v).collect();
        let n = values.len() as f64;

        let sum: f64 = values.iter().sum();
        let mean = sum / n;

        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let std_dev = variance.sqrt();

        let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        self.current_stats = Some(PowerStatistics {
            mean,
            std_dev,
            min,
            max,
            sample_count: values.len(),
        });
    }

    /// Checks power against thresholds and generates alerts if violated
    fn check_thresholds(&mut self, power_watts: f64) {
        if power_watts < self.config.low_threshold {
            self.low_threshold_violations += 1;
            log::warn!(
                "Module '{}': Power below threshold: {:.2} W < {:.2} W (violation #{})",
                self.name,
                power_watts,
                self.config.low_threshold,
                self.low_threshold_violations
            );
            // TODO: Trigger alert system when implemented
        }

        if power_watts > self.config.high_threshold {
            self.high_threshold_violations += 1;
            log::warn!(
                "Module '{}': Power above threshold: {:.2} W > {:.2} W (violation #{})",
                self.name,
                power_watts,
                self.config.high_threshold,
                self.high_threshold_violations
            );
            // TODO: Trigger alert system when implemented
        }
    }

    /// Returns current power statistics if available
    pub fn get_statistics(&self) -> Option<&PowerStatistics> {
        self.current_stats.as_ref()
    }

    /// Returns threshold violation counts
    pub fn get_violation_counts(&self) -> (u64, u64) {
        (self.low_threshold_violations, self.high_threshold_violations)
    }

    /// Clears power history and resets statistics
    pub fn reset_statistics(&mut self) {
        self.power_history.clear();
        self.current_stats = None;
        self.low_threshold_violations = 0;
        self.high_threshold_violations = 0;
    }
}

impl<M: Measure + 'static> Module for PowerMeterModule<M> {
    fn name(&self) -> &str {
        &self.name
    }

    fn init(&mut self, config: ModuleConfig) -> Result<()> {
        // Parse configuration
        if let Some(low) = config.get("low_threshold").and_then(|v| v.as_f64()) {
            self.config.low_threshold = low;
        }

        if let Some(high) = config.get("high_threshold").and_then(|v| v.as_f64()) {
            self.config.high_threshold = high;
        }

        if let Some(window) = config.get("window_duration_s").and_then(|v| v.as_f64()) {
            self.config.window_duration_s = window;
        }

        if let Some(callback) = config.get("alert_callback").and_then(|v| v.as_str()) {
            self.config.alert_callback = Some(callback.to_string());
        }

        // Validate configuration
        if self.config.low_threshold >= self.config.high_threshold {
            return Err(anyhow!(
                "Invalid thresholds: low ({}) must be less than high ({})",
                self.config.low_threshold,
                self.config.high_threshold
            ));
        }

        if self.config.window_duration_s <= 0.0 {
            return Err(anyhow!(
                "Invalid window duration: {} (must be > 0)",
                self.config.window_duration_s
            ));
        }

        self.status = ModuleStatus::Initialized;
        log::info!(
            "Module '{}' initialized: thresholds [{:.1}, {:.1}] W, window {:.1} s",
            self.name,
            self.config.low_threshold,
            self.config.high_threshold,
            self.config.window_duration_s
        );

        Ok(())
    }

    fn status(&self) -> ModuleStatus {
        self.status
    }
}

impl<M: Measure + 'static> ModuleWithInstrument<M> for PowerMeterModule<M> {
    fn assign_instrument(
        &mut self,
        _id: String,
        instrument: Arc<dyn Instrument<Measure = M>>,
    ) -> Result<()> {
        // Enforce assignment restrictions
        if self.status == ModuleStatus::Running {
            return Err(anyhow!(
                "Cannot assign instrument while module is running. Call stop() first."
            ));
        }

        // Store instrument
        self.power_meter = Some(instrument);

        // Reset statistics on new instrument assignment
        self.reset_statistics();

        log::info!(
            "Module '{}': Instrument assigned and statistics reset",
            self.name
        );

        Ok(())
    }

    fn get_instrument(&self, _id: &str) -> Option<Arc<dyn Instrument<Measure = M>>> {
        self.power_meter.clone()
    }

    fn list_instruments(&self) -> Vec<String> {
        if self.power_meter.is_some() {
            vec!["main".to_string()]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock measure type for testing
    #[derive(Clone)]
    struct MockPowerMeasure;

    impl Measure for MockPowerMeasure {
        type Data = f64;
        fn unit() -> &'static str {
            "W"
        }
    }

    #[test]
    fn test_module_creation() {
        let module: PowerMeterModule<MockPowerMeasure> =
            PowerMeterModule::new("test".to_string());
        assert_eq!(module.name(), "test");
        assert_eq!(module.status(), ModuleStatus::Idle);
    }

    #[test]
    fn test_module_initialization() {
        let mut module: PowerMeterModule<MockPowerMeasure> =
            PowerMeterModule::new("test".to_string());

        let mut config = ModuleConfig::new();
        config.set("low_threshold".to_string(), serde_json::json!(40.0));
        config.set("high_threshold".to_string(), serde_json::json!(120.0));
        config.set("window_duration_s".to_string(), serde_json::json!(30.0));

        module.init(config).unwrap();
        assert_eq!(module.status(), ModuleStatus::Initialized);
        assert_eq!(module.config.low_threshold, 40.0);
        assert_eq!(module.config.high_threshold, 120.0);
        assert_eq!(module.config.window_duration_s, 30.0);
    }

    #[test]
    fn test_invalid_threshold_config() {
        let mut module: PowerMeterModule<MockPowerMeasure> =
            PowerMeterModule::new("test".to_string());

        let mut config = ModuleConfig::new();
        config.set("low_threshold".to_string(), serde_json::json!(150.0));
        config.set("high_threshold".to_string(), serde_json::json!(50.0));

        let result = module.init(config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid thresholds"));
    }

    #[test]
    fn test_threshold_violation_detection() {
        let mut module: PowerMeterModule<MockPowerMeasure> =
            PowerMeterModule::new("test".to_string());

        let mut config = ModuleConfig::new();
        config.set("low_threshold".to_string(), serde_json::json!(50.0));
        config.set("high_threshold".to_string(), serde_json::json!(150.0));
        module.init(config).unwrap();

        // Process power measurements
        module.process_power_measurement(30.0); // Low violation
        module.process_power_measurement(100.0); // Normal
        module.process_power_measurement(200.0); // High violation

        let (low_count, high_count) = module.get_violation_counts();
        assert_eq!(low_count, 1);
        assert_eq!(high_count, 1);
    }

    #[test]
    fn test_statistics_calculation() {
        let mut module: PowerMeterModule<MockPowerMeasure> =
            PowerMeterModule::new("test".to_string());

        module.init(ModuleConfig::new()).unwrap();

        // Add measurements
        module.process_power_measurement(100.0);
        module.process_power_measurement(110.0);
        module.process_power_measurement(90.0);

        let stats = module.get_statistics().unwrap();
        assert_eq!(stats.sample_count, 3);
        assert!((stats.mean - 100.0).abs() < 0.1);
        assert_eq!(stats.min, 90.0);
        assert_eq!(stats.max, 110.0);
    }

    #[test]
    fn test_statistics_reset() {
        let mut module: PowerMeterModule<MockPowerMeasure> =
            PowerMeterModule::new("test".to_string());

        module.init(ModuleConfig::new()).unwrap();
        module.process_power_measurement(100.0);

        assert!(module.get_statistics().is_some());

        module.reset_statistics();

        assert!(module.get_statistics().is_none());
        assert_eq!(module.get_violation_counts(), (0, 0));
    }
}
