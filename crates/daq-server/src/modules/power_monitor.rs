//! PowerMonitor Module
//!
//! Monitors power readings with threshold alerts and statistics.
//!
//! # Features
//!
//! - Configurable sample rate (0.1 - 100 Hz)
//! - Low/high threshold alerts
//! - Running statistics (mean, std, min, max)
//! - Event emission for threshold crossings
//! - Data streaming for power readings and statistics
//!
//! # Roles
//!
//! | Role ID | Required Capability | Description |
//! |---------|---------------------|-------------|
//! | `power_meter` | `Readable` | Device providing power readings |
//!
//! # Parameters
//!
//! | Parameter | Type | Default | Units | Description |
//! |-----------|------|---------|-------|-------------|
//! | `sample_rate_hz` | float | 10.0 | Hz | Sampling rate |
//! | `low_threshold` | float | - | mW | Alert if below (optional) |
//! | `high_threshold` | float | - | mW | Alert if above (optional) |
//! | `averaging_window_s` | float | 1.0 | s | Window for statistics |
//!
//! # Events
//!
//! - `threshold_low` - Power dropped below low threshold
//! - `threshold_high` - Power exceeded high threshold
//! - `threshold_normal` - Power returned to normal range
//!
//! # Data Types
//!
//! - `power_reading` - Individual readings: `{value}`
//! - `statistics` - Computed stats: `{mean, std, min, max, count}`

use super::{Module, ModuleContext};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use common::limits::SHUTDOWN_TIMEOUT;
use common::modules::{
    ModuleEventSeverity, ModuleParameter, ModuleRole, ModuleState, ModuleTypeInfo,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{info, warn};

/// PowerMonitor module configuration
#[derive(Debug, Clone)]
pub struct PowerMonitorConfig {
    /// Sampling rate in Hz
    pub sample_rate_hz: f64,
    /// Low threshold for alerts (optional)
    pub low_threshold: Option<f64>,
    /// High threshold for alerts (optional)
    pub high_threshold: Option<f64>,
    /// Averaging window in seconds
    pub averaging_window_s: f64,
}

impl Default for PowerMonitorConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 10.0,
            low_threshold: None,
            high_threshold: None,
            averaging_window_s: 1.0,
        }
    }
}

/// Running statistics for power readings
#[derive(Debug, Default)]
struct Statistics {
    values: VecDeque<f64>,
    sum: f64,
    sum_sq: f64,
    min: f64,
    max: f64,
    count: u64,
}

impl Statistics {
    fn new() -> Self {
        Self {
            values: VecDeque::new(),
            sum: 0.0,
            sum_sq: 0.0,
            min: f64::MAX,
            max: f64::MIN,
            count: 0,
        }
    }

    fn add(&mut self, value: f64, max_window_size: usize) {
        // Remove old values if window is full
        while self.values.len() >= max_window_size {
            if let Some(old) = self.values.pop_front() {
                self.sum -= old;
                self.sum_sq -= old * old;
            }
        }

        // Add new value
        self.values.push_back(value);
        self.sum += value;
        self.sum_sq += value * value;
        self.count += 1;

        // Update min/max (for the current window)
        self.min = self.values.iter().copied().fold(f64::MAX, |a, b| a.min(b));
        self.max = self.values.iter().copied().fold(f64::MIN, |a, b| a.max(b));
    }

    fn mean(&self) -> f64 {
        if self.values.is_empty() {
            0.0
        } else {
            self.sum / self.values.len() as f64
        }
    }

    fn std(&self) -> f64 {
        if self.values.len() < 2 {
            0.0
        } else {
            let n = self.values.len() as f64;
            let mean = self.sum / n;
            let variance = (self.sum_sq / n) - (mean * mean);
            variance.max(0.0).sqrt()
        }
    }

    fn as_hashmap(&self) -> HashMap<String, f64> {
        let mut map = HashMap::new();
        map.insert("mean".to_string(), self.mean());
        map.insert("std".to_string(), self.std());
        map.insert("min".to_string(), self.min);
        map.insert("max".to_string(), self.max);
        map.insert("count".to_string(), self.count as f64);
        map.insert("window_size".to_string(), self.values.len() as f64);
        map
    }
}

/// Threshold state for detecting crossings
#[derive(Debug, Clone, Copy, PartialEq)]
enum ThresholdState {
    Normal,
    Low,
    High,
}

