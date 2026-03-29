//! Translation from visual node graph to executable Plan.

use super::nodes::ExperimentNode;
use egui_snarl::{NodeId, Snarl};
use experiment::plans::{Plan, PlanCommand};
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

/// Type alias for the adjacency result from graph analysis.
/// Contains (adjacency_map, root_nodes) for topological sorting.
type AdjacencyResult = (HashMap<NodeId, Vec<NodeId>>, Vec<NodeId>);

/// Plan generated from a visual node graph
#[derive(Debug)]
pub struct GraphPlan {
    commands: Vec<PlanCommand>,
    current_idx: usize,
    total_events: usize,
    movers: Vec<String>,
    detectors: Vec<String>,
}

impl GraphPlan {
    /// Get the pre-translated commands (for serialization to DB).
    pub fn commands(&self) -> &[PlanCommand] {
        &self.commands
    }

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

        // Validate loop termination modes before translation.
        // Phase 4: Condition loops are now supported via RepeatWhile.
        // Infinite loops remain rejected — they require manual abort support.
        for (node_id, node) in snarl.node_ids() {
            if let ExperimentNode::Loop(config) = node {
                if let super::nodes::LoopTermination::Infinite { .. } = &config.termination {
                    return Err(TranslationError::InvalidNode {
                        node_id,
                        reason: "Infinite loops are not supported. \
                                Use Count or Condition termination instead."
                            .to_string(),
                    });
                }
            }
        }

        // Topological sort with cycle detection
        let sorted = topological_sort(&adjacency, &roots, snarl.node_ids().count())?;

