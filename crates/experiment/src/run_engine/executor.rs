//! Plan execution engine.
//!
//! Contains `execute_plan`, `process_command`, condition evaluation, and
//! the individual hardware command handlers (move, read, trigger, set).

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};
use tracing::{debug, error, info, warn};

use common::experiment::document::{
    DataKey, DescriptorDoc, Document, EventDoc, ExperimentManifest, StartDoc, StopDoc,
};

use super::state_machine::ExperimentFrameObserver;
use super::{QueuedPlan, RunEngine};
use crate::feedback::FeedbackEvent;
use crate::plans::{EvalCondition, PlanCommand};

impl RunEngine {
    /// Execute a single plan
    #[tracing::instrument(skip(self, queued), fields(run_uid = %queued.run_uid, plan_type = %queued.plan.plan_type()), err)]
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) async fn execute_plan(&self, mut queued: QueuedPlan) -> anyhow::Result<()> {
        let plan = &mut queued.plan;

        // Create and emit StartDoc
        let mut start_doc = StartDoc::new(plan.plan_type(), plan.plan_name());
        start_doc.uid = queued.run_uid.clone();
        start_doc.plan_args = plan.plan_args();
        start_doc.metadata = queued.metadata;
        start_doc.hints = plan.movers();

        let run_uid = start_doc.uid.clone();
        self.emit_document(Document::Start(start_doc.clone())).await;

        // Capture experiment manifest - snapshot all hardware parameters (bd-ej44)
        let parameter_snapshot = self.device_registry.snapshot_all_parameters();
        let manifest = ExperimentManifest::new(
            &run_uid,
            &start_doc.plan_type,
            &start_doc.plan_name,
            parameter_snapshot,
        )
        .with_metadata(start_doc.metadata.clone());

        // Log manifest creation
        info!(
            run_uid = %run_uid,
            num_devices = %manifest.parameters.len(),
            "Captured experiment manifest with hardware parameters"
        );

        // Emit manifest document for persistence (bd-ib06)
        // Storage backends (e.g., HDF5Writer) can subscribe to this document
        // and persist the hardware state snapshot for experiment reproducibility
        self.emit_document(Document::Manifest(manifest)).await;

        // Setup frame observers for any FrameProducers in the plan (bd-b86g.3)
        // Using observer pattern for secondary frame capture (experiment persistence)
        let mut frame_observers = HashMap::new();
        let mut frame_channels = HashMap::new();

        for det_id in plan.detectors() {
            if let Some(producer) = self.device_registry.get_frame_producer(&det_id) {
                if producer.supports_observers() {
                    // Create channel for frame capture
                    let (tx, rx) = mpsc::channel(16);

                    // Create observer
                    let observer = Box::new(ExperimentFrameObserver {
                        tx,
                        device_id: det_id.to_string(),
                    });

                    // Register observer
                    match producer.register_observer(observer).await {
                        Ok(handle) => {
                            info!("Registered frame observer for {}", det_id);
                            frame_observers.insert(det_id.to_string(), handle);
                            frame_channels.insert(det_id.to_string(), rx);
                        }
                        Err(e) => {
                            warn!("Failed to register observer for {}: {}", det_id, e);
                        }
                    }
                }
            }
        }

        // Create and emit DescriptorDoc for the primary stream
        let mut descriptor = DescriptorDoc::new(&run_uid, "primary");

        // Populate descriptor data keys
        for det in plan.detectors() {
            if let Some(producer) = self.device_registry.get_frame_producer(&det) {
                let (w, h) = producer.resolution();
                // Assume uint16 for now, or check metadata if available
                // Assume uint16 for now, or check metadata if available
                #[allow(clippy::cast_possible_wrap)]
                // SAFETY: value fits in target type range
                let mut key = DataKey::array(&det, vec![h as i32, w as i32]);
                key.dtype = "uint16".to_string();
                descriptor.data_keys.insert(det.clone(), key);
            } else {
                descriptor
                    .data_keys
                    .insert(det.clone(), DataKey::scalar(&det, ""));
            }
        }
        for mover in plan.movers() {
            descriptor
                .data_keys
                .insert(mover.clone(), DataKey::scalar(&mover, ""));
        }

        let descriptor_uid = descriptor.uid.clone();
        self.emit_document(Document::Descriptor(descriptor)).await;

        // Initialize run context
        {
            let mut ctx = self.run_context.lock().await;
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: Unix epoch nanos will not exceed u64::MAX until year 2554
            let start_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            *ctx = Some(super::RunContext {
                run_uid: run_uid.clone(),
                descriptor_uid,
                seq_num: 0,
                collected_data: HashMap::new(),
                collected_frames: HashMap::new(),
                current_positions: HashMap::new(),
                frame_observers,
                frame_channels,
                run_start_ns: start_ns,
            });
        }

        // Spawn heartbeat background task if lifecycle hook is configured.
        // The task runs every ~10s and calls on_heartbeat(run_uid) until
        // the stop signal is sent (when the run completes/aborts).
        let (heartbeat_stop_tx, heartbeat_stop_rx) = tokio::sync::watch::channel(false);
        let _heartbeat_handle = if let Some(hook) = &self.lifecycle_hook {
            let hook = Arc::clone(hook);
            let uid = run_uid.clone();
            let mut stop_rx = heartbeat_stop_rx;
            Some(tokio::spawn(async move {
                let interval = Duration::from_secs(10);
                // Send initial heartbeat immediately so the run is never
                // without a timestamp (avoids false-positive stale detection
                // during the first interval).
                if let Err(e) = hook.on_heartbeat(&uid).await {
                    warn!(run_uid = %uid, error = %e, "initial heartbeat failed");
                }
                loop {
                    tokio::select! {
                        () = sleep(interval) => {}
                        _ = stop_rx.changed() => {
                            break;
                        }
                    }
                    if let Err(e) = hook.on_heartbeat(&uid).await {
                        warn!(run_uid = %uid, error = %e, "heartbeat update failed");
                    }
                }
            }))
        } else {
            None
        };

        // Execute plan commands
        let mut num_events = 0u32;
        let mut exit_status = "success";
        let mut exit_reason = String::new();

        loop {
            // Check for abort
            if *self.abort_requested.read().await {
                exit_status = "abort";
                exit_reason = "User requested abort".to_string();
                break;
            }

            // Check for pause (only at checkpoints, handled in command processing)
            if *self.state.read().await == super::state_machine::EngineState::Paused {
                // Wait for resume or abort
                loop {
                    sleep(Duration::from_millis(100)).await;
                    if *self.abort_requested.read().await {
                        exit_status = "abort";
                        exit_reason = "User requested abort during pause".to_string();
                        break;
                    }
                    if *self.state.read().await == super::state_machine::EngineState::Running {
                        break;
                    }
                }
                if exit_reason.is_empty() {
                    continue;
                } else {
                    break;
                }
            }

            // Get next command
            let cmd = match plan.next_command() {
                Some(cmd) => cmd,
                None => {
                    // Plan completed successfully
                    break;
                }
            };

            // ---- Adaptive feedback integration (bd-0za1) ----
            // Between plan steps, drain the feedback channel. At adaptive
            // checkpoints this can influence subsequent MoveTo positions by
            // refining scan points near interesting features.
            if let PlanCommand::Checkpoint { ref label } = cmd {
                if label.contains("adaptive") {
                    // Extract the current planned position from context, if any.
                    let planned_pos = self
                        .run_context
                        .lock()
                        .await
                        .as_ref()
                        .and_then(|ctx| ctx.current_positions.values().last().copied())
                        .unwrap_or(0.0);

                    if let Some(adjusted) = self.drain_feedback_with_adaptation(planned_pos) {
                        // AC-3: Record the adjustment in context so the next
                        // EmitEvent includes the adapted position.
                        if let Some(ctx) = self.run_context.lock().await.as_mut() {
                            // Update all tracked positions with the adjustment
                            // ratio so downstream Event documents reflect it.
                            for pos in ctx.current_positions.values_mut() {
                                if (*pos - planned_pos).abs() < f64::EPSILON {
                                    *pos = adjusted;
                                }
                            }
                        }
                    }
                }
            }

            // Process command
            match self.process_command(cmd).await {
                Ok(event_emitted) => {
                    if event_emitted {
                        num_events += 1;
                    }
                }
                Err(e) => {
                    error!(error = %e, "Plan execution failed");
                    exit_status = "fail";
                    exit_reason = e.to_string();
                    break;
                }
            }
        }

        // Stop the heartbeat background task.
        let _ = heartbeat_stop_tx.send(true);

        // Clean up frame observers before emitting StopDoc (bd-b86g.3)
        {
            let mut ctx_guard = self.run_context.lock().await;
            if let Some(ctx) = ctx_guard.as_mut() {
                for (det_id, handle) in ctx.frame_observers.drain() {
                    if let Some(producer) = self.device_registry.get_frame_producer(&det_id) {
                        if let Err(e) = producer.unregister_observer(handle).await {
                            warn!(
                                device = %det_id,
                                error = %e,
                                "Failed to unregister frame observer"
                            );
                        } else {
                            debug!(device = %det_id, "Unregistered frame observer");
                        }
                    }
                }
                // Clear channels
                ctx.frame_channels.clear();
            }
        }

        // Emit StopDoc
        let stop_doc = match exit_status {
            "success" => StopDoc::success(&run_uid, num_events),
            "abort" => StopDoc::abort(&run_uid, &exit_reason, num_events),
            _ => StopDoc::fail(&run_uid, &exit_reason, num_events),
        };
        self.emit_document(Document::Stop(stop_doc)).await;

        // Clear run context
        *self.run_context.lock().await = None;
        *self.state.write().await = super::state_machine::EngineState::Idle;

        info!(
            run_uid = %run_uid,
            exit_status = %exit_status,
            num_events = %num_events,
            "Plan execution complete"
        );

        Ok(())
    }

    /// Process a single plan command
    /// Returns true if an event was emitted
    pub(crate) async fn process_command(&self, cmd: PlanCommand) -> anyhow::Result<bool> {
        debug!(?cmd, "Processing command");

        match cmd {
            PlanCommand::MoveTo {
                device_id,
                position,
            } => {
                self.touch_activity().await;
                self.execute_move(&device_id, position).await?;

                // Update current positions in context
                if let Some(ctx) = self.run_context.lock().await.as_mut() {
                    ctx.current_positions.insert(device_id, position);
                }
                Ok(false)
            }

            PlanCommand::Read { device_id } => {
                self.touch_activity().await;
                // Check if we have a frame channel for this device
                let mut is_frame_device = false;

                {
                    // Scope to hold lock briefly
                    let mut ctx_guard = self.run_context.lock().await;
                    if let Some(ctx) = ctx_guard.as_mut() {
                        if let Some(rx) = ctx.frame_channels.get_mut(&device_id) {
                            is_frame_device = true;
                            // Wait for a frame (async, non-blocking channel receive)
                            match rx.recv().await {
                                Some(capture) => {
                                    let data_len = capture.data.len();
                                    let frame_num = capture.frame_number;
                                    ctx.collected_frames
                                        .insert(device_id.clone(), Bytes::from(capture.data));
                                    debug!(
                                        device = %device_id,
                                        size = %data_len,
                                        frame_num = %frame_num,
                                        "Captured frame"
                                    );
                                }
                                None => {
                                    warn!(device = %device_id, "Frame channel closed");
                                }
                            }
                        }
                    }
                }

                if !is_frame_device {
                    // Standard scalar read
                    let value = self.execute_read(&device_id).await?;

                    // Store in context for next EmitEvent
                    if let Some(ctx) = self.run_context.lock().await.as_mut() {
                        ctx.collected_data.insert(device_id, value);
                    }
                }
                Ok(false)
            }

            PlanCommand::Trigger { device_id } => {
                self.touch_activity().await;
                self.execute_trigger(&device_id).await?;
                Ok(false)
            }

            PlanCommand::Wait { seconds } => {
                debug!(seconds = %seconds, "Waiting");

                // Make wait interruptible by checking abort flag periodically (bd-lnoi)
                // Using chunked sleep approach: check every 100ms for responsiveness
                let total = Duration::from_secs_f64(seconds);
                let chunk = Duration::from_millis(100);
                let mut elapsed = Duration::ZERO;

                while elapsed < total {
                    // Check for abort before each chunk
                    if *self.abort_requested.read().await {
                        info!(
                            elapsed_ms = %elapsed.as_millis(),
                            total_ms = %total.as_millis(),
                            "Wait interrupted by abort request"
                        );
                        // Return Ok here - the abort will be handled by the main loop
                        // after this command returns, ensuring proper cleanup
                        return Ok(false);
                    }

                    let remaining = total - elapsed;
                    let sleep_duration = chunk.min(remaining);
                    sleep(sleep_duration).await;
                    elapsed += sleep_duration;
                }

                Ok(false)
            }

            PlanCommand::Checkpoint { label } => {
                debug!(label = %label, "Checkpoint");
                *self.last_checkpoint.write().await = Some(label);

                // Check if pause was requested
                if *self.pause_requested.read().await {
                    info!("Pausing at checkpoint");
                    *self.state.write().await = super::state_machine::EngineState::Paused;
                }
                Ok(false)
            }

            PlanCommand::EmitEvent {
                stream: _,
                mut data,
                positions,
                scan_indices,
            } => {
                self.touch_activity().await;
                let mut ctx_guard = self.run_context.lock().await;
                let ctx = ctx_guard
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("No active run context"))?;

                // Merge collected data
                data.extend(ctx.collected_data.drain());

                // Get frames
                let collected_arrays = if !ctx.collected_frames.is_empty() {
                    let mut frames = HashMap::new();
                    for (k, v) in ctx.collected_frames.drain() {
                        frames.insert(k, v);
                    }
                    frames
                } else {
                    HashMap::new()
                };

                // Merge positions
                let mut all_positions = ctx.current_positions.clone();
                all_positions.extend(positions);

                let mut event = EventDoc::new(&ctx.run_uid, &ctx.descriptor_uid, ctx.seq_num);
                event.data = data;
                event.arrays = collected_arrays;
                event.positions = all_positions;
                event.scan_indices = scan_indices;

                ctx.seq_num += 1;

                drop(ctx_guard);
                self.emit_document(Document::Event(event)).await;
                Ok(true)
            }

            PlanCommand::Set {
                device_id,
                parameter,
                value,
            } => {
                self.touch_activity().await;
                debug!(device = %device_id, param = %parameter, value = %value, "Setting parameter");
                self.execute_set_parameter(&device_id, &parameter, &value)
                    .await?;
                Ok(false)
            }

            PlanCommand::ConditionalBranch {
                condition,
                then_commands,
                else_commands,
            } => {
                let take_then = self.evaluate_condition(&condition).await;
                let branch_label = if take_then { "then" } else { "else" };
                debug!(?condition, branch = %branch_label, "ConditionalBranch evaluated");

                let commands = if take_then {
                    then_commands
                } else {
                    else_commands
                };

                // Box::pin required because process_command is recursive here.
                let mut emitted = false;
                for sub_cmd in commands {
                    if Box::pin(self.process_command(sub_cmd)).await? {
                        emitted = true;
                    }
                }
                Ok(emitted)
            }

            PlanCommand::WaitSettled {
                device_id,
                timeout_seconds,
            } => {
                self.touch_activity().await;
                let deadline = Instant::now() + Duration::from_secs_f64(timeout_seconds);
                let poll_interval = Duration::from_millis(100);

                // Try to get the readable capability for active monitoring
                if let Some(readable) = self.device_registry.get_readable(&device_id) {
                    let mut last_value: Option<f64> = None;
                    let mut stable_since: Option<Instant> = None;
                    let stability_window = Duration::from_millis(500);
                    let tolerance = 0.01; // 1% relative tolerance

                    debug!(%device_id, timeout_seconds, "WaitSettled: polling for stability");

                    loop {
                        if Instant::now() >= deadline {
                            warn!(%device_id, "WaitSettled timed out");
                            break;
                        }

                        // Check for abort
                        if *self.abort_requested.read().await {
                            info!(%device_id, "WaitSettled interrupted by abort");
                            return Ok(false);
                        }

                        match readable.read().await {
                            Ok(value) => {
                                // Send value update feedback
                                if let Err(e) =
                                    self.feedback_tx.try_send(FeedbackEvent::ValueUpdate {
                                        device_id: device_id.clone(),
                                        field: "value".to_string(),
                                        value,
                                    })
                                {
                                    warn!("Feedback event dropped (channel full): {e}");
                                }

                                if let Some(prev) = last_value {
                                    let delta = (value - prev).abs();
                                    let rel_delta = if prev.abs() > f64::EPSILON {
                                        delta / prev.abs()
                                    } else {
                                        delta
                                    };

                                    if rel_delta < tolerance {
                                        let now = Instant::now();
                                        let since = stable_since.get_or_insert(now);
                                        if now.duration_since(*since) >= stability_window {
                                            info!(%device_id, %value, "Device settled");
                                            if let Err(e) = self.feedback_tx.try_send(
                                                FeedbackEvent::StabilityReached {
                                                    device_id: device_id.clone(),
                                                    field: "value".to_string(),
                                                    variance: rel_delta,
                                                },
                                            ) {
                                                warn!("Feedback event dropped (channel full): {e}");
                                            }
                                            break;
                                        }
                                    } else {
                                        stable_since = None;
                                    }
                                }
                                last_value = Some(value);
                            }
                            Err(e) => {
                                warn!(%device_id, error = %e, "Read failed during WaitSettled");
                            }
                        }

                        sleep(poll_interval).await;
                    }
                } else {
                    // No readable capability - fall back to simple timeout
                    warn!(
                        %device_id,
                        timeout_seconds,
                        "No Readable capability, falling back to timeout"
                    );
                    sleep(Duration::from_secs_f64(timeout_seconds)).await;
                }
                // WaitSettled emits feedback events (ValueUpdate/StabilityReached)
                // but not Event documents, so return false.
                Ok(false)
            }

            PlanCommand::RepeatWhile {
                condition,
                body,
                max_iterations,
            } => {
                let mut iteration = 0u32;
                let mut emitted = false;

                debug!(?condition, max_iterations, "RepeatWhile: starting loop");

                loop {
                    if iteration >= max_iterations {
                        warn!(
                            iteration,
                            max_iterations,
                            "RepeatWhile: max iterations reached without condition becoming false"
                        );
                        break;
                    }

                    // Check for abort
                    if *self.abort_requested.read().await {
                        info!(iteration, "RepeatWhile interrupted by abort");
                        return Ok(emitted);
                    }

                    // Evaluate loop condition
                    if !self.evaluate_condition(&condition).await {
                        debug!(iteration, "RepeatWhile: condition false, exiting loop");
                        break;
                    }

                    debug!(iteration, "RepeatWhile: executing body");
                    // Box::pin required because process_command is recursive here.
                    for sub_cmd in &body {
                        if Box::pin(self.process_command(sub_cmd.clone())).await? {
                            emitted = true;
                        }
                    }

                    iteration += 1;
                }

                info!(iterations = iteration, "RepeatWhile: loop complete");
                Ok(emitted)
            }
        }
    }

    /// Evaluate an `EvalCondition` by reading from the device registry (bd-up05).
    ///
    /// Returns `true` if the condition is satisfied, `false` otherwise.
    /// On read errors the condition evaluates to `false` and a warning is logged.
    pub(crate) async fn evaluate_condition(&self, condition: &EvalCondition) -> bool {
        match condition {
            EvalCondition::Threshold {
                device_id,
                field: _,
                threshold,
                above,
            } => {
                let Some(readable) = self.device_registry.get_readable(device_id) else {
                    warn!(%device_id, "evaluate_condition: device not readable");
                    return false;
                };
                match readable.read().await {
                    Ok(value) => {
                        let result = if *above {
                            value > *threshold
                        } else {
                            value < *threshold
                        };
                        // Send threshold feedback when crossed
                        if result {
                            if let Err(e) =
                                self.feedback_tx.try_send(FeedbackEvent::ThresholdCrossed {
                                    device_id: device_id.clone(),
                                    field: "value".to_string(),
                                    value,
                                    threshold: *threshold,
                                })
                            {
                                warn!("Feedback event dropped (channel full): {e}");
                            }
                        }
                        result
                    }
                    Err(e) => {
                        warn!(%device_id, error = %e, "evaluate_condition: read failed");
                        false
                    }
                }
            }
            EvalCondition::Comparison {
                left_device_id,
                left_field: _,
                right_device_id,
                right_field: _,
                operator,
            } => {
                let left = self.device_registry.get_readable(left_device_id);
                let right = self.device_registry.get_readable(right_device_id);

                let (Some(left_r), Some(right_r)) = (left, right) else {
                    warn!(
                        %left_device_id,
                        %right_device_id,
                        "evaluate_condition: one or both devices not readable"
                    );
                    return false;
                };

                let (left_val, right_val) = match (left_r.read().await, right_r.read().await) {
                    (Ok(l), Ok(r)) => (l, r),
                    (Err(e), _) | (_, Err(e)) => {
                        warn!(error = %e, "evaluate_condition: read failed");
                        return false;
                    }
                };

                operator.evaluate(left_val, right_val)
            }
        }
    }

    /// Execute a move command
    async fn execute_move(&self, device_id: &str, position: f64) -> anyhow::Result<()> {
        debug!(device = %device_id, position = %position, "Moving");

        // Get the device from registry and move it
        let device = self.device_registry.get_movable(device_id);
        if let Some(device) = device {
            device.move_abs(position).await?;
        } else {
            warn!(device = %device_id, "Device not found or not movable, skipping move");
        }

        Ok(())
    }

    /// Execute a read command
    async fn execute_read(&self, device_id: &str) -> anyhow::Result<f64> {
        debug!(device = %device_id, "Reading");

        // Get the device from registry and read it
        let device = self.device_registry.get_readable(device_id);
        if let Some(device) = device {
            let value = device.read().await?;
            Ok(value)
        } else {
            warn!(device = %device_id, "Device not found or not readable, returning 0.0");
            Ok(0.0)
        }
    }

    /// Execute a trigger command
    async fn execute_trigger(&self, device_id: &str) -> anyhow::Result<()> {
        debug!(device = %device_id, "Triggering");

        // Get the device from registry and trigger it
        let device = self.device_registry.get_triggerable(device_id);
        if let Some(device) = device {
            device.trigger().await?;
        } else {
            debug!(device = %device_id, "Device not triggerable, skipping");
        }

        Ok(())
    }

    /// Execute a set parameter command
    async fn execute_set_parameter(
        &self,
        device_id: &str,
        parameter: &str,
        value: &str,
    ) -> anyhow::Result<()> {
        debug!(device = %device_id, param = %parameter, value = %value, "Setting parameter");

        // Try legacy Settable trait first (backwards compatibility)
        let settable = self.device_registry.get_settable(device_id);
        if let Some(settable) = settable {
            // Parse the value string to JSON
            let json_value: serde_json::Value = serde_json::from_str(value)
                .or_else(|_| {
                    // Try as raw string if JSON parsing fails
                    Ok::<_, serde_json::Error>(serde_json::Value::String(value.to_string()))
                })
                .map_err(|e| anyhow::anyhow!("Invalid value format: {}", e))?;

            settable.set_value(parameter, json_value).await?;
            return Ok(());
        }

        // New path - use Parameterized trait and Parameter<T> system
        // Parse the value string to JSON first
        let json_value: serde_json::Value = serde_json::from_str(value)
            .or_else(|_| {
                // Try as raw string if JSON parsing fails
                Ok::<_, serde_json::Error>(serde_json::Value::String(value.to_string()))
            })
            .map_err(|e| anyhow::anyhow!("Invalid value format: {}", e))?;

        if let Some(parameterized) = self.device_registry.get_parameterized(device_id) {
            let params = parameterized.parameters();
            if let Some(param) = params.get(parameter) {
                // Set the parameter (synchronous call via ParameterBase trait)
                param.set_json(json_value)?;
                return Ok(());
            } else {
                anyhow::bail!(
                    "Parameter '{}' not found on device '{}'",
                    parameter,
                    device_id
                );
            }
        }

        // Neither Settable nor Parameterized - device not found
        anyhow::bail!(
            "Device '{}' not found or does not support parameter setting",
            device_id
        );
    }
}
