//! DualRuntimeManager - Coordinates V2/V4 coexistence
//!
//! This module provides the core manager for coordinating simultaneous operation
//! of V2 (tokio-based) and V4 (Kameo-based) runtime systems.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use thiserror::Error;
use tracing::{info, warn, error, debug};

/// Errors that can occur during runtime management
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Runtime is not initialized")]
    NotInitialized,

    #[error("Runtime is already running")]
    AlreadyRunning,

    #[error("Shutdown timeout exceeded: {0:?}")]
    ShutdownTimeout(Duration),

    #[error("V2 runtime error: {0}")]
    V2Error(String),

    #[error("V4 runtime error: {0}")]
    V4Error(String),

    #[error("Runtime state error: expected {expected}, got {actual}")]
    StateError { expected: String, actual: String },

    #[error("Shutdown broadcast error")]
    BroadcastError,

    #[error("Concurrent operation not allowed in current state")]
    InvalidOperation,
}

/// Runtime state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerState {
    /// Initial state, no runtimes started
    Uninitialized,

    /// Starting phase: initializing V2 and V4 runtimes
    Starting,

    /// Both runtimes running and operational
    Running,

    /// Shutdown in progress
    ShuttingDown,

    /// All runtimes stopped and resources released
    Stopped,
}

impl std::fmt::Display for ManagerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uninitialized => write!(f, "Uninitialized"),
            Self::Starting => write!(f, "Starting"),
            Self::Running => write!(f, "Running"),
            Self::ShuttingDown => write!(f, "ShuttingDown"),
            Self::Stopped => write!(f, "Stopped"),
        }
    }
}

/// Placeholder for V2 runtime handle
/// In production, this would wrap the actual V2 DaqManagerActor or similar
#[derive(Debug)]
pub struct V2RuntimeHandle {
    /// Identifier for the V2 subsystem
    id: String,
    /// Whether V2 is currently active
    active: bool,
}

impl V2RuntimeHandle {
    fn new() -> Self {
        Self {
            id: "v2-subsystem".to_string(),
            active: false,
        }
    }

    fn activate(&mut self) {
        self.active = true;
    }

    fn deactivate(&mut self) {
        self.active = false;
    }
}

/// Dual Runtime Manager
///
/// Coordinates simultaneous operation of V2 (tokio-based) and V4 (Kameo-based) runtimes.
///
/// # Shutdown Sequence
///
/// When shutdown is initiated:
/// 1. V4 actors are gracefully shut down first (Kameo handles actor tree shutdown)
/// 2. V2 subsystem receives shutdown signal and completes in-flight work
/// 3. Broadcast channel is closed to signal global shutdown
/// 4. State transitions to Stopped
pub struct DualRuntimeManager {
    /// V2 runtime handle (placeholder for future implementation)
    v2_runtime: Option<V2RuntimeHandle>,

    /// V4 tokio runtime handle (Kameo runs on tokio)
    v4_runtime: Option<tokio::runtime::Handle>,

    /// Broadcast channel for coordinating shutdown across all actors
    shutdown_tx: Option<broadcast::Sender<()>>,

    /// Current runtime state
    state: Arc<Mutex<ManagerState>>,

    /// Configuration: default shutdown timeout
    shutdown_timeout: Duration,
}