/// PowerMonitor module
pub struct PowerMonitor {
    config: PowerMonitorConfig,
    state: ModuleState,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for PowerMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PowerMonitor")
            .field("config", &self.config)
            .field("state", &self.state)
            .field("running", &self.running.load(Ordering::Relaxed))
            .field("paused", &self.paused.load(Ordering::Relaxed))
            .field("task_handle", &self.task_handle.is_some())
            .finish()
    }
}

impl Default for PowerMonitor {
    fn default() -> Self {
        Self {
            config: PowerMonitorConfig::default(),
            state: ModuleState::Created,
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            task_handle: None,
        }
    }
}

#[async_trait]
impl Module for PowerMonitor {
    fn type_info() -> ModuleTypeInfo {
        ModuleTypeInfo {
            type_id: "power_monitor".to_string(),
            display_name: "Power Monitor".to_string(),
            description:
                "Monitors power readings with configurable threshold alerts and running statistics"
                    .to_string(),
            version: "1.0.0".to_string(),
            required_roles: vec![ModuleRole {
                role_id: "power_meter".to_string(),
                display_name: "Power Meter".to_string(),
                description: "Device providing power readings".to_string(),
                required_capability: "readable".to_string(),
                allows_multiple: false,
            }],
            optional_roles: vec![],
            parameters: vec![
                ModuleParameter {
                    param_id: "sample_rate_hz".to_string(),
                    display_name: "Sample Rate".to_string(),
                    description: "How often to read and check thresholds".to_string(),
                    param_type: "float".to_string(),
                    default_value: "10.0".to_string(),
                    min_value: Some("0.1".to_string()),
                    max_value: Some("100.0".to_string()),
                    enum_values: vec![],
                    units: "Hz".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "low_threshold".to_string(),
                    display_name: "Low Threshold".to_string(),
                    description: "Alert when power drops below this value".to_string(),
                    param_type: "float".to_string(),
                    default_value: String::new(),
                    min_value: Some("0.0".to_string()),
                    max_value: None,
                    enum_values: vec![],
                    units: "mW".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "high_threshold".to_string(),
                    display_name: "High Threshold".to_string(),
                    description: "Alert when power exceeds this value".to_string(),
                    param_type: "float".to_string(),
                    default_value: String::new(),
                    min_value: Some("0.0".to_string()),
                    max_value: None,
                    enum_values: vec![],
                    units: "mW".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "averaging_window_s".to_string(),
                    display_name: "Averaging Window".to_string(),
                    description: "Time window for computing statistics".to_string(),
                    param_type: "float".to_string(),
                    default_value: "1.0".to_string(),
                    min_value: Some("0.1".to_string()),
                    max_value: Some("60.0".to_string()),
                    enum_values: vec![],
                    units: "s".to_string(),
                    required: false,
                },
            ],
            event_types: vec![
                "threshold_low".to_string(),
                "threshold_high".to_string(),
                "threshold_normal".to_string(),
            ],
            data_types: vec!["power_reading".to_string(), "statistics".to_string()],
        }
    }

    fn type_id(&self) -> &str {
        "power_monitor"
    }

    fn configure(&mut self, params: HashMap<String, String>) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        // Parse sample_rate_hz
        if let Some(val) = params.get("sample_rate_hz") {
            match val.parse::<f64>() {
                Ok(rate) => {
                    if rate < 0.1 {
                        self.config.sample_rate_hz = 0.1;
                        warnings.push("sample_rate_hz clamped to minimum 0.1 Hz".to_string());
                    } else if rate > 100.0 {
                        self.config.sample_rate_hz = 100.0;
                        warnings.push("sample_rate_hz clamped to maximum 100 Hz".to_string());
                    } else {
                        self.config.sample_rate_hz = rate;
                    }
                }
                Err(_) => warnings.push(format!("Invalid sample_rate_hz: {}", val)),
            }
        }

        // Parse low_threshold
        if let Some(val) = params.get("low_threshold") {
            if val.is_empty() {
                self.config.low_threshold = None;
            } else {
                match val.parse::<f64>() {
                    Ok(thresh) => self.config.low_threshold = Some(thresh),
                    Err(_) => warnings.push(format!("Invalid low_threshold: {}", val)),
                }
            }
        }

        // Parse high_threshold
        if let Some(val) = params.get("high_threshold") {
            if val.is_empty() {
                self.config.high_threshold = None;
            } else {
                match val.parse::<f64>() {
                    Ok(thresh) => self.config.high_threshold = Some(thresh),
                    Err(_) => warnings.push(format!("Invalid high_threshold: {}", val)),
                }
            }
        }

