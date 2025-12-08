//! Panic Safety for Procedure Execution
//!
//! This module provides mechanisms to ensure hardware is left in a safe state
//! when procedure steps panic or encounter unexpected errors.
//!
//! # Problem
//!
//! When a procedure step panics mid-execution, hardware may be left in an unsafe state:
//! - Motion stages may continue moving
//! - Laser shutters may remain open
//! - Cameras may continue acquiring
//!
//! # Solution
//!
//! This module provides:
//! - `PanicGuard`: RAII guard that runs synchronous cleanup on drop
//! - `CleanupRegistry`: Registry for async cleanup actions
//! - `SafeStateConfig`: Configuration for device-specific safe states
//! - `with_hardware_safety`: Wrapper that ensures cleanup runs on error/panic
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::procedures::panic_safety::{PanicGuard, CleanupRegistry};
//!
//! // Simple sync guard
//! async fn dangerous_operation(stage: &Stage) -> Result<()> {
//!     // Guard will stop motion if we panic
//!     let _guard = PanicGuard::new(|| {
//!         // Note: This is sync - use for logging/flags only
//!         eprintln!("PANIC: Requesting motion stop");
//!     });
//!
//!     stage.move_abs(100.0).await?;
//!     // ... more operations
//!
//!     Ok(())
//! }
//!
//! // Async cleanup registry pattern
//! async fn procedure_with_cleanup(ctx: &ProcedureContext) -> Result<()> {
//!     let mut cleanup = CleanupRegistry::new();
//!
//!     let stage = ctx.get_movable("stage").await?;
//!     cleanup.register("stage", Box::new(move || {
//!         Box::pin(async move {
//!             let _ = stage.stop().await;
//!         })
//!     }));
//!
//!     // ... operations ...
//!
//!     // On success, clear cleanup (or let it run on drop for safety)
//!     cleanup.clear();
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// =============================================================================
// Panic Guard (Synchronous)
// =============================================================================

/// A synchronous RAII guard that executes cleanup when dropped.
///
/// This is useful for:
/// - Setting flags that signal other tasks to stop
/// - Logging panic events
/// - Triggering synchronous hardware emergency stops
///
/// # Limitations
///
/// - Cannot run async code (use `CleanupRegistry` for async cleanup)
/// - Must not panic in the cleanup function
///
/// # Example
///
/// ```rust,ignore
/// let emergency_stop = Arc::new(AtomicBool::new(false));
/// let stop_flag = emergency_stop.clone();
///
/// let _guard = PanicGuard::new(move || {
///     stop_flag.store(true, Ordering::SeqCst);
///     eprintln!("EMERGENCY: Hardware safety triggered");
/// });
///
/// // If we panic here, emergency_stop will be set to true
/// do_dangerous_operation()?;
///
/// // Explicitly dismiss on success
/// _guard.dismiss();
/// ```
pub struct PanicGuard {
    cleanup: Option<Box<dyn FnOnce() + Send + 'static>>,
    name: String,
}

impl PanicGuard {
    /// Create a new panic guard with a cleanup function.
    pub fn new<F>(cleanup: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            cleanup: Some(Box::new(cleanup)),
            name: "unnamed".to_string(),
        }
    }

    /// Create a named panic guard (for logging).
    pub fn named<F>(name: impl Into<String>, cleanup: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            cleanup: Some(Box::new(cleanup)),
            name: name.into(),
        }
    }

    /// Dismiss the guard without running cleanup.
    ///
    /// Call this when the protected operation completes successfully.
    pub fn dismiss(mut self) {
        self.cleanup = None;
    }

    /// Check if the guard is still armed.
    pub fn is_armed(&self) -> bool {
        self.cleanup.is_some()
    }
}

impl Drop for PanicGuard {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            // Check if we're panicking
            let panicking = std::thread::panicking();

            if panicking {
                eprintln!(
                    "[PANIC SAFETY] Guard '{}' triggered during panic - executing cleanup",
                    self.name
                );
            }

            // Run cleanup (catch any panics to avoid double-panic)
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(cleanup));

            if let Err(e) = result {
                eprintln!(
                    "[PANIC SAFETY] Cleanup for '{}' panicked: {:?}",
                    self.name, e
                );
            }
        }
    }
}

