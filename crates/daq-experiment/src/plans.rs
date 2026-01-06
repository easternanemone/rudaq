//! Plan system for declarative experiment definitions (bd-73yh.2)
//!
//! Plans are declarative generators that yield commands for the RunEngine to execute.
//! Inspired by Bluesky's plan protocol, plans don't execute directly—they describe
//! what should happen, and the RunEngine orchestrates execution.
//!
//! # Plan Commands
//!
//! Plans yield a sequence of `PlanCommand` values:
//! - `MoveTo` - Move a device to a position
//! - `Read` - Read a value from a device
//! - `Trigger` - Trigger a device (e.g., start acquisition)
//! - `Wait` - Wait for a duration
//! - `Checkpoint` - Mark a pause/resume point
//! - `EmitEvent` - Record data in an EventDoc
//!
//! # Example Plan
//!
//! ```rust,ignore
//! let plan = LineScan::new("stage_x", 0.0, 10.0, 11)
//!     .with_detector("power_meter")
//!     .build();
//!
//! // Plan yields commands like:
//! // MoveTo("stage_x", 0.0)
//! // Checkpoint
//! // Trigger("power_meter")
//! // Read("power_meter")
//! // EmitEvent { power: 0.042 }
//! // MoveTo("stage_x", 1.0)
//! // ...
//! ```

use std::collections::HashMap;

/// Commands that plans yield for the RunEngine to execute
#[derive(Debug, Clone)]
pub enum PlanCommand {
    /// Move a device to an absolute position
    MoveTo {
        /// Device ID to move
        device_id: String,
        /// Target position
        position: f64,
    },
    /// Read a value from a device
    Read {
        /// Device to read
        device_id: String,
    },
    /// Trigger a device (e.g., start camera acquisition)
    Trigger {
        /// Device to trigger
        device_id: String,
    },
    /// Wait for a duration in seconds
    Wait {
        /// Duration in seconds
        seconds: f64,
    },
    /// Checkpoint - safe point for pause/resume
    Checkpoint {
        /// Checkpoint label
        label: String,
    },
    /// Emit an event document with collected data
    EmitEvent {
        /// Stream name (e.g., "primary")
        stream: String,
        /// Data collected (key -> value)
        data: HashMap<String, f64>,
        /// Device positions at time of event
        positions: HashMap<String, f64>,
    },
    /// Set a device parameter
    Set {
        /// Device to set
        device_id: String,
        /// Parameter name
        parameter: String,
        /// Value to set
        value: String,
    },
}

/// Plan trait - all plans implement this to generate commands
pub trait Plan: Send + Sync {
    /// Plan type identifier (e.g., "line_scan", "grid_scan")
    fn plan_type(&self) -> &str;

    /// Human-readable plan name
    fn plan_name(&self) -> &str;

    /// Plan arguments for documentation
    fn plan_args(&self) -> HashMap<String, String>;

    /// Devices that will be moved (for hints)
    fn movers(&self) -> Vec<String>;

    /// Devices that will be read (for hints)
    fn detectors(&self) -> Vec<String>;

    /// Total number of points in the scan
    fn num_points(&self) -> usize;

    /// Generate the next command, returning None when complete
    fn next_command(&mut self) -> Option<PlanCommand>;

    /// Reset the plan to start from the beginning
    fn reset(&mut self);
}

/// Line scan - scan a single axis with one or more detectors
#[derive(Debug, Clone)]
pub struct LineScan {
    axis: String,
    start: f64,
    stop: f64,
    num_points: usize,
    detectors: Vec<String>,
    settle_time: f64,

    // Execution state
    current_point: usize,
    current_step: LineScanStep,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum LineScanStep {
    Move,
    Settle,
    Checkpoint,
    TriggerDetectors,
    ReadDetectors { detector_idx: usize },
    EmitEvent,
    Done,
}

impl LineScan {
    /// Create a new LineScan
    pub fn new(axis: &str, start: f64, stop: f64, num_points: usize) -> Self {
        Self {
            axis: axis.to_string(),
            start,
            stop,
            num_points,
            detectors: Vec::new(),
            settle_time: 0.0,
            current_point: 0,
            current_step: LineScanStep::Move,
        }
    }