        // Identify loop body nodes (these will be skipped in main traversal)
        let mut loop_body_set = HashSet::new();
        for (loop_id, loop_node) in snarl.node_ids() {
            if matches!(
                loop_node,
                ExperimentNode::Loop(..) | ExperimentNode::NestedScan(..)
            ) {
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
    fn plan_type(&self) -> &'static str {
        "graph_plan"
    }

    fn plan_name(&self) -> &'static str {
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
pub fn build_adjacency(snarl: &Snarl<ExperimentNode>) -> Result<AdjacencyResult, TranslationError> {
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
                    (stop - start) / (f64::from(*points) - 1.0)
                } else {
                    0.0
                };
                for i in 0..*points {
                    let pos = start + step * f64::from(i);
                    commands.push(PlanCommand::MoveTo {
                        device_id: actuator.clone(),
                        position: pos,
                    });
                    commands.push(PlanCommand::Checkpoint {
                        label: format!("node_{:?}_point_{}", node_id, i),
                    });
                    #[allow(clippy::cast_possible_truncation)]
                    commands.push(PlanCommand::EmitEvent {
                        stream: "primary".to_string(),
                        data: HashMap::new(),
                        positions: [(actuator.clone(), pos)].into_iter().collect(),
                        scan_indices: Some(vec![(actuator.clone(), i as usize)]),
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
                        scan_indices: None,
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
                    // Phase 4: Emit real WaitSettled for move-and-settle
                    commands.push(PlanCommand::WaitSettled {
                        device_id: config.device.clone(),
                        timeout_seconds: 30.0, // Default 30s settle timeout for moves
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
                WaitCondition::Threshold {
                    device_id,
                    timeout_ms,
                    ..
                } => {
                    // Phase 4: Emit WaitSettled which performs real threshold
                    // polling in RunEngine. The device_id + timeout are the
                    // primary knobs; threshold field/operator details are
                    // evaluated by the RunEngine's settled-detection logic.
                    commands.push(PlanCommand::WaitSettled {
                        device_id: device_id.clone(),
                        timeout_seconds: *timeout_ms / 1000.0,
                    });
                }
                WaitCondition::Stability {
                    device_id,
                    timeout_ms,
                    ..
                } => {
                    // Phase 4: Emit WaitSettled which performs real stability
                    // detection in RunEngine (1% tolerance, 500ms window).
                    // The device_id + timeout are the primary knobs; tolerance
                    // and duration_ms from the node config are available for
                    // future WaitSettled field extensions.
                    commands.push(PlanCommand::WaitSettled {
                        device_id: device_id.clone(),
                        timeout_seconds: *timeout_ms / 1000.0,
                    });
                }
            }
        }
        ExperimentNode::Loop(config) => {
            use super::nodes::{LoopTermination, ThresholdOp};

            // Get loop body nodes
            let body_nodes = find_loop_body_nodes(node_id, snarl);

            // Helper: translate body nodes into a flat command list
            let translate_body =
                |body_nodes: &[NodeId],
                 snarl: &Snarl<ExperimentNode>|
                 -> (Vec<PlanCommand>, Vec<String>, Vec<String>, usize) {
                    let mut cmds = Vec::new();
                    let mut mvs = Vec::new();
                    let mut dets = Vec::new();
                    let mut evts = 0;
                    for &body_node_id in body_nodes {
                        if let Some(body_node) = snarl.get_node(body_node_id) {
                            let (body_cmds, body_movers, body_detectors, body_events) =
                                translate_node_with_snarl(body_node, body_node_id, snarl);
                            cmds.extend(body_cmds);
                            mvs.extend(body_movers);
                            dets.extend(body_detectors);
                            evts += body_events;
                        }
                    }
                    (cmds, mvs, dets, evts)
                };

            match &config.termination {
                LoopTermination::Count { iterations } => {
                    // Fixed-count: unroll loop body N times
                    for i in 0..*iterations {
                        commands.push(PlanCommand::Checkpoint {
                            label: format!("loop_{:?}_iter_{}_start", node_id, i),
                        });

                        let (body_cmds, body_movers, body_detectors, body_events) =
                            translate_body(&body_nodes, snarl);
                        commands.extend(body_cmds);
                        movers.extend(body_movers);
                        detectors.extend(body_detectors);
                        events += body_events;

                        commands.push(PlanCommand::Checkpoint {
                            label: format!("loop_{:?}_iter_{}_end", node_id, i),
                        });
                    }
                }
                LoopTermination::Condition {
                    device_id,
                    operator,
                    value,
                    max_iterations,
                } => {
                    // Phase 4: Emit RepeatWhile with EvalCondition::Threshold.
                    // RunEngine evaluates the condition before each iteration.
                    let (body_cmds, body_movers, body_detectors, _body_events) =
                        translate_body(&body_nodes, snarl);
                    movers.extend(body_movers);
                    detectors.extend(body_detectors);
                    // Note: events from body are generated dynamically at runtime,
                    // so we don't add them to the static count here.

                    let above = matches!(operator, ThresholdOp::GreaterThan);
                    commands.push(PlanCommand::RepeatWhile {
                        condition: experiment::plans::EvalCondition::Threshold {
                            device_id: device_id.clone(),
                            field: "value".to_string(),
                            threshold: *value,
                            above,
                        },
                        body: body_cmds,
                        max_iterations: *max_iterations,
                    });
                }
                LoopTermination::Infinite { max_iterations } => {
                    // Infinite loops are rejected at validation; this branch
                    // is unreachable but kept for exhaustiveness.
                    tracing::warn!(
                        "Infinite loop at {:?} using max_iterations={} as safety limit",
                        node_id,
                        max_iterations
                    );
                    for i in 0..*max_iterations {
                        commands.push(PlanCommand::Checkpoint {
                            label: format!("loop_{:?}_iter_{}_start", node_id, i),
                        });

                        let (body_cmds, body_movers, body_detectors, body_events) =
                            translate_body(&body_nodes, snarl);
                        commands.extend(body_cmds);
                        movers.extend(body_movers);
                        detectors.extend(body_detectors);
                        events += body_events;

                        commands.push(PlanCommand::Checkpoint {
                            label: format!("loop_{:?}_iter_{}_end", node_id, i),
                        });
                    }
                }
            }
        }
        ExperimentNode::NestedScan(config) => {
            // Nested scan generates outer x inner grid with body nodes at each point
            // Get body nodes (reuse existing find_loop_body_nodes - same pin 1 convention)
            //
            // ==== ZARR INTEGRATION FOR NESTED SCANS ====
            // Nested scans produce multi-dimensional data that should be stored in Zarr format
            // with proper dimensional metadata for scientific analysis tools (xarray, napari).
            //
            // Key Zarr V3 attributes required:
            // - _ARRAY_DIMENSIONS: ["outer_dim_name", "inner_dim_name", ...] for xarray compat
            // - Each EmitEvent includes dimensional indices for Zarr coordinate assignment
            //
            // TODO(bd-wev5): Implement Zarr writer setup:
            // 1. Create Zarr V3 store with shape (outer_points, inner_points, ...)
            // 2. Set _ARRAY_DIMENSIONS attribute with dimension names from config
            // 3. Create coordinate arrays for each dimension
            // 4. On EmitEvent, use dimensional indices to write to correct Zarr position
            //
            // Dimension naming convention:
            // - config.outer.dimension_name -> outer array dimension (e.g., "wavelength")
            // - config.inner.dimension_name -> inner array dimension (e.g., "position")
            let body_nodes = find_loop_body_nodes(node_id, snarl);

            // Add actuators to movers list
            if !config.outer.actuator.is_empty() {
                movers.push(config.outer.actuator.clone());
            }
            if !config.inner.actuator.is_empty() {
                movers.push(config.inner.actuator.clone());
            }

            // Calculate step sizes
            let outer_step = if config.outer.points > 1 {
                (config.outer.stop - config.outer.start) / (f64::from(config.outer.points) - 1.0)
            } else {
                0.0
            };
            let inner_step = if config.inner.points > 1 {
                (config.inner.stop - config.inner.start) / (f64::from(config.inner.points) - 1.0)
            } else {
                0.0
            };

            // Nested iteration: outer × inner
            for outer_idx in 0..config.outer.points {
                let outer_pos = config.outer.start + outer_step * f64::from(outer_idx);

                // Move outer actuator
                if !config.outer.actuator.is_empty() {
                    commands.push(PlanCommand::MoveTo {
                        device_id: config.outer.actuator.clone(),
                        position: outer_pos,
                    });
                }

                commands.push(PlanCommand::Checkpoint {
                    label: format!("nested_{:?}_outer_{}_start", node_id, outer_idx),
                });

                for inner_idx in 0..config.inner.points {
                    let inner_pos = config.inner.start + inner_step * f64::from(inner_idx);

                    // Move inner actuator
                    if !config.inner.actuator.is_empty() {
                        commands.push(PlanCommand::MoveTo {
                            device_id: config.inner.actuator.clone(),
                            position: inner_pos,
                        });
                    }

                    commands.push(PlanCommand::Checkpoint {
                        label: format!(
                            "nested_{:?}_outer_{}_inner_{}",
                            node_id, outer_idx, inner_idx
                        ),
                    });

                    // Execute body nodes at this point
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

                    // Emit event with dimensional positions and indices
                    // The positions map contains actuator -> position for coordinate tracking
                    //
                    // ==== DIMENSIONAL INDEXING FOR ZARR ====
                    // For Zarr V3 multi-dimensional storage, the RunEngine needs to know
                    // which array indices to write data to. We encode this via:
                    //
                    // 1. Special position keys: "_outer_idx", "_inner_idx" (f64-encoded indices)
                    //    These are used by the Zarr writer to determine array position
                    //
                    // 2. Dimension names are passed via checkpoint labels earlier:
                    //    "nested_{node_id}_outer_{idx}_start" -> outer dimension progress
                    //    Combined with GraphPlan metadata for dimension names
                    //
                    // Example: For nested scan with outer=wavelength (10 pts), inner=position (100 pts)
                    //    positions = {
                    //      "wavelength": 450.0,  // actual wavelength value
                    //      "position": 25.5,     // actual position value
                    //      "_outer_idx": 3.0,    // outer array index (wavelength index 3)
                    //      "_inner_idx": 45.0,   // inner array index (position index 45)
                    //    }
                    //    Zarr writes to array[3, 45, ...]
                    let mut positions = HashMap::new();
                    if !config.outer.actuator.is_empty() {
                        positions.insert(config.outer.actuator.clone(), outer_pos);
                    }
                    if !config.inner.actuator.is_empty() {
                        positions.insert(config.inner.actuator.clone(), inner_pos);
                    }

                    // Include dimensional indices for Zarr coordinate assignment
                    // Convention: "_outer_idx", "_inner_idx" are reserved keys
                    // TODO(bd-wev5): Remove these once all consumers use scan_indices
                    positions.insert("_outer_idx".to_string(), f64::from(outer_idx));
                    positions.insert("_inner_idx".to_string(), f64::from(inner_idx));

                    // Use dimension names from config, falling back to actuator ID, then "outer"/"inner"
                    let outer_dim = if !config.outer.dimension_name.is_empty() {
                        config.outer.dimension_name.clone()
                    } else if !config.outer.actuator.is_empty() {
                        config.outer.actuator.clone()
                    } else {
                        "outer".to_string()
                    };
                    let inner_dim = if !config.inner.dimension_name.is_empty() {
                        config.inner.dimension_name.clone()
                    } else if !config.inner.actuator.is_empty() {
                        config.inner.actuator.clone()
                    } else {
                        "inner".to_string()
                    };

                    #[allow(clippy::cast_possible_truncation)]
                    let scan_indices = Some(vec![
                        (outer_dim, outer_idx as usize),
                        (inner_dim, inner_idx as usize),
                    ]);

                    commands.push(PlanCommand::EmitEvent {
                        stream: "primary".to_string(),
                        data: HashMap::new(),
                        positions,
                        scan_indices,
                    });
                    events += 1;
                }

                commands.push(PlanCommand::Checkpoint {
                    label: format!("nested_{:?}_outer_{}_end", node_id, outer_idx),
                });
            }
        }
        ExperimentNode::AdaptiveScan(config) => {
            // Adaptive scan with trigger evaluation checkpoints
            // NOTE: Actual trigger evaluation happens at runtime in RunEngine
            // Translation generates checkpoints that mark where evaluation occurs

            if config.scan.points > 0 && !config.scan.actuator.is_empty() {
                movers.push(config.scan.actuator.clone());

                let step = if config.scan.points > 1 {
                    (config.scan.stop - config.scan.start) / (f64::from(config.scan.points) - 1.0)
                } else {
                    0.0
                };

                // Adaptive scan start checkpoint
                commands.push(PlanCommand::Checkpoint {
                    label: format!("adaptive_{:?}_start", node_id),
                });

                // Generate scan points
                for i in 0..config.scan.points {
                    let pos = config.scan.start + step * f64::from(i);

                    // Move actuator
                    commands.push(PlanCommand::MoveTo {
                        device_id: config.scan.actuator.clone(),
                        position: pos,
                    });

                    // Point checkpoint with trigger metadata
                    commands.push(PlanCommand::Checkpoint {
                        label: format!(
                            "adaptive_{:?}_point_{}_triggers_{}",
                            node_id,
                            i,
                            config.triggers.len()
                        ),
                    });

                    // Emit event with scan index
                    let dim_name = if !config.scan.dimension_name.is_empty() {
                        config.scan.dimension_name.clone()
                    } else if !config.scan.actuator.is_empty() {
                        config.scan.actuator.clone()
                    } else {
                        "scan".to_string()
                    };
                    #[allow(clippy::cast_possible_truncation)]
                    commands.push(PlanCommand::EmitEvent {
                        stream: "primary".to_string(),
                        data: HashMap::new(),
                        positions: [(config.scan.actuator.clone(), pos)].into_iter().collect(),
                        scan_indices: Some(vec![(dim_name, i as usize)]),
                    });
                    events += 1;
                }

                // Add adaptive evaluation checkpoint
                // This is where the RunEngine evaluates accumulated data against triggers
                commands.push(PlanCommand::Checkpoint {
                    label: format!("adaptive_{:?}_evaluate_action_{:?}", node_id, config.action),
                });

                // If require_approval, add an approval checkpoint
                if config.require_approval {
                    commands.push(PlanCommand::Checkpoint {
                        label: format!("adaptive_{:?}_approval_required", node_id),
                    });
                }
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
    use crate::graph::nodes::{
        AcquireConfig, LoopConfig, LoopTermination, NestedScanConfig, ScanDimension, ThresholdOp,
    };

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

    #[test]
    fn test_nested_scan_event_count() {
        let mut snarl = Snarl::new();

        // Create NestedScan node with 10 outer x 5 inner = 50 points
        let _nested_node = snarl.insert_node(
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

        // Translate to plan
        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Should have 10 × 5 = 50 EmitEvent commands
        let emit_count = plan
            .commands
            .iter()
            .filter(|cmd| matches!(cmd, PlanCommand::EmitEvent { .. }))
            .count();
        assert_eq!(emit_count, 50, "Expected 50 EmitEvent commands (10 × 5)");

        // Verify total_events matches
        assert_eq!(plan.total_events, 50, "Expected 50 events total");

        // Verify movers include both actuators
        assert!(
            plan.movers.contains(&"stage_x".to_string()),
            "Should include outer actuator"
        );
        assert!(
            plan.movers.contains(&"stage_y".to_string()),
            "Should include inner actuator"
        );
    }

    #[test]
    fn test_nested_scan_with_body_nodes() {
        let mut snarl = Snarl::new();

        // Create NestedScan node with 2 outer x 3 inner = 6 points
        let nested_node = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::NestedScan(NestedScanConfig {
                outer: ScanDimension {
                    actuator: "stage_x".to_string(),
                    start: 0.0,
                    stop: 10.0,
                    points: 2,
                    dimension_name: "x".to_string(),
                },
                inner: ScanDimension {
                    actuator: "stage_y".to_string(),
                    start: 0.0,
                    stop: 20.0,
                    points: 3,
                    dimension_name: "y".to_string(),
                },
                nesting_warning_depth: 3,
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

        // Connect NestedScan body output (pin 1) to acquire
        snarl.connect(
            egui_snarl::OutPinId {
                node: nested_node,
                output: 1,
            },
            egui_snarl::InPinId {
                node: acquire,
                input: 0,
            },
        );

        // Translate to plan
        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Should have 2 × 3 = 6 Trigger commands (body executes at each point)
        let trigger_count = plan
            .commands
            .iter()
            .filter(|cmd| matches!(cmd, PlanCommand::Trigger { .. }))
            .count();
        assert_eq!(
            trigger_count, 6,
            "Expected 6 triggers (body executes at each outer × inner point)"
        );

        // Total events = 6 from NestedScan + 6 from Acquire = 12
        assert_eq!(
            plan.total_events, 12,
            "Expected 12 events (6 from scan + 6 from body)"
        );

        // Verify detector is included
        assert!(
            plan.detectors.contains(&"camera".to_string()),
            "Should include body node detector"
        );
    }

    #[test]
    fn test_adaptive_scan_translation() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, ThresholdOp, TriggerCondition,
            TriggerLogic,
        };

        let mut snarl = Snarl::new();

        // Create AdaptiveScan node with 5 points and 2 triggers
        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::AdaptiveScan(AdaptiveScanConfig {
                scan: ScanDimension {
                    actuator: "wavelength".to_string(),
                    dimension_name: "lambda".to_string(),
                    start: 400.0,
                    stop: 800.0,
                    points: 5,
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
                        min_height: None,
                    },
                ],
                trigger_logic: TriggerLogic::Any,
                action: AdaptiveAction::Zoom2x,
                require_approval: true,
            }),
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Should have start checkpoint
        let has_start = plan.commands.iter().any(|cmd| {
            matches!(cmd, PlanCommand::Checkpoint { label } if label.contains("adaptive") && label.contains("start"))
        });
        assert!(has_start, "Should have adaptive scan start checkpoint");

        // Should have 5 point checkpoints with trigger count
        let point_checkpoints: Vec<_> = plan
            .commands
            .iter()
            .filter(|cmd| {
                matches!(cmd, PlanCommand::Checkpoint { label } if label.contains("point") && label.contains("triggers_2"))
            })
            .collect();
        assert_eq!(
            point_checkpoints.len(),
            5,
            "Should have 5 point checkpoints with trigger count"
        );

        // Should have evaluate checkpoint with action
        let has_evaluate = plan.commands.iter().any(|cmd| {
            matches!(cmd, PlanCommand::Checkpoint { label } if label.contains("evaluate_action") && label.contains("Zoom2x"))
        });
        assert!(has_evaluate, "Should have evaluate checkpoint with action");

        // Should have approval checkpoint (require_approval = true)
        let has_approval = plan.commands.iter().any(|cmd| {
            matches!(cmd, PlanCommand::Checkpoint { label } if label.contains("approval_required"))
        });
        assert!(
            has_approval,
            "Should have approval checkpoint when require_approval = true"
        );

        // Should have 5 events
        assert_eq!(plan.total_events, 5, "Should have 5 events");

        // Should include actuator as mover
        assert!(
            plan.movers.contains(&"wavelength".to_string()),
            "Should include wavelength actuator"
        );
    }

    #[test]
    fn test_adaptive_scan_without_approval() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, ScanDimension, TriggerCondition, TriggerLogic,
        };

        let mut snarl = Snarl::new();

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
                triggers: vec![TriggerCondition::default()],
                trigger_logic: TriggerLogic::Any,
                action: AdaptiveAction::MoveToPeak,
                require_approval: false, // No approval needed
            }),
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Should NOT have approval checkpoint
        let has_approval = plan.commands.iter().any(|cmd| {
            matches!(cmd, PlanCommand::Checkpoint { label } if label.contains("approval_required"))
        });
        assert!(
            !has_approval,
            "Should NOT have approval checkpoint when require_approval = false"
        );
    }

    #[test]
    fn test_condition_loop_emits_repeat_while() {
        use crate::graph::nodes::AcquireConfig;

        let mut snarl = Snarl::new();

        // Create a condition-based loop
        let loop_node = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Condition {
                    device_id: "sensor".to_string(),
                    operator: ThresholdOp::GreaterThan,
                    value: 1.0,
                    max_iterations: 100,
                },
            }),
        );

