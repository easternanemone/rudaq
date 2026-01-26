//! Connection validation logic for experiment graphs.

use super::nodes::{AdaptiveScanConfig, ExperimentNode, NestedScanConfig, TriggerCondition};

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

/// Validate a NestedScan configuration.
///
/// Returns a list of error/warning strings for invalid configuration.
pub fn validate_nested_scan(config: &NestedScanConfig) -> Vec<String> {
    let mut errors = Vec::new();

    // Outer dimension validation
    if config.outer.actuator.is_empty() {
        errors.push("Outer scan: device not selected".to_string());
    }
    if config.outer.points == 0 {
        errors.push("Outer scan: points must be > 0".to_string());
    }
    if config.outer.dimension_name.is_empty() {
        errors.push("Outer scan: dimension name required".to_string());
    }

    // Inner dimension validation
    if config.inner.actuator.is_empty() {
        errors.push("Inner scan: device not selected".to_string());
    }
    if config.inner.points == 0 {
        errors.push("Inner scan: points must be > 0".to_string());
    }
    if config.inner.dimension_name.is_empty() {
        errors.push("Inner scan: dimension name required".to_string());
    }

    // Warn on same actuator for outer and inner (likely user error)
    if config.outer.actuator == config.inner.actuator && !config.outer.actuator.is_empty() {
        errors.push("Warning: outer and inner use same actuator".to_string());
    }

    // Warn on duplicate dimension names
    if config.outer.dimension_name == config.inner.dimension_name
        && !config.outer.dimension_name.is_empty()
    {
        errors.push("Warning: outer and inner have same dimension name".to_string());
    }

    errors
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

/// Validate AdaptiveScan configuration.
pub fn validate_adaptive_scan(config: &AdaptiveScanConfig) -> Vec<String> {
    let mut errors = Vec::new();

    // Validate base scan
    if config.scan.actuator.is_empty() {
        errors.push("Adaptive scan: device not selected".to_string());
    }
    if config.scan.points == 0 {
        errors.push("Adaptive scan: points must be > 0".to_string());
    }

    // Validate triggers
    if config.triggers.is_empty() {
        errors.push("Adaptive scan: at least one trigger required".to_string());
    }

    for (i, trigger) in config.triggers.iter().enumerate() {
        match trigger {
            TriggerCondition::Threshold { device_id, .. } => {
                if device_id.is_empty() {
                    errors.push(format!("Trigger {}: device not selected", i + 1));
                }
            }
            TriggerCondition::PeakDetection {
                device_id,
                min_prominence,
                ..
            } => {
                if device_id.is_empty() {
                    errors.push(format!("Trigger {}: device not selected", i + 1));
                }
                if *min_prominence <= 0.0 {
                    errors.push(format!("Trigger {}: prominence must be > 0", i + 1));
                }
            }
        }
    }

    errors
}

/// Validate all loop bodies in the graph (Loop, NestedScan, and AdaptiveScan nodes).
#[allow(dead_code)]
pub fn validate_loop_bodies(
    snarl: &egui_snarl::Snarl<ExperimentNode>,
) -> Vec<(egui_snarl::NodeId, String)> {
    let mut errors = Vec::new();

    for (node_id, node) in snarl.node_ids() {
        match node {
            ExperimentNode::Loop(..) => {
                // Check for back-edges
                if let Some(error) = validate_loop_body(node_id, snarl) {
                    errors.push((node_id, error));
                }

                // Warn about relative moves
                if let Some(warning) = warn_relative_moves_in_loop(node_id, snarl) {
                    errors.push((node_id, warning));
                }
            }
            ExperimentNode::NestedScan(config) => {
                // Validate NestedScan configuration
                for error in validate_nested_scan(config) {
                    errors.push((node_id, error));
                }

                // Check for back-edges (NestedScan also has body nodes)
                if let Some(error) = validate_loop_body(node_id, snarl) {
                    errors.push((node_id, error));
                }

                // Warn about relative moves in body
                if let Some(warning) = warn_relative_moves_in_loop(node_id, snarl) {
                    errors.push((node_id, warning));
                }
            }
            ExperimentNode::AdaptiveScan(config) => {
                // Validate AdaptiveScan configuration
                for error in validate_adaptive_scan(config) {
                    errors.push((node_id, error));
                }
            }
            _ => {}
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
    use crate::graph::nodes::{
        AcquireConfig, LoopConfig, LoopTermination, MoveConfig, MoveMode, NestedScanConfig,
        ScanDimension,
    };

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

    #[test]
    fn test_nested_scan_validation_empty_actuators() {
        let config = NestedScanConfig {
            outer: ScanDimension {
                actuator: String::new(),
                start: 0.0,
                stop: 100.0,
                points: 10,
                dimension_name: "x".to_string(),
            },
            inner: ScanDimension {
                actuator: String::new(),
                start: 0.0,
                stop: 50.0,
                points: 5,
                dimension_name: "y".to_string(),
            },
            nesting_warning_depth: 3,
        };

        let errors = validate_nested_scan(&config);
        assert!(
            errors.iter().any(|e| e.contains("Outer scan: device")),
            "Should catch empty outer actuator"
        );
        assert!(
            errors.iter().any(|e| e.contains("Inner scan: device")),
            "Should catch empty inner actuator"
        );
    }

    #[test]
    fn test_nested_scan_validation_zero_points() {
        let config = NestedScanConfig {
            outer: ScanDimension {
                actuator: "stage_x".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 0,
                dimension_name: "x".to_string(),
            },
            inner: ScanDimension {
                actuator: "stage_y".to_string(),
                start: 0.0,
                stop: 50.0,
                points: 0,
                dimension_name: "y".to_string(),
            },
            nesting_warning_depth: 3,
        };

        let errors = validate_nested_scan(&config);
        assert!(
            errors.iter().any(|e| e.contains("points must be > 0")),
            "Should catch zero points: {:?}",
            errors
        );
    }

    #[test]
    fn test_nested_scan_validation_same_actuator_warning() {
        let config = NestedScanConfig {
            outer: ScanDimension {
                actuator: "stage_x".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
                dimension_name: "x".to_string(),
            },
            inner: ScanDimension {
                actuator: "stage_x".to_string(), // Same as outer
                start: 0.0,
                stop: 50.0,
                points: 5,
                dimension_name: "y".to_string(),
            },
            nesting_warning_depth: 3,
        };

        let errors = validate_nested_scan(&config);
        assert!(
            errors.iter().any(|e| e.contains("same actuator")),
            "Should warn about same actuator: {:?}",
            errors
        );
    }

    #[test]
    fn test_nested_scan_validation_valid() {
        let config = NestedScanConfig {
            outer: ScanDimension {
                actuator: "stage_x".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
                dimension_name: "x".to_string(),
            },
            inner: ScanDimension {
                actuator: "stage_y".to_string(),
                start: 0.0,
                stop: 50.0,
                points: 5,
                dimension_name: "y".to_string(),
            },
            nesting_warning_depth: 3,
        };

        let errors = validate_nested_scan(&config);
        assert!(
            errors.is_empty(),
            "Valid config should have no errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_nested_scan_body_validation() {
        let mut snarl = egui_snarl::Snarl::new();

        // Create NestedScan node
        let nested_node = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::NestedScan(NestedScanConfig {
                outer: ScanDimension {
                    actuator: "stage_x".to_string(),
                    start: 0.0,
                    stop: 100.0,
                    points: 10,
                    dimension_name: "x".to_string(),
                },
                inner: ScanDimension {
                    actuator: "stage_y".to_string(),
                    start: 0.0,
                    stop: 50.0,
                    points: 5,
                    dimension_name: "y".to_string(),
                },
                nesting_warning_depth: 3,
            }),
        );

        // Create relative move in body (should warn)
        let move_node = snarl.insert_node(
            egui::pos2(100.0, 0.0),
            ExperimentNode::Move(MoveConfig {
                device: "stage_z".to_string(),
                position: 10.0,
                mode: MoveMode::Relative,
                wait_settled: true,
            }),
        );

        // Connect NestedScan body -> move
        snarl.connect(
            egui_snarl::OutPinId {
                node: nested_node,
                output: 1,
            },
            egui_snarl::InPinId {
                node: move_node,
                input: 0,
            },
        );

        // Validate - should warn about relative move in body
        let warnings = validate_loop_bodies(&snarl);
        assert!(
            warnings
                .iter()
                .any(|(_, msg)| msg.contains("Relative move")),
            "Should warn about relative move in NestedScan body: {:?}",
            warnings
        );
    }

    #[test]
    fn test_adaptive_scan_validation_empty_actuator() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, TriggerCondition, TriggerLogic,
        };

        let config = AdaptiveScanConfig {
            scan: ScanDimension {
                actuator: String::new(), // Empty
                dimension_name: "pos".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
            },
            triggers: vec![TriggerCondition::default()],
            trigger_logic: TriggerLogic::Any,
            action: AdaptiveAction::Zoom2x,
            require_approval: false,
        };

        let errors = validate_adaptive_scan(&config);
        assert!(
            errors.iter().any(|e| e.contains("device not selected")),
            "Should catch empty actuator: {:?}",
            errors
        );
    }

    #[test]
    fn test_adaptive_scan_validation_no_triggers() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, TriggerLogic,
        };

        let config = AdaptiveScanConfig {
            scan: ScanDimension {
                actuator: "stage".to_string(),
                dimension_name: "pos".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
            },
            triggers: vec![], // No triggers
            trigger_logic: TriggerLogic::Any,
            action: AdaptiveAction::Zoom2x,
            require_approval: false,
        };

        let errors = validate_adaptive_scan(&config);
        assert!(
            errors.iter().any(|e| e.contains("at least one trigger")),
            "Should catch empty triggers: {:?}",
            errors
        );
    }

    #[test]
    fn test_adaptive_scan_validation_trigger_no_device() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, ThresholdOp, TriggerCondition,
            TriggerLogic,
        };

        let config = AdaptiveScanConfig {
            scan: ScanDimension {
                actuator: "stage".to_string(),
                dimension_name: "pos".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
            },
            triggers: vec![TriggerCondition::Threshold {
                device_id: String::new(), // Empty device
                operator: ThresholdOp::GreaterThan,
                value: 100.0,
            }],
            trigger_logic: TriggerLogic::Any,
            action: AdaptiveAction::Zoom2x,
            require_approval: false,
        };

        let errors = validate_adaptive_scan(&config);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("Trigger 1") && e.contains("device not selected")),
            "Should catch trigger without device: {:?}",
            errors
        );
    }

    #[test]
    fn test_adaptive_scan_validation_peak_prominence() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, TriggerCondition, TriggerLogic,
        };

        let config = AdaptiveScanConfig {
            scan: ScanDimension {
                actuator: "stage".to_string(),
                dimension_name: "pos".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
            },
            triggers: vec![TriggerCondition::PeakDetection {
                device_id: "sensor".to_string(),
                min_prominence: -1.0, // Invalid
                min_height: None,
            }],
            trigger_logic: TriggerLogic::Any,
            action: AdaptiveAction::Zoom2x,
            require_approval: false,
        };

        let errors = validate_adaptive_scan(&config);
        assert!(
            errors.iter().any(|e| e.contains("prominence must be > 0")),
            "Should catch invalid prominence: {:?}",
            errors
        );
    }

    #[test]
    fn test_adaptive_scan_validation_valid() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, ThresholdOp, TriggerCondition,
            TriggerLogic,
        };

        let config = AdaptiveScanConfig {
            scan: ScanDimension {
                actuator: "wavelength".to_string(),
                dimension_name: "lambda".to_string(),
                start: 400.0,
                stop: 800.0,
                points: 50,
            },
            triggers: vec![
                TriggerCondition::Threshold {
                    device_id: "power_meter".to_string(),
                    operator: ThresholdOp::GreaterThan,
                    value: 1000.0,
                },
                TriggerCondition::PeakDetection {
                    device_id: "power_meter".to_string(),
                    min_prominence: 100.0,
                    min_height: Some(500.0),
                },
            ],
            trigger_logic: TriggerLogic::Any,
            action: AdaptiveAction::MoveToPeak,
            require_approval: true,
        };

        let errors = validate_adaptive_scan(&config);
        assert!(
            errors.is_empty(),
            "Valid config should have no errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_adaptive_scan_in_graph_validation() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, TriggerLogic,
        };

        let mut snarl = egui_snarl::Snarl::new();

        // Create AdaptiveScan node with invalid config (no triggers)
        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::AdaptiveScan(AdaptiveScanConfig {
                scan: ScanDimension {
                    actuator: "stage".to_string(),
                    dimension_name: "pos".to_string(),
                    start: 0.0,
                    stop: 100.0,
                    points: 10,
                },
                triggers: vec![], // Invalid - no triggers
                trigger_logic: TriggerLogic::Any,
                action: AdaptiveAction::Zoom2x,
                require_approval: false,
            }),
        );

        let errors = validate_loop_bodies(&snarl);
        assert!(
            errors
                .iter()
                .any(|(_, msg)| msg.contains("at least one trigger")),
            "Should catch no triggers in graph validation: {:?}",
            errors
        );
    }
}