        // Parse averaging_window_s
        if let Some(val) = params.get("averaging_window_s") {
            match val.parse::<f64>() {
                Ok(window) => {
                    self.config.averaging_window_s = window.clamp(0.1, 60.0);
                }
                Err(_) => warnings.push(format!("Invalid averaging_window_s: {}", val)),
            }
        }

        // Validate threshold relationship
        if let (Some(low), Some(high)) = (self.config.low_threshold, self.config.high_threshold)
            && low >= high
        {
            warnings.push("low_threshold should be less than high_threshold".to_string());
        }

        self.state = ModuleState::Configured;
        Ok(warnings)
    }

    fn get_config(&self) -> HashMap<String, String> {
        let mut config = HashMap::new();
        config.insert(
            "sample_rate_hz".to_string(),
            format!("{}", self.config.sample_rate_hz as u32),
        );
        if let Some(low) = self.config.low_threshold {
            config.insert("low_threshold".to_string(), format!("{}", low));
        }
        if let Some(high) = self.config.high_threshold {
            config.insert("high_threshold".to_string(), format!("{}", high));
        }
        config.insert(
            "averaging_window_s".to_string(),
            format!("{}", self.config.averaging_window_s),
        );
        config
    }

    async fn start(&mut self, ctx: ModuleContext) -> Result<()> {
        if self.state == ModuleState::Running {
            return Err(anyhow!("Module is already running"));
        }

        // Verify power meter is assigned
        let power_meter = ctx.get_readable("power_meter").ok_or_else(|| {
            anyhow!("No power meter assigned. Assign a readable device to the 'power_meter' role.")
        })?;

        self.running.store(true, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.state = ModuleState::Running;

        let config = self.config.clone();
        let running = Arc::clone(&self.running);
        let paused = Arc::clone(&self.paused);

        // Spawn the monitoring task
        let handle = tokio::spawn(async move {
            power_monitor_task(ctx, config, running, paused, power_meter).await;
        });

        self.task_handle = Some(handle);
        info!("PowerMonitor started");
        Ok(())
    }

    async fn pause(&mut self) -> Result<()> {
        if self.state != ModuleState::Running {
            return Err(anyhow!("Module is not running"));
        }

        self.paused.store(true, Ordering::SeqCst);
        self.state = ModuleState::Paused;
        info!("PowerMonitor paused");
        Ok(())
    }

    async fn resume(&mut self) -> Result<()> {
        if self.state != ModuleState::Paused {
            return Err(anyhow!("Module is not paused"));
        }

        self.paused.store(false, Ordering::SeqCst);
        self.state = ModuleState::Running;
        info!("PowerMonitor resumed");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if self.state != ModuleState::Running && self.state != ModuleState::Paused {
            return Err(anyhow!("Module is not running"));
        }

        self.running.store(false, Ordering::SeqCst);

        // Wait for task to complete
        if let Some(handle) = self.task_handle.take() {
            // Give it a moment to finish gracefully
            tokio::time::timeout(SHUTDOWN_TIMEOUT, handle).await.ok();
        }

        self.state = ModuleState::Stopped;
        info!("PowerMonitor stopped");
        Ok(())
    }

    fn state(&self) -> ModuleState {
        self.state
    }
}

/// Main monitoring task
async fn power_monitor_task(
    mut ctx: ModuleContext,
    config: PowerMonitorConfig,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    power_meter: Arc<dyn daq_hardware::capabilities::Readable>,
) {
    let interval = Duration::from_secs_f64(1.0 / config.sample_rate_hz);
    let window_size = (config.sample_rate_hz * config.averaging_window_s).ceil() as usize;
    let window_size = window_size.max(1);

    let mut stats = Statistics::new();
    let mut threshold_state = ThresholdState::Normal;
    let mut ticker = tokio::time::interval(interval);

    info!(
        "PowerMonitor task started: rate={:.1}Hz, window={}samples",
        config.sample_rate_hz, window_size
    );

    // Emit start event
    ctx.emit_event(
        "state_change",
        ModuleEventSeverity::Info,
        "Power monitoring started",
    )
    .await;

    while running.load(Ordering::SeqCst) {
        ticker.tick().await;

        // Check for shutdown
        if ctx.is_shutdown_requested() {
            break;
        }

        // Skip if paused
        if paused.load(Ordering::SeqCst) {
            continue;
        }

        // Read power value
        let value = match power_meter.read().await {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to read power meter: {}", e);
                ctx.emit_event(
                    "read_error",
                    ModuleEventSeverity::Warning,
                    &format!("Failed to read: {}", e),
                )
                .await;
                continue;
            }
        };

        // Update statistics
        stats.add(value, window_size);

        // Emit power reading
        let mut values = HashMap::new();
        values.insert("value".to_string(), value);
        ctx.emit_data("power_reading", values).await;

        // Emit statistics periodically (every window_size samples)
        if stats.count.is_multiple_of(window_size as u64) {
            ctx.emit_data("statistics", stats.as_hashmap()).await;
        }

        // Check thresholds
        let new_state = check_thresholds(value, &config);
        if new_state != threshold_state {
            emit_threshold_event(&ctx, threshold_state, new_state, value).await;
            threshold_state = new_state;
        }
    }

    // Emit stop event
    ctx.emit_event(
        "state_change",
        ModuleEventSeverity::Info,
        "Power monitoring stopped",
    )
    .await;

    info!("PowerMonitor task ended");
}

