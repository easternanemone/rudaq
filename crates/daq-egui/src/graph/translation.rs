//! Translation from visual node graph to executable Plan.

use super::nodes::ExperimentNode;
use daq_experiment::plans::{Plan, PlanCommand};
use egui_snarl::{NodeId, Snarl};
use std::collections::{HashMap, HashSet, VecDeque};

/// Errors that can occur during graph translation
#[derive(Debug, Clone)]
pub enum TranslationError {
    /// Graph contains a cycle
    CycleDetected,
    /// Node has invalid configuration
    #[allow(dead_code)]
    InvalidNode { node_id: NodeId, reason: String },
    /// Graph is empty
    EmptyGraph,
    /// No root nodes found (all nodes have inputs)
    NoRootNodes,
}

impl std::fmt::Display for TranslationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CycleDetected => write!(f, "Graph contains a cycle"),
            Self::InvalidNode { node_id, reason } => {
                write!(f, "Invalid node {:?}: {}", node_id, reason)
            }
            Self::EmptyGraph => write!(f, "Graph is empty"),
            Self::NoRootNodes => write!(f, "No root nodes found"),
        }
    }
}

impl std::error::Error for TranslationError {}

/// Plan generated from a visual node graph
pub struct GraphPlan {
    commands: Vec<PlanCommand>,
    current_idx: usize,
    total_events: usize,
    movers: Vec<String>,
    detectors: Vec<String>,
}

impl GraphPlan {
    /// Translate a Snarl graph into an executable GraphPlan
    pub fn from_snarl(snarl: &Snarl<ExperimentNode>) -> Result<Self, TranslationError> {
        if snarl.node_ids().count() == 0 {
            return Err(TranslationError::EmptyGraph);
        }

        // Build adjacency list and find roots
        let (adjacency, roots) = build_adjacency(snarl)?;

        if roots.is_empty() {
            return Err(TranslationError::NoRootNodes);
        }

        // Topological sort with cycle detection
        let sorted = topological_sort(&adjacency, &roots, snarl.node_ids().count())?;

        // Identify loop body nodes (these will be skipped in main traversal)
        let mut loop_body_set = HashSet::new();
        for (loop_id, loop_node) in snarl.node_ids() {
            if matches!(loop_node, ExperimentNode::Loop(..)) {
                let body_nodes = find_loop_body_nodes(loop_id, snarl);
                loop_body_set.extend(body_nodes);
            }
        }

        // Translate nodes to commands
        let mut commands = Vec::new();
        let mut movers = HashSet::new();
        let mut detectors = HashSet::new();
        let mut total_events = 0;

        for node_id in sorted {
            // Skip nodes that are part of loop bodies (they're handled by loop translation)
            if loop_body_set.contains(&node_id) {
                continue;
            }

            if let Some(node) = snarl.get_node(node_id) {
                let (node_commands, node_movers, node_detectors, node_events) =
                    translate_node_with_snarl(node, node_id, snarl);
                commands.extend(node_commands);
                movers.extend(node_movers);
                detectors.extend(node_detectors);
                total_events += node_events;
            }
        }

        Ok(Self {
            commands,
            current_idx: 0,
            total_events,
            movers: movers.into_iter().collect(),
            detectors: detectors.into_iter().collect(),
        })
    }
}

impl Plan for GraphPlan {
    fn plan_type(&self) -> &str {
        "graph_plan"
    }