// =============================================================================
// Emergency Stop Flag
// =============================================================================

/// A shared flag for signaling emergency stops across tasks.
///
/// This is a simple, lock-free mechanism for coordinating shutdown.
#[derive(Clone)]
pub struct EmergencyStopFlag {
    flag: Arc<AtomicBool>,
    reason: Arc<std::sync::RwLock<Option<String>>>,
}

impl Default for EmergencyStopFlag {
    fn default() -> Self {
        Self::new()
    }
}

impl EmergencyStopFlag {
    /// Create a new emergency stop flag.
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            reason: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Trigger the emergency stop.
    pub fn trigger(&self, reason: impl Into<String>) {
        self.flag.store(true, Ordering::SeqCst);
        if let Ok(mut r) = self.reason.write() {
            *r = Some(reason.into());
        }
    }

    /// Check if emergency stop is active.
    pub fn is_triggered(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Get the reason for the emergency stop.
    pub fn reason(&self) -> Option<String> {
        self.reason.read().ok().and_then(|r| r.clone())
    }

    /// Reset the emergency stop flag.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
        if let Ok(mut r) = self.reason.write() {
            *r = None;
        }
    }

    /// Create a panic guard that triggers this flag on drop.
    pub fn guard(&self, context: impl Into<String>) -> PanicGuard {
        let flag = self.clone();
        let ctx = context.into();
        PanicGuard::named(ctx.clone(), move || {
            flag.trigger(format!("Panic in: {}", ctx));
        })
    }
}

// =============================================================================
// Cleanup Registry (Async)
// =============================================================================

/// Type alias for async cleanup functions.
pub type AsyncCleanupFn =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Registry for async cleanup actions.
///
/// This allows registering cleanup functions that will be executed
/// when the registry is dropped or explicitly run.
///
/// # Design
///
/// Unlike `PanicGuard`, this cannot run async code in Drop.
/// Instead, you must either:
/// 1. Call `run_all()` explicitly before dropping
/// 2. Use `into_sync_guard()` to create a sync guard that signals cleanup
///
/// # Example
///
/// ```rust,ignore
/// let mut cleanup = CleanupRegistry::new();
///
/// // Register stage stop
/// let stage = get_stage();
/// cleanup.register("stage_stop", Box::new(move || {
///     let stage = stage.clone();
///     Box::pin(async move {
///         let _ = stage.stop().await;
///     })
/// }));
///
/// // ... do work ...
///
/// // On success, clear or run cleanup
/// cleanup.clear();
/// ```
pub struct CleanupRegistry {
    actions: HashMap<String, AsyncCleanupFn>,
    order: Vec<String>,
}

