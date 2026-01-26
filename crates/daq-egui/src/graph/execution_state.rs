//! Execution state tracking for visual feedback in the graph editor.

use egui_snarl::NodeId;
use std::collections::HashSet;
use std::fmt::Write;
use std::time::{Duration, Instant};

/// Progress for a single scan dimension (e.g., "wavelength 3/10")
#[derive(Debug, Clone)]
pub struct DimensionProgress {
    /// Human-readable dimension name (e.g., "wavelength", "position")
    pub name: String,
    /// Current index within this dimension (0-based)
    pub current: u32,
    /// Total count for this dimension
    pub total: u32,
}

impl DimensionProgress {
    pub fn new(name: impl Into<String>, current: u32, total: u32) -> Self {
        Self {
            name: name.into(),
            current,
            total,
        }
    }

    /// Format as "name current/total" (1-based display)
    pub fn format(&self) -> String {
        format!("{} {}/{}", self.name, self.current + 1, self.total)
    }
}

/// Nested progress tracking for multi-dimensional scans.
///
/// Tracks progress across multiple nested scan dimensions (e.g., outer wavelength scan
/// with inner position scan) and provides both nested and flattened views.
#[derive(Debug, Clone, Default)]
pub struct NestedProgress {
    /// Progress for each dimension, from outermost to innermost
    pub dimensions: Vec<DimensionProgress>,
    /// Current flat index (0 to flat_total - 1)
    pub flat_current: u32,
    /// Total flat count (product of all dimension totals)
    pub flat_total: u32,
}

impl NestedProgress {
    /// Create new nested progress with the given dimensions.
    ///
    /// flat_total is automatically computed as the product of all dimension totals.
    pub fn new(dimensions: Vec<DimensionProgress>) -> Self {
        let flat_total = dimensions.iter().map(|d| d.total).product::<u32>().max(1);
        Self {
            dimensions,
            flat_current: 0,
            flat_total,
        }
    }

    /// Update progress from flat index.
    ///
    /// Decomposes flat_current into per-dimension indices.
    pub fn set_flat_current(&mut self, flat_current: u32) {
        self.flat_current = flat_current;

        // Decompose flat index into per-dimension indices (row-major order)
        // For dimensions [outer, inner], flat_idx = outer * inner_total + inner
        let mut remaining = flat_current;
        for i in (0..self.dimensions.len()).rev() {
            let dim_total = self.dimensions[i].total;
            if dim_total > 0 {
                self.dimensions[i].current = remaining % dim_total;
                remaining /= dim_total;
            }
        }
    }

    /// Format as nested string: "wavelength 3/10, position 45/100"
    pub fn format_nested(&self) -> String {
        if self.dimensions.is_empty() {
            return "No dimensions".to_string();
        }

        let mut result = String::new();
        for (i, dim) in self.dimensions.iter().enumerate() {
            if i > 0 {
                result.push_str(", ");
            }
            let _ = write!(result, "{}", dim.format());
        }
        result
    }

    /// Format as flattened string: "345/1000 (34.5%)"
    pub fn format_flat(&self) -> String {
        if self.flat_total == 0 {
            return "0/0 (0.0%)".to_string();
        }
        let pct = (self.flat_current as f64 / self.flat_total as f64) * 100.0;
        format!(
            "{}/{} ({:.1}%)",
            self.flat_current + 1,
            self.flat_total,
            pct
        )
    }

    /// Get progress as fraction (0.0 to 1.0)
    pub fn progress(&self) -> f32 {
        if self.flat_total == 0 {
            0.0
        } else {
            (self.flat_current as f32 / self.flat_total as f32).min(1.0)
        }
    }
}

/// State of a single node during execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum NodeExecutionState {
    /// Not yet executed
    Pending,
    /// Currently executing
    Running,
    /// Completed successfully
    Completed,
    /// Execution was skipped or aborted
    Skipped,
}

