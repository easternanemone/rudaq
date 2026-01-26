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
    /// Nested scan with outer/inner loop structure
    NestedScan(NestedScanConfig),
    /// Adaptive scan that responds to acquired data
    AdaptiveScan(AdaptiveScanConfig),
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
        Self::Duration {
            milliseconds: 1000.0,
        }
    }
}

/// Threshold operators for condition-based waits.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum ThresholdOp {
    #[default]
    LessThan,
    GreaterThan,
    EqualWithin {
        tolerance: f64,
    },
}

/// Configuration for Acquire node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcquireConfig {
    pub detector: String,
    pub exposure_ms: Option<f64>, // None = use device default
    pub frame_count: u32,         // 1 for single, >1 for burst
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LoopConfig {
    pub termination: LoopTermination,
}

/// Single dimension of a nested scan.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanDimension {
    /// Device ID for the actuator
    pub actuator: String,
    /// Dimension name for data labeling (e.g., "wavelength", "position_x")
    pub dimension_name: String,
    /// Start position
    pub start: f64,
    /// Stop position
    pub stop: f64,
    /// Number of points
    pub points: u32,
}

impl Default for ScanDimension {
    fn default() -> Self {
        Self {
            actuator: String::new(),
            dimension_name: String::new(),
            start: 0.0,
            stop: 100.0,
            points: 10,
        }
    }
}

/// Configuration for NestedScan node (outer/inner loop combination).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NestedScanConfig {
    /// Outer scan configuration
    pub outer: ScanDimension,
    /// Inner scan configuration
    pub inner: ScanDimension,
    /// Warning threshold for deep nesting (default: 3)
    pub nesting_warning_depth: u32,
}

impl Default for NestedScanConfig {
    fn default() -> Self {
        Self {
            outer: ScanDimension {
                dimension_name: "outer".to_string(),
                ..Default::default()
            },
            inner: ScanDimension {
                dimension_name: "inner".to_string(),
                ..Default::default()
            },
            nesting_warning_depth: 3,
        }
    }
}

/// Conditions that can trigger adaptive scan actions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TriggerCondition {
    /// Signal crosses a threshold
    Threshold {
        device_id: String,
        operator: ThresholdOp,
        value: f64,
    },
    /// Peak detected in signal
    PeakDetection {
        device_id: String,
        min_prominence: f64,
        min_height: Option<f64>,
    },
}

impl Default for TriggerCondition {
    fn default() -> Self {
        Self::Threshold {
            device_id: String::new(),
            operator: ThresholdOp::GreaterThan,
            value: 1000.0,
        }
    }
}

/// Actions to take when trigger fires.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum AdaptiveAction {
    /// Narrow range and increase resolution (2x)
    #[default]
    Zoom2x,
    /// Narrow range and increase resolution (4x)
    Zoom4x,
    /// Move actuator to detected peak position
    MoveToPeak,
    /// Trigger acquisition at peak position
    AcquireAtPeak,
    /// Record peak location but continue scan unchanged
    MarkAndContinue,
}

/// Logic for combining multiple trigger conditions.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub enum TriggerLogic {
    /// Fire if any trigger matches
    #[default]
    Any,
    /// Fire only if all triggers match
    All,
}

/// Configuration for AdaptiveScan node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdaptiveScanConfig {
    /// Base scan configuration
    pub scan: ScanDimension,
    /// Trigger conditions
    pub triggers: Vec<TriggerCondition>,
    /// How to combine multiple triggers
    pub trigger_logic: TriggerLogic,
    /// Action to take when triggered
    pub action: AdaptiveAction,
    /// Pause for user approval before action (default: false)
    pub require_approval: bool,
}

impl Default for AdaptiveScanConfig {
    fn default() -> Self {
        Self {
            scan: ScanDimension::default(),
            triggers: vec![TriggerCondition::default()],
            trigger_logic: TriggerLogic::Any,
            action: AdaptiveAction::Zoom2x,
            require_approval: false,
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
            ExperimentNode::NestedScan(..) => "Nested Scan",
            ExperimentNode::AdaptiveScan(..) => "Adaptive Scan",
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

    /// Create a default NestedScan node with sensible defaults.
    pub fn default_nested_scan() -> Self {
        ExperimentNode::NestedScan(NestedScanConfig::default())
    }

    /// Create a default AdaptiveScan node with sensible defaults.
    pub fn default_adaptive_scan() -> Self {
        ExperimentNode::AdaptiveScan(AdaptiveScanConfig::default())
    }
}