impl DualRuntimeManager {
    /// Creates a new DualRuntimeManager with default configuration
    ///
    /// # Default Configuration
    /// - Shutdown timeout: 30 seconds
    /// - Initial state: Uninitialized
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(30))
    }

    /// Creates a new DualRuntimeManager with custom shutdown timeout
    ///
    /// # Arguments
    /// * `shutdown_timeout` - Maximum time to wait for graceful shutdown before forcing
    pub fn with_timeout(shutdown_timeout: Duration) -> Self {
        let state = Arc::new(Mutex::new(ManagerState::Uninitialized));

        info!(
            "DualRuntimeManager created with shutdown timeout: {:?}",
            shutdown_timeout
        );

        Self {
            v2_runtime: None,
            v4_runtime: None,
            shutdown_tx: None,
            state,
            shutdown_timeout,
        }
    }

    /// Returns the current runtime state
    pub async fn state(&self) -> ManagerState {
        *self.state.lock().await
    }

    /// Starts both V2 and V4 runtimes
    ///
    /// # Startup Sequence
    /// 1. Validates current state is Uninitialized
    /// 2. Transitions to Starting
    /// 3. Initializes V2 runtime
    /// 4. Initializes V4 runtime (tokio-based)
    /// 5. Creates shutdown broadcast channel
    /// 6. Transitions to Running
    ///
    /// # Errors
    /// Returns error if:
    /// - Manager is not in Uninitialized state
    /// - V2 or V4 startup fails
    pub async fn start(&mut self) -> Result<(), RuntimeError> {
        let mut state = self.state.lock().await;

        // Verify we're in the correct initial state
        if *state != ManagerState::Uninitialized {
            return Err(RuntimeError::StateError {
                expected: "Uninitialized".to_string(),
                actual: format!("{}", state),
            });
        }

        *state = ManagerState::Starting;
        drop(state); // Release lock before async operations

        info!("Starting dual runtime manager");

        // Phase 1: Initialize V2 runtime
        self.start_v2_runtime().await?;

        // Phase 2: Initialize V4 runtime
        self.start_v4_runtime().await?;

        // Phase 3: Create shutdown coordination channel
        let (shutdown_tx, _) = broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Phase 4: Transition to Running
        let mut state = self.state.lock().await;
        *state = ManagerState::Running;
        drop(state);

        info!("Dual runtime manager started successfully");
        Ok(())
    }

    /// Initiates graceful shutdown of both runtimes
    ///
    /// # Shutdown Sequence
    /// 1. Validates current state is Running
    /// 2. Transitions to ShuttingDown
    /// 3. Broadcasts shutdown signal to all components
    /// 4. Stops V4 runtime first (Kameo handles actor tree)
    /// 5. Stops V2 runtime
    /// 6. Releases broadcast channel
    /// 7. Transitions to Stopped
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for graceful shutdown
    ///
    /// # Errors
    /// Returns error if:
    /// - Manager is not in Running state
    /// - Shutdown operations exceed timeout
    pub async fn shutdown(&mut self, timeout: Duration) -> Result<(), RuntimeError> {
        let mut state = self.state.lock().await;

        // Verify we're in the correct state
        if *state != ManagerState::Running {
            return Err(RuntimeError::StateError {
                expected: "Running".to_string(),
                actual: format!("{}", state),
            });
        }

        *state = ManagerState::ShuttingDown;
        drop(state); // Release lock before async operations

        info!("Starting shutdown sequence with timeout {:?}", timeout);

        // Start shutdown timer
        let start = std::time::Instant::now();

        // Phase 1: Broadcast shutdown signal
        if let Some(shutdown_tx) = &self.shutdown_tx {
            // Send shutdown signal - receivers will hear this once
            let _ = shutdown_tx.send(());
        }

        // Phase 2: Shutdown V4 runtime first (Kameo actors)
        if let Err(e) = self.stop_v4_runtime(timeout).await {
            error!("Error stopping V4 runtime: {}", e);
            // Continue with V2 shutdown regardless
        }

        // Check remaining time
        let elapsed = start.elapsed();
        if elapsed > timeout {
            error!("Shutdown timeout exceeded after V4 shutdown");
            return Err(RuntimeError::ShutdownTimeout(timeout));
        }
        let remaining = timeout - elapsed;

        // Phase 3: Shutdown V2 runtime
        if let Err(e) = self.stop_v2_runtime(remaining).await {
            error!("Error stopping V2 runtime: {}", e);
        }

        // Phase 4: Release shutdown channel
        self.shutdown_tx = None;

        // Phase 5: Transition to Stopped
        let mut state = self.state.lock().await;
        *state = ManagerState::Stopped;
        drop(state);

        info!(
            "Shutdown completed in {:?}",
            start.elapsed()
        );
        Ok(())
    }

    /// Internal: Start V2 runtime
    async fn start_v2_runtime(&mut self) -> Result<(), RuntimeError> {
        debug!("Starting V2 runtime");

        // Placeholder implementation
        // In production, this would:
        // 1. Load V2 configuration
        // 2. Spawn DaqManagerActor with tokio::spawn
        // 3. Wait for actor to become ready
        // 4. Store ActorHandle for later shutdown

        let mut v2_handle = V2RuntimeHandle::new();
        v2_handle.activate();
        self.v2_runtime = Some(v2_handle);

        info!("V2 runtime started");
        Ok(())
    }

    /// Internal: Start V4 runtime
    async fn start_v4_runtime(&mut self) -> Result<(), RuntimeError> {
        debug!("Starting V4 runtime");

        // Placeholder implementation
        // In production, this would:
        // 1. Get current tokio runtime handle
        // 2. Spawn InstrumentManager actor via kameo::spawn
        // 3. Wait for manager to become ready
        // 4. Store ActorRef for later shutdown

        // Get current tokio runtime handle if available
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                self.v4_runtime = Some(handle);
                info!("V4 runtime started (using current tokio runtime)");
                Ok(())
            }
            Err(_) => {
                // No tokio runtime in current context
                // This is expected in some test scenarios
                warn!("No tokio runtime found; V4 runtime will be in limited mode");
                Ok(())
            }
        }
    }

    /// Internal: Stop V4 runtime
    async fn stop_v4_runtime(&mut self, timeout: Duration) -> Result<(), RuntimeError> {
        debug!("Stopping V4 runtime");

        // Placeholder implementation
        // In production, this would:
        // 1. Send shutdown message to InstrumentManager
        // 2. Wait for all Kameo actors to complete with timeout
        // 3. Verify no actors remain running

        // Simulate graceful shutdown
        tokio::time::sleep(Duration::from_millis(10)).await;

        self.v4_runtime = None;
        info!("V4 runtime stopped");
        Ok(())
    }

    /// Internal: Stop V2 runtime
    async fn stop_v2_runtime(&mut self, timeout: Duration) -> Result<(), RuntimeError> {
        debug!("Stopping V2 runtime with timeout {:?}", timeout);

        // Placeholder implementation
        // In production, this would:
        // 1. Send DaqCommand::Shutdown to DaqManagerActor
        // 2. Wait for actor task to complete (JoinHandle)
        // 3. Enforce timeout on join
        // 4. If timeout, force abort/panic the task

        // Simulate graceful shutdown
        if let Some(ref mut v2) = self.v2_runtime {
            v2.deactivate();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        self.v2_runtime = None;
        info!("V2 runtime stopped");
        Ok(())
    }

    /// Returns a shutdown broadcast receiver
    ///
    /// Useful for actors and subsystems that need to respond to shutdown signals.
    ///
    /// # Returns
    /// A broadcast receiver that will receive a message when shutdown is initiated.
    /// Returns None if shutdown channel is not yet initialized.
    ///
    /// # Example
    /// ```no_run
    /// # use v4_daq::runtime::DualRuntimeManager;
    /// # #[tokio::main]
    /// # async fn main() {
    /// # let mut manager = DualRuntimeManager::new();
    /// # manager.start().await.unwrap();
    /// if let Some(mut shutdown_rx) = manager.shutdown_broadcast() {
    ///     // Wait for shutdown signal
    ///     let _ = shutdown_rx.recv().await;
    ///     println!("Shutdown signal received!");
    /// }
    /// # }
    /// ```
    pub fn shutdown_broadcast(&self) -> Option<broadcast::Receiver<()>> {
        self.shutdown_tx.as_ref().map(|tx| tx.subscribe())
    }

    /// Checks if the runtime is currently running
    pub async fn is_running(&self) -> bool {
        *self.state.lock().await == ManagerState::Running
    }

    /// Forcefully aborts all runtimes (emergency shutdown)
    ///
    /// This bypasses graceful shutdown and immediately stops all activity.
    /// Should only be used when graceful shutdown fails or as an emergency measure.
    ///
    /// # Warning
    /// This may leave resources in inconsistent states. Prefer `shutdown()` when possible.
    pub async fn abort(&mut self) {
        warn!("Emergency abort triggered!");

        let mut state = self.state.lock().await;
        *state = ManagerState::Stopped;
        drop(state);

        // Release all resources
        self.v2_runtime = None;
        self.v4_runtime = None;
        self.shutdown_tx = None;

        warn!("Emergency abort completed");
    }
}