        // Add a body node
        let acquire = snarl.insert_node(
            egui::pos2(100.0, 0.0),
            ExperimentNode::Acquire(AcquireConfig {
                detector: "camera".to_string(),
                exposure_ms: Some(50.0),
                frame_count: 1,
            }),
        );

        // Connect loop body output (pin 1) to acquire
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

        let plan = GraphPlan::from_snarl(&snarl).expect("Condition loops should translate");

        // Should contain a RepeatWhile command
        let repeat_while = plan
            .commands
            .iter()
            .find(|cmd| matches!(cmd, PlanCommand::RepeatWhile { .. }));
        assert!(
            repeat_while.is_some(),
            "Should emit RepeatWhile for condition-based loop"
        );

        // Verify RepeatWhile fields
        if let Some(PlanCommand::RepeatWhile {
            condition,
            body,
            max_iterations,
        }) = repeat_while
        {
            assert_eq!(*max_iterations, 100, "max_iterations should be 100");

            // Condition should be a Threshold with above=true (GreaterThan)
            match condition {
                experiment::plans::EvalCondition::Threshold {
                    device_id,
                    threshold,
                    above,
                    ..
                } => {
                    assert_eq!(device_id, "sensor");
                    assert!((threshold - 1.0).abs() < f64::EPSILON);
                    assert!(above, "GreaterThan should map to above=true");
                }
                _ => panic!("Expected EvalCondition::Threshold"),
            }

            // Body should contain Trigger + Read for the acquire node
            let has_trigger = body
                .iter()
                .any(|cmd| matches!(cmd, PlanCommand::Trigger { .. }));
            assert!(has_trigger, "Body should contain Trigger command");
        }
    }

    #[test]
    fn test_condition_loop_less_than_emits_above_false() {
        let mut snarl = Snarl::new();

        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Condition {
                    device_id: "temp_sensor".to_string(),
                    operator: ThresholdOp::LessThan,
                    value: 25.0,
                    max_iterations: 50,
                },
            }),
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Condition loops should translate");

        let repeat_while = plan
            .commands
            .iter()
            .find(|cmd| matches!(cmd, PlanCommand::RepeatWhile { .. }));
        assert!(repeat_while.is_some());

        if let Some(PlanCommand::RepeatWhile { condition, .. }) = repeat_while {
            match condition {
                experiment::plans::EvalCondition::Threshold { above, .. } => {
                    assert!(!above, "LessThan should map to above=false");
                }
                _ => panic!("Expected EvalCondition::Threshold"),
            }
        }
    }

    #[test]
    fn test_threshold_wait_emits_wait_settled() {
        use crate::graph::nodes::WaitCondition;

        let mut snarl = Snarl::new();

        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Wait {
                condition: WaitCondition::Threshold {
                    device_id: "power_meter".to_string(),
                    operator: ThresholdOp::GreaterThan,
                    value: 100.0,
                    timeout_ms: 5000.0,
                },
            },
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Should emit WaitSettled, not Wait
        let wait_settled = plan
            .commands
            .iter()
            .find(|cmd| matches!(cmd, PlanCommand::WaitSettled { .. }));
        assert!(
            wait_settled.is_some(),
            "Threshold wait should emit WaitSettled"
        );

        if let Some(PlanCommand::WaitSettled {
            device_id,
            timeout_seconds,
        }) = wait_settled
        {
            assert_eq!(device_id, "power_meter");
            assert!((timeout_seconds - 5.0).abs() < f64::EPSILON);
        }

        // Should NOT emit a plain Wait
        let plain_wait = plan
            .commands
            .iter()
            .any(|cmd| matches!(cmd, PlanCommand::Wait { .. }));
        assert!(
            !plain_wait,
            "Threshold wait should not fall back to plain Wait"
        );
    }

    #[test]
    fn test_stability_wait_emits_wait_settled() {
        use crate::graph::nodes::WaitCondition;

        let mut snarl = Snarl::new();

        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Wait {
                condition: WaitCondition::Stability {
                    device_id: "temperature".to_string(),
                    tolerance: 0.01,
                    duration_ms: 500.0,
                    timeout_ms: 10000.0,
                },
            },
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Should emit WaitSettled, not Wait
        let wait_settled = plan
            .commands
            .iter()
            .find(|cmd| matches!(cmd, PlanCommand::WaitSettled { .. }));
        assert!(
            wait_settled.is_some(),
            "Stability wait should emit WaitSettled"
        );

        if let Some(PlanCommand::WaitSettled {
            device_id,
            timeout_seconds,
        }) = wait_settled
        {
            assert_eq!(device_id, "temperature");
            assert!((timeout_seconds - 10.0).abs() < f64::EPSILON);
        }

        // Should NOT emit a plain Wait
        let plain_wait = plan
            .commands
            .iter()
            .any(|cmd| matches!(cmd, PlanCommand::Wait { .. }));
        assert!(
            !plain_wait,
            "Stability wait should not fall back to plain Wait"
        );
    }

    #[test]
    fn test_move_wait_settled_emits_wait_settled() {
        use crate::graph::nodes::MoveConfig;

        let mut snarl = Snarl::new();

        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Move(MoveConfig {
                device: "stage_x".to_string(),
                position: 42.0,
                wait_settled: true,
                ..Default::default()
            }),
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Should have MoveTo followed by WaitSettled
        let has_move = plan.commands.iter().any(
            |cmd| matches!(cmd, PlanCommand::MoveTo { device_id, .. } if device_id == "stage_x"),
        );
        assert!(has_move, "Should emit MoveTo");

        let wait_settled = plan
            .commands
            .iter()
            .find(|cmd| matches!(cmd, PlanCommand::WaitSettled { .. }));
        assert!(
            wait_settled.is_some(),
            "Move with wait_settled=true should emit WaitSettled"
        );

        if let Some(PlanCommand::WaitSettled { device_id, .. }) = wait_settled {
            assert_eq!(device_id, "stage_x");
        }
    }

    #[test]
    fn test_infinite_loop_rejected() {
        let mut snarl = Snarl::new();

        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Loop(LoopConfig {
                termination: LoopTermination::Infinite {
                    max_iterations: 1000,
                },
            }),
        );

        let result = GraphPlan::from_snarl(&snarl);
        assert!(result.is_err(), "Infinite loops should be rejected");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Infinite loops are not supported"),
            "Error should mention infinite loop: {err}"
        );
    }

    #[test]
    fn test_nested_scan_dimension_names_propagated() {
        let mut snarl = Snarl::new();

        // Create NestedScan with explicit dimension names
        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::NestedScan(NestedScanConfig {
                outer: ScanDimension {
                    actuator: "stage_x".to_string(),
                    dimension_name: "wavelength".to_string(),
                    start: 400.0,
                    stop: 800.0,
                    points: 3,
                },
                inner: ScanDimension {
                    actuator: "stage_y".to_string(),
                    dimension_name: "position".to_string(),
                    start: 0.0,
                    stop: 10.0,
                    points: 2,
                },
                nesting_warning_depth: 3,
            }),
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        // Collect all EmitEvent scan_indices
        let indices: Vec<_> = plan
            .commands
            .iter()
            .filter_map(|cmd| {
                if let PlanCommand::EmitEvent { scan_indices, .. } = cmd {
                    scan_indices.clone()
                } else {
                    None
                }
            })
            .collect();

        // Should have 3 x 2 = 6 events with indices
        assert_eq!(indices.len(), 6, "Expected 6 events with scan_indices");

        // All indices should use the configured dimension names
        for idx_vec in &indices {
            assert_eq!(idx_vec.len(), 2);
            assert_eq!(
                idx_vec[0].0, "wavelength",
                "Outer dimension name should be 'wavelength'"
            );
            assert_eq!(
                idx_vec[1].0, "position",
                "Inner dimension name should be 'position'"
            );
        }

        // Check specific indices: (0,0), (0,1), (1,0), (1,1), (2,0), (2,1)
        assert_eq!(
            indices[0],
            vec![("wavelength".to_string(), 0), ("position".to_string(), 0)]
        );
        assert_eq!(
            indices[1],
            vec![("wavelength".to_string(), 0), ("position".to_string(), 1)]
        );
        assert_eq!(
            indices[2],
            vec![("wavelength".to_string(), 1), ("position".to_string(), 0)]
        );
        assert_eq!(
            indices[5],
            vec![("wavelength".to_string(), 2), ("position".to_string(), 1)]
        );
    }

    #[test]
    fn test_nested_scan_fallback_to_actuator_name() {
        let mut snarl = Snarl::new();

        // Create NestedScan with empty dimension names -- should fall back to actuator IDs
        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::NestedScan(NestedScanConfig {
                outer: ScanDimension {
                    actuator: "motor_x".to_string(),
                    dimension_name: String::new(),
                    start: 0.0,
                    stop: 10.0,
                    points: 2,
                },
                inner: ScanDimension {
                    actuator: "motor_y".to_string(),
                    dimension_name: String::new(),
                    start: 0.0,
                    stop: 5.0,
                    points: 2,
                },
                nesting_warning_depth: 3,
            }),
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        let first_indices = plan
            .commands
            .iter()
            .find_map(|cmd| {
                if let PlanCommand::EmitEvent { scan_indices, .. } = cmd {
                    scan_indices.clone()
                } else {
                    None
                }
            })
            .expect("Should have at least one EmitEvent with scan_indices");

        // Should fall back to actuator IDs
        assert_eq!(
            first_indices[0].0, "motor_x",
            "Should fall back to outer actuator ID"
        );
        assert_eq!(
            first_indices[1].0, "motor_y",
            "Should fall back to inner actuator ID"
        );
    }

    #[test]
    fn test_single_scan_has_scan_indices() {
        let mut snarl = Snarl::new();

        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::Scan {
                actuator: "stage_z".to_string(),
                start: 0.0,
                stop: 100.0,
                points: 5,
            },
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        let indices: Vec<_> = plan
            .commands
            .iter()
            .filter_map(|cmd| {
                if let PlanCommand::EmitEvent { scan_indices, .. } = cmd {
                    scan_indices.clone()
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(indices.len(), 5, "Expected 5 events with scan_indices");

        // Single-level scan uses actuator ID as dimension name
        for (i, idx_vec) in indices.iter().enumerate() {
            assert_eq!(idx_vec.len(), 1, "Single scan should have 1 dimension");
            assert_eq!(
                idx_vec[0].0, "stage_z",
                "Dimension name should be actuator ID"
            );
            assert_eq!(idx_vec[0].1, i, "Index should match iteration");
        }
    }

    #[test]
    fn test_adaptive_scan_has_scan_indices() {
        use crate::graph::nodes::{
            AdaptiveAction, AdaptiveScanConfig, TriggerCondition, TriggerLogic,
        };

        let mut snarl = Snarl::new();

        snarl.insert_node(
            egui::pos2(0.0, 0.0),
            ExperimentNode::AdaptiveScan(AdaptiveScanConfig {
                scan: ScanDimension {
                    actuator: "wavelength_motor".to_string(),
                    dimension_name: "lambda".to_string(),
                    start: 400.0,
                    stop: 800.0,
                    points: 4,
                },
                triggers: vec![TriggerCondition::default()],
                trigger_logic: TriggerLogic::Any,
                action: AdaptiveAction::Zoom2x,
                require_approval: false,
            }),
        );

        let plan = GraphPlan::from_snarl(&snarl).expect("Translation failed");

        let indices: Vec<_> = plan
            .commands
            .iter()
            .filter_map(|cmd| {
                if let PlanCommand::EmitEvent { scan_indices, .. } = cmd {
                    scan_indices.clone()
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(indices.len(), 4, "Expected 4 events with scan_indices");

        // Adaptive scan uses dimension_name from config
        for (i, idx_vec) in indices.iter().enumerate() {
            assert_eq!(idx_vec.len(), 1, "Adaptive scan should have 1 dimension");
            assert_eq!(
                idx_vec[0].0, "lambda",
                "Should use configured dimension name"
            );
            assert_eq!(idx_vec[0].1, i, "Index should match iteration");
        }
    }
}