    /// Add a detector to the scan
    pub fn with_detector(mut self, detector: &str) -> Self {
        self.detectors.push(detector.to_string());
        self
    }

    /// Add multiple detectors to the scan
    pub fn with_detectors(mut self, detectors: &[&str]) -> Self {
        self.detectors
            .extend(detectors.iter().map(|s| s.to_string()));
        self
    }

    /// Set settle time in seconds
    pub fn with_settle_time(mut self, seconds: f64) -> Self {
        self.settle_time = seconds;
        self
    }

    fn position_at(&self, point: usize) -> f64 {
        if self.num_points <= 1 {
            self.start
        } else {
            let step = (self.stop - self.start) / (self.num_points - 1) as f64;
            self.start + step * point as f64
        }
    }
}

impl Plan for LineScan {
    fn plan_type(&self) -> &str {
        "line_scan"
    }

    fn plan_name(&self) -> &str {
        "Line Scan"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        let mut args = HashMap::new();
        args.insert("axis".to_string(), self.axis.clone());
        args.insert("start".to_string(), self.start.to_string());
        args.insert("stop".to_string(), self.stop.to_string());
        args.insert("num_points".to_string(), self.num_points.to_string());
        args.insert("detectors".to_string(), self.detectors.join(","));
        args
    }

    fn movers(&self) -> Vec<String> {
        vec![self.axis.clone()]
    }

    fn detectors(&self) -> Vec<String> {
        self.detectors.clone()
    }

    fn num_points(&self) -> usize {
        self.num_points
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        if self.current_point >= self.num_points {
            return None;
        }

        let cmd = match self.current_step {
            LineScanStep::Move => {
                let pos = self.position_at(self.current_point);
                self.current_step = if self.settle_time > 0.0 {
                    LineScanStep::Settle
                } else {
                    LineScanStep::Checkpoint
                };
                PlanCommand::MoveTo {
                    device_id: self.axis.clone(),
                    position: pos,
                }
            }
            LineScanStep::Settle => {
                self.current_step = LineScanStep::Checkpoint;
                PlanCommand::Wait {
                    seconds: self.settle_time,
                }
            }
            LineScanStep::Checkpoint => {
                self.current_step = LineScanStep::TriggerDetectors;
                PlanCommand::Checkpoint {
                    label: format!("point_{}", self.current_point),
                }
            }
            LineScanStep::TriggerDetectors => {
                // Trigger all detectors, then start reading
                self.current_step = LineScanStep::ReadDetectors { detector_idx: 0 };
                // For simplicity, emit a single trigger command
                // In a more sophisticated implementation, this would trigger each detector
                if let Some(det) = self.detectors.first() {
                    PlanCommand::Trigger {
                        device_id: det.clone(),
                    }
                } else {
                    // No detectors, skip to emit
                    self.current_step = LineScanStep::EmitEvent;
                    return self.next_command();
                }
            }
            LineScanStep::ReadDetectors { detector_idx } => {
                if detector_idx < self.detectors.len() {
                    let det = &self.detectors[detector_idx];
                    self.current_step = LineScanStep::ReadDetectors {
                        detector_idx: detector_idx + 1,
                    };
                    PlanCommand::Read {
                        device_id: det.clone(),
                    }
                } else {
                    self.current_step = LineScanStep::EmitEvent;
                    return self.next_command();
                }
            }
            LineScanStep::EmitEvent => {
                let pos = self.position_at(self.current_point);
                let mut positions = HashMap::new();
                positions.insert(self.axis.clone(), pos);

                self.current_point += 1;
                self.current_step = LineScanStep::Move;

                PlanCommand::EmitEvent {
                    stream: "primary".to_string(),
                    data: HashMap::new(), // Data filled in by RunEngine from Read results
                    positions,
                }
            }
            LineScanStep::Done => return None,
        };

        Some(cmd)
    }

    fn reset(&mut self) {
        self.current_point = 0;
        self.current_step = LineScanStep::Move;
    }
}

/// Grid scan - scan two axes in a grid pattern
#[derive(Debug, Clone)]
pub struct GridScan {
    axis_outer: String,
    outer_start: f64,
    outer_stop: f64,
    outer_points: usize,

    axis_inner: String,
    inner_start: f64,
    inner_stop: f64,
    inner_points: usize,

    detectors: Vec<String>,
    settle_time: f64,
    snake: bool, // Bidirectional scanning