/// Check if value crosses thresholds
fn check_thresholds(value: f64, config: &PowerMonitorConfig) -> ThresholdState {
    if let Some(low) = config.low_threshold
        && value < low
    {
        return ThresholdState::Low;
    }
    if let Some(high) = config.high_threshold
        && value > high
    {
        return ThresholdState::High;
    }
    ThresholdState::Normal
}

/// Emit event for threshold crossing
async fn emit_threshold_event(
    ctx: &ModuleContext,
    old_state: ThresholdState,
    new_state: ThresholdState,
    value: f64,
) {
    let mut data = HashMap::new();
    data.insert("value".to_string(), format!("{:.3}", value));
    data.insert("previous_state".to_string(), format!("{:?}", old_state));

    match new_state {
        ThresholdState::Low => {
            ctx.emit_event_with_data(
                "threshold_low",
                ModuleEventSeverity::Warning,
                &format!("Power dropped below threshold: {:.3} mW", value),
                data,
            )
            .await;
        }
        ThresholdState::High => {
            ctx.emit_event_with_data(
                "threshold_high",
                ModuleEventSeverity::Warning,
                &format!("Power exceeded threshold: {:.3} mW", value),
                data,
            )
            .await;
        }
        ThresholdState::Normal => {
            ctx.emit_event_with_data(
                "threshold_normal",
                ModuleEventSeverity::Info,
                &format!("Power returned to normal: {:.3} mW", value),
                data,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statistics() {
        let mut stats = Statistics::new();

        // Add some values
        for i in 1..=5 {
            stats.add(i as f64, 10);
        }

        assert_eq!(stats.count, 5);
        assert!((stats.mean() - 3.0).abs() < 0.001);
        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.max, 5.0);
    }

    #[test]
    fn test_statistics_window() {
        let mut stats = Statistics::new();

        // Add more values than window size
        for i in 1..=10 {
            stats.add(i as f64, 5);
        }

        // Window should only contain last 5 values (6-10)
        assert_eq!(stats.values.len(), 5);
        assert!((stats.mean() - 8.0).abs() < 0.001);
        assert_eq!(stats.min, 6.0);
        assert_eq!(stats.max, 10.0);
    }

    #[test]
    fn test_threshold_detection() {
        let config = PowerMonitorConfig {
            low_threshold: Some(10.0),
            high_threshold: Some(100.0),
            ..Default::default()
        };

        assert_eq!(check_thresholds(50.0, &config), ThresholdState::Normal);
        assert_eq!(check_thresholds(5.0, &config), ThresholdState::Low);
        assert_eq!(check_thresholds(150.0, &config), ThresholdState::High);
    }

    #[test]
    fn test_config_parsing() {
        let mut monitor = PowerMonitor::default();

        let mut params = HashMap::new();
        params.insert("sample_rate_hz".to_string(), "20.0".to_string());
        params.insert("low_threshold".to_string(), "10.0".to_string());
        params.insert("high_threshold".to_string(), "100.0".to_string());

        let warnings = monitor.configure(params).unwrap();
        assert!(warnings.is_empty());

        assert_eq!(monitor.config.sample_rate_hz, 20.0);
        assert_eq!(monitor.config.low_threshold, Some(10.0));
        assert_eq!(monitor.config.high_threshold, Some(100.0));
    }

    #[test]
    fn test_config_clamping() {
        let mut monitor = PowerMonitor::default();

        let mut params = HashMap::new();
        params.insert("sample_rate_hz".to_string(), "200.0".to_string()); // Too high

        let warnings = monitor.configure(params).unwrap();
        assert!(!warnings.is_empty());
        assert_eq!(monitor.config.sample_rate_hz, 100.0); // Clamped to max
    }
}
