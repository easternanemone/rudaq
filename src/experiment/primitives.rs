//! Common experiment plan primitives.
//!
//! This module provides ready-to-use plan implementations for common experiment
//! patterns: time series acquisition, 1D scans, 2D grid scans, and adaptive sampling.

use super::plan::{LogLevel, Message, Plan, PlanStream};
use anyhow::Result;
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

/// Time series data acquisition plan.
///
/// Collects data from a module at regular intervals for a specified duration.
/// This is one of the most common experiment patterns.
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::experiment::{TimeSeriesPlan, RunEngine};
/// use std::time::Duration;
///
/// // Collect data every 1 second for 60 seconds
/// let plan = TimeSeriesPlan::new(
///     "power_meter".to_string(),
///     Duration::from_secs(60),
///     Duration::from_secs(1),
/// );
///
/// engine.run(Box::new(plan)).await?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPlan {
    /// Module to collect data from
    pub module_id: String,
    /// Total acquisition duration
    pub duration: Duration,
    /// Sampling interval
    pub interval: Duration,
    /// Current step (for resumption)
    #[serde(skip)]
    current_step: usize,
}

impl TimeSeriesPlan {
    /// Create a new time series plan.
    ///
    /// # Arguments
    ///
    /// * `module_id` - ID of module to collect data from
    /// * `duration` - Total acquisition duration
    /// * `interval` - Time between samples
    pub fn new(module_id: String, duration: Duration, interval: Duration) -> Self {
        Self {
            module_id,
            duration,
            interval,
            current_step: 0,
        }
    }

    /// Get the total number of steps (samples).
    pub fn total_steps(&self) -> usize {
        (self.duration.as_secs_f64() / self.interval.as_secs_f64()).ceil() as usize
    }
}

impl Plan for TimeSeriesPlan {
    fn execute(&mut self) -> PlanStream<'_> {
        let module_id = self.module_id.clone();
        let total_steps = self.total_steps();
        let interval_secs = self.interval.as_secs_f64();
        let start_step = self.current_step;

        let mut metadata = HashMap::new();
        metadata.insert("experiment_type".to_string(), "time_series".to_string());
        metadata.insert("module".to_string(), module_id.clone());
        metadata.insert("total_steps".to_string(), total_steps.to_string());
        metadata.insert(
            "interval".to_string(),
            format!("{:.2}s", interval_secs),
        );

        Box::pin(stream::iter((start_step..total_steps).flat_map(
            move |step| {
                let module_id = module_id.clone();
                let metadata = metadata.clone();

                let mut messages = Vec::new();

                // First step: BeginRun
                if step == 0 {
                    messages.push(Ok(Message::BeginRun {
                        metadata: metadata.clone(),
                    }));
                }

                // Log progress every 10 steps
                if step % 10 == 0 {
                    messages.push(Ok(Message::Log {
                        level: LogLevel::Info,
                        message: format!("Time series: step {}/{}", step + 1, total_steps),
                    }));
                }

                // Trigger and read
                messages.push(Ok(Message::Trigger {
                    module_id: module_id.clone(),
                }));
                messages.push(Ok(Message::Read {
                    module_id: module_id.clone(),
                }));

                // Sleep until next sample
                if step < total_steps - 1 {
                    messages.push(Ok(Message::Sleep {
                        duration_secs: interval_secs,
                    }));
                }

                // Checkpoint every 100 steps
                if (step + 1) % 100 == 0 {
                    messages.push(Ok(Message::Checkpoint {
                        label: Some(format!("step_{}", step + 1)),
                    }));
                }

                // Last step: EndRun
                if step == total_steps - 1 {
                    messages.push(Ok(Message::EndRun));
                }

                messages
            },
        )))
    }

    fn metadata(&self) -> (String, String) {
        (
            format!("Time Series: {}", self.module_id),
            format!(
                "{} samples @ {:.2}s interval",
                self.total_steps(),
                self.interval.as_secs_f64()
            ),
        )
    }
}

/// 1D scan plan.
///
/// Sweeps a module parameter across a range and collects data at each point.
/// Useful for optimization, calibration, and characterization experiments.
///
/// # Example
///
/// ```rust,ignore
/// // Scan laser power from 0 to 100 mW in 21 steps
/// let plan = ScanPlan::new(
///     "laser".to_string(),
///     "power".to_string(),
///     0.0,
///     100.0,
///     21,
///     "power_meter".to_string(),
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanPlan {
    /// Module or instrument to control
    pub actuator_id: String,
    /// Parameter to scan
    pub parameter: String,
    /// Start value
    pub start: f64,
    /// End value
    pub end: f64,
    /// Number of points
    pub num_points: usize,
    /// Module to collect data from
    pub detector_id: String,
    /// Current step (for resumption)
    #[serde(skip)]
    current_step: usize,
}

impl ScanPlan {
    /// Create a new 1D scan plan.
    pub fn new(
        actuator_id: String,
        parameter: String,
        start: f64,
        end: f64,
        num_points: usize,
        detector_id: String,
    ) -> Self {
        Self {
            actuator_id,
            parameter,
            start,
            end,
            num_points,
            detector_id,
            current_step: 0,
        }
    }

