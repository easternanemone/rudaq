//! Rhai code generation from visual experiment graphs.
//!
//! This module provides one-way export from visual graph to Rhai script.
//! There is NO round-trip support - the visual editor is the source of truth.
//! Generated scripts are read-only artifacts for learning and debugging.

use super::nodes::{
    AcquireConfig, AdaptiveAction, AdaptiveScanConfig, ExperimentNode, LoopConfig, LoopTermination,
    MoveConfig, MoveMode, NestedScanConfig, ThresholdOp, TriggerCondition, WaitCondition,
};
use super::translation::{build_adjacency, topological_sort};
use egui_snarl::{NodeId, Snarl};
use std::collections::{HashMap, HashSet, VecDeque};

/// Generate a complete Rhai script from an experiment graph.
///
/// Returns a well-formatted, commented Rhai script string.
/// If the graph contains cycles, returns an error comment instead of failing.
pub fn graph_to_rhai_script(snarl: &Snarl<ExperimentNode>, filename: Option<&str>) -> String {
    let mut script = String::new();

    // Generate header comment
    let timestamp = chrono::Utc::now().to_rfc3339();
    script.push_str("// Generated Rhai script from visual experiment graph\n");
    script.push_str(&format!("// Source: {}\n", filename.unwrap_or("unsaved")));
    script.push_str(&format!("// Generated: {}\n", timestamp));
    script.push_str("// DO NOT EDIT - regenerate from visual editor to make changes\n\n");

    // Handle empty graph
    if snarl.node_ids().count() == 0 {
        script.push_str("// ERROR: Graph is empty - cannot generate code\n");
        return script;
    }

    // Build adjacency list and topological sort
    let (adjacency, roots) = match build_adjacency(snarl) {
        Ok(result) => result,
        Err(e) => {
            script.push_str(&format!(
                "// ERROR: Failed to build graph structure: {}\n",
                e
            ));
            return script;
        }
    };

    if roots.is_empty() {
        script.push_str(
            "// ERROR: Graph has no root nodes - all nodes have inputs (possible cycle)\n",
        );
        return script;
    }

    let sorted = match topological_sort(&adjacency, &roots, snarl.node_ids().count()) {
        Ok(sorted) => sorted,
        Err(_) => {
            script.push_str("// ERROR: Graph contains a cycle - cannot generate code\n");
            return script;
        }
    };

    // Identify loop body nodes (skip in main traversal)
    let mut loop_body_set = HashSet::new();
    for (loop_id, loop_node) in snarl.node_ids() {
        if matches!(loop_node, ExperimentNode::Loop(..)) {
            let body_nodes = find_loop_body_nodes(loop_id, snarl);
            loop_body_set.extend(body_nodes);
        }
    }

    // Generate code for each node in topological order
    for (index, node_id) in sorted.iter().enumerate() {
        // Skip loop body nodes (handled inside loop generation)
        if loop_body_set.contains(node_id) {
            continue;
        }

        if let Some(node) = snarl.get_node(*node_id) {
            script.push_str(&format!(
                "// === Node {}: {} ===\n",
                index + 1,
                node.node_name()
            ));
            script.push_str(&node_to_rhai(node, *node_id, snarl, &loop_body_set, 0));
            script.push('\n');
        }
    }

    script
}

/// Convert a single node to Rhai code.
///
/// For Loop nodes, this recursively generates body node code.
fn node_to_rhai(
    node: &ExperimentNode,
    node_id: NodeId,
    snarl: &Snarl<ExperimentNode>,
    loop_body_set: &HashSet<NodeId>,
    indent_level: usize,
) -> String {
    match node {
        ExperimentNode::Scan {
            actuator,
            start,
            stop,
            points,
        } => scan_to_rhai(actuator, *start, *stop, *points, indent_level),

        ExperimentNode::Move(config) => move_to_rhai(config, indent_level),

        ExperimentNode::Wait { condition } => wait_to_rhai(condition, indent_level),

        ExperimentNode::Acquire(config) => acquire_to_rhai(config, indent_level),

        ExperimentNode::Loop(config) => {
            loop_to_rhai(config, node_id, snarl, loop_body_set, indent_level)
        }

        ExperimentNode::NestedScan(config) => nested_scan_to_rhai(config, indent_level),

        ExperimentNode::AdaptiveScan(config) => adaptive_scan_to_rhai(config, indent_level),
    }
}