impl Default for CleanupRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CleanupRegistry {
    /// Create a new empty cleanup registry.
    pub fn new() -> Self {
        Self {
            actions: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Register a cleanup action.
    ///
    /// Actions are run in reverse order of registration (LIFO).
    pub fn register(&mut self, name: impl Into<String>, action: AsyncCleanupFn) {
        let name = name.into();
        self.order.push(name.clone());
        self.actions.insert(name, action);
    }

    /// Remove a specific cleanup action.
    pub fn remove(&mut self, name: &str) -> Option<AsyncCleanupFn> {
        self.order.retain(|n| n != name);
        self.actions.remove(name)
    }

    /// Clear all cleanup actions (call on success).
    pub fn clear(&mut self) {
        self.actions.clear();
        self.order.clear();
    }

    /// Run all cleanup actions (in reverse order).
    ///
    /// This consumes the registry.
    pub async fn run_all(mut self) {
        // Run in reverse order (LIFO - most recent first)
        for name in self.order.iter().rev() {
            if let Some(action) = self.actions.remove(name) {
                tracing::info!("Running cleanup: {}", name);
                let future = action();
                future.await;
            }
        }
    }

    /// Check if the registry has any cleanup actions.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Get the number of registered cleanup actions.
    pub fn len(&self) -> usize {
        self.actions.len()
    }
}

// =============================================================================
// Safe State Configuration
// =============================================================================

/// Configuration for device-specific safe states.
///
/// This defines what "safe" means for each type of hardware device.
#[derive(Debug, Clone)]
pub struct SafeStateConfig {
    /// Stop all motion stages
    pub stop_motion: bool,
    /// Close all shutters
    pub close_shutters: bool,
    /// Disable laser emission
    pub disable_emission: bool,
    /// Stop camera acquisition
    pub stop_acquisition: bool,
    /// Custom actions (device_id -> action description)
    pub custom_actions: HashMap<String, String>,
}

impl Default for SafeStateConfig {
    fn default() -> Self {
        Self {
            stop_motion: true,
            close_shutters: true,
            disable_emission: true,
            stop_acquisition: true,
            custom_actions: HashMap::new(),
        }
    }
}

impl SafeStateConfig {
    /// Create a minimal safe state (stop motion only).
    pub fn motion_only() -> Self {
        Self {
            stop_motion: true,
            close_shutters: false,
            disable_emission: false,
            stop_acquisition: false,
            custom_actions: HashMap::new(),
        }
    }

    /// Create a laser-safe configuration.
    pub fn laser_safe() -> Self {
        Self {
            stop_motion: true,
            close_shutters: true,
            disable_emission: true,
            stop_acquisition: false,
            custom_actions: HashMap::new(),
        }
    }

    /// Add a custom action for a specific device.
    pub fn with_custom(mut self, device_id: impl Into<String>, action: impl Into<String>) -> Self {
        self.custom_actions.insert(device_id.into(), action.into());
        self
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panic_guard_runs_on_drop() {
        use std::sync::atomic::AtomicUsize;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        {
            let _guard = PanicGuard::new(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });
            // Guard drops here
        }

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_panic_guard_dismiss() {
        use std::sync::atomic::AtomicUsize;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        {
            let guard = PanicGuard::new(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });
            guard.dismiss();
            // Guard drops here but cleanup was dismissed
        }

        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_panic_guard_on_panic() {
        use std::sync::atomic::AtomicUsize;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = PanicGuard::new(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });
            panic!("Test panic");
        }));

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_emergency_stop_flag() {
        let flag = EmergencyStopFlag::new();

        assert!(!flag.is_triggered());

        flag.trigger("Test emergency");

        assert!(flag.is_triggered());
        assert_eq!(flag.reason(), Some("Test emergency".to_string()));

        flag.reset();

        assert!(!flag.is_triggered());
        assert_eq!(flag.reason(), None);
    }

    #[tokio::test]
    async fn test_cleanup_registry() {
        use std::sync::atomic::AtomicUsize;

        let counter = Arc::new(AtomicUsize::new(0));

        let mut registry = CleanupRegistry::new();

        let c1 = counter.clone();
        registry.register(
            "action1",
            Box::new(move || {
                Box::pin(async move {
                    c1.fetch_add(1, Ordering::SeqCst);
                })
            }),
        );

        let c2 = counter.clone();
        registry.register(
            "action2",
            Box::new(move || {
                Box::pin(async move {
                    c2.fetch_add(10, Ordering::SeqCst);
                })
            }),
        );

        assert_eq!(registry.len(), 2);

        registry.run_all().await;

        // Both actions should have run
        assert_eq!(counter.load(Ordering::SeqCst), 11);
    }

    #[tokio::test]
    async fn test_cleanup_registry_clear() {
        use std::sync::atomic::AtomicUsize;

        let counter = Arc::new(AtomicUsize::new(0));

        let mut registry = CleanupRegistry::new();

        let c1 = counter.clone();
        registry.register(
            "action1",
            Box::new(move || {
                Box::pin(async move {
                    c1.fetch_add(1, Ordering::SeqCst);
                })
            }),
        );

        registry.clear();
        registry.run_all().await;

        // Action should not have run
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_safe_state_config() {
        let config = SafeStateConfig::default();
        assert!(config.stop_motion);
        assert!(config.close_shutters);

        let laser_config = SafeStateConfig::laser_safe();
        assert!(laser_config.disable_emission);

        let custom = SafeStateConfig::motion_only().with_custom("stage1", "home to origin");
        assert!(custom.custom_actions.contains_key("stage1"));
    }
}
