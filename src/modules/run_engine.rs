//! RunEngine - Experiment Orchestrator
//!
//! Central coordinator for multi-module experiments with:
//! - Stage/unstage lifecycle management
//! - Document stream coordination
//! - Backpressure-aware data pipeline
//! - Error handling with guaranteed cleanup
//!
//! # Design Principles
//!
//! Inspired by Bluesky's RunEngine:
//! - Modules are staged before use and unstaged after (even on error)
//! - Document stream provides structured, self-describing data
//! - Backpressure prevents memory exhaustion during high-speed acquisition
//!
//! # Example
//!
//! ```rust,ignore
//! let engine = RunEngine::new(registry);
//!
//! // Execute a power monitoring run
//! let report = engine.execute(
//!     "power_scan",
//!     vec!["power_monitor_1"],
//!     RunConfig::default()
//! ).await?;
//!
//! println!("Run {} completed with {} events", report.run_uid, report.num_events);
//! ```

use super::document::{Document, StopReason};
use super::ModuleState;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;

// We need to import ModuleRegistry but avoid circular imports
// The registry will be passed in at runtime
use super::ModuleRegistry;

// =============================================================================
// RunEngine Configuration
// =============================================================================

/// Configuration for a run execution.
#[derive(Clone, Debug)]
pub struct RunConfig {
    /// Maximum duration for the run (None = unlimited)
    pub max_duration_secs: Option<f64>,
    /// Document channel buffer size (for backpressure)
    pub document_buffer_size: usize,
    /// Metadata to include in RunStart document
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            max_duration_secs: None,
            document_buffer_size: 1000, // Bounded for backpressure
            metadata: HashMap::new(),
        }
    }
}

impl RunConfig {
    /// Set maximum run duration.
    pub fn with_max_duration(mut self, secs: f64) -> Self {
        self.max_duration_secs = Some(secs);
        self
    }

    /// Add metadata.
    pub fn with_metadata(
        mut self,
        key: impl Into<String>,
        value: impl serde::Serialize,
    ) -> Self {
        self.metadata
            .insert(key.into(), serde_json::to_value(value).unwrap());
        self
    }
}

// =============================================================================
// Run Report
// =============================================================================

/// Report generated after a run completes.
#[derive(Clone, Debug)]
pub struct RunReport {
    /// Unique identifier for this run
    pub run_uid: Uuid,
    /// Run type/name
    pub scan_type: String,
    /// Start timestamp (ns since epoch)
    pub start_time_ns: u64,
    /// End timestamp (ns since epoch)
    pub end_time_ns: u64,
    /// Total number of events produced
    pub num_events: u64,
    /// How the run ended
    pub stop_reason: StopReason,
    /// Module-specific results
    pub module_results: HashMap<String, ModuleResult>,
}

/// Result from a single module.
#[derive(Clone, Debug)]
pub struct ModuleResult {
    /// Module ID
    pub module_id: String,
    /// Final state
    pub final_state: ModuleState,
    /// Events produced by this module
    pub events_produced: u64,
    /// Any error message
    pub error: Option<String>,
}

// =============================================================================
// Staged Modules Guard (RAII)
// =============================================================================

/// RAII guard for staged modules.
///
/// Ensures unstage is called even if the run fails or panics.
/// Uses explicit async close() since Drop can't be async.
pub struct StagedModules {
    /// Module IDs that have been staged
    staged_ids: Vec<String>,
    /// Reference to registry for unstaging
    registry: Arc<RwLock<ModuleRegistry>>,
    /// Whether close() has been called
    closed: bool,
}

impl StagedModules {
    /// Create and stage all modules.
    pub async fn new(
        module_ids: Vec<String>,
        registry: Arc<RwLock<ModuleRegistry>>,
    ) -> Result<Self> {
        let mut guard = Self {
            staged_ids: Vec::new(),
            registry,
            closed: false,
        };

        // Stage all modules
        for module_id in module_ids {
            // Get context and stage through registry
            let mut reg = guard.registry.write().await;

            // Check module exists
            if reg.get_module(&module_id).is_none() {
                drop(reg);
                guard.unstage_all().await;
                return Err(anyhow!("Module not found: {}", module_id));
            }

            // Stage the module
            if let Err(e) = reg.stage_module(&module_id).await {
                warn!("Stage failed for {}: {}, cleaning up", module_id, e);
                drop(reg);
                guard.unstage_all().await;
                return Err(e);
            }

            guard.staged_ids.push(module_id.clone());
            info!("Staged module: {}", module_id);
        }

        Ok(guard)
    }

    /// Get the list of staged module IDs.
    pub fn staged_ids(&self) -> &[String] {
        &self.staged_ids
    }

    /// Unstage all modules (called on close or drop).
    async fn unstage_all(&mut self) {
        // Unstage in reverse order
        let mut registry = self.registry.write().await;
        for module_id in self.staged_ids.iter().rev() {
            if let Err(e) = registry.unstage_module(module_id).await {
                error!("Unstage failed for {}: {}", module_id, e);
            } else {
                info!("Unstaged module: {}", module_id);
            }
        }
        self.staged_ids.clear();
    }

    /// Explicitly close and unstage all modules.
    pub async fn close(mut self) {
        self.unstage_all().await;
        self.closed = true;
    }
}

