//! NI DAQ-specific plans for RunEngine integration.
//!
//! This module provides specialized plans for National Instruments DAQ hardware:
//!
//! - [`VoltageScan`] - Scan analog output voltage while reading analog input
//! - [`TimeSeries`] - Continuous time series logging at fixed sample rate
//! - [`TriggeredAcquisition`] - Hardware-triggered multi-channel acquisition
//!
//! # Architecture
//!
//! These plans follow the same pattern as other plans in `plans.rs`:
//! - Implement the [`Plan`] trait
//! - Yield [`PlanCommand`] values for the RunEngine to execute
//! - Support pause/resume via checkpoint commands
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_experiment::plans_daq::{VoltageScan, TimeSeries, TriggeredAcquisition};
//!
//! // Voltage scan: sweep AO from 0-5V while reading AI
//! let scan = VoltageScan::new("ao0", "ai0", 0.0, 5.0, 51);
//!
//! // Time series: log AI for 10 seconds at 1kHz
//! let series = TimeSeries::new("ai0", 1000.0, 10.0);
//!
//! // Triggered acquisition: read AI0-AI3 on external trigger
//! let triggered = TriggeredAcquisition::new()
//!     .with_channels(&["ai0", "ai1", "ai2", "ai3"])
//!     .with_trigger("pfi0")
//!     .with_num_triggers(100);
//! ```

use std::collections::HashMap;

use crate::plans::{Plan, PlanCommand};

// =============================================================================
// Voltage Scan Plan
// =============================================================================

/// Voltage scan plan - sweep analog output while reading analog input.
///
/// This plan scans an analog output channel through a voltage range
/// while simultaneously reading one or more analog input channels.
/// Useful for characterization measurements like I-V curves.
///
/// # Example
///
/// ```rust,ignore
/// // Scan AO0 from 0V to 5V in 51 steps, reading AI0
/// let scan = VoltageScan::new("ao0", "ai0", 0.0, 5.0, 51)
///     .with_settle_time(0.01);  // 10ms settle time
///
/// engine.queue(Box::new(scan)).await;
/// engine.start().await?;
/// ```
#[derive(Debug, Clone)]
pub struct VoltageScan {
    /// Analog output device ID
    ao_device: String,
    /// Primary analog input device ID
    ai_device: String,
    /// Additional AI channels to read
    extra_ai: Vec<String>,
    /// Start voltage
    start_v: f64,
    /// Stop voltage
    stop_v: f64,
    /// Number of points
    num_points: usize,
    /// Settle time in seconds
    settle_time: f64,

    // Execution state
    current_point: usize,
    current_step: VoltageScanStep,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum VoltageScanStep {
    SetVoltage,
    Settle,
    Checkpoint,
    ReadAI { ai_idx: usize },
    EmitEvent,
}

impl VoltageScan {
    /// Create a new voltage scan plan.
    ///
    /// # Arguments
    ///
    /// * `ao_device` - Analog output device ID for voltage sweep
    /// * `ai_device` - Primary analog input device ID for reading
    /// * `start_v` - Starting voltage
    /// * `stop_v` - Ending voltage
    /// * `num_points` - Number of scan points
    pub fn new(
        ao_device: &str,
        ai_device: &str,
        start_v: f64,
        stop_v: f64,
        num_points: usize,
    ) -> Self {
        Self {
            ao_device: ao_device.to_string(),
            ai_device: ai_device.to_string(),
            extra_ai: Vec::new(),
            start_v,
            stop_v,
            num_points,
            settle_time: 0.0,
            current_point: 0,
            current_step: VoltageScanStep::SetVoltage,
        }
    }

    /// Add an additional AI channel to read at each point.
    pub fn with_extra_ai(mut self, ai_device: &str) -> Self {
        self.extra_ai.push(ai_device.to_string());
        self
    }

    /// Add multiple additional AI channels.
    pub fn with_extra_ais(mut self, ai_devices: &[&str]) -> Self {
        self.extra_ai
            .extend(ai_devices.iter().map(|s| s.to_string()));
        self
    }

    /// Set settle time in seconds.
    pub fn with_settle_time(mut self, seconds: f64) -> Self {
        self.settle_time = seconds;
        self
    }

    /// Calculate voltage at a given point index.
    fn voltage_at(&self, point: usize) -> f64 {
        if self.num_points <= 1 {
            self.start_v
        } else {
            let step = (self.stop_v - self.start_v) / (self.num_points - 1) as f64;
            self.start_v + step * point as f64
        }
    }