    /// Get the value at a given step.
    fn value_at_step(&self, step: usize) -> f64 {
        let fraction = step as f64 / (self.num_points - 1).max(1) as f64;
        self.start + fraction * (self.end - self.start)
    }
}

impl Plan for ScanPlan {
    fn execute(&mut self) -> PlanStream<'_> {
        let actuator_id = self.actuator_id.clone();
        let detector_id = self.detector_id.clone();
        let parameter = self.parameter.clone();
        let num_points = self.num_points;
        let start = self.start;
        let end = self.end;
        let start_step = self.current_step;

        let mut metadata = HashMap::new();
        metadata.insert("experiment_type".to_string(), "scan".to_string());
        metadata.insert("actuator".to_string(), actuator_id.clone());
        metadata.insert("parameter".to_string(), parameter.clone());
        metadata.insert("start".to_string(), start.to_string());
        metadata.insert("end".to_string(), end.to_string());
        metadata.insert("num_points".to_string(), num_points.to_string());

        Box::pin(stream::iter((start_step..num_points).flat_map(
            move |step| {
                let actuator_id = actuator_id.clone();
                let detector_id = detector_id.clone();
                let parameter = parameter.clone();
                let metadata = metadata.clone();
                let value = start + (step as f64 / (num_points - 1).max(1) as f64) * (end - start);

                let mut messages = Vec::new();

                // First step: BeginRun
                if step == 0 {
                    messages.push(Ok(Message::BeginRun {
                        metadata: metadata.clone(),
                    }));
                }

                // Set parameter
                messages.push(Ok(Message::Set {
                    target: actuator_id.clone(),
                    param: parameter.clone(),
                    value: value.to_string(),
                }));

                // Log progress
                messages.push(Ok(Message::Log {
                    level: LogLevel::Info,
                    message: format!(
                        "Scan: step {}/{}, {} = {:.3}",
                        step + 1,
                        num_points,
                        parameter,
                        value
                    ),
                }));

                // Wait for settling (0.1s)
                messages.push(Ok(Message::Sleep {
                    duration_secs: 0.1,
                }));

                // Trigger and read
                messages.push(Ok(Message::Trigger {
                    module_id: detector_id.clone(),
                }));
                messages.push(Ok(Message::Read {
                    module_id: detector_id.clone(),
                }));

                // Checkpoint every 10 steps
                if (step + 1) % 10 == 0 {
                    messages.push(Ok(Message::Checkpoint {
                        label: Some(format!("step_{}", step + 1)),
                    }));
                }

                // Last step: EndRun
                if step == num_points - 1 {
                    messages.push(Ok(Message::EndRun));
                }

                messages
            },
        )))
    }

    fn metadata(&self) -> (String, String) {
        (
            format!("1D Scan: {}.{}", self.actuator_id, self.parameter),
            format!(
                "{} points from {:.2} to {:.2}",
                self.num_points, self.start, self.end
            ),
        )
    }
}

/// 2D grid scan plan.
///
/// Sweeps two module parameters across ranges in a grid pattern and collects
/// data at each point. Useful for 2D optimization and surface mapping.
///
/// # Example
///
/// ```rust,ignore
/// // Scan stage X and Y positions
/// let plan = GridScanPlan::new(
///     "stage".to_string(),
///     "x_position".to_string(),
///     0.0,
///     10.0,
///     11,
///     "y_position".to_string(),
///     0.0,
///     10.0,
///     11,
///     "camera".to_string(),
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridScanPlan {
    /// Module or instrument to control
    pub actuator_id: String,
    /// First parameter to scan (outer loop)
    pub param1: String,
    /// First parameter start value
    pub start1: f64,
    /// First parameter end value
    pub end1: f64,
    /// First parameter number of points
    pub num1: usize,
    /// Second parameter to scan (inner loop)
    pub param2: String,
    /// Second parameter start value
    pub start2: f64,
    /// Second parameter end value
    pub end2: f64,
    /// Second parameter number of points
    pub num2: usize,
    /// Module to collect data from
    pub detector_id: String,
    /// Current step (for resumption)
    #[serde(skip)]
    current_step: usize,
}

impl GridScanPlan {
    /// Create a new 2D grid scan plan.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        actuator_id: String,
        param1: String,
        start1: f64,
        end1: f64,
        num1: usize,
        param2: String,
        start2: f64,
        end2: f64,
        num2: usize,
        detector_id: String,
    ) -> Self {
        Self {
            actuator_id,
            param1,
            start1,
            end1,
            num1,
            param2,
            start2,
            end2,
            num2,
            detector_id,
            current_step: 0,
        }
    }

    /// Get total number of points.
    pub fn total_points(&self) -> usize {
        self.num1 * self.num2
    }
}