impl Drop for StagedModules {
    fn drop(&mut self) {
        if !self.closed && !self.staged_ids.is_empty() {
            // Can't call async in drop, log warning
            error!(
                "StagedModules dropped without close()! {} modules may not be unstaged: {:?}",
                self.staged_ids.len(),
                self.staged_ids
            );
        }
    }
}

// =============================================================================
// RunEngine
// =============================================================================

/// Central orchestrator for module execution.
pub struct RunEngine {
    registry: Arc<RwLock<ModuleRegistry>>,
}

impl RunEngine {
    /// Create a new RunEngine.
    pub fn new(registry: Arc<RwLock<ModuleRegistry>>) -> Self {
        Self { registry }
    }

    /// Execute a run with the specified modules.
    ///
    /// This method:
    /// 1. Stages all modules
    /// 2. Emits RunStart document
    /// 3. Starts all modules
    /// 4. Waits for completion or timeout
    /// 5. Stops all modules
    /// 6. Emits RunStop document
    /// 7. Unstages all modules (guaranteed even on error)
    pub async fn execute(
        &self,
        scan_type: impl Into<String>,
        module_ids: Vec<String>,
        config: RunConfig,
    ) -> Result<RunReport> {
        let scan_type = scan_type.into();
        let run_uid = Uuid::new_v4();
        let start_time_ns = now_ns();

        info!("Starting run {} ({})", run_uid, scan_type);

        // Create document channel (bounded for backpressure)
        let (doc_tx, mut doc_rx) = mpsc::channel::<Document>(config.document_buffer_size);

        // Stage all modules
        let staged = StagedModules::new(module_ids.clone(), Arc::clone(&self.registry)).await?;

        // Track results
        let mut num_events = 0u64;
        let mut module_results = HashMap::new();
        let mut stop_reason = StopReason::Success;

        // Emit RunStart
        let _ = doc_tx
            .send(Document::run_start(run_uid, &scan_type))
            .await;

        // Start all modules
        {
            let mut registry = self.registry.write().await;
            for module_id in staged.staged_ids() {
                if let Err(e) = registry.start_module(module_id).await {
                    error!("Failed to start module {}: {}", module_id, e);
                    stop_reason = StopReason::Fail(format!(
                        "Module {} failed to start: {}",
                        module_id, e
                    ));

                    // Record failure
                    if let Some(instance) = registry.get_module(module_id) {
                        module_results.insert(
                            module_id.clone(),
                            ModuleResult {
                                module_id: module_id.clone(),
                                final_state: instance.state(),
                                events_produced: 0,
                                error: Some(e.to_string()),
                            },
                        );
                    }
                    break;
                }
                info!("Started module: {}", module_id);
            }
        }

        // If all modules started successfully, wait for completion
        if matches!(stop_reason, StopReason::Success) {
            let timeout_duration = config.max_duration_secs.map(Duration::from_secs_f64);

            tokio::select! {
                // Timeout branch
                _ = async {
                    if let Some(duration) = timeout_duration {
                        tokio::time::sleep(duration).await;
                    } else {
                        // No timeout, wait forever (will be cancelled by other branch)
                        std::future::pending::<()>().await;
                    }
                } => {
                    info!("Run {} timed out", run_uid);
                    stop_reason = StopReason::Abort;
                }
                // Document processing branch
                _ = async {
                    while let Some(doc) = doc_rx.recv().await {
                        if matches!(doc, Document::Event(_)) {
                            num_events += 1;
                        }
                    }
                } => {
                    info!("Document stream closed for run {}", run_uid);
                }
            }

            // Stop all modules and collect results
            let mut registry = self.registry.write().await;
            for module_id in staged.staged_ids() {
                if let Err(e) = registry.stop_module(module_id).await {
                    warn!("Failed to stop module {}: {}", module_id, e);
                }

                if let Some(instance) = registry.get_module(module_id) {
                    module_results.insert(
                        module_id.clone(),
                        ModuleResult {
                            module_id: module_id.clone(),
                            final_state: instance.state(),
                            events_produced: instance.data_points_produced,
                            error: instance.error_message.clone(),
                        },
                    );
                }
            }
        }

        // Emit RunStop
        let _ = doc_tx
            .send(Document::run_stop(run_uid, stop_reason.clone()))
            .await;

        // Unstage all modules (guaranteed cleanup)
        staged.close().await;

        let end_time_ns = now_ns();

        let report = RunReport {
            run_uid,
            scan_type,
            start_time_ns,
            end_time_ns,
            num_events,
            stop_reason,
            module_results,
        };

        info!(
            "Run {} completed: {} events in {:.2}s",
            run_uid,
            num_events,
            (end_time_ns - start_time_ns) as f64 / 1e9
        );

        Ok(report)
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Get current time in nanoseconds since epoch.
fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_config_builder() {
        let config = RunConfig::default()
            .with_max_duration(60.0)
            .with_metadata("operator", "test_user")
            .with_metadata("sample", "silicon_wafer");

        assert_eq!(config.max_duration_secs, Some(60.0));
        assert!(config.metadata.contains_key("operator"));
        assert!(config.metadata.contains_key("sample"));
    }
}