    /// Get all AI devices (primary + extra).
    fn all_ai_devices(&self) -> Vec<String> {
        let mut all = vec![self.ai_device.clone()];
        all.extend(self.extra_ai.clone());
        all
    }
}

impl Plan for VoltageScan {
    fn plan_type(&self) -> &str {
        "voltage_scan"
    }

    fn plan_name(&self) -> &str {
        "Voltage Scan"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        let mut args = HashMap::new();
        args.insert("ao_device".to_string(), self.ao_device.clone());
        args.insert("ai_device".to_string(), self.ai_device.clone());
        args.insert("start_v".to_string(), self.start_v.to_string());
        args.insert("stop_v".to_string(), self.stop_v.to_string());
        args.insert("num_points".to_string(), self.num_points.to_string());
        args.insert("settle_time".to_string(), self.settle_time.to_string());
        args
    }

    fn movers(&self) -> Vec<String> {
        // AO is treated as a "mover" in the scan
        vec![self.ao_device.clone()]
    }

    fn detectors(&self) -> Vec<String> {
        self.all_ai_devices()
    }

    fn num_points(&self) -> usize {
        self.num_points
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        if self.current_point >= self.num_points {
            return None;
        }

        let cmd = match self.current_step {
            VoltageScanStep::SetVoltage => {
                let voltage = self.voltage_at(self.current_point);
                self.current_step = if self.settle_time > 0.0 {
                    VoltageScanStep::Settle
                } else {
                    VoltageScanStep::Checkpoint
                };

                // Use Set command for AO voltage
                PlanCommand::Set {
                    device_id: self.ao_device.clone(),
                    parameter: "voltage".to_string(),
                    value: voltage.to_string(),
                }
            }

            VoltageScanStep::Settle => {
                self.current_step = VoltageScanStep::Checkpoint;
                PlanCommand::Wait {
                    seconds: self.settle_time,
                }
            }

            VoltageScanStep::Checkpoint => {
                self.current_step = VoltageScanStep::ReadAI { ai_idx: 0 };
                PlanCommand::Checkpoint {
                    label: format!("point_{}", self.current_point),
                }
            }

            VoltageScanStep::ReadAI { ai_idx } => {
                let all_ai = self.all_ai_devices();
                if ai_idx < all_ai.len() {
                    let ai_device = &all_ai[ai_idx];
                    self.current_step = VoltageScanStep::ReadAI { ai_idx: ai_idx + 1 };
                    PlanCommand::Read {
                        device_id: ai_device.clone(),
                    }
                } else {
                    self.current_step = VoltageScanStep::EmitEvent;
                    return self.next_command();
                }
            }

            VoltageScanStep::EmitEvent => {
                let voltage = self.voltage_at(self.current_point);
                let mut positions = HashMap::new();
                positions.insert(self.ao_device.clone(), voltage);

                self.current_point += 1;
                self.current_step = VoltageScanStep::SetVoltage;

                PlanCommand::EmitEvent {
                    stream: "primary".to_string(),
                    data: HashMap::new(), // Filled by RunEngine from reads
                    positions,
                }
            }
        };

        Some(cmd)
    }

    fn reset(&mut self) {
        self.current_point = 0;
        self.current_step = VoltageScanStep::SetVoltage;
    }
}

// =============================================================================
// Time Series Plan
// =============================================================================

/// Time series plan - continuous logging at fixed sample rate.
///
/// This plan reads one or more analog input channels at a specified
/// sample rate for a given duration. Useful for monitoring and
/// waveform capture.
///
/// # Example
///
/// ```rust,ignore
/// // Log AI0 at 1kHz for 10 seconds (10,000 samples)
/// let series = TimeSeries::new("ai0", 1000.0, 10.0)
///     .with_channel("ai1")  // Also log AI1
///     .with_channel("ai2"); // And AI2
///
/// engine.queue(Box::new(series)).await;
/// engine.start().await?;
/// ```
#[derive(Debug, Clone)]
pub struct TimeSeries {
    /// Primary AI device
    ai_device: String,
    /// Additional AI channels
    extra_ai: Vec<String>,
    /// Sample rate in Hz
    sample_rate: f64,
    /// Total duration in seconds
    duration: f64,
    /// Total number of samples (computed)
    num_samples: usize,

