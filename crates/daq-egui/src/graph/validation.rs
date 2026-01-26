//! Connection validation logic for experiment graphs.

use super::nodes::ExperimentNode;

/// Pin types for connection validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinType {
    /// Sequential execution flow (output -> input)
    Flow,
    /// Loop body connection (special output from Loop node)
    LoopBody,
}

/// Get the type of an output pin for a node.
pub fn output_pin_type(node: &ExperimentNode, output_idx: usize) -> PinType {
    match node {
        ExperimentNode::Loop { .. } | ExperimentNode::NestedScan { .. } => {
            if output_idx == 0 {
                PinType::Flow // "Next" output (continues after loop completes)
            } else {
                PinType::LoopBody // "Body" output (runs each iteration)
            }
        }
        _ => PinType::Flow,
    }
}

/// Get the type of an input pin for a node.
pub fn input_pin_type(_node: &ExperimentNode, _input_idx: usize) -> PinType {
    // For now, all inputs accept flow connections.
    // Loop body is handled by output_pin_type.
    PinType::Flow
}

/// Validate a proposed connection between two nodes.
///
/// Returns `Ok(())` if the connection is valid, or `Err(message)` explaining
/// why the connection cannot be made.
pub fn validate_connection(
    from_node: &ExperimentNode,
    from_output: usize,
    to_node: &ExperimentNode,
    to_input: usize,
) -> Result<(), String> {
    let out_type = output_pin_type(from_node, from_output);
    let in_type = input_pin_type(to_node, to_input);

    // Flow pins can connect to flow pins
    // LoopBody can connect to flow (it's still a flow, just semantically different)
    match (out_type, in_type) {
        (PinType::Flow, PinType::Flow) => Ok(()),
        (PinType::LoopBody, PinType::Flow) => Ok(()),
        _ => Err(format!(
            "Cannot connect {:?} output to {:?} input",
            out_type, in_type
        )),
    }
}

/// Find all nodes that can reach the given node (ancestors).
#[allow(dead_code)]
fn find_ancestors(
    node_id: egui_snarl::NodeId,
    snarl: &egui_snarl::Snarl<ExperimentNode>,
) -> std::collections::HashSet<egui_snarl::NodeId> {
    use std::collections::{HashSet, VecDeque};

    let mut ancestors = HashSet::new();
    let mut to_visit = VecDeque::new();

    // Find all nodes that have edges TO node_id
    for (out_pin, in_pin) in snarl.wires() {
        if in_pin.node == node_id {
            to_visit.push_back(out_pin.node);
        }
    }

    // BFS backward to find all ancestors
    while let Some(ancestor) = to_visit.pop_front() {
        if ancestors.insert(ancestor) {
            // Add this ancestor's predecessors
            for (out_pin, in_pin) in snarl.wires() {
                if in_pin.node == ancestor {
                    to_visit.push_back(out_pin.node);
                }
            }
        }
    }

    ancestors
}

/// Find all nodes in a loop's body sub-graph (duplicated from translation.rs to avoid circular dep).
#[allow(dead_code)]
fn find_loop_body_nodes(
    loop_node_id: egui_snarl::NodeId,
    snarl: &egui_snarl::Snarl<ExperimentNode>,
) -> Vec<egui_snarl::NodeId> {
    use std::collections::{HashSet, VecDeque};

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
    body_nodes
        .into_iter()
        .filter(|n| !next_nodes.contains(n))
        .collect()
}

/// Validate a single loop node's body structure.
#[allow(dead_code)]
fn validate_loop_body(
    loop_id: egui_snarl::NodeId,
    snarl: &egui_snarl::Snarl<ExperimentNode>,
) -> Option<String> {
    // Find loop's ancestors (nodes that can reach loop_id)
    let ancestors = find_ancestors(loop_id, snarl);

    // Find body nodes
    let body_nodes = find_loop_body_nodes(loop_id, snarl);

    // Check for back-edges from body to ancestors or loop itself
    for &body_node in &body_nodes {
        for (out_pin, in_pin) in snarl.wires() {
            if out_pin.node == body_node {
                if in_pin.node == loop_id {
                    return Some(format!(
                        "Loop body node {:?} connects back to loop {:?} - would cause infinite recursion",
                        body_node, loop_id
                    ));
                }
                if ancestors.contains(&in_pin.node) {
                    return Some(format!(
                        "Loop body node {:?} connects back to ancestor {:?} - would cause infinite recursion",
                        body_node, in_pin.node
                    ));
                }
            }
        }
    }

    None
}

