//! Experiment node definitions for the graph editor.

use serde::{Deserialize, Serialize};

/// Node types for experiment workflows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ExperimentNode {
    /// 1D or 2D parameter scan
    Scan {
        actuator: String, // Device ID for movable
        start: f64,
        stop: f64,
        points: u32,
    },
    /// Move actuator to position
    Move(MoveConfig),
    /// Wait/delay step
    Wait { condition: WaitCondition },
    /// Single or burst acquisition from detector
    Acquire(AcquireConfig),
    /// Loop control node
    Loop(LoopConfig),
}

/// Movement mode for Move nodes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum MoveMode {
    #[default]
    Absolute,
    Relative,
}

/// Configuration for Move node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MoveConfig {
    pub device: String,
    pub position: f64,
    pub mode: MoveMode,
    pub wait_settled: bool,
}

impl Default for MoveConfig {
    fn default() -> Self {
        Self {
            device: String::new(),
            position: 0.0,
            mode: MoveMode::Absolute,
            wait_settled: true,
        }
    }
}

/// Wait condition for Wait nodes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WaitCondition {
    /// Simple duration-based wait
    Duration { milliseconds: f64 },
    /// Wait until threshold condition met
    Threshold {
        device_id: String,
        operator: ThresholdOp,
        value: f64,
        timeout_ms: f64,
    },
    /// Wait until value stabilizes
    Stability {
        device_id: String,
        tolerance: f64,
        duration_ms: f64,
        timeout_ms: f64,
    },
}

impl Default for WaitCondition {
    fn default() -> Self {
        Self::Duration { milliseconds: 1000.0 }
    }
}

/// Threshold operators for condition-based waits.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum ThresholdOp {
    #[default]
    LessThan,
    GreaterThan,
    EqualWithin { tolerance: f64 },
}

/// Configuration for Acquire node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcquireConfig {
    pub detector: String,
    pub exposure_ms: Option<f64>, // None = use device default
    pub frame_count: u32,          // 1 for single, >1 for burst
}

impl Default for AcquireConfig {
    fn default() -> Self {
        Self {
            detector: String::new(),
            exposure_ms: None,
            frame_count: 1,
        }
    }
}

/// Loop termination modes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LoopTermination {
    /// Fixed number of iterations
    Count { iterations: u32 },
    /// Loop until condition met
    Condition {
        device_id: String,
        operator: ThresholdOp,
        value: f64,
        max_iterations: u32, // Safety limit
    },
    /// Infinite loop (requires manual abort)
    Infinite { max_iterations: u32 }, // Safety limit
}

impl Default for LoopTermination {
    fn default() -> Self {
        Self::Count { iterations: 10 }
    }
}

/// Configuration for Loop node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopConfig {
    pub termination: LoopTermination,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            termination: LoopTermination::default(),
        }
    }
}

impl ExperimentNode {
    /// Get human-readable name for this node type.
    pub fn node_name(&self) -> &'static str {
        match self {
            ExperimentNode::Scan { .. } => "Scan",
            ExperimentNode::Acquire(..) => "Acquire",
            ExperimentNode::Move(..) => "Move",
            ExperimentNode::Wait { .. } => "Wait",
            ExperimentNode::Loop(..) => "Loop",
        }
    }

    /// Create a default Scan node with sensible defaults.
    pub fn default_scan() -> Self {
        ExperimentNode::Scan {
            actuator: String::new(),
            start: 0.0,
            stop: 100.0,
            points: 10,
        }
    }

    /// Create a default Acquire node with sensible defaults.
    pub fn default_acquire() -> Self {
        ExperimentNode::Acquire(AcquireConfig::default())
    }

    /// Create a default Move node with sensible defaults.
    pub fn default_move() -> Self {
        ExperimentNode::Move(MoveConfig::default())
    }

    /// Create a default Wait node with sensible defaults.
    pub fn default_wait() -> Self {
        ExperimentNode::Wait {
            condition: WaitCondition::default(),
        }
    }

    /// Create a default Loop node with sensible defaults.
    pub fn default_loop() -> Self {
        ExperimentNode::Loop(LoopConfig::default())
    }
}