    // Execution state
    outer_idx: usize,
    inner_idx: usize,
    inner_direction: i32, // 1 or -1
    current_step: GridScanStep,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GridScanStep {
    MoveOuter,
    MoveInner,
    Settle,
    Checkpoint,
    TriggerDetectors,
    ReadDetectors { detector_idx: usize },
    EmitEvent,
}

impl GridScan {
    pub fn new(
        axis_outer: &str,
        outer_start: f64,
        outer_stop: f64,
        outer_points: usize,
        axis_inner: &str,
        inner_start: f64,
        inner_stop: f64,
        inner_points: usize,
    ) -> Self {
        Self {
            axis_outer: axis_outer.to_string(),
            outer_start,
            outer_stop,
            outer_points,
            axis_inner: axis_inner.to_string(),
            inner_start,
            inner_stop,
            inner_points,
            detectors: Vec::new(),
            settle_time: 0.0,
            snake: true,
            outer_idx: 0,
            inner_idx: 0,
            inner_direction: 1,
            current_step: GridScanStep::MoveOuter,
        }
    }

    /// Add a detector to the scan
    pub fn with_detector(mut self, detector: &str) -> Self {
        self.detectors.push(detector.to_string());
        self
    }

    /// Set settle time in seconds
    pub fn with_settle_time(mut self, seconds: f64) -> Self {
        self.settle_time = seconds;
        self
    }

    /// Set snake mode (bidirectional scanning)
    pub fn with_snake(mut self, snake: bool) -> Self {
        self.snake = snake;
        self
    }

    fn outer_position(&self, idx: usize) -> f64 {
        if self.outer_points <= 1 {
            self.outer_start
        } else {
            let step = (self.outer_stop - self.outer_start) / (self.outer_points - 1) as f64;
            self.outer_start + step * idx as f64
        }
    }

    fn inner_position(&self, idx: usize) -> f64 {
        if self.inner_points <= 1 {
            self.inner_start
        } else {
            let step = (self.inner_stop - self.inner_start) / (self.inner_points - 1) as f64;
            self.inner_start + step * idx as f64
        }
    }
}

impl Plan for GridScan {
    fn plan_type(&self) -> &str {
        "grid_scan"
    }

    fn plan_name(&self) -> &str {
        "Grid Scan"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        let mut args = HashMap::new();
        args.insert("axis_outer".to_string(), self.axis_outer.clone());
        args.insert("outer_start".to_string(), self.outer_start.to_string());
        args.insert("outer_stop".to_string(), self.outer_stop.to_string());
        args.insert("outer_points".to_string(), self.outer_points.to_string());
        args.insert("axis_inner".to_string(), self.axis_inner.clone());
        args.insert("inner_start".to_string(), self.inner_start.to_string());
        args.insert("inner_stop".to_string(), self.inner_stop.to_string());
        args.insert("inner_points".to_string(), self.inner_points.to_string());
        args.insert("snake".to_string(), self.snake.to_string());
        args
    }

    fn movers(&self) -> Vec<String> {
        vec![self.axis_outer.clone(), self.axis_inner.clone()]
    }

    fn detectors(&self) -> Vec<String> {
        self.detectors.clone()
    }