    fn plan_name(&self) -> &str {
        "Graph Plan"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn movers(&self) -> Vec<String> {
        self.movers.clone()
    }

    fn detectors(&self) -> Vec<String> {
        self.detectors.clone()
    }

    fn num_points(&self) -> usize {
        self.total_events
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        if self.current_idx >= self.commands.len() {
            return None;
        }
        let cmd = self.commands[self.current_idx].clone();
        self.current_idx += 1;
        Some(cmd)
    }

    fn reset(&mut self) {
        self.current_idx = 0;
    }
}

/// Build adjacency list from snarl wires
pub fn build_adjacency(
    snarl: &Snarl<ExperimentNode>,
) -> Result<(HashMap<NodeId, Vec<NodeId>>, Vec<NodeId>), TranslationError> {
    let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut has_input: HashSet<NodeId> = HashSet::new();

    // Initialize all nodes in adjacency
    for (node_id, _) in snarl.node_ids() {
        adjacency.insert(node_id, Vec::new());
    }

    // Build edges from wires
    for (out_pin, in_pin) in snarl.wires() {
        let from = out_pin.node;
        let to = in_pin.node;
        if let Some(v) = adjacency.get_mut(&from) {
            v.push(to);
        }
        has_input.insert(to);
    }

    // Roots are nodes with no inputs
    let roots: Vec<NodeId> = snarl
        .node_ids()
        .filter(|(id, _)| !has_input.contains(id))
        .map(|(id, _)| id)
        .collect();

    Ok((adjacency, roots))
}

/// Topological sort with cycle detection using Kahn's algorithm
pub fn topological_sort(
    adjacency: &HashMap<NodeId, Vec<NodeId>>,
    roots: &[NodeId],
    total_nodes: usize,
) -> Result<Vec<NodeId>, TranslationError> {
    // Count incoming edges
    let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
    for node_id in adjacency.keys() {
        in_degree.insert(*node_id, 0);
    }
    for neighbors in adjacency.values() {
        for neighbor in neighbors {
            *in_degree.get_mut(neighbor).unwrap_or(&mut 0) += 1;
        }
    }

    // Start with roots (zero in-degree)
    let mut queue: VecDeque<NodeId> = roots.iter().copied().collect();
    let mut sorted = Vec::new();

    while let Some(node_id) = queue.pop_front() {
        sorted.push(node_id);
        if let Some(neighbors) = adjacency.get(&node_id) {
            for neighbor in neighbors {
                if let Some(degree) = in_degree.get_mut(neighbor) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(*neighbor);
                    }
                }
            }
        }
    }

    if sorted.len() != total_nodes {
        return Err(TranslationError::CycleDetected);
    }

    Ok(sorted)
}