/// Warn if a loop body contains relative moves (position compounds each iteration).
#[allow(dead_code)]
fn warn_relative_moves_in_loop(
    loop_id: egui_snarl::NodeId,
    snarl: &egui_snarl::Snarl<ExperimentNode>,
) -> Option<String> {
    use super::nodes::MoveMode;

    let body_nodes = find_loop_body_nodes(loop_id, snarl);
    for body_node_id in body_nodes {
        if let Some(ExperimentNode::Move(config)) = snarl.get_node(body_node_id) {
            if config.mode == MoveMode::Relative {
                return Some(format!(
                    "Warning: Relative move in loop body (node {:?}) - position will compound each iteration",
                    body_node_id
                ));
            }
        }
    }
    None
}

/// Validate all loop bodies in the graph.
#[allow(dead_code)]
pub fn validate_loop_bodies(
    snarl: &egui_snarl::Snarl<ExperimentNode>,
) -> Vec<(egui_snarl::NodeId, String)> {
    let mut errors = Vec::new();

    for (loop_id, loop_node) in snarl.node_ids() {
        if matches!(loop_node, ExperimentNode::Loop(..)) {
            // Check for back-edges
            if let Some(error) = validate_loop_body(loop_id, snarl) {
                errors.push((loop_id, error));
            }

            // Warn about relative moves
            if let Some(warning) = warn_relative_moves_in_loop(loop_id, snarl) {
                errors.push((loop_id, warning));
            }
        }
    }

    errors
}