/// Overall execution state for the graph
#[derive(Debug, Clone)]
pub struct ExecutionState {
    /// Engine state (mirrors EngineState from proto)
    pub engine_state: EngineStateLocal,
    /// Currently executing node (parsed from checkpoint labels)
    pub active_node: Option<NodeId>,
    /// Nodes that have completed
    pub completed_nodes: HashSet<NodeId>,
    /// Current run UID
    pub run_uid: Option<String>,
    /// Current event number
    pub current_event: u32,
    /// Total expected events
    pub total_events: u32,
    /// When execution started
    pub start_time: Option<Instant>,
    /// Last status update time
    pub last_update: Instant,
    /// Nested progress for multi-dimensional scans (None for simple scans)
    pub nested_progress: Option<NestedProgress>,
}

/// Local copy of engine state (avoids proto dependency in this module)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineStateLocal {
    #[default]
    Idle,
    Running,
    Paused,
    Aborting,
}

impl ExecutionState {
    /// Create a new idle execution state
    pub fn new() -> Self {
        Self {
            engine_state: EngineStateLocal::Idle,
            active_node: None,
            completed_nodes: HashSet::new(),
            run_uid: None,
            current_event: 0,
            total_events: 0,
            start_time: None,
            last_update: Instant::now(),
            nested_progress: None,
        }
    }

    /// Reset to idle state
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Start a new execution
    pub fn start_execution(&mut self, run_uid: String, total_events: u32) {
        self.engine_state = EngineStateLocal::Running;
        self.run_uid = Some(run_uid);
        self.total_events = total_events;
        self.current_event = 0;
        self.start_time = Some(Instant::now());
        self.active_node = None;
        self.completed_nodes.clear();
        self.last_update = Instant::now();
        self.nested_progress = None;
    }

    /// Start a new execution with nested progress tracking for multi-dimensional scans.
    pub fn start_nested_execution(&mut self, run_uid: String, nested: NestedProgress) {
        self.engine_state = EngineStateLocal::Running;
        self.run_uid = Some(run_uid);
        self.total_events = nested.flat_total;
        self.current_event = 0;
        self.start_time = Some(Instant::now());
        self.active_node = None;
        self.completed_nodes.clear();
        self.last_update = Instant::now();
        self.nested_progress = Some(nested);
    }

    /// Update from engine status
    pub fn update_from_status(
        &mut self,
        state: i32,
        current_event: Option<u32>,
        total_events: Option<u32>,
    ) {
        // Map proto EngineState enum values
        self.engine_state = match state {
            0 => EngineStateLocal::Idle,
            1 => EngineStateLocal::Running,
            2 => EngineStateLocal::Paused,
            3 => EngineStateLocal::Aborting,
            _ => EngineStateLocal::Idle,
        };

        if let Some(ev) = current_event {
            self.current_event = ev;
            // Also update nested progress if present
            if let Some(ref mut nested) = self.nested_progress {
                nested.set_flat_current(ev);
            }
        }
        if let Some(total) = total_events {
            self.total_events = total;
        }
        self.last_update = Instant::now();
    }

    /// Update active node from checkpoint label
    /// Labels are formatted as "node_{NodeId}_start" or "node_{NodeId}_end"
    #[allow(dead_code)]
    pub fn update_from_checkpoint(&mut self, label: &str) {
        // Parse "node_NodeId(X)_start" or "node_NodeId(X)_end"
        if let Some(rest) = label.strip_prefix("node_") {
            // Find the node ID portion (between "node_" and "_start"/"_end")
            if let Some(end_idx) = rest.find("_start").or_else(|| rest.find("_end")) {
                let id_str = &rest[..end_idx];
                // NodeId is printed as "NodeId(X)" in Debug format
                // Try to extract the number
                if let Some(num_str) = id_str
                    .strip_prefix("NodeId(")
                    .and_then(|s| s.strip_suffix(")"))
                {
                    if let Ok(idx) = num_str.parse::<usize>() {
                        let node_id = NodeId(idx);
                        if label.ends_with("_start") {
                            // Mark previous active node as completed
                            if let Some(prev) = self.active_node.take() {
                                self.completed_nodes.insert(prev);
                            }
                            self.active_node = Some(node_id);
                        } else if label.ends_with("_end") && self.active_node == Some(node_id) {
                            self.completed_nodes.insert(node_id);
                            self.active_node = None;
                        }
                    }
                }
            }
        }
    }

