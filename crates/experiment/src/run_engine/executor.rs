//! Plan execution engine.
//!
//! Contains `execute_plan` and `process_command` — the orchestration loop
//! that drives plan execution. Hardware command dispatch (move, read,
//! trigger, set, evaluate condition) is delegated to
//! [`CommandDispatcher`](super::command_dispatch::CommandDispatcher).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};
use tracing::{debug, error, info, warn};

use common::experiment::document::{
    DataKey, DescriptorDoc, Document, EventDoc, ExperimentManifest, StartDoc, StopDoc,
};

use super::command_dispatch::CommandDispatcher;
use super::context::RunContext;
use super::state_machine::ExperimentFrameObserver;
use super::task_queue::QueuedPlan;
use super::RunEngine;
use crate::feedback::FeedbackEvent;
use crate::plans::PlanCommand;

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

        info!(
            run_uid = %run_uid,
            num_devices = %manifest.parameters.len(),
            "Captured experiment manifest with hardware parameters"
        );

        self.emit_document(Document::Manifest(manifest)).await;

        // Setup frame observers for any FrameProducers in the plan (bd-b86g.3)
        let mut frame_observers = HashMap::new();
        let mut frame_channels = HashMap::new();

        for det_id in plan.detectors() {
            if let Some(producer) = self.device_registry.get_frame_producer(&det_id) {
                if producer.supports_observers() {
                    let (tx, rx) = mpsc::channel(16);

                    let observer = Box::new(ExperimentFrameObserver {
                        tx,
                        device_id: det_id.to_string(),
                    });

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

        for det in plan.detectors() {
            if let Some(producer) = self.device_registry.get_frame_producer(&det) {
                let (w, h) = producer.resolution();
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
            *ctx = Some(RunContext {
                run_uid: run_uid.clone(),
                descriptor_uid,
                seq_num: 0,
                collected_data: HashMap::new(),
                collected_frames: HashMap::new(),
                collected_summing_counts: HashMap::new(),
                collected_metadata: HashMap::new(),
                current_positions: HashMap::new(),
                frame_observers,
                frame_channels,
                run_start_ns: start_ns,
            });
        }

        // Spawn heartbeat background task if lifecycle hook is configured.
        let (heartbeat_stop_tx, heartbeat_stop_rx) = tokio::sync::watch::channel(false);
        let _heartbeat_handle = if let Some(hook) = &self.lifecycle_hook {
            let hook = Arc::clone(hook);
            let uid = run_uid.clone();
            let mut stop_rx = heartbeat_stop_rx;
            Some(tokio::spawn(async move {
                let interval = Duration::from_secs(10);
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

        // Create command dispatcher for hardware interactions
        let dispatcher = CommandDispatcher {
            registry: &self.device_registry,
            feedback_tx: &self.feedback_tx,
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
                    break;
                }
            };

            // ---- Adaptive feedback integration (bd-0za1) ----
            if let PlanCommand::Checkpoint { ref label } = cmd {
                if label.contains("adaptive") {
                    let planned_pos = self
                        .run_context
                        .lock()
                        .await
                        .as_ref()
                        .and_then(|ctx| ctx.current_positions.values().last().copied())
                        .unwrap_or(0.0);

                    if let Some(adjusted) = self.drain_feedback_with_adaptation(planned_pos) {
                        if let Some(ctx) = self.run_context.lock().await.as_mut() {
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
            match self.process_command(cmd, &dispatcher).await {
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
        self.set_state(super::state_machine::EngineState::Idle)
            .await;

        info!(
            run_uid = %run_uid,
            exit_status = %exit_status,
            num_events = %num_events,
            "Plan execution complete"
        );

        Ok(())
    }

    /// Process a single plan command.
    ///
    /// Hardware interactions (move, read, trigger, set, condition eval) are
    /// delegated to the [`CommandDispatcher`]. Orchestration concerns (abort,
    /// pause, event emission, context management) stay here.
    ///
    /// Returns true if an event was emitted.
    pub(crate) async fn process_command(
        &self,
        cmd: PlanCommand,
        dispatcher: &CommandDispatcher<'_>,
    ) -> anyhow::Result<bool> {
        debug!(?cmd, "Processing command");

        match cmd {
            PlanCommand::MoveTo {
                device_id,
                position,
            } => {
                self.watchdog.touch().await;
                dispatcher
                    .execute_move(device_id.as_str(), position)
                    .await?;

                if let Some(ctx) = self.run_context.lock().await.as_mut() {
                    ctx.current_positions
                        .insert(device_id.to_string(), position);
                }
                Ok(false)
            }

            PlanCommand::Read { device_id } => {
                self.watchdog.touch().await;
                let mut is_frame_device = false;
                let id_str = device_id.to_string();

                {
                    let mut ctx_guard = self.run_context.lock().await;
                    if let Some(ctx) = ctx_guard.as_mut() {
                        if let Some(rx) = ctx.frame_channels.get_mut(&id_str) {
                            is_frame_device = true;
                            match rx.recv().await {
                                Some(capture) => {
                                    let data_len = capture.data.len();
                                    let frame_num = capture.frame_number;
                                    let summing_count = capture.summing_count;
                                    ctx.collected_frames.insert(id_str.clone(), capture.data);
                                    ctx.collected_summing_counts
                                        .insert(id_str.clone(), summing_count);
                                    // bd-p6r4: Collect frame metadata for EventDoc propagation
                                    if !capture.metadata.is_empty() {
                                        ctx.collected_metadata
                                            .insert(id_str.clone(), capture.metadata);
                                    }
                                    debug!(
                                        device = %device_id,
                                        size = %data_len,
                                        frame_num = %frame_num,
                                        ?summing_count,
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
                    let value = dispatcher.execute_read(device_id.as_str()).await?;

                    if let Some(ctx) = self.run_context.lock().await.as_mut() {
                        ctx.collected_data.insert(id_str, value);
                    }
                }
                Ok(false)
            }

            PlanCommand::Trigger { device_id } => {
                self.watchdog.touch().await;
                dispatcher.execute_trigger(&device_id).await?;
                Ok(false)
            }

            PlanCommand::Wait { seconds } => {
                debug!(seconds = %seconds, "Waiting");

                let total = Duration::from_secs_f64(seconds);
                let chunk = Duration::from_millis(100);
                let mut elapsed = Duration::ZERO;

                while elapsed < total {
                    if *self.abort_requested.read().await {
                        info!(
                            elapsed_ms = %elapsed.as_millis(),
                            total_ms = %total.as_millis(),
                            "Wait interrupted by abort request"
                        );
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

                if *self.pause_requested.read().await {
                    info!("Pausing at checkpoint");
                    self.set_state(super::state_machine::EngineState::Paused)
                        .await;
                }
                Ok(false)
            }

            PlanCommand::EmitEvent {
                stream: _,
                mut data,
                positions,
                scan_indices,
            } => {
                self.watchdog.touch().await;
                let mut ctx_guard = self.run_context.lock().await;
                let ctx = ctx_guard
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("No active run context"))?;

                data.extend(ctx.collected_data.drain());

                let collected_arrays = if !ctx.collected_frames.is_empty() {
                    let mut frames = HashMap::new();
                    for (k, v) in ctx.collected_frames.drain() {
                        frames.insert(k, v);
                    }
                    frames
                } else {
                    HashMap::new()
                };

                // bd-oqo7.7: Propagate summing counts into EventDoc metadata
                // so downstream consumers can normalize summed pixel data.
                let summing_metadata: HashMap<String, Option<u32>> =
                    ctx.collected_summing_counts.drain().collect();

                let mut all_positions = ctx.current_positions.clone();
                all_positions.extend(positions);

                let mut event = EventDoc::new(&ctx.run_uid, &ctx.descriptor_uid, ctx.seq_num);
                event.data = data;
                event.arrays = collected_arrays;
                event.positions = all_positions;
                event.scan_indices = scan_indices;

                // bd-oqo7.7: Add summing_count to event metadata for each detector
                for (device_id, count) in &summing_metadata {
                    if let Some(n) = count {
                        if *n > 1 {
                            event
                                .metadata
                                .insert(format!("{device_id}.summing_count"), n.to_string());
                        }
                    }
                }

                // bd-p6r4: Propagate frame metadata into EventDoc
                for (device_id, frame_md) in ctx.collected_metadata.drain() {
                    for (key, value) in frame_md {
                        event.metadata.insert(format!("{device_id}.{key}"), value);
                    }
                }

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
                self.watchdog.touch().await;
                debug!(device = %device_id, param = %parameter, value = %value, "Setting parameter");
                dispatcher
                    .execute_set_parameter(&device_id, &parameter, &value)
                    .await?;
                Ok(false)
            }

            PlanCommand::ConditionalBranch {
                condition,
                then_commands,
                else_commands,
            } => {
                let take_then = dispatcher.evaluate_condition(&condition).await;
                let branch_label = if take_then { "then" } else { "else" };
                debug!(?condition, branch = %branch_label, "ConditionalBranch evaluated");

                let commands = if take_then {
                    then_commands
                } else {
                    else_commands
                };

                let mut emitted = false;
                for sub_cmd in commands {
                    if Box::pin(self.process_command(sub_cmd, dispatcher)).await? {
                        emitted = true;
                    }
                }
                Ok(emitted)
            }

            PlanCommand::WaitSettled {
                device_id,
                timeout_seconds,
            } => {
                self.watchdog.touch().await;
                let deadline = Instant::now() + Duration::from_secs_f64(timeout_seconds);
                let poll_interval = Duration::from_millis(100);

                if let Some(readable) = dispatcher.registry.get_readable(&device_id) {
                    let mut last_value: Option<f64> = None;
                    let mut stable_since: Option<Instant> = None;
                    let stability_window = Duration::from_millis(500);
                    let tolerance = 0.01;

                    debug!(%device_id, timeout_seconds, "WaitSettled: polling for stability");

                    loop {
                        if Instant::now() >= deadline {
                            warn!(%device_id, "WaitSettled timed out");
                            break;
                        }

                        if *self.abort_requested.read().await {
                            info!(%device_id, "WaitSettled interrupted by abort");
                            return Ok(false);
                        }

                        match readable.read().await {
                            Ok(value) => {
                                if let Err(e) =
                                    dispatcher.feedback_tx.try_send(FeedbackEvent::ValueUpdate {
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
                                            if let Err(e) = dispatcher.feedback_tx.try_send(
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
                    warn!(
                        %device_id,
                        timeout_seconds,
                        "No Readable capability, falling back to timeout"
                    );
                    sleep(Duration::from_secs_f64(timeout_seconds)).await;
                }
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

                    if *self.abort_requested.read().await {
                        info!(iteration, "RepeatWhile interrupted by abort");
                        return Ok(emitted);
                    }

                    if !dispatcher.evaluate_condition(&condition).await {
                        debug!(iteration, "RepeatWhile: condition false, exiting loop");
                        break;
                    }

                    debug!(iteration, "RepeatWhile: executing body");
                    for sub_cmd in &body {
                        if Box::pin(self.process_command(sub_cmd.clone(), dispatcher)).await? {
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
}