/// Validate entire graph structure, including cycle detection.
/// Returns None if valid, or Some(error_message) if invalid.
pub fn validate_graph_structure<N>(snarl: &egui_snarl::Snarl<N>) -> Option<String>
where
    N: Clone,
{
    use std::collections::{HashMap, HashSet, VecDeque};

    if snarl.node_ids().count() == 0 {
        return None; // Empty is valid (just nothing to run)
    }

    // Build adjacency and find roots
    let mut adjacency: HashMap<egui_snarl::NodeId, Vec<egui_snarl::NodeId>> = HashMap::new();
    let mut has_input: HashSet<egui_snarl::NodeId> = HashSet::new();

    for (node_id, _) in snarl.node_ids() {
        adjacency.insert(node_id, Vec::new());
    }

    for (out_pin, in_pin) in snarl.wires() {
        if let Some(v) = adjacency.get_mut(&out_pin.node) {
            v.push(in_pin.node);
        }
        has_input.insert(in_pin.node);
    }

    let roots: Vec<_> = snarl
        .node_ids()
        .filter(|(id, _)| !has_input.contains(id))
        .map(|(id, _)| id)
        .collect();

    if roots.is_empty() {
        return Some("No root nodes - graph may contain cycles".to_string());
    }

    // Kahn's algorithm for cycle detection
    let mut in_degree: HashMap<egui_snarl::NodeId, usize> = HashMap::new();
    for node_id in adjacency.keys() {
        in_degree.insert(*node_id, 0);
    }
    for neighbors in adjacency.values() {
        for n in neighbors {
            *in_degree.get_mut(n).unwrap_or(&mut 0) += 1;
        }
    }

    let mut queue: VecDeque<_> = roots.iter().copied().collect();
    let mut sorted_count = 0;

    while let Some(node_id) = queue.pop_front() {
        sorted_count += 1;
        if let Some(neighbors) = adjacency.get(&node_id) {
            for neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(*neighbor);
                    }
                }
            }
        }
    }

    if sorted_count != snarl.node_ids().count() {
        return Some("Graph contains a cycle".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::nodes::{AcquireConfig, LoopConfig, LoopTermination, MoveConfig, MoveMode};

    #[test]
    fn test_flow_to_flow_valid() {
        let scan = ExperimentNode::default_scan();
        let acquire = ExperimentNode::default_acquire();

        let result = validate_connection(&scan, 0, &acquire, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_loop_body_to_flow_valid() {
        let loop_node = ExperimentNode::default_loop();
        let acquire = ExperimentNode::default_acquire();

        // Loop body output (index 1) to acquire input
        let result = validate_connection(&loop_node, 1, &acquire, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_loop_next_to_flow_valid() {
        let loop_node = ExperimentNode::default_loop();
        let acquire = ExperimentNode::default_acquire();

        // Loop next output (index 0) to acquire input
        let result = validate_connection(&loop_node, 0, &acquire, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_loop_backedge_detection() {
        let mut snarl = egui_snarl::Snarl::new();

        // Create a scan node (will be ancestor)
        let scan = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Scan {
                actuator: "stage".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
            },
        );

        // Create loop node
        let loop_node = snarl.insert_node(
            egui::pos2(100.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Count { iterations: 5 },
            }),
        );

        // Create acquire node in loop body
        let acquire = snarl.insert_node(
            egui::pos2(200.0, 0.0),
            ExperimentNode::Acquire(AcquireConfig {
                detector: "camera".to_string(),
                exposure_ms: Some(100.0),
                frame_count: 1,
            }),
        );

        // Connect scan -> loop (scan is ancestor of loop)
        snarl.connect(
            egui_snarl::OutPinId {
                node: scan,
                output: 0,
            },
            egui_snarl::InPinId {
                node: loop_node,
                input: 0,
            },
        );

        // Connect loop body -> acquire
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

        // Invalid: connect acquire back to scan (back-edge to ancestor)
        snarl.connect(
            egui_snarl::OutPinId {
                node: acquire,
                output: 0,
            },
            egui_snarl::InPinId {
                node: scan,
                input: 0,
            },
        );

        // Validate - should detect back-edge
        let errors = validate_loop_bodies(&snarl);
        assert!(!errors.is_empty(), "Should detect back-edge");
        // The error could mention "ancestor" or "loop" depending on which back-edge is detected first
        assert!(
            errors[0].1.contains("infinite recursion"),
            "Error should mention infinite recursion: {}",
            errors[0].1
        );
    }

    #[test]
    fn test_relative_move_warning() {
        let mut snarl = egui_snarl::Snarl::new();

        // Create loop node
        let loop_node = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Count { iterations: 5 },
            }),
        );

        // Create relative move node in loop body
        let move_node = snarl.insert_node(
            egui::pos2(100.0, 0.0),
            ExperimentNode::Move(MoveConfig {
                device: "stage".to_string(),
                position: 10.0,
                mode: MoveMode::Relative, // Relative move
                wait_settled: true,
            }),
        );

        // Connect loop body -> move
        snarl.connect(
            egui_snarl::OutPinId {
                node: loop_node,
                output: 1,
            },
            egui_snarl::InPinId {
                node: move_node,
                input: 0,
            },
        );

        // Validate - should warn about relative move
        let warnings = validate_loop_bodies(&snarl);
        assert!(!warnings.is_empty(), "Should warn about relative move");
        assert!(
            warnings[0].1.contains("Relative move"),
            "Should mention relative move: {}",
            warnings[0].1
        );
    }

    #[test]
    fn test_absolute_move_in_loop_ok() {
        let mut snarl = egui_snarl::Snarl::new();

        // Create loop node
        let loop_node = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Count { iterations: 5 },
            }),
        );

        // Create absolute move node in loop body
        let move_node = snarl.insert_node(
            egui::pos2(100.0, 0.0),
            ExperimentNode::Move(MoveConfig {
                device: "stage".to_string(),
                position: 50.0,
                mode: MoveMode::Absolute, // Absolute move is OK
                wait_settled: true,
            }),
        );

        // Connect loop body -> move
        snarl.connect(
            egui_snarl::OutPinId {
                node: loop_node,
                output: 1,
            },
            egui_snarl::InPinId {
                node: move_node,
                input: 0,
            },
        );

        // Validate - should have no warnings
        let warnings = validate_loop_bodies(&snarl);
        assert!(warnings.is_empty(), "Absolute moves should not warn");
    }
}