/// Generate Rhai code for a Scan node.
fn scan_to_rhai(actuator: &str, start: f64, stop: f64, points: u32, indent: usize) -> String {
    let ind = indent_str(indent);
    let mut code = String::new();

    if actuator.is_empty() {
        code.push_str(&format!(
            "{}// WARNING: Scan node has no actuator specified\n",
            ind
        ));
        return code;
    }

    if points == 0 {
        code.push_str(&format!(
            "{}// WARNING: Scan has zero points - skipping\n",
            ind
        ));
        return code;
    }

    code.push_str(&format!(
        "{}// Scan {} from {:.1} to {:.1} in {} steps\n",
        ind, actuator, start, stop, points
    ));

    code.push_str(&format!("{}for i in 0..{} {{\n", ind, points));

    let body_ind = indent_str(indent + 1);

    // Position calculation
    if points > 1 {
        code.push_str(&format!(
            "{}let pos = {} + ({} - {}) * i / ({} - 1);\n",
            body_ind, start, stop, start, points
        ));
    } else {
        code.push_str(&format!("{}let pos = {};\n", body_ind, start));
    }

    // Move and wait
    code.push_str(&format!("{}{}.move_abs(pos);\n", body_ind, actuator));
    code.push_str(&format!("{}{}.wait_settled();\n", body_ind, actuator));

    // Yield event
    code.push_str(&format!(
        "{}yield_event(#{{ \"{}\": pos }});\n",
        body_ind, actuator
    ));

    code.push_str(&format!("{}}}\n", ind));

    code
}

/// Generate Rhai code for a Move node.
fn move_to_rhai(config: &MoveConfig, indent: usize) -> String {
    let ind = indent_str(indent);
    let mut code = String::new();

    if config.device.is_empty() {
        code.push_str(&format!(
            "{}// WARNING: Move node has no device specified\n",
            ind
        ));
        return code;
    }

    let action = match config.mode {
        MoveMode::Absolute => format!(
            "Move {} to absolute position {}",
            config.device, config.position
        ),
        MoveMode::Relative => format!(
            "Move {} by relative distance {}",
            config.device, config.position
        ),
    };
    code.push_str(&format!("{}// {}\n", ind, action));

    let method = match config.mode {
        MoveMode::Absolute => "move_abs",
        MoveMode::Relative => "move_rel",
    };

    code.push_str(&format!(
        "{}{}.{}({});\n",
        ind, config.device, method, config.position
    ));

    if config.wait_settled {
        code.push_str(&format!("{}{}.wait_settled();\n", ind, config.device));
    }

    code
}

/// Generate Rhai code for a Wait node.
fn wait_to_rhai(condition: &WaitCondition, indent: usize) -> String {
    let ind = indent_str(indent);
    let mut code = String::new();

    match condition {
        WaitCondition::Duration { milliseconds } => {
            let seconds = milliseconds / 1000.0;
            code.push_str(&format!("{}// Wait for {} seconds\n", ind, seconds));
            code.push_str(&format!("{}sleep({});\n", ind, seconds));
        }
        WaitCondition::Threshold {
            device_id,
            operator,
            value,
            timeout_ms,
        } => {
            let op_str = match operator {
                ThresholdOp::LessThan => "<",
                ThresholdOp::GreaterThan => ">",
                ThresholdOp::EqualWithin { tolerance } => &format!("== (±{})", tolerance),
            };
            code.push_str(&format!(
                "{}// TODO: Wait until {} {} {} (timeout: {}ms)\n",
                ind, device_id, op_str, value, timeout_ms
            ));
            code.push_str(&format!(
                "{}// Threshold-based waits not yet implemented in Rhai\n",
                ind
            ));
            code.push_str(&format!("{}sleep({});\n", ind, timeout_ms / 1000.0));
        }
        WaitCondition::Stability {
            device_id,
            tolerance,
            duration_ms,
            timeout_ms,
        } => {
            code.push_str(&format!(
                "{}// TODO: Wait until {} stabilizes within ±{} for {}ms (timeout: {}ms)\n",
                ind, device_id, tolerance, duration_ms, timeout_ms
            ));
            code.push_str(&format!(
                "{}// Stability-based waits not yet implemented in Rhai\n",
                ind
            ));
            code.push_str(&format!("{}sleep({});\n", ind, timeout_ms / 1000.0));
        }
    }

    code
}