/// Translate a single node to PlanCommands (with access to full graph for loops)
/// Returns (commands, movers, detectors, event_count)
fn translate_node_with_snarl(
    node: &ExperimentNode,
    node_id: NodeId,
    snarl: &Snarl<ExperimentNode>,
) -> (Vec<PlanCommand>, Vec<String>, Vec<String>, usize) {
    let mut commands = vec![PlanCommand::Checkpoint {
        label: format!("node_{:?}_start", node_id),
    }];
    let mut movers = Vec::new();
    let mut detectors = Vec::new();
    let mut events = 0;

    match node {
        ExperimentNode::Scan {
            actuator,
            start,
            stop,
            points,
        } => {
            if *points > 0 && !actuator.is_empty() {
                movers.push(actuator.clone());
                let step = if *points > 1 {
                    (stop - start) / (*points as f64 - 1.0)
                } else {
                    0.0
                };
                for i in 0..*points {
                    let pos = start + step * i as f64;
                    commands.push(PlanCommand::MoveTo {
                        device_id: actuator.clone(),
                        position: pos,
                    });
                    commands.push(PlanCommand::Checkpoint {
                        label: format!("node_{:?}_point_{}", node_id, i),
                    });
                    commands.push(PlanCommand::EmitEvent {
                        stream: "primary".to_string(),
                        data: HashMap::new(),
                        positions: [(actuator.clone(), pos)].into_iter().collect(),
                    });
                    events += 1;
                }
            }
        }
        ExperimentNode::Acquire(config) => {
            if !config.detector.is_empty() {
                detectors.push(config.detector.clone());
                // Set exposure if specified
                if let Some(exposure_ms) = config.exposure_ms {
                    if exposure_ms > 0.0 {
                        commands.push(PlanCommand::Set {
                            device_id: config.detector.clone(),
                            parameter: "exposure_ms".to_string(),
                            value: exposure_ms.to_string(),
                        });
                    }
                }
                // Generate Trigger+Read for each frame in burst
                for _ in 0..config.frame_count {
                    commands.push(PlanCommand::Trigger {
                        device_id: config.detector.clone(),
                    });
                    commands.push(PlanCommand::Read {
                        device_id: config.detector.clone(),
                    });
                    commands.push(PlanCommand::EmitEvent {
                        stream: "primary".to_string(),
                        data: HashMap::new(),
                        positions: HashMap::new(),
                    });
                    events += 1;
                }
            }
        }
        ExperimentNode::Move(config) => {
            if !config.device.is_empty() {
                movers.push(config.device.clone());
                commands.push(PlanCommand::MoveTo {
                    device_id: config.device.clone(),
                    position: config.position,
                });
                if config.wait_settled {
                    // TODO: Add WaitSettled command when available
                    // For now, just add a checkpoint
                    commands.push(PlanCommand::Checkpoint {
                        label: format!("node_{:?}_settled", node_id),
                    });
                }
            }
        }
        ExperimentNode::Wait { condition } => {
            use super::nodes::WaitCondition;
            match condition {
                WaitCondition::Duration { milliseconds } => {
                    commands.push(PlanCommand::Wait {
                        seconds: *milliseconds / 1000.0,
                    });
                }
                WaitCondition::Threshold { timeout_ms, .. } => {
                    // TODO: Implement threshold-based waits
                    tracing::warn!(
                        "Threshold-based waits not yet implemented, using timeout fallback"
                    );
                    commands.push(PlanCommand::Wait {
                        seconds: *timeout_ms / 1000.0,
                    });
                }
                WaitCondition::Stability { timeout_ms, .. } => {
                    // TODO: Implement stability-based waits
                    tracing::warn!(
                        "Stability-based waits not yet implemented, using timeout fallback"
                    );
                    commands.push(PlanCommand::Wait {
                        seconds: *timeout_ms / 1000.0,
                    });
                }
            }
        }
        ExperimentNode::Loop(config) => {
            use super::nodes::LoopTermination;

            // Get loop body nodes
            let body_nodes = find_loop_body_nodes(node_id, snarl);

            // Determine iteration count based on termination mode
            let iterations = match &config.termination {
                LoopTermination::Count { iterations } => *iterations,
                LoopTermination::Condition { max_iterations, .. } => {
                    tracing::warn!(
                        "Condition-based loop at {:?} using max_iterations={} as safety limit. \
                        True condition evaluation requires RunEngine runtime support.",
                        node_id,
                        max_iterations
                    );
                    *max_iterations
                }
                LoopTermination::Infinite { max_iterations } => {
                    tracing::warn!(
                        "Infinite loop at {:?} using max_iterations={} as safety limit",
                        node_id,
                        max_iterations
                    );
                    *max_iterations
                }
            };

            // Unroll loop body N times
            for i in 0..iterations {
                commands.push(PlanCommand::Checkpoint {
                    label: format!("loop_{:?}_iter_{}_start", node_id, i),
                });

                // Translate each body node for this iteration
                for &body_node_id in &body_nodes {
                    if let Some(body_node) = snarl.get_node(body_node_id) {
                        let (body_cmds, body_movers, body_detectors, body_events) =
                            translate_node_with_snarl(body_node, body_node_id, snarl);
                        commands.extend(body_cmds);
                        movers.extend(body_movers);
                        detectors.extend(body_detectors);
                        events += body_events;
                    }
                }

                commands.push(PlanCommand::Checkpoint {
                    label: format!("loop_{:?}_iter_{}_end", node_id, i),
                });
            }
        }
    }

    commands.push(PlanCommand::Checkpoint {
        label: format!("node_{:?}_end", node_id),
    });

    (commands, movers, detectors, events)
}

