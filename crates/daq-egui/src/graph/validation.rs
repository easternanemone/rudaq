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
        ExperimentNode::Loop { .. } => {
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
        adjacency.get_mut(&out_pin.node).map(|v| v.push(in_pin.node));
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
}