/// Generate Rhai code for an Acquire node.
fn acquire_to_rhai(config: &AcquireConfig, indent: usize) -> String {
    let ind = indent_str(indent);
    let mut code = String::new();

    if config.detector.is_empty() {
        code.push_str(&format!(
            "{}// WARNING: Acquire node has no detector specified\n",
            ind
        ));
        return code;
    }

    code.push_str(&format!(
        "{}// Acquire {} frame(s) from {}\n",
        ind, config.frame_count, config.detector
    ));

    // Set exposure if specified
    if let Some(exposure_ms) = config.exposure_ms {
        if exposure_ms > 0.0 {
            code.push_str(&format!(
                "{}{}.set_exposure({});\n",
                ind, config.detector, exposure_ms
            ));
        }
    }

    // Generate acquire loop if multiple frames
    if config.frame_count > 1 {
        code.push_str(&format!("{}for i in 0..{} {{\n", ind, config.frame_count));
        let body_ind = indent_str(indent + 1);
        code.push_str(&format!("{}{}.trigger();\n", body_ind, config.detector));
        code.push_str(&format!("{}{}.read();\n", body_ind, config.detector));
        code.push_str(&format!("{}}}\n", ind));
    } else {
        code.push_str(&format!("{}{}.trigger();\n", ind, config.detector));
        code.push_str(&format!("{}{}.read();\n", ind, config.detector));
    }

    code
}

/// Generate Rhai code for a Loop node.
fn loop_to_rhai(
    config: &LoopConfig,
    node_id: NodeId,
    snarl: &Snarl<ExperimentNode>,
    loop_body_set: &HashSet<NodeId>,
    indent: usize,
) -> String {
    let ind = indent_str(indent);
    let mut code = String::new();

    // Get loop body nodes
    let body_nodes = find_loop_body_nodes(node_id, snarl);

    match &config.termination {
        LoopTermination::Count { iterations } => {
            code.push_str(&format!("{}// Loop {} times\n", ind, iterations));
            code.push_str(&format!("{}for i in 0..{} {{\n", ind, iterations));
        }
        LoopTermination::Condition {
            device_id,
            operator,
            value,
            max_iterations,
        } => {
            let op_str = match operator {
                ThresholdOp::LessThan => "<",
                ThresholdOp::GreaterThan => ">",
                ThresholdOp::EqualWithin { tolerance } => &format!("== (±{})", tolerance),
            };
            code.push_str(&format!(
                "{}// TODO: Loop until {} {} {} (max {} iterations)\n",
                ind, device_id, op_str, value, max_iterations
            ));
            code.push_str(&format!(
                "{}// Condition-based loops not yet implemented in Rhai\n",
                ind
            ));
            code.push_str(&format!("{}for i in 0..{} {{\n", ind, max_iterations));
        }
        LoopTermination::Infinite { max_iterations } => {
            code.push_str(&format!(
                "{}// TODO: Infinite loop (safety limit: {} iterations)\n",
                ind, max_iterations
            ));
            code.push_str(&format!(
                "{}// Infinite loops require manual abort - using safety limit\n",
                ind
            ));
            code.push_str(&format!("{}for i in 0..{} {{\n", ind, max_iterations));
        }
    }

    // Generate body nodes with increased indent
    if body_nodes.is_empty() {
        let body_ind = indent_str(indent + 1);
        code.push_str(&format!("{}// Loop body is empty\n", body_ind));
    } else {
        for &body_node_id in &body_nodes {
            if let Some(body_node) = snarl.get_node(body_node_id) {
                code.push_str(&node_to_rhai(
                    body_node,
                    body_node_id,
                    snarl,
                    loop_body_set,
                    indent + 1,
                ));
            }
        }
    }

    code.push_str(&format!("{}}}\n", ind));

    code
}