impl Plan for GridScanPlan {
    fn execute(&mut self) -> PlanStream<'_> {
        let actuator_id = self.actuator_id.clone();
        let detector_id = self.detector_id.clone();
        let param1 = self.param1.clone();
        let param2 = self.param2.clone();
        let start1 = self.start1;
        let end1 = self.end1;
        let num1 = self.num1;
        let start2 = self.start2;
        let end2 = self.end2;
        let num2 = self.num2;
        let total_points = self.total_points();
        let start_step = self.current_step;

        let mut metadata = HashMap::new();
        metadata.insert("experiment_type".to_string(), "grid_scan".to_string());
        metadata.insert("actuator".to_string(), actuator_id.clone());
        metadata.insert("param1".to_string(), param1.clone());
        metadata.insert("param2".to_string(), param2.clone());
        metadata.insert("total_points".to_string(), total_points.to_string());

        Box::pin(stream::iter((start_step..total_points).flat_map(
            move |step| {
                let actuator_id = actuator_id.clone();
                let detector_id = detector_id.clone();
                let param1 = param1.clone();
                let param2 = param2.clone();
                let metadata = metadata.clone();

                // Convert linear step to (i, j) grid coordinates
                let i = step / num2;
                let j = step % num2;

                let value1 = start1 + (i as f64 / (num1 - 1).max(1) as f64) * (end1 - start1);
                let value2 = start2 + (j as f64 / (num2 - 1).max(1) as f64) * (end2 - start2);

                let mut messages = Vec::new();

                // First step: BeginRun
                if step == 0 {
                    messages.push(Ok(Message::BeginRun {
                        metadata: metadata.clone(),
                    }));
                }

                // Set both parameters
                messages.push(Ok(Message::Set {
                    target: actuator_id.clone(),
                    param: param1.clone(),
                    value: value1.to_string(),
                }));
                messages.push(Ok(Message::Set {
                    target: actuator_id.clone(),
                    param: param2.clone(),
                    value: value2.to_string(),
                }));

                // Log progress
                messages.push(Ok(Message::Log {
                    level: LogLevel::Info,
                    message: format!(
                        "Grid scan: point {}/{} ({}, {})",
                        step + 1,
                        total_points,
                        i,
                        j
                    ),
                }));

                // Wait for settling
                messages.push(Ok(Message::Sleep {
                    duration_secs: 0.1,
                }));

                // Trigger and read
                messages.push(Ok(Message::Trigger {
                    module_id: detector_id.clone(),
                }));
                messages.push(Ok(Message::Read {
                    module_id: detector_id.clone(),
                }));

                // Checkpoint every row
                if (step + 1) % num2 == 0 {
                    messages.push(Ok(Message::Checkpoint {
                        label: Some(format!("row_{}", i + 1)),
                    }));
                }

                // Last step: EndRun
                if step == total_points - 1 {
                    messages.push(Ok(Message::EndRun));
                }

                messages
            },
        )))
    }

    fn metadata(&self) -> (String, String) {
        (
            format!(
                "2D Grid Scan: {}.{} × {}",
                self.actuator_id, self.param1, self.param2
            ),
            format!("{} × {} = {} points", self.num1, self.num2, self.total_points()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::pin_mut;

    #[tokio::test]
    async fn test_time_series_plan() {
        let mut plan = TimeSeriesPlan::new(
            "test_module".to_string(),
            Duration::from_secs(5),
            Duration::from_secs(1),
        );

        assert_eq!(plan.total_steps(), 5);

        let stream = plan.execute();
        pin_mut!(stream);

        let mut message_count = 0;
        let mut has_begin = false;
        let mut has_end = false;

        while let Some(Ok(message)) = stream.next().await {
            message_count += 1;
            match message {
                Message::BeginRun { .. } => has_begin = true,
                Message::EndRun => has_end = true,
                _ => {}
            }
        }

        assert!(has_begin, "Plan should emit BeginRun");
        assert!(has_end, "Plan should emit EndRun");
        assert!(message_count > 0, "Plan should emit messages");
    }

    #[tokio::test]
    async fn test_scan_plan() {
        let mut plan = ScanPlan::new(
            "laser".to_string(),
            "power".to_string(),
            0.0,
            100.0,
            11,
            "detector".to_string(),
        );

        assert_eq!(plan.num_points, 11);
        assert_eq!(plan.value_at_step(0), 0.0);
        assert_eq!(plan.value_at_step(10), 100.0);
        assert!((plan.value_at_step(5) - 50.0).abs() < 1e-10);

        let (name, desc) = plan.metadata();
        assert!(name.contains("laser"));
        assert!(desc.contains("11"));
    }

    #[tokio::test]
    async fn test_grid_scan_plan() {
        let plan = GridScanPlan::new(
            "stage".to_string(),
            "x".to_string(),
            0.0,
            10.0,
            3,
            "y".to_string(),
            0.0,
            5.0,
            2,
            "camera".to_string(),
        );

        assert_eq!(plan.total_points(), 6); // 3 × 2

        let (name, desc) = plan.metadata();
        assert!(name.contains("Grid Scan"));
        assert!(desc.contains("3 × 2"));
    }
}