    /// Get state for a specific node
    pub fn node_state(&self, node_id: NodeId) -> NodeExecutionState {
        if self.active_node == Some(node_id) {
            NodeExecutionState::Running
        } else if self.completed_nodes.contains(&node_id) {
            NodeExecutionState::Completed
        } else {
            NodeExecutionState::Pending
        }
    }

    /// Calculate progress percentage (0.0 - 1.0)
    pub fn progress(&self) -> f32 {
        if self.total_events == 0 {
            0.0
        } else {
            (self.current_event as f32 / self.total_events as f32).min(1.0)
        }
    }

    /// Calculate estimated time remaining
    pub fn estimated_remaining(&self) -> Option<Duration> {
        let elapsed = self.start_time?.elapsed();
        if self.current_event == 0 || self.current_event >= self.total_events {
            return None;
        }
        let avg_time_per_event = elapsed / self.current_event;
        let remaining_events = self.total_events - self.current_event;
        Some(avg_time_per_event * remaining_events)
    }

    /// Check if execution is active (running or paused)
    pub fn is_active(&self) -> bool {
        matches!(
            self.engine_state,
            EngineStateLocal::Running | EngineStateLocal::Paused
        )
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        matches!(self.engine_state, EngineStateLocal::Running)
    }

    /// Check if paused
    pub fn is_paused(&self) -> bool {
        matches!(self.engine_state, EngineStateLocal::Paused)
    }
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_calculation() {
        let mut state = ExecutionState::new();
        state.total_events = 10;
        state.current_event = 5;
        assert!((state.progress() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_checkpoint_parsing() {
        let mut state = ExecutionState::new();
        state.update_from_checkpoint("node_NodeId(3)_start");
        // Verify parsing succeeded
        assert!(state.active_node.is_some());
        assert_eq!(state.active_node, Some(NodeId(3)));
    }

    #[test]
    fn test_nested_progress_format_nested() {
        let progress = NestedProgress::new(vec![
            DimensionProgress::new("wavelength", 2, 10),
            DimensionProgress::new("position", 44, 100),
        ]);

        // Should show human-readable nested format
        let nested = progress.format_nested();
        assert_eq!(nested, "wavelength 3/10, position 45/100");
    }

    #[test]
    fn test_nested_progress_format_flat() {
        let mut progress = NestedProgress::new(vec![
            DimensionProgress::new("outer", 0, 10),
            DimensionProgress::new("inner", 0, 100),
        ]);
        progress.flat_current = 344; // 0-based

        // Should show flattened format
        let flat = progress.format_flat();
        assert_eq!(flat, "345/1000 (34.5%)");
    }

    #[test]
    fn test_nested_progress_set_flat_current() {
        let mut progress = NestedProgress::new(vec![
            DimensionProgress::new("outer", 0, 10),
            DimensionProgress::new("inner", 0, 100),
        ]);

        // Set to flat index 345 (0-based: 344)
        // = outer 3 (0-based), inner 44 (0-based)
        progress.set_flat_current(344);

        assert_eq!(progress.dimensions[0].current, 3);
        assert_eq!(progress.dimensions[1].current, 44);
    }

    #[test]
    fn test_nested_progress_three_dimensions() {
        let mut progress = NestedProgress::new(vec![
            DimensionProgress::new("z", 0, 5),
            DimensionProgress::new("y", 0, 10),
            DimensionProgress::new("x", 0, 20),
        ]);

        // Total should be 5 * 10 * 20 = 1000
        assert_eq!(progress.flat_total, 1000);

        // Set to flat index 234 (z=1, y=1, x=14)
        // 234 = 1*200 + 1*20 + 14
        progress.set_flat_current(234);

        assert_eq!(progress.dimensions[0].current, 1); // z
        assert_eq!(progress.dimensions[1].current, 1); // y
        assert_eq!(progress.dimensions[2].current, 14); // x
    }

    #[test]
    fn test_nested_progress_progress_fraction() {
        let mut progress = NestedProgress::new(vec![
            DimensionProgress::new("outer", 0, 10),
            DimensionProgress::new("inner", 0, 10),
        ]);

        // 50% complete
        progress.flat_current = 50;
        assert!((progress.progress() - 0.5).abs() < 0.001);
    }
}