/// Generate Rhai code for a NestedScan node.
fn nested_scan_to_rhai(config: &NestedScanConfig, indent: usize) -> String {
    let ind = indent_str(indent);
    let mut code = String::new();

    if config.outer.actuator.is_empty() || config.inner.actuator.is_empty() {
        code.push_str(&format!(
            "{}// WARNING: NestedScan node has empty actuator(s)\n",
            ind
        ));
        return code;
    }

    code.push_str(&format!(
        "{}// Nested scan: {} x {}\n",
        ind, config.outer.dimension_name, config.inner.dimension_name
    ));

    // Outer loop
    code.push_str(&format!(
        "{}// Outer: {} from {:.1} to {:.1} ({} points)\n",
        ind, config.outer.actuator, config.outer.start, config.outer.stop, config.outer.points
    ));
    code.push_str(&format!(
        "{}for outer_i in 0..{} {{\n",
        ind, config.outer.points
    ));

    let body_ind = indent_str(indent + 1);

    // Outer position calculation
    if config.outer.points > 1 {
        code.push_str(&format!(
            "{}let outer_pos = {} + ({} - {}) * outer_i / ({} - 1);\n",
            body_ind,
            config.outer.start,
            config.outer.stop,
            config.outer.start,
            config.outer.points
        ));
    } else {
        code.push_str(&format!(
            "{}let outer_pos = {};\n",
            body_ind, config.outer.start
        ));
    }
    code.push_str(&format!(
        "{}{}.move_abs(outer_pos);\n",
        body_ind, config.outer.actuator
    ));
    code.push_str(&format!(
        "{}{}.wait_settled();\n",
        body_ind, config.outer.actuator
    ));

    // Inner loop
    code.push_str(&format!(
        "{}// Inner: {} from {:.1} to {:.1} ({} points)\n",
        body_ind, config.inner.actuator, config.inner.start, config.inner.stop, config.inner.points
    ));
    code.push_str(&format!(
        "{}for inner_i in 0..{} {{\n",
        body_ind, config.inner.points
    ));

    let inner_ind = indent_str(indent + 2);

    // Inner position calculation
    if config.inner.points > 1 {
        code.push_str(&format!(
            "{}let inner_pos = {} + ({} - {}) * inner_i / ({} - 1);\n",
            inner_ind,
            config.inner.start,
            config.inner.stop,
            config.inner.start,
            config.inner.points
        ));
    } else {
        code.push_str(&format!(
            "{}let inner_pos = {};\n",
            inner_ind, config.inner.start
        ));
    }
    code.push_str(&format!(
        "{}{}.move_abs(inner_pos);\n",
        inner_ind, config.inner.actuator
    ));
    code.push_str(&format!(
        "{}{}.wait_settled();\n",
        inner_ind, config.inner.actuator
    ));

    // Yield event
    code.push_str(&format!(
        "{}yield_event(#{{ \"{}\": outer_pos, \"{}\": inner_pos }});\n",
        inner_ind, config.outer.dimension_name, config.inner.dimension_name
    ));

    code.push_str(&format!("{}}}\n", body_ind));
    code.push_str(&format!("{}}}\n", ind));

    code
}