    // Execution state
    current_sample: usize,
    current_step: TimeSeriesStep,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TimeSeriesStep {
    Checkpoint,
    ReadAI { ai_idx: usize },
    EmitEvent,
    Wait,
}

impl TimeSeries {
    /// Create a new time series plan.
    ///
    /// # Arguments
    ///
    /// * `ai_device` - Primary analog input device ID
    /// * `sample_rate` - Sample rate in Hz
    /// * `duration` - Total duration in seconds
    pub fn new(ai_device: &str, sample_rate: f64, duration: f64) -> Self {
        let num_samples = (sample_rate * duration).ceil() as usize;
        Self {
            ai_device: ai_device.to_string(),
            extra_ai: Vec::new(),
            sample_rate,
            duration,
            num_samples,
            current_sample: 0,
            current_step: TimeSeriesStep::Checkpoint,
        }
    }

    /// Add an additional AI channel.
    pub fn with_channel(mut self, ai_device: &str) -> Self {
        self.extra_ai.push(ai_device.to_string());
        self
    }

    /// Add multiple additional AI channels.
    pub fn with_channels(mut self, ai_devices: &[&str]) -> Self {
        self.extra_ai
            .extend(ai_devices.iter().map(|s| s.to_string()));
        self
    }

    /// Get all AI devices.
    fn all_ai_devices(&self) -> Vec<String> {
        let mut all = vec![self.ai_device.clone()];
        all.extend(self.extra_ai.clone());
        all
    }

    /// Sample period in seconds.
    fn sample_period(&self) -> f64 {
        1.0 / self.sample_rate
    }
}

impl Plan for TimeSeries {
    fn plan_type(&self) -> &str {
        "time_series"
    }

    fn plan_name(&self) -> &str {
        "Time Series"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        let mut args = HashMap::new();
        args.insert("ai_device".to_string(), self.ai_device.clone());
        args.insert("sample_rate".to_string(), self.sample_rate.to_string());
        args.insert("duration".to_string(), self.duration.to_string());
        args.insert("num_samples".to_string(), self.num_samples.to_string());
        args
    }

    fn movers(&self) -> Vec<String> {
        Vec::new() // No movers in time series
    }

    fn detectors(&self) -> Vec<String> {
        self.all_ai_devices()
    }

    fn num_points(&self) -> usize {
        self.num_samples
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        if self.current_sample >= self.num_samples {
            return None;
        }

        let cmd = match self.current_step {
            TimeSeriesStep::Checkpoint => {
                self.current_step = TimeSeriesStep::ReadAI { ai_idx: 0 };
                PlanCommand::Checkpoint {
                    label: format!("sample_{}", self.current_sample),
                }
            }

            TimeSeriesStep::ReadAI { ai_idx } => {
                let all_ai = self.all_ai_devices();
                if ai_idx < all_ai.len() {
                    let ai_device = &all_ai[ai_idx];
                    self.current_step = TimeSeriesStep::ReadAI { ai_idx: ai_idx + 1 };
                    PlanCommand::Read {
                        device_id: ai_device.clone(),
                    }
                } else {
                    self.current_step = TimeSeriesStep::EmitEvent;
                    return self.next_command();
                }
            }

            TimeSeriesStep::EmitEvent => {
                let timestamp = self.current_sample as f64 / self.sample_rate;
                let mut positions = HashMap::new();
                positions.insert("time".to_string(), timestamp);

                self.current_sample += 1;

                // Wait between samples if not done
                self.current_step = if self.current_sample < self.num_samples {
                    TimeSeriesStep::Wait
                } else {
                    TimeSeriesStep::Checkpoint // Will exit on next iteration
                };

                PlanCommand::EmitEvent {
                    stream: "primary".to_string(),
                    data: HashMap::new(),
                    positions,
                }
            }

            TimeSeriesStep::Wait => {
                self.current_step = TimeSeriesStep::Checkpoint;
                PlanCommand::Wait {
                    seconds: self.sample_period(),
                }
            }
        };

        Some(cmd)
    }