impl Default for DualRuntimeManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_new_initialization() {
        let manager = DualRuntimeManager::new();
        assert_eq!(manager.state().await, ManagerState::Uninitialized);
    }

    #[tokio::test]
    async fn test_with_custom_timeout() {
        let timeout = Duration::from_secs(60);
        let manager = DualRuntimeManager::with_timeout(timeout);
        assert_eq!(manager.state().await, ManagerState::Uninitialized);
        assert_eq!(manager.shutdown_timeout, timeout);
    }

    #[tokio::test]
    async fn test_startup_sequence() {
        let mut manager = DualRuntimeManager::new();

        // Verify initial state
        assert_eq!(manager.state().await, ManagerState::Uninitialized);

        // Start runtimes
        let result = manager.start().await;
        assert!(result.is_ok(), "Startup should succeed");

        // Verify running state
        assert_eq!(manager.state().await, ManagerState::Running);
        assert!(manager.is_running().await);
    }

    #[tokio::test]
    async fn test_shutdown_sequence() {
        let mut manager = DualRuntimeManager::new();

        // Start runtimes
        manager.start().await.expect("Startup failed");
        assert_eq!(manager.state().await, ManagerState::Running);

        // Shutdown with timeout
        let shutdown_result = manager.shutdown(Duration::from_secs(5)).await;
        assert!(shutdown_result.is_ok(), "Shutdown should succeed");

        // Verify stopped state
        assert_eq!(manager.state().await, ManagerState::Stopped);
        assert!(!manager.is_running().await);
    }

    #[tokio::test]
    async fn test_startup_from_non_uninitialized_state() {
        let mut manager = DualRuntimeManager::new();

        // Start successfully
        manager.start().await.expect("First startup failed");

        // Try to start again - should fail
        let result = manager.start().await;
        assert!(result.is_err(), "Second startup should fail");

        match result {
            Err(RuntimeError::StateError { expected, actual }) => {
                assert_eq!(expected, "Uninitialized");
                assert_eq!(actual, "Running");
            }
            other => panic!("Expected StateError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_shutdown_from_non_running_state() {
        let mut manager = DualRuntimeManager::new();

        // Try to shutdown without starting - should fail
        let result = manager.shutdown(Duration::from_secs(5)).await;
        assert!(result.is_err(), "Shutdown should fail");

        match result {
            Err(RuntimeError::StateError { expected, actual }) => {
                assert_eq!(expected, "Running");
                assert_eq!(actual, "Uninitialized");
            }
            other => panic!("Expected StateError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_shutdown_broadcast_before_start() {
        let manager = DualRuntimeManager::new();

        // Should return None before start
        assert!(manager.shutdown_broadcast().is_none());
    }

    #[tokio::test]
    async fn test_shutdown_broadcast_after_start() {
        let mut manager = DualRuntimeManager::new();
        manager.start().await.expect("Startup failed");

        // Should return Some after start
        let shutdown_rx = manager.shutdown_broadcast();
        assert!(shutdown_rx.is_some());
    }

    #[tokio::test]
    async fn test_abort_from_running_state() {
        let mut manager = DualRuntimeManager::new();

        // Start and verify running
        manager.start().await.expect("Startup failed");
        assert_eq!(manager.state().await, ManagerState::Running);

        // Abort
        manager.abort().await;

        // Verify stopped
        assert_eq!(manager.state().await, ManagerState::Stopped);
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let mut manager = DualRuntimeManager::new();

        // Track state transitions
        assert_eq!(manager.state().await, ManagerState::Uninitialized);

        manager.start().await.expect("Startup failed");
        assert_eq!(manager.state().await, ManagerState::Running);

        manager.shutdown(Duration::from_secs(5)).await.expect("Shutdown failed");
        assert_eq!(manager.state().await, ManagerState::Stopped);
    }

    #[tokio::test]
    async fn test_multiple_shutdown_broadcast_subscribers() {
        let mut manager = DualRuntimeManager::new();
        manager.start().await.expect("Startup failed");

        // Create multiple subscribers
        let mut rx1 = manager.shutdown_broadcast().expect("Should have broadcast");
        let mut rx2 = manager.shutdown_broadcast().expect("Should have broadcast");

        // Trigger shutdown in background
        let manager_handle = tokio::spawn({
            let mut mgr = manager;
            async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let _ = mgr.shutdown(Duration::from_secs(5)).await;
            }
        });

        // Both receivers should get the signal
        let signal1 = tokio::spawn(async move {
            rx1.recv().await.is_ok()
        });

        let signal2 = tokio::spawn(async move {
            rx2.recv().await.is_ok()
        });

        let (_, sig1, sig2) = tokio::join!(manager_handle, signal1, signal2);
        assert!(sig1.unwrap(), "Receiver 1 should get signal");
        assert!(sig2.unwrap(), "Receiver 2 should get signal");
    }

    #[test]
    fn test_manager_state_display() {
        assert_eq!(format!("{}", ManagerState::Uninitialized), "Uninitialized");
        assert_eq!(format!("{}", ManagerState::Starting), "Starting");
        assert_eq!(format!("{}", ManagerState::Running), "Running");
        assert_eq!(format!("{}", ManagerState::ShuttingDown), "ShuttingDown");
        assert_eq!(format!("{}", ManagerState::Stopped), "Stopped");
    }

    #[test]
    fn test_runtime_error_display() {
        let err = RuntimeError::NotInitialized;
        assert_eq!(format!("{}", err), "Runtime is not initialized");

        let err = RuntimeError::AlreadyRunning;
        assert_eq!(format!("{}", err), "Runtime is already running");
    }

    #[tokio::test]
    async fn test_v2_runtime_handle() {
        let mut v2 = V2RuntimeHandle::new();
        assert!(!v2.active);

        v2.activate();
        assert!(v2.active);

        v2.deactivate();
        assert!(!v2.active);
    }
}