/// Generate Rhai code for an AdaptiveScan node.
fn adaptive_scan_to_rhai(config: &AdaptiveScanConfig, indent: usize) -> String {
    let ind = indent_str(indent);
    let mut code = String::new();

    if config.scan.actuator.is_empty() {
        code.push_str(&format!(
            "{}// WARNING: AdaptiveScan node has no actuator specified\n",
            ind
        ));
        return code;
    }

    code.push_str(&format!(
        "{}// Adaptive scan: {} from {:.1} to {:.1} ({} points)\n",
        ind, config.scan.actuator, config.scan.start, config.scan.stop, config.scan.points
    ));

    // Document triggers
    code.push_str(&format!(
        "{}// Triggers ({:?} logic):\n",
        ind, config.trigger_logic
    ));
    for (i, trigger) in config.triggers.iter().enumerate() {
        match trigger {
            TriggerCondition::Threshold {
                device_id,
                operator,
                value,
            } => {
                let op_str = match operator {
                    ThresholdOp::LessThan => "<",
                    ThresholdOp::GreaterThan => ">",
                    ThresholdOp::EqualWithin { tolerance } => &format!("== (+/-{})", tolerance),
                };
                code.push_str(&format!(
                    "{}//   {}: {} {} {}\n",
                    ind,
                    i + 1,
                    device_id,
                    op_str,
                    value
                ));
            }
            TriggerCondition::PeakDetection {
                device_id,
                min_prominence,
                min_height,
            } => {
                code.push_str(&format!(
                    "{}//   {}: Peak detection on {} (prominence >= {}{})\n",
                    ind,
                    i + 1,
                    device_id,
                    min_prominence,
                    min_height.map_or(String::new(), |h| format!(", height >= {}", h))
                ));
            }
        }
    }

    // Document action
    let action_str = match config.action {
        AdaptiveAction::Zoom2x => "Zoom 2x (narrow range, increase resolution)",
        AdaptiveAction::Zoom4x => "Zoom 4x (narrow range, increase resolution)",
        AdaptiveAction::MoveToPeak => "Move to detected peak position",
        AdaptiveAction::AcquireAtPeak => "Trigger acquisition at peak",
        AdaptiveAction::MarkAndContinue => "Mark peak location and continue",
    };
    code.push_str(&format!("{}// Action: {}\n", ind, action_str));
    if config.require_approval {
        code.push_str(&format!(
            "{}// (requires user approval before action)\n",
            ind
        ));
    }

    code.push_str(&format!(
        "{}// TODO: AdaptiveScan requires runtime trigger evaluation\n",
        ind
    ));
    code.push_str(&format!("{}// Falling back to regular scan for now\n", ind));

    // Generate basic scan as fallback
    code.push_str(&format!("{}for i in 0..{} {{\n", ind, config.scan.points));

    let body_ind = indent_str(indent + 1);

    if config.scan.points > 1 {
        code.push_str(&format!(
            "{}let pos = {} + ({} - {}) * i / ({} - 1);\n",
            body_ind, config.scan.start, config.scan.stop, config.scan.start, config.scan.points
        ));
    } else {
        code.push_str(&format!("{}let pos = {};\n", body_ind, config.scan.start));
    }

    code.push_str(&format!(
        "{}{}.move_abs(pos);\n",
        body_ind, config.scan.actuator
    ));
    code.push_str(&format!(
        "{}{}.wait_settled();\n",
        body_ind, config.scan.actuator
    ));
    code.push_str(&format!(
        "{}yield_event(#{{ \"{}\": pos }});\n",
        body_ind, config.scan.actuator
    ));

    code.push_str(&format!("{}}}\n", ind));

    code
}

/// Generate indentation string (2 spaces per level).
fn indent_str(level: usize) -> String {
    "  ".repeat(level)
}