    fn reset(&mut self) {
        self.current_sample = 0;
        self.current_step = TimeSeriesStep::Checkpoint;
    }
}

// =============================================================================
// Triggered Acquisition Plan
// =============================================================================

/// Triggered acquisition plan - hardware-triggered multi-channel reading.
///
/// This plan waits for hardware triggers and reads multiple AI channels
/// on each trigger event. Useful for synchronized acquisition with
/// external events like laser pulses or encoder signals.
///
/// # Example
///
/// ```rust,ignore
/// // Read AI0-AI3 on 100 PFI0 triggers
/// let triggered = TriggeredAcquisition::new()
///     .with_channels(&["ai0", "ai1", "ai2", "ai3"])
///     .with_trigger("pfi0")
///     .with_num_triggers(100)
///     .with_timeout(30.0);  // 30 second timeout
///
/// engine.queue(Box::new(triggered)).await;
/// engine.start().await?;
/// ```
#[derive(Debug, Clone)]
pub struct TriggeredAcquisition {
    /// AI channels to read on each trigger
    ai_channels: Vec<String>,
    /// Trigger source device/signal
    trigger_source: String,
    /// Number of trigger events to capture
    num_triggers: usize,
    /// Timeout per trigger in seconds
    trigger_timeout: f64,

    // Execution state
    current_trigger: usize,
    current_step: TriggeredAcqStep,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TriggeredAcqStep {
    WaitTrigger,
    Checkpoint,
    ReadAI { ai_idx: usize },
    EmitEvent,
}

impl TriggeredAcquisition {
    /// Create a new triggered acquisition plan.
    pub fn new() -> Self {
        Self {
            ai_channels: Vec::new(),
            trigger_source: "external".to_string(),
            num_triggers: 1,
            trigger_timeout: 10.0,
            current_trigger: 0,
            current_step: TriggeredAcqStep::WaitTrigger,
        }
    }

    /// Add an AI channel to read on each trigger.
    pub fn with_channel(mut self, ai_device: &str) -> Self {
        self.ai_channels.push(ai_device.to_string());
        self
    }

    /// Add multiple AI channels.
    pub fn with_channels(mut self, ai_devices: &[&str]) -> Self {
        self.ai_channels
            .extend(ai_devices.iter().map(|s| s.to_string()));
        self
    }

    /// Set the trigger source.
    pub fn with_trigger(mut self, trigger_source: &str) -> Self {
        self.trigger_source = trigger_source.to_string();
        self
    }

    /// Set the number of triggers to capture.
    pub fn with_num_triggers(mut self, n: usize) -> Self {
        self.num_triggers = n;
        self
    }

    /// Set timeout per trigger in seconds.
    pub fn with_timeout(mut self, seconds: f64) -> Self {
        self.trigger_timeout = seconds;
        self
    }
}

impl Default for TriggeredAcquisition {
    fn default() -> Self {
        Self::new()
    }
}

impl Plan for TriggeredAcquisition {
    fn plan_type(&self) -> &str {
        "triggered_acquisition"
    }

    fn plan_name(&self) -> &str {
        "Triggered Acquisition"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        let mut args = HashMap::new();
        args.insert("ai_channels".to_string(), self.ai_channels.join(","));
        args.insert("trigger_source".to_string(), self.trigger_source.clone());
        args.insert("num_triggers".to_string(), self.num_triggers.to_string());
        args.insert(
            "trigger_timeout".to_string(),
            self.trigger_timeout.to_string(),
        );
        args
    }

    fn movers(&self) -> Vec<String> {
        Vec::new()
    }

    fn detectors(&self) -> Vec<String> {
        self.ai_channels.clone()
    }

    fn num_points(&self) -> usize {
        self.num_triggers
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        if self.current_trigger >= self.num_triggers {
            return None;
        }

        let cmd = match self.current_step {
            TriggeredAcqStep::WaitTrigger => {
                // Trigger command tells the RunEngine to wait for hardware trigger
                // The RunEngine will call device.trigger() which blocks until trigger arrives
                self.current_step = TriggeredAcqStep::Checkpoint;
                PlanCommand::Trigger {
                    device_id: self.trigger_source.clone(),
                }
            }

            TriggeredAcqStep::Checkpoint => {
                self.current_step = TriggeredAcqStep::ReadAI { ai_idx: 0 };
                PlanCommand::Checkpoint {
                    label: format!("trigger_{}", self.current_trigger),
                }
            }

            TriggeredAcqStep::ReadAI { ai_idx } => {
                if ai_idx < self.ai_channels.len() {
                    let ai_device = &self.ai_channels[ai_idx];
                    self.current_step = TriggeredAcqStep::ReadAI { ai_idx: ai_idx + 1 };
                    PlanCommand::Read {
                        device_id: ai_device.clone(),
                    }
                } else {
                    self.current_step = TriggeredAcqStep::EmitEvent;
                    return self.next_command();
                }
            }

            TriggeredAcqStep::EmitEvent => {
                let mut positions = HashMap::new();
                positions.insert("trigger_num".to_string(), self.current_trigger as f64);

                self.current_trigger += 1;
                self.current_step = TriggeredAcqStep::WaitTrigger;

                PlanCommand::EmitEvent {
                    stream: "primary".to_string(),
                    data: HashMap::new(),
                    positions,
                }
            }
        };

        Some(cmd)
    }