    fn num_points(&self) -> usize {
        self.outer_points * self.inner_points
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        if self.outer_idx >= self.outer_points {
            return None;
        }

        let cmd = match self.current_step {
            GridScanStep::MoveOuter => {
                let pos = self.outer_position(self.outer_idx);
                self.current_step = GridScanStep::MoveInner;
                PlanCommand::MoveTo {
                    device_id: self.axis_outer.clone(),
                    position: pos,
                }
            }
            GridScanStep::MoveInner => {
                let pos = self.inner_position(self.inner_idx);
                self.current_step = if self.settle_time > 0.0 {
                    GridScanStep::Settle
                } else {
                    GridScanStep::Checkpoint
                };
                PlanCommand::MoveTo {
                    device_id: self.axis_inner.clone(),
                    position: pos,
                }
            }
            GridScanStep::Settle => {
                self.current_step = GridScanStep::Checkpoint;
                PlanCommand::Wait {
                    seconds: self.settle_time,
                }
            }
            GridScanStep::Checkpoint => {
                self.current_step = GridScanStep::TriggerDetectors;
                PlanCommand::Checkpoint {
                    label: format!("point_{}_{}", self.outer_idx, self.inner_idx),
                }
            }
            GridScanStep::TriggerDetectors => {
                self.current_step = GridScanStep::ReadDetectors { detector_idx: 0 };
                if let Some(det) = self.detectors.first() {
                    PlanCommand::Trigger {
                        device_id: det.clone(),
                    }
                } else {
                    self.current_step = GridScanStep::EmitEvent;
                    return self.next_command();
                }
            }
            GridScanStep::ReadDetectors { detector_idx } => {
                if detector_idx < self.detectors.len() {
                    let det = &self.detectors[detector_idx];
                    self.current_step = GridScanStep::ReadDetectors {
                        detector_idx: detector_idx + 1,
                    };
                    PlanCommand::Read {
                        device_id: det.clone(),
                    }
                } else {
                    self.current_step = GridScanStep::EmitEvent;
                    return self.next_command();
                }
            }
            GridScanStep::EmitEvent => {
                let outer_pos = self.outer_position(self.outer_idx);
                let inner_pos = self.inner_position(self.inner_idx);
                let mut positions = HashMap::new();
                positions.insert(self.axis_outer.clone(), outer_pos);
                positions.insert(self.axis_inner.clone(), inner_pos);

                // Advance to next point
                if self.snake {
                    // Snake pattern: alternate direction on inner axis
                    let next_inner = self.inner_idx as i32 + self.inner_direction;
                    if next_inner < 0 || next_inner >= self.inner_points as i32 {
                        // Move to next outer row
                        self.outer_idx += 1;
                        self.inner_direction = -self.inner_direction;
                        self.current_step = GridScanStep::MoveOuter;
                    } else {
                        self.inner_idx = next_inner as usize;
                        self.current_step = GridScanStep::MoveInner;
                    }
                } else {
                    // Raster pattern: always start inner from 0
                    self.inner_idx += 1;
                    if self.inner_idx >= self.inner_points {
                        self.inner_idx = 0;
                        self.outer_idx += 1;
                        self.current_step = GridScanStep::MoveOuter;
                    } else {
                        self.current_step = GridScanStep::MoveInner;
                    }
                }

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
        self.outer_idx = 0;
        self.inner_idx = 0;
        self.inner_direction = 1;
        self.current_step = GridScanStep::MoveOuter;
    }
}

/// Count plan - take N readings at current position
#[derive(Debug, Clone)]
pub struct Count {
    num_points: usize,
    delay: f64,
    detectors: Vec<String>,
    current_point: usize,
    current_step: CountStep,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CountStep {
    Checkpoint,
    Trigger,
    Read { detector_idx: usize },
    Emit,
    Wait,
}

impl Count {
    /// Create a new Count plan
    pub fn new(num_points: usize) -> Self {
        Self {
            num_points,
            delay: 0.0,
            detectors: Vec::new(),
            current_point: 0,
            current_step: CountStep::Checkpoint,
        }
    }

    /// Add a detector to the scan
    pub fn with_detector(mut self, detector: &str) -> Self {
        self.detectors.push(detector.to_string());
        self
    }

    /// Set delay between points in seconds
    pub fn with_delay(mut self, seconds: f64) -> Self {
        self.delay = seconds;
        self
    }
}

impl Plan for Count {
    fn plan_type(&self) -> &str {
        "count"
    }

    fn plan_name(&self) -> &str {
        "Count"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        let mut args = HashMap::new();
        args.insert("num_points".to_string(), self.num_points.to_string());
        args.insert("delay".to_string(), self.delay.to_string());
        args
    }

    fn movers(&self) -> Vec<String> {
        Vec::new()
    }

    fn detectors(&self) -> Vec<String> {
        self.detectors.clone()
    }

    fn num_points(&self) -> usize {
        self.num_points
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        if self.current_point >= self.num_points {
            return None;
        }

        let cmd = match self.current_step {
            CountStep::Checkpoint => {
                self.current_step = CountStep::Trigger;
                PlanCommand::Checkpoint {
                    label: format!("count_{}", self.current_point),
                }
            }
            CountStep::Trigger => {
                self.current_step = CountStep::Read { detector_idx: 0 };
                if let Some(det) = self.detectors.first() {
                    PlanCommand::Trigger {
                        device_id: det.clone(),
                    }
                } else {
                    self.current_step = CountStep::Emit;
                    return self.next_command();
                }
            }
            CountStep::Read { detector_idx } => {
                if detector_idx < self.detectors.len() {
                    let det = &self.detectors[detector_idx];
                    self.current_step = CountStep::Read {
                        detector_idx: detector_idx + 1,
                    };
                    PlanCommand::Read {
                        device_id: det.clone(),
                    }
                } else {
                    self.current_step = CountStep::Emit;
                    return self.next_command();
                }
            }
            CountStep::Emit => {
                self.current_point += 1;
                self.current_step = if self.delay > 0.0 && self.current_point < self.num_points {
                    CountStep::Wait
                } else {
                    CountStep::Checkpoint
                };
                PlanCommand::EmitEvent {
                    stream: "primary".to_string(),
                    data: HashMap::new(),
                    positions: HashMap::new(),
                }
            }
            CountStep::Wait => {
                self.current_step = CountStep::Checkpoint;
                PlanCommand::Wait {
                    seconds: self.delay,
                }
            }
        };

        Some(cmd)
    }

    fn reset(&mut self) {
        self.current_point = 0;
        self.current_step = CountStep::Checkpoint;
    }
}

/// Builder trait for creating plans from string parameters
pub trait PlanBuilder: Send + Sync {
    /// Build a plan instance from parameters and device mappings
    fn build(
        &self,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String>;

    /// Get human-readable description of the plan type
    fn description(&self) -> String;

    /// Get category tags for this plan type (e.g., "scanning", "0d", "1d", "2d")
    fn categories(&self) -> Vec<String>;
}

/// Builder for Count plans
pub struct CountBuilder;

impl PlanBuilder for CountBuilder {
    fn build(
        &self,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String> {
        // Parse count parameters
        let num_points = parameters
            .get("num_points")
            .ok_or("Missing parameter: num_points")?
            .parse::<usize>()
            .map_err(|e| format!("Invalid num_points: {}", e))?;

        // Validate parameters
        if num_points == 0 {
            return Err("num_points must be > 0".to_string());
        }
        if num_points > 10_000_000 {
            return Err(
                "num_points must be <= 10,000,000 to prevent resource exhaustion".to_string(),
            );
        }

        let mut plan = Count::new(num_points);

        // Optional detector
        if let Some(detector) = device_mapping.get("detector") {
            if detector.is_empty() {
                return Err("detector device name cannot be empty".to_string());
            }
            plan = plan.with_detector(detector);
        }

        // Optional delay
        if let Some(delay_str) = parameters.get("delay") {
            let delay = delay_str
                .parse::<f64>()
                .map_err(|e| format!("Invalid delay: {}", e))?;
            if !delay.is_finite() {
                return Err("delay must be a finite number (not NaN or infinity)".to_string());
            }
            if delay < 0.0 {
                return Err("delay must be >= 0".to_string());
            }
            plan = plan.with_delay(delay);
        }

        Ok(Box::new(plan))
    }

    fn description(&self) -> String {
        "Repeated measurements at current position".to_string()
    }

    fn categories(&self) -> Vec<String> {
        vec!["0d".to_string()]
    }
}

/// Builder for LineScan plans
pub struct LineScanBuilder;

impl PlanBuilder for LineScanBuilder {
    fn build(
        &self,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String> {
        // Parse line scan parameters
        let start = parameters
            .get("start")
            .ok_or("Missing parameter: start")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid start: {}", e))?;

        let end = parameters
            .get("end")
            .ok_or("Missing parameter: end")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid end: {}", e))?;

        let num_points = parameters
            .get("num_points")
            .ok_or("Missing parameter: num_points")?
            .parse::<usize>()
            .map_err(|e| format!("Invalid num_points: {}", e))?;

        let motor = device_mapping
            .get("motor")
            .ok_or("Missing device mapping: motor")?;

        // Validate parameters
        if !start.is_finite() {
            return Err("start must be a finite number (not NaN or infinity)".to_string());
        }
        if !end.is_finite() {
            return Err("end must be a finite number (not NaN or infinity)".to_string());
        }
        if num_points == 0 {
            return Err("num_points must be > 0".to_string());
        }
        if num_points > 10_000_000 {
            return Err(
                "num_points must be <= 10,000,000 to prevent resource exhaustion".to_string(),
            );
        }
        if start == end {
            return Err("start and end must be different for line scan".to_string());
        }
        if motor.is_empty() {
            return Err("motor device name cannot be empty".to_string());
        }

        let mut plan = LineScan::new(motor, start, end, num_points);

        // Optional detector
        if let Some(detector) = device_mapping.get("detector") {
            if detector.is_empty() {
                return Err("detector device name cannot be empty".to_string());
            }
            plan = plan.with_detector(detector);
        }

        // Optional settle time
        if let Some(settle_str) = parameters.get("settle_time") {
            let settle = settle_str
                .parse::<f64>()
                .map_err(|e| format!("Invalid settle_time: {}", e))?;
            if !settle.is_finite() {
                return Err("settle_time must be a finite number (not NaN or infinity)".to_string());
            }
            if settle < 0.0 {
                return Err("settle_time must be >= 0".to_string());
            }
            plan = plan.with_settle_time(settle);
        }

        Ok(Box::new(plan))
    }

    fn description(&self) -> String {
        "1D linear scan along a motor axis".to_string()
    }

    fn categories(&self) -> Vec<String> {
        vec!["scanning".to_string(), "1d".to_string()]
    }
}

/// Builder for GridScan plans
pub struct GridScanBuilder;

impl PlanBuilder for GridScanBuilder {
    fn build(
        &self,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String> {
        // Parse grid scan parameters
        let x_start = parameters
            .get("x_start")
            .ok_or("Missing parameter: x_start")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid x_start: {}", e))?;

        let x_end = parameters
            .get("x_end")
            .ok_or("Missing parameter: x_end")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid x_end: {}", e))?;

        let x_points = parameters
            .get("x_points")
            .ok_or("Missing parameter: x_points")?
            .parse::<usize>()
            .map_err(|e| format!("Invalid x_points: {}", e))?;

        let y_start = parameters
            .get("y_start")
            .ok_or("Missing parameter: y_start")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid y_start: {}", e))?;

        let y_end = parameters
            .get("y_end")
            .ok_or("Missing parameter: y_end")?
            .parse::<f64>()
            .map_err(|e| format!("Invalid y_end: {}", e))?;

        let y_points = parameters
            .get("y_points")
            .ok_or("Missing parameter: y_points")?
            .parse::<usize>()
            .map_err(|e| format!("Invalid y_points: {}", e))?;

        let x_motor = device_mapping
            .get("x_motor")
            .ok_or("Missing device mapping: x_motor")?;

        let y_motor = device_mapping
            .get("y_motor")
            .ok_or("Missing device mapping: y_motor")?;

        // Validate parameters
        if !x_start.is_finite() {
            return Err("x_start must be a finite number (not NaN or infinity)".to_string());
        }
        if !x_end.is_finite() {
            return Err("x_end must be a finite number (not NaN or infinity)".to_string());
        }
        if !y_start.is_finite() {
            return Err("y_start must be a finite number (not NaN or infinity)".to_string());
        }
        if !y_end.is_finite() {
            return Err("y_end must be a finite number (not NaN or infinity)".to_string());
        }
        if x_points == 0 {
            return Err("x_points must be > 0".to_string());
        }
        if x_points > 100_000 {
            return Err("x_points must be <= 100,000 to prevent resource exhaustion".to_string());
        }
        if y_points == 0 {
            return Err("y_points must be > 0".to_string());
        }
        if y_points > 100_000 {
            return Err("y_points must be <= 100,000 to prevent resource exhaustion".to_string());
        }
        if x_start == x_end {
            return Err("x_start and x_end must be different for grid scan".to_string());
        }
        if y_start == y_end {
            return Err("y_start and y_end must be different for grid scan".to_string());
        }
        if x_motor.is_empty() {
            return Err("x_motor device name cannot be empty".to_string());
        }
        if y_motor.is_empty() {
            return Err("y_motor device name cannot be empty".to_string());
        }
        if x_motor == y_motor {
            return Err("x_motor and y_motor must be different".to_string());
        }

        // Note: GridScan takes (outer/slow, inner/fast) axes
        // Convention: y is outer (slow), x is inner (fast)
        let mut plan = GridScan::new(
            y_motor, y_start, y_end, y_points, x_motor, x_start, x_end, x_points,
        );

        // Optional detector
        if let Some(detector) = device_mapping.get("detector") {
            if detector.is_empty() {
                return Err("detector device name cannot be empty".to_string());
            }
            plan = plan.with_detector(detector);
        }

        // Optional snake scanning
        if let Some(snake_str) = parameters.get("snake") {
            let snake = snake_str
                .parse::<bool>()
                .map_err(|e| format!("Invalid snake: {}", e))?;
            plan = plan.with_snake(snake);
        }

        Ok(Box::new(plan))
    }

    fn description(&self) -> String {
        "2D grid scan over two motor axes".to_string()
    }

    fn categories(&self) -> Vec<String> {
        vec!["scanning".to_string(), "2d".to_string()]
    }
}

/// Plan registry for looking up and creating plans by type
pub struct PlanRegistry {
    builders: HashMap<String, Box<dyn PlanBuilder>>,
}

impl Default for PlanRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanRegistry {
    /// Create a new PlanRegistry
    pub fn new() -> Self {
        Self {
            builders: HashMap::new(),
        }
    }

    /// Register a plan builder
    pub fn register<B>(&mut self, plan_type: &str, builder: B)
    where
        B: PlanBuilder + 'static,
    {
        self.builders
            .insert(plan_type.to_string(), Box::new(builder));
    }

    /// List available plan types with descriptions and categories
    pub fn list_types(&self) -> Vec<(String, String, Vec<String>)> {
        self.builders
            .iter()
            .map(|(k, v)| (k.clone(), v.description(), v.categories()))
            .collect()
    }

    /// Check if a plan type is registered
    pub fn has_type(&self, plan_type: &str) -> bool {
        self.builders.contains_key(plan_type)
    }

    /// Create a plan instance
    pub fn create_plan(
        &self,
        plan_type: &str,
        parameters: &HashMap<String, String>,
        device_mapping: &HashMap<String, String>,
    ) -> Result<Box<dyn Plan>, String> {
        let builder = self
            .builders
            .get(plan_type)
            .ok_or_else(|| format!("Unknown plan type: {}", plan_type))?;

        builder.build(parameters, device_mapping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_scan_commands() {
        let mut plan = LineScan::new("stage_x", 0.0, 10.0, 3)
            .with_detector("power_meter")
            .with_settle_time(0.1);

        let mut commands = Vec::new();
        while let Some(cmd) = plan.next_command() {
            commands.push(cmd);
        }

        // Should have commands for 3 points
        // Each point: Move, Wait, Checkpoint, Trigger, Read, EmitEvent
        assert!(commands.len() >= 15); // At least 5 commands per point × 3 points
    }

    #[test]
    fn test_line_scan_positions() {
        let mut plan = LineScan::new("x", 0.0, 10.0, 11);

        let mut positions = Vec::new();
        while let Some(cmd) = plan.next_command() {
            if let PlanCommand::MoveTo { position, .. } = cmd {
                positions.push(position);
            }
        }

        assert_eq!(positions.len(), 11);
        assert!((positions[0] - 0.0).abs() < 1e-10);
        assert!((positions[5] - 5.0).abs() < 1e-10);
        assert!((positions[10] - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_grid_scan_points() {
        let mut plan = GridScan::new("y", 0.0, 2.0, 3, "x", 0.0, 1.0, 2).with_detector("detector");

        let mut event_count = 0;
        while let Some(cmd) = plan.next_command() {
            if matches!(cmd, PlanCommand::EmitEvent { .. }) {
                event_count += 1;
            }
        }

        assert_eq!(event_count, 6); // 3 outer × 2 inner
    }

    #[test]
    fn test_count_plan() {
        let mut plan = Count::new(5).with_detector("power_meter");

        let mut event_count = 0;
        while let Some(cmd) = plan.next_command() {
            if matches!(cmd, PlanCommand::EmitEvent { .. }) {
                event_count += 1;
            }
        }

        assert_eq!(event_count, 5);
    }

    #[test]
    fn test_plan_reset() {
        let mut plan = Count::new(3);

        // Run through once
        while plan.next_command().is_some() {}

        // Reset and run again
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
