//! RunEngine - State machine for experiment orchestration (bd-73yh.1)
//!
//! The RunEngine executes plans, manages pause/resume, and emits documents.
//! It provides a clean abstraction between experiment logic (plans) and
//! hardware operations.
//!
//! # State Machine
//!
//! ```text
//! ┌──────┐   start()   ┌─────────┐
//! │ Idle │────────────▶│ Running │
//! └──────┘             └────┬────┘
//!    ▲                      │
//!    │  completed           │ pause() at checkpoint
//!    │                      ▼
//!    │                 ┌────────┐
//!    │◀────resume()────│ Paused │
//!    │                 └────────┘
//!    │
//!    │  abort()/halt()
//!    └────────────────────────────
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! let engine = RunEngine::new(device_registry);
//!
//! // Subscribe to documents
//! let mut docs = engine.subscribe();
//!
//! // Queue and run a plan
//! let run_uid = engine.queue(plan).await?;
//! engine.start().await?;
//!
//! // Process documents as they arrive
//! while let Some(doc) = docs.recv().await {
//!     match doc {
//!         Document::Event(e) => {
//!             println!("Data: {:?}", e.data);
//!             println!("Frames: {:?}", e.arrays.keys());
//!         }
//!         Document::Stop(_) => break,
//!         _ => {}
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

use super::plans::{Plan, PlanCommand};
use daq_core::data::Frame;
use daq_core::experiment::document::{
    new_uid, DataKey, DescriptorDoc, Document, EventDoc, ExperimentManifest, StartDoc, StopDoc,
};
use daq_hardware::registry::DeviceRegistry;

/// Engine state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    /// No plan running, ready to accept new plans
    Idle,
    /// Executing a plan
    Running,
    /// Paused at a checkpoint, can resume or abort
    Paused,
    /// Aborting current plan (will return to Idle)
    Aborting,
}

impl std::fmt::Display for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineState::Idle => write!(f, "idle"),
            EngineState::Running => write!(f, "running"),
            EngineState::Paused => write!(f, "paused"),
            EngineState::Aborting => write!(f, "aborting"),
        }
    }
}

/// A queued plan waiting to be executed
struct QueuedPlan {
    plan: Box<dyn Plan>,
    metadata: HashMap<String, String>,
    run_uid: String,
}

/// Run context for the currently executing plan
struct RunContext {
    run_uid: String,
    descriptor_uid: String,
    seq_num: u32,
    collected_data: HashMap<String, f64>,
    collected_frames: HashMap<String, Vec<u8>>,
    current_positions: HashMap<String, f64>,
    frame_subscriptions: HashMap<String, broadcast::Receiver<Arc<Frame>>>,
    /// Unix timestamp in nanoseconds when the run started
    run_start_ns: u64,
}

/// The RunEngine orchestrates experiment execution
pub struct RunEngine {
    /// Current engine state
    state: RwLock<EngineState>,

    /// Device registry for hardware operations
    device_registry: Arc<DeviceRegistry>,

    /// Queue of plans to execute
    plan_queue: Mutex<Vec<QueuedPlan>>,

    /// Document broadcast channel
    doc_sender: broadcast::Sender<Document>,

    /// Pause request flag
    pause_requested: RwLock<bool>,

    /// Abort request flag
    abort_requested: RwLock<bool>,

    /// Current run context (when running)
    run_context: Mutex<Option<RunContext>>,

    /// Last checkpoint label (for resume)
    last_checkpoint: RwLock<Option<String>>,
}

impl RunEngine {
    /// Create a new RunEngine
    pub fn new(device_registry: Arc<DeviceRegistry>) -> Self {
        let (doc_sender, _) = broadcast::channel(1024);

        Self {
            state: RwLock::new(EngineState::Idle),
            device_registry,
            plan_queue: Mutex::new(Vec::new()),
            doc_sender,
            pause_requested: RwLock::new(false),
            abort_requested: RwLock::new(false),
            run_context: Mutex::new(None),
            last_checkpoint: RwLock::new(None),
        }
    }

    /// Subscribe to document stream
    pub fn subscribe(&self) -> broadcast::Receiver<Document> {
        self.doc_sender.subscribe()
    }

    /// Get current engine state
    pub async fn state(&self) -> EngineState {
        *self.state.read().await
    }

    /// Get the start time (Unix nanoseconds) of the current run, if any
    pub async fn current_run_start_ns(&self) -> Option<u64> {
        self.run_context.lock().await.as_ref().map(|ctx| ctx.run_start_ns)
    }