    fn reset(&mut self) {
        self.current_trigger = 0;
        self.current_step = TriggeredAcqStep::WaitTrigger;
    }
}

// =============================================================================
// Plan Builders
// =============================================================================

use crate::plans::PlanBuilder;

/// Builder for VoltageScan plans
pub struct VoltageScanBuilder;

impl PlanBuilder for VoltageScanBuilder {
    fn build(
        &self,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String> {
        let ao_device = device_mapping
            .get("ao_device")
            .ok_or("Missing device mapping: ao_device")?;
        let ai_device = device_mapping
            .get("ai_device")
            .ok_or("Missing device mapping: ai_device")?;

        let start_v = parameters
            .get("start_v")
            .ok_or("Missing parameter: start_v")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid start_v: {}", e))?;

        let stop_v = parameters
            .get("stop_v")
            .ok_or("Missing parameter: stop_v")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid stop_v: {}", e))?;

        let num_points = parameters
            .get("num_points")
            .ok_or("Missing parameter: num_points")?
            .parse::<usize>()
            .map_err(|e| format!("Invalid num_points: {}", e))?;

        // Validation
        if !start_v.is_finite() {
            return Err("start_v must be finite".to_string());
        }
        if !stop_v.is_finite() {
            return Err("stop_v must be finite".to_string());
        }
        if num_points == 0 {
            return Err("num_points must be > 0".to_string());
        }
        if num_points > 1_000_000 {
            return Err("num_points must be <= 1,000,000".to_string());
        }

        let mut plan = VoltageScan::new(ao_device, ai_device, start_v, stop_v, num_points);

        if let Some(settle) = parameters.get("settle_time") {
            let settle = settle
                .parse::<f64>()
                .map_err(|e| format!("Invalid settle_time: {}", e))?;
            plan = plan.with_settle_time(settle);
        }

        Ok(Box::new(plan))
    }

    fn description(&self) -> String {
        "Scan AO voltage while reading AI".to_string()
    }

    fn categories(&self) -> Vec<String> {
        vec!["scanning".to_string(), "1d".to_string(), "daq".to_string()]
    }
}

/// Builder for TimeSeries plans
pub struct TimeSeriesBuilder;

impl PlanBuilder for TimeSeriesBuilder {
    fn build(
        &self,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String> {
        let ai_device = device_mapping
            .get("ai_device")
            .ok_or("Missing device mapping: ai_device")?;

        let sample_rate = parameters
            .get("sample_rate")
            .ok_or("Missing parameter: sample_rate")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid sample_rate: {}", e))?;

        let duration = parameters
            .get("duration")
            .ok_or("Missing parameter: duration")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid duration: {}", e))?;

        // Validation
        if sample_rate <= 0.0 {
            return Err("sample_rate must be > 0".to_string());
        }
        if sample_rate > 1_000_000.0 {
            return Err("sample_rate must be <= 1MHz".to_string());
        }
        if duration <= 0.0 {
            return Err("duration must be > 0".to_string());
        }
        if duration > 86400.0 {
            return Err("duration must be <= 24 hours".to_string());
        }

        let plan = TimeSeries::new(ai_device, sample_rate, duration);

        Ok(Box::new(plan))
    }

    fn description(&self) -> String {
        "Continuous AI logging at fixed sample rate".to_string()
    }

    fn categories(&self) -> Vec<String> {
        vec![
            "time_series".to_string(),
            "1d".to_string(),
            "daq".to_string(),
        ]
    }
}

/// Builder for TriggeredAcquisition plans
pub struct TriggeredAcquisitionBuilder;

impl PlanBuilder for TriggeredAcquisitionBuilder {
    fn build(
        &self,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String> {
        let mut plan = TriggeredAcquisition::new();

        // Get AI channels from device mapping
        if let Some(channels) = device_mapping.get("ai_channels") {
            for ch in channels.split(',') {
                let ch = ch.trim();
                if !ch.is_empty() {
                    plan = plan.with_channel(ch);
                }
            }
        }

        // Get trigger source
        if let Some(trigger) = device_mapping.get("trigger_source") {
            plan = plan.with_trigger(trigger);
        }

        // Get number of triggers
        if let Some(n) = parameters.get("num_triggers") {
            let n = n
                .parse::<usize>()
                .map_err(|e| format!("Invalid num_triggers: {}", e))?;
            if n == 0 {
                return Err("num_triggers must be > 0".to_string());
            }
            if n > 10_000_000 {
                return Err("num_triggers must be <= 10,000,000".to_string());
            }
            plan = plan.with_num_triggers(n);
        }

        // Get timeout
        if let Some(timeout) = parameters.get("trigger_timeout") {
            let timeout = timeout
                .parse::<f64>()
                .map_err(|e| format!("Invalid trigger_timeout: {}", e))?;
            plan = plan.with_timeout(timeout);
        }

        Ok(Box::new(plan))
    }

    fn description(&self) -> String {
        "Hardware-triggered multi-channel acquisition".to_string()
    }

    fn categories(&self) -> Vec<String> {
        vec!["triggered".to_string(), "0d".to_string(), "daq".to_string()]
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voltage_scan_positions() {
        let mut plan = VoltageScan::new("ao0", "ai0", 0.0, 10.0, 11);

        let mut voltages = Vec::new();
        while let Some(cmd) = plan.next_command() {
            if let PlanCommand::Set { value, .. } = cmd {
                voltages.push(value.parse::<f64>().unwrap());
            }
        }

        assert_eq!(voltages.len(), 11);
        assert!((voltages[0] - 0.0).abs() < 1e-10);
        assert!((voltages[5] - 5.0).abs() < 1e-10);
        assert!((voltages[10] - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_voltage_scan_events() {
        let mut plan = VoltageScan::new("ao0", "ai0", 0.0, 5.0, 6);

        let mut events = 0;
        while let Some(cmd) = plan.next_command() {
            if matches!(cmd, PlanCommand::EmitEvent { .. }) {
                events += 1;
            }
        }

        assert_eq!(events, 6);
    }

    #[test]
    fn test_time_series_samples() {
        let mut plan = TimeSeries::new("ai0", 10.0, 1.0); // 10 Hz for 1 second = 10 samples

        let mut events = 0;
        while let Some(cmd) = plan.next_command() {
            if matches!(cmd, PlanCommand::EmitEvent { .. }) {
                events += 1;
            }
        }

        assert_eq!(events, 10);
    }

    #[test]
    fn test_time_series_reset() {
        let mut plan = TimeSeries::new("ai0", 100.0, 0.1); // 10 samples

        // Run through once
        while plan.next_command().is_some() {}

        // Reset and run again
        plan.reset();
        let mut events = 0;
        while let Some(cmd) = plan.next_command() {
            if matches!(cmd, PlanCommand::EmitEvent { .. }) {
                events += 1;
            }
        }

        assert_eq!(events, 10);
    }

    #[test]
    fn test_triggered_acquisition_events() {
        let mut plan = TriggeredAcquisition::new()
            .with_channels(&["ai0", "ai1"])
            .with_num_triggers(5);

        let mut events = 0;
        let mut triggers = 0;
        while let Some(cmd) = plan.next_command() {
            match cmd {
                PlanCommand::EmitEvent { .. } => events += 1,
                PlanCommand::Trigger { .. } => triggers += 1,
                _ => {}
            }
        }

        assert_eq!(events, 5);
        assert_eq!(triggers, 5);
    }

    #[test]
    fn test_plan_reset() {
        let mut plan = VoltageScan::new("ao0", "ai0", 0.0, 1.0, 3);

        while plan.next_command().is_some() {}
        assert_eq!(plan.num_points(), 3); // num_points unchanged

        plan.reset();

        let mut count = 0;
        while let Some(cmd) = plan.next_command() {
            if matches!(cmd, PlanCommand::EmitEvent { .. }) {
                count += 1;
            }
        }
        assert_eq!(count, 3);
    }
}