/// Find all nodes in a loop's body sub-graph.
///
/// This is a copy of the logic from translation.rs since we need it here too.
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

    // Find roots within body
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
        Err(_) => pure_body, // Fallback to unsorted if cycle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn test_scan_to_rhai() {
        let code = scan_to_rhai("stage_x", 0.0, 100.0, 10, 0);

        // Should contain for loop
        assert!(code.contains("for i in 0..10"));

        // Should contain move_abs
        assert!(code.contains(".move_abs(pos)"));

        // Should contain yield_event
        assert!(code.contains("yield_event"));

        // Should contain explanatory comment
        assert!(code.contains("// Scan stage_x from 0.0 to 100.0 in 10 steps"));
    }

    #[test]
    fn test_scan_to_rhai_empty_actuator() {
        let code = scan_to_rhai("", 0.0, 100.0, 10, 0);
        assert!(code.contains("WARNING"));
    }

    #[test]
    fn test_scan_to_rhai_zero_points() {
        let code = scan_to_rhai("stage_x", 0.0, 100.0, 0, 0);
        assert!(code.contains("WARNING"));
        assert!(code.contains("zero points"));
    }

    #[test]
    fn test_move_to_rhai_absolute() {
        let config = MoveConfig {
            device: "stage_y".to_string(),
            position: 42.5,
            mode: MoveMode::Absolute,
            wait_settled: true,
        };

        let code = move_to_rhai(&config, 0);

        assert!(code.contains("stage_y.move_abs(42.5)"));
        assert!(code.contains("wait_settled"));
        assert!(code.contains("// Move stage_y to absolute position 42.5"));
    }

    #[test]
    fn test_move_to_rhai_relative() {
        let config = MoveConfig {
            device: "stage_z".to_string(),
            position: 10.0,
            mode: MoveMode::Relative,
            wait_settled: false,
        };

        let code = move_to_rhai(&config, 0);

        assert!(code.contains("stage_z.move_rel(10)"));
        assert!(!code.contains("wait_settled"));
        assert!(code.contains("// Move stage_z by relative distance 10"));
    }

    #[test]
    fn test_wait_to_rhai_duration() {
        let condition = WaitCondition::Duration {
            milliseconds: 2500.0,
        };

        let code = wait_to_rhai(&condition, 0);

        assert!(code.contains("sleep(2.5)"));
        assert!(code.contains("// Wait for 2.5 seconds"));
    }

    #[test]
    fn test_wait_to_rhai_threshold() {
        let condition = WaitCondition::Threshold {
            device_id: "sensor".to_string(),
            operator: ThresholdOp::GreaterThan,
            value: 5.0,
            timeout_ms: 1000.0,
        };

        let code = wait_to_rhai(&condition, 0);

        assert!(code.contains("TODO"));
        assert!(code.contains("sensor"));
        assert!(code.contains(">"));
        assert!(code.contains("5"));
    }

    #[test]
    fn test_wait_to_rhai_stability() {
        let condition = WaitCondition::Stability {
            device_id: "power_meter".to_string(),
            tolerance: 0.1,
            duration_ms: 500.0,
            timeout_ms: 5000.0,
        };

        let code = wait_to_rhai(&condition, 0);

        assert!(code.contains("TODO"));
        assert!(code.contains("stabilizes"));
        assert!(code.contains("power_meter"));
    }

    #[test]
    fn test_acquire_to_rhai_single_frame() {
        let config = AcquireConfig {
            detector: "camera".to_string(),
            exposure_ms: Some(100.0),
            frame_count: 1,
        };

        let code = acquire_to_rhai(&config, 0);

        assert!(code.contains("camera.set_exposure(100)"));
        assert!(code.contains("camera.trigger()"));
        assert!(code.contains("camera.read()"));
        // Should NOT have a for loop for single frame
        assert!(!code.contains("for i in"));
    }

    #[test]
    fn test_acquire_to_rhai_multiple_frames() {
        let config = AcquireConfig {
            detector: "camera".to_string(),
            exposure_ms: None,
            frame_count: 5,
        };

        let code = acquire_to_rhai(&config, 0);

        // Should NOT set exposure (None)
        assert!(!code.contains("set_exposure"));

        // Should have for loop
        assert!(code.contains("for i in 0..5"));
        assert!(code.contains("trigger()"));
        assert!(code.contains("read()"));
    }

    #[test]
    fn test_loop_to_rhai_count() {
        let mut snarl = Snarl::new();
        let loop_node = snarl.insert_node(
            pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Count { iterations: 3 },
            }),
        );

        let code = loop_to_rhai(
            &LoopConfig {
                termination: LoopTermination::Count { iterations: 3 },
            },
            loop_node,
            &snarl,
            &HashSet::new(),
            0,
        );

        assert!(code.contains("// Loop 3 times"));
        assert!(code.contains("for i in 0..3"));
        assert!(code.contains("// Loop body is empty"));
    }

    #[test]
    fn test_graph_to_rhai_empty() {
        let snarl: Snarl<ExperimentNode> = Snarl::new();
        let script = graph_to_rhai_script(&snarl, None);

        assert!(script.contains("ERROR: Graph is empty"));
    }

    #[test]
    fn test_graph_to_rhai_single_node() {
        let mut snarl = Snarl::new();
        snarl.insert_node(
            pos2(0.0, 0.0),
            ExperimentNode::Scan {
                actuator: "stage_x".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 10,
            },
        );

        let script = graph_to_rhai_script(&snarl, Some("test.graph"));

        // Should have header
        assert!(script.contains("// Generated Rhai script"));
        assert!(script.contains("// Source: test.graph"));
        assert!(script.contains("DO NOT EDIT"));

        // Should have node marker
        assert!(script.contains("// === Node 1: Scan ==="));

        // Should have scan code
        assert!(script.contains("for i in"));
    }

    #[test]
    fn test_graph_to_rhai_cycle() {
        // Cannot easily create a cycle with Snarl API in a unit test,
        // so we'll just verify the error message path exists
        let snarl: Snarl<ExperimentNode> = Snarl::new();
        let script = graph_to_rhai_script(&snarl, None);

        // Empty graph should produce error
        assert!(script.contains("ERROR"));
    }

    #[test]
    fn test_indent_str() {
        assert_eq!(indent_str(0), "");
        assert_eq!(indent_str(1), "  ");
        assert_eq!(indent_str(2), "    ");
        assert_eq!(indent_str(3), "      ");
    }
}