    /// Get the run_uids of all queued plans
    pub async fn queued_run_uids(&self) -> Vec<String> {
        self.plan_queue
            .lock()
            .await
            .iter()
            .map(|q| q.run_uid.clone())
            .collect()
    }

    /// Queue a plan for execution
    pub async fn queue(&self, plan: Box<dyn Plan>) -> String {
        self.queue_with_metadata(plan, HashMap::new()).await
    }

    /// Queue a plan with user-provided metadata
    pub async fn queue_with_metadata(
        &self,
        plan: Box<dyn Plan>,
        metadata: HashMap<String, String>,
    ) -> String {
        let run_uid = new_uid();
        info!(run_uid = %run_uid, plan_type = %plan.plan_type(), "Queueing plan");

        let mut queue = self.plan_queue.lock().await;
        queue.push(QueuedPlan {
            plan,
            metadata,
            run_uid: run_uid.clone(),
        });

        run_uid
    }

    /// Start executing queued plans
    pub async fn start(&self) -> anyhow::Result<()> {
        let current_state = *self.state.read().await;
        if current_state != EngineState::Idle {
            anyhow::bail!("Cannot start: engine is {}", current_state);
        }

        // Reset flags
        *self.pause_requested.write().await = false;
        *self.abort_requested.write().await = false;

        // Get next plan from queue
        let queued = {
            let mut queue = self.plan_queue.lock().await;
            if queue.is_empty() {
                anyhow::bail!("No plans in queue");
            }
            queue.remove(0)
        };

        *self.state.write().await = EngineState::Running;
        info!("Engine started");

        // Execute the plan
        self.execute_plan(queued).await
    }

    /// Request pause at next checkpoint
    pub async fn pause(&self) -> anyhow::Result<()> {
        let current_state = *self.state.read().await;
        if current_state != EngineState::Running {
            anyhow::bail!("Cannot pause: engine is {}", current_state);
        }

        info!("Pause requested");
        *self.pause_requested.write().await = true;
        Ok(())
    }

    /// Resume from paused state
    pub async fn resume(&self) -> anyhow::Result<()> {
        let current_state = *self.state.read().await;
        if current_state != EngineState::Paused {
            anyhow::bail!("Cannot resume: engine is {}", current_state);
        }

        info!("Resuming from pause");
        *self.pause_requested.write().await = false;
        *self.state.write().await = EngineState::Running;
        Ok(())
    }

    /// Abort a plan by run_uid or the current plan if run_uid is None/empty
    ///
    /// - If `run_uid` is None or empty, aborts the currently executing plan
    /// - If `run_uid` matches the current run, aborts it
    /// - If `run_uid` matches a queued plan, removes it from the queue
    /// - Returns error if `run_uid` is specified but not found
    pub async fn abort(&self, reason: &str) -> anyhow::Result<()> {
        self.abort_run(None, reason).await
    }

    /// Abort a specific run by run_uid, or current if None/empty (bd-vi16.3)
    pub async fn abort_run(&self, run_uid: Option<&str>, reason: &str) -> anyhow::Result<()> {
        let target_uid = run_uid.filter(|s| !s.is_empty());

        match target_uid {
            None => {
                // Abort current run (existing behavior)
                let current_state = *self.state.read().await;
                match current_state {
                    EngineState::Running | EngineState::Paused => {
                        info!(reason = %reason, "Abort requested for current run");
                        *self.abort_requested.write().await = true;
                        *self.state.write().await = EngineState::Aborting;
                        Ok(())
                    }
                    _ => anyhow::bail!("Cannot abort: engine is {}", current_state),
                }
            }
            Some(uid) => {
                // Check if it matches current run
                let current_run_uid = self.current_run_uid().await;
                if current_run_uid.as_deref() == Some(uid) {
                    info!(run_uid = %uid, reason = %reason, "Abort requested for current run");
                    *self.abort_requested.write().await = true;
                    *self.state.write().await = EngineState::Aborting;
                    return Ok(());
                }

                // Check if it matches a queued plan
                let mut queue = self.plan_queue.lock().await;
                if let Some(pos) = queue.iter().position(|q| q.run_uid == uid) {
                    let removed = queue.remove(pos);
                    info!(
                        run_uid = %uid,
                        plan_type = %removed.plan.plan_type(),
                        reason = %reason,
                        "Removed queued plan"
                    );
                    return Ok(());
                }

                // Not found
                anyhow::bail!("Run '{}' not found (not current and not queued)", uid)
            }
        }
    }

    /// Halt immediately (emergency stop)
    pub async fn halt(&self) -> anyhow::Result<()> {
        warn!("HALT requested - emergency stop");
        *self.abort_requested.write().await = true;
        *self.state.write().await = EngineState::Aborting;
        // In a real implementation, this would also send stop commands to all hardware
        Ok(())
    }

