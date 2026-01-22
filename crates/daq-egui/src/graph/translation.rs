//! Translation from visual node graph to executable Plan.

use std::collections::{HashMap, HashSet, VecDeque};
use egui_snarl::{NodeId, Snarl};
use daq_experiment::plans::{Plan, PlanCommand};
use super::nodes::ExperimentNode;

/// Errors that can occur during graph translation
#[derive(Debug, Clone)]
pub enum TranslationError {
    /// Graph contains a cycle
    CycleDetected,
    /// Node has invalid configuration
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

        // Translate nodes to commands
        let mut commands = Vec::new();
        let mut movers = HashSet::new();
        let mut detectors = HashSet::new();
        let mut total_events = 0;

        for node_id in sorted {
            if let Some(node) = snarl.get_node(node_id) {
                let (node_commands, node_movers, node_detectors, node_events) =
                    translate_node(node, node_id);
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
fn build_adjacency(snarl: &Snarl<ExperimentNode>) -> Result<(HashMap<NodeId, Vec<NodeId>>, Vec<NodeId>), TranslationError> {
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
        adjacency.get_mut(&from).map(|v| v.push(to));
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
fn topological_sort(
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

/// Translate a single node to PlanCommands
/// Returns (commands, movers, detectors, event_count)
fn translate_node(
    node: &ExperimentNode,
    node_id: NodeId,
) -> (Vec<PlanCommand>, Vec<String>, Vec<String>, usize) {
    let mut commands = vec![
        PlanCommand::Checkpoint {
            label: format!("node_{:?}_start", node_id),
        },
    ];
    let mut movers = Vec::new();
    let mut detectors = Vec::new();
    let mut events = 0;

    match node {
        ExperimentNode::Scan { actuator, start, stop, points } => {
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
        ExperimentNode::Acquire { detector, duration_ms } => {
            if !detector.is_empty() {
                detectors.push(detector.clone());
                // Set exposure if duration specified
                if *duration_ms > 0.0 {
                    commands.push(PlanCommand::Set {
                        device_id: detector.clone(),
                        parameter: "exposure_ms".to_string(),
                        value: duration_ms.to_string(),
                    });
                }
                commands.push(PlanCommand::Trigger {
                    device_id: detector.clone(),
                });
                commands.push(PlanCommand::Read {
                    device_id: detector.clone(),
                });
                commands.push(PlanCommand::EmitEvent {
                    stream: "primary".to_string(),
                    data: HashMap::new(),
                    positions: HashMap::new(),
                });
                events += 1;
            }
        }
        ExperimentNode::Move { device, position } => {
            if !device.is_empty() {
                movers.push(device.clone());
                commands.push(PlanCommand::MoveTo {
                    device_id: device.clone(),
                    position: *position,
                });
            }
        }
        ExperimentNode::Wait { duration_ms } => {
            commands.push(PlanCommand::Wait {
                seconds: *duration_ms / 1000.0,
            });
        }
        ExperimentNode::Loop { iterations } => {
            // Loop node itself just marks checkpoint
            // Loop body is handled by graph structure (body output connects to loop content)
            // For now, loops are not fully implemented - just add checkpoint
            commands.push(PlanCommand::Checkpoint {
                label: format!("node_{:?}_loop_iter_{}", node_id, iterations),
            });
        }
    }

    commands.push(PlanCommand::Checkpoint {
        label: format!("node_{:?}_end", node_id),
    });

    (commands, movers, detectors, events)
}

/// Detect cycles in the graph (for validation before translation)
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
}