/// Find all nodes in a loop's body sub-graph.
///
/// Loop body nodes are those reachable from the loop's body output (pin 1),
/// but NOT reachable from the loop's Next output (pin 0).
fn find_loop_body_nodes(loop_node_id: NodeId, snarl: &Snarl<ExperimentNode>) -> Vec<NodeId> {
    let mut body_nodes = HashSet::new();
    let mut to_visit = VecDeque::new();

    // Find all nodes reachable from loop's body output (pin 1)
    for (out_pin, in_pin) in snarl.wires() {
        if out_pin.node == loop_node_id && out_pin.output == 1 {
            to_visit.push_back(in_pin.node);
        }
    }

    // BFS to find all reachable nodes from body output
    while let Some(node_id) = to_visit.pop_front() {
        if body_nodes.insert(node_id) {
            // Add this node's outputs to visit
            for (out_pin, in_pin) in snarl.wires() {
                if out_pin.node == node_id {
                    to_visit.push_back(in_pin.node);
                }
            }
        }
    }

    // Filter out nodes reachable from Next output (pin 0)
    let mut next_nodes = HashSet::new();
    let mut to_visit_next = VecDeque::new();
    for (out_pin, in_pin) in snarl.wires() {
        if out_pin.node == loop_node_id && out_pin.output == 0 {
            to_visit_next.push_back(in_pin.node);
        }
    }
    while let Some(node_id) = to_visit_next.pop_front() {
        if next_nodes.insert(node_id) {
            for (out_pin, in_pin) in snarl.wires() {
                if out_pin.node == node_id {
                    to_visit_next.push_back(in_pin.node);
                }
            }
        }
    }

    // Body = reachable from body output but NOT from next output
    let pure_body: Vec<NodeId> = body_nodes
        .into_iter()
        .filter(|n| !next_nodes.contains(n))
        .collect();

    // Sort topologically for correct execution order
    // Build adjacency for body nodes only
    let mut body_adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for &node_id in &pure_body {
        body_adjacency.insert(node_id, Vec::new());
    }
    for (out_pin, in_pin) in snarl.wires() {
        if pure_body.contains(&out_pin.node) && pure_body.contains(&in_pin.node) {
            if let Some(v) = body_adjacency.get_mut(&out_pin.node) {
                v.push(in_pin.node);
            }
        }
    }

    // Find roots within body (nodes with no inputs from other body nodes)
    let mut body_has_input: HashSet<NodeId> = HashSet::new();
    for neighbors in body_adjacency.values() {
        for &n in neighbors {
            body_has_input.insert(n);
        }
    }
    let body_roots: Vec<NodeId> = pure_body
        .iter()
        .filter(|n| !body_has_input.contains(n))
        .copied()
        .collect();

    // Topological sort of body nodes
    match topological_sort(&body_adjacency, &body_roots, pure_body.len()) {
        Ok(sorted) => sorted,
        Err(_) => pure_body, // Fallback to unsorted if cycle (should be caught by validation)
    }
}

/// Check if a node is part of a loop body (should be skipped in main traversal).
#[allow(dead_code)]
fn is_loop_body_node(node_id: NodeId, snarl: &Snarl<ExperimentNode>) -> bool {
    // Check if this node is reachable from any Loop node's body output
    for (loop_id, loop_node) in snarl.node_ids() {
        if matches!(loop_node, ExperimentNode::Loop(..))
            && find_loop_body_nodes(loop_id, snarl).contains(&node_id)
        {
            return true;
        }
    }
    false
}