    /// Execute a single plan
    async fn execute_plan(&self, mut queued: QueuedPlan) -> anyhow::Result<()> {
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

        // Setup frame subscriptions for any FrameProducers in the plan
        let mut frame_subscriptions = HashMap::new();
        for det_id in plan.detectors() {
            // Check if it's a FrameProducer
            if let Some(producer) = self.device_registry.get_frame_producer(&det_id) {
                if let Some(rx) = producer.subscribe_frames().await {
                    info!("Subscribed to frames from {}", det_id);
                    frame_subscriptions.insert(det_id.to_string(), rx);
                } else {
                    warn!(
                        "Device {} is FrameProducer but returned no subscription",
                        det_id
                    );
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
            *ctx = Some(RunContext {
                run_uid: run_uid.clone(),
                descriptor_uid,
                seq_num: 0,
                collected_data: HashMap::new(),
                collected_frames: HashMap::new(),
                current_positions: HashMap::new(),
                frame_subscriptions,
                run_start_ns: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0),
            });
        }

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
            if *self.state.read().await == EngineState::Paused {
                // Wait for resume or abort
                loop {
                    sleep(Duration::from_millis(100)).await;
                    if *self.abort_requested.read().await {
                        exit_status = "abort";
                        exit_reason = "User requested abort during pause".to_string();
                        break;
                    }
                    if *self.state.read().await == EngineState::Running {
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

        // Emit StopDoc
        let stop_doc = match exit_status {
            "success" => StopDoc::success(&run_uid, num_events),
            "abort" => StopDoc::abort(&run_uid, &exit_reason, num_events),
            _ => StopDoc::fail(&run_uid, &exit_reason, num_events),
        };
        self.emit_document(Document::Stop(stop_doc)).await;

        // Clear run context
        *self.run_context.lock().await = None;
        *self.state.write().await = EngineState::Idle;

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
    async fn process_command(&self, cmd: PlanCommand) -> anyhow::Result<bool> {
        debug!(?cmd, "Processing command");

        match cmd {
            PlanCommand::MoveTo {
                device_id,
                position,
            } => {
                self.execute_move(&device_id, position).await?;

                // Update current positions in context
                if let Some(ctx) = self.run_context.lock().await.as_mut() {
                    ctx.current_positions.insert(device_id, position);
                }
                Ok(false)
            }

            PlanCommand::Read { device_id } => {
                // Check if we have a frame subscription for this device
                let mut is_frame_device = false;

                {
                    // Scope to hold lock briefly
                    let mut ctx_guard = self.run_context.lock().await;
                    if let Some(ctx) = ctx_guard.as_mut() {
                        if let Some(rx) = ctx.frame_subscriptions.get_mut(&device_id) {
                            is_frame_device = true;
                            // Wait for a frame (async)
                            match rx.recv().await {
                                Ok(frame) => {
                                    let data_copy = frame.data.clone();
                                    ctx.collected_frames.insert(device_id.clone(), data_copy);
                                    debug!(device = %device_id, size = %frame.data.len(), "Captured frame");
                                }
                                Err(e) => {
                                    warn!(device = %device_id, error = %e, "Failed to receive frame");
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
                self.execute_trigger(&device_id).await?;
                Ok(false)
            }

            PlanCommand::Wait { seconds } => {
                debug!(seconds = %seconds, "Waiting");
                sleep(Duration::from_secs_f64(seconds)).await;
                Ok(false)
            }

            PlanCommand::Checkpoint { label } => {
                debug!(label = %label, "Checkpoint");
                *self.last_checkpoint.write().await = Some(label);

                // Check if pause was requested
                if *self.pause_requested.read().await {
                    info!("Pausing at checkpoint");
                    *self.state.write().await = EngineState::Paused;
                }
                Ok(false)
            }

            PlanCommand::EmitEvent {
                stream: _,
                mut data,
                positions,
            } => {
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
                debug!(device = %device_id, param = %parameter, value = %value, "Setting parameter");
                self.execute_set_parameter(&device_id, &parameter, &value)
                    .await?;
                Ok(false)
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

    /// Emit a document to all subscribers
    async fn emit_document(&self, doc: Document) {
        debug!(doc_type = ?std::mem::discriminant(&doc), uid = %doc.uid(), "Emitting document");

        // Ignore send errors (no subscribers)
        let _ = self.doc_sender.send(doc);
    }

    /// Get the number of queued plans
    pub async fn queue_len(&self) -> usize {
        self.plan_queue.lock().await.len()
    }

    /// Clear all queued plans
    pub async fn clear_queue(&self) {
        self.plan_queue.lock().await.clear();
    }

    /// Get the current run UID (if running)
    pub async fn current_run_uid(&self) -> Option<String> {
        self.run_context
            .lock()
            .await
            .as_ref()
            .map(|ctx| ctx.run_uid.clone())
    }

    /// Get current progress (events emitted so far)
    pub async fn current_progress(&self) -> Option<u32> {
        self.run_context
            .lock()
            .await
            .as_ref()
            .map(|ctx| ctx.seq_num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plans::Count;

    #[tokio::test]
    async fn test_engine_state_transitions() {
        let registry = Arc::new(DeviceRegistry::new());
        let engine = RunEngine::new(registry);

        assert_eq!(engine.state().await, EngineState::Idle);

        // Can't pause when idle
        assert!(engine.pause().await.is_err());

        // Can't resume when idle
        assert!(engine.resume().await.is_err());
    }

    #[tokio::test]
    async fn test_queue_plan() {
        let registry = Arc::new(DeviceRegistry::new());
        let engine = RunEngine::new(registry);

        let plan = Box::new(Count::new(5));
        let _run_uid = engine.queue(plan).await;

        assert_eq!(engine.queue_len().await, 1);
    }

    #[tokio::test]
    async fn test_document_subscription() {
        let registry = Arc::new(DeviceRegistry::new());
        let engine = RunEngine::new(registry);

        let mut rx = engine.subscribe();

        // Queue a simple plan
        let plan = Box::new(Count::new(3));
        engine.queue(plan).await;

        // Start in a separate task
        let engine_clone = Arc::new(engine);
        let engine_for_task = engine_clone.clone();
        tokio::spawn(async move {
            let _ = engine_for_task.start().await;
        });

        // Should receive StartDoc
        let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(doc.is_ok());
        if let Ok(Ok(Document::Start(start))) = doc {
            assert_eq!(start.plan_type, "count");
        }

        // Should receive Manifest document after Start (bd-ib06)
        let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(doc.is_ok());
        if let Ok(Ok(Document::Manifest(manifest))) = doc {
            assert_eq!(manifest.plan_type, "count");
            // Manifest should contain system info
            assert!(manifest.system_info.contains_key("software_version"));
        }
    }

    #[tokio::test]
    async fn test_engine_with_frame_producer() {
        use daq_hardware::registry::{DeviceConfig, DriverType};

        // 1. Setup Registry with MockCamera
        let registry = Arc::new(DeviceRegistry::new());
        {
            registry
                .register(DeviceConfig {
                    id: "cam1".to_string(),
                    name: "Mock Camera".to_string(),
                    driver: DriverType::MockCamera {
                        width: 10,
                        height: 10,
                    },
                })
                .await
                .unwrap();
        }

        // 2. Setup Engine
        // Arm the camera first (since Count plan doesn't stage/arm)
        {
            let cam = registry.get_triggerable("cam1").expect("cam1 not found");
            cam.arm().await.unwrap();
        }

        let engine = RunEngine::new(registry);
        let mut rx = engine.subscribe();

        // 3. Queue Count plan using camera
        // Note: Count plan uses "detectors" for reading
        let plan = Box::new(Count::new(3).with_detector("cam1"));

        let engine_clone = Arc::new(engine);
        let run_future = tokio::spawn(async move {
            let _ = engine_clone.queue(plan).await;
            engine_clone.start().await
        });

        // 4. Verify Documents
        let mut descriptor_seen = false;
        let mut events_seen = 0;

        // processing loop
        while let Ok(doc_result) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            match doc_result {
                Ok(Document::Descriptor(desc)) => {
                    descriptor_seen = true;
                    // Check if cam1 is registered as array
                    if let Some(key) = desc.data_keys.get("cam1") {
                        assert_eq!(key.dtype, "uint16");
                        assert_eq!(key.shape, vec![10, 10]);
                    }
                }
                Ok(Document::Event(event)) => {
                    events_seen += 1;
                    // Check if arrays has cam1 data
                    assert!(
                        event.arrays.contains_key("cam1"),
                        "Event missing cam1 array"
                    );
                    let data = event.arrays.get("cam1").unwrap();
                    assert!(!data.is_empty());
                }
                Ok(Document::Stop(_)) => break,
                Err(_) => break, // Channel closed
                _ => {}
            }
        }

        // Allow graceful finish
        let _ = run_future.await;

        assert!(descriptor_seen, "Did not receive DescriptorDoc");
        assert_eq!(events_seen, 3, "Did not receive 3 EventDocs");
    }
}
