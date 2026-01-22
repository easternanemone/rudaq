//! Execution state tracking for visual feedback in the graph editor.

use std::collections::HashSet;
use std::time::{Duration, Instant};
use egui_snarl::NodeId;

/// State of a single node during execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    }

    /// Update from engine status
    pub fn update_from_status(&mut self, state: i32, current_event: Option<u32>, total_events: Option<u32>) {
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
        }
        if let Some(total) = total_events {
            self.total_events = total;
        }
        self.last_update = Instant::now();
    }

    /// Update active node from checkpoint label
    /// Labels are formatted as "node_{NodeId}_start" or "node_{NodeId}_end"
    pub fn update_from_checkpoint(&mut self, label: &str) {
        // Parse "node_NodeId(X)_start" or "node_NodeId(X)_end"
        if let Some(rest) = label.strip_prefix("node_") {
            // Find the node ID portion (between "node_" and "_start"/"_end")
            if let Some(end_idx) = rest.find("_start").or_else(|| rest.find("_end")) {
                let id_str = &rest[..end_idx];
                // NodeId is printed as "NodeId(X)" in Debug format
                // Try to extract the number
                if let Some(num_str) = id_str.strip_prefix("NodeId(").and_then(|s| s.strip_suffix(")")) {
                    if let Ok(idx) = num_str.parse::<usize>() {
                        let node_id = NodeId(idx);
                        if label.ends_with("_start") {
                            // Mark previous active node as completed
                            if let Some(prev) = self.active_node.take() {
                                self.completed_nodes.insert(prev);
                            }
                            self.active_node = Some(node_id);
                        } else if label.ends_with("_end") {
                            if self.active_node == Some(node_id) {
                                self.completed_nodes.insert(node_id);
                                self.active_node = None;
                            }
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
        matches!(self.engine_state, EngineStateLocal::Running | EngineStateLocal::Paused)
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
}