/// Detect cycles in the graph (for validation before translation)
#[allow(dead_code)]
pub fn detect_cycles(snarl: &Snarl<ExperimentNode>) -> Option<String> {
    if snarl.node_ids().count() == 0 {
        return None; // Empty graph has no cycles
    }

    match build_adjacency(snarl) {
        Ok((adjacency, roots)) => {
            if roots.is_empty() && snarl.node_ids().count() > 0 {
                return Some("All nodes have inputs - possible cycle".to_string());
            }
            match topological_sort(&adjacency, &roots, snarl.node_ids().count()) {
                Ok(_) => None,
                Err(TranslationError::CycleDetected) => Some("Graph contains a cycle".to_string()),
                Err(e) => Some(e.to_string()),
            }
        }
        Err(e) => Some(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::nodes::{AcquireConfig, LoopConfig, LoopTermination};

    #[test]
    fn test_empty_graph() {
        let snarl: Snarl<ExperimentNode> = Snarl::new();
        let result = GraphPlan::from_snarl(&snarl);
        assert!(matches!(result, Err(TranslationError::EmptyGraph)));
    }

    #[test]
    fn test_single_node() {
        let mut snarl = Snarl::new();
        snarl.insert_node(egui::pos2(0.0, 0.0), ExperimentNode::default_scan());

        let plan = GraphPlan::from_snarl(&snarl);
        assert!(plan.is_ok());
    }

    #[test]
    fn test_cycle_detection() {
        let snarl: Snarl<ExperimentNode> = Snarl::new();
        // Empty graph - no cycles
        assert!(detect_cycles(&snarl).is_none());
    }

    #[test]
    fn test_loop_body_detection() {
        let mut snarl = Snarl::new();

        // Create Loop node
        let loop_node = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Count { iterations: 3 },
            }),
        );

        // Create two body nodes
        let acquire1 = snarl.insert_node(
            egui::pos2(100.0, 0.0),
            ExperimentNode::Acquire(AcquireConfig {
                detector: "camera".to_string(),
                exposure_ms: Some(100.0),
                frame_count: 1,
            }),
        );

        let acquire2 = snarl.insert_node(
            egui::pos2(200.0, 0.0),
            ExperimentNode::Acquire(AcquireConfig {
                detector: "camera".to_string(),
                exposure_ms: Some(100.0),
                frame_count: 1,
            }),
        );

        // Connect loop body output (pin 1) to acquire1
        snarl.connect(
            egui_snarl::OutPinId {
                node: loop_node,
                output: 1,
            },
            egui_snarl::InPinId {
                node: acquire1,
                input: 0,
            },
        );

        // Connect acquire1 to acquire2
        snarl.connect(
            egui_snarl::OutPinId {
                node: acquire1,
                output: 0,
            },
            egui_snarl::InPinId {
                node: acquire2,
                input: 0,
            },
        );

        // Find body nodes
        let body_nodes = find_loop_body_nodes(loop_node, &snarl);

        // Should find both acquire nodes
        assert_eq!(body_nodes.len(), 2);
        assert!(body_nodes.contains(&acquire1));
        assert!(body_nodes.contains(&acquire2));
    }

    #[test]
    fn test_loop_unrolling() {
        let mut snarl = Snarl::new();

        // Create Loop node with 3 iterations
        let loop_node = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Count { iterations: 3 },
            }),
        );

        // Create an Acquire node in the body
        let acquire = snarl.insert_node(
            egui::pos2(100.0, 0.0),
            ExperimentNode::Acquire(AcquireConfig {
                detector: "camera".to_string(),
                exposure_ms: Some(100.0),
                frame_count: 1,
            }),
        );

        // Connect loop body to acquire
        snarl.connect(
            egui_snarl::OutPinId {
                node: loop_node,
                output: 1,
            },
            egui_snarl::InPinId {
                node: acquire,
                input: 0,
            },
        );

        // Translate to plan
        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Count iteration markers
        let iteration_starts = plan.commands.iter().filter(|cmd| {
            matches!(cmd, PlanCommand::Checkpoint { label } if label.contains("iter_") && label.contains("_start"))
        }).count();

        let iteration_ends = plan.commands.iter().filter(|cmd| {
            matches!(cmd, PlanCommand::Checkpoint { label } if label.contains("iter_") && label.contains("_end"))
        }).count();

        // Should have 3 iteration start/end pairs
        assert_eq!(iteration_starts, 3, "Expected 3 loop iterations");
        assert_eq!(iteration_ends, 3, "Expected 3 loop iterations");

        // Should have 3 Trigger+Read pairs (one per iteration)
        let trigger_count = plan
            .commands
            .iter()
            .filter(|cmd| matches!(cmd, PlanCommand::Trigger { .. }))
            .count();
        assert_eq!(trigger_count, 3, "Expected 3 triggers (one per iteration)");

        // Should have 3 events (one per iteration)
        assert_eq!(plan.total_events, 3, "Expected 3 events total");
    }
}
