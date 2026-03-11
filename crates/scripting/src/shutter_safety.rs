//! Shutter Safety Module for Laser Control
//!
//! This module provides enhanced safety mechanisms for laser shutter control
//! that go beyond simple Rust Drop semantics.
//!
//! # Problem Statement
//!
//! The basic `with_shutter_open()` function relies on Rust's control flow
//! to close the shutter after the callback completes. However, this does NOT
//! protect against:
//!
//! - **SIGKILL**: Cannot be intercepted (kill -9, OOM killer)
//! - **Power failure**: Immediate loss of control
//! - **Process hangs**: Infinite loops, deadlocks, hardware timeouts
//! - **Hardware crashes**: Host machine failure
//!
//! # Safety Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │                    DEFENSE IN DEPTH                        │
//! ├────────────────────────────────────────────────────────────┤
//! │ Layer 1: with_shutter_open()     - Rust control flow      │
//! │ Layer 2: HeartbeatShutterGuard   - Timeout-based closure  │
//! │ Layer 3: SIGTERM/SIGINT handlers - Graceful shutdown      │
//! │ Layer 4: ShutterRegistry         - Emergency close-all    │
//! │ Layer 5: Hardware interlock      - EXTERNAL (recommended) │
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Limitations
//!
//! **CRITICAL**: Software protections cannot protect against SIGKILL or power
//! failure. For production laser labs, **always use hardware interlocks** in
//! addition to software safety mechanisms.
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_scripting::shutter_safety::{HeartbeatShutterGuard, ShutterRegistry};
//!
//! // Register a global emergency shutdown handler
//! ShutterRegistry::install_signal_handlers();
//!
//! // Use heartbeat-based guard (closes if no heartbeat for 5s)
//! let guard = HeartbeatShutterGuard::new(shutter_driver, Duration::from_secs(5)).await?;
//!
//! // Script must call heartbeat() periodically
//! loop {
//!     guard.heartbeat();
//!     do_work();
//! }
//!
//! // Shutter auto-closes on drop OR if heartbeats stop
//! ```

use common::driver::Capability;
use hardware::capabilities::ShutterControl;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Default heartbeat timeout (5 seconds)
pub const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum allowed heartbeat timeout (60 seconds)
pub const MAX_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(60);

/// Pre-allocated emergency runtime for panic/signal handlers.
///
/// This runtime is created at startup to avoid allocation failures during emergencies.
/// If this runtime cannot be created, the process will fail-fast at initialization
/// rather than during an emergency shutdown.
static EMERGENCY_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Initialize the emergency runtime.
///
/// This must be called during application startup. If runtime creation fails,
/// the process will panic immediately (fail-fast) rather than during an emergency.
///
/// This is called automatically by `ShutterRegistry::global()`.
pub fn init_emergency_runtime() {
    EMERGENCY_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("CRITICAL: Failed to create emergency Tokio runtime during initialization")
    });
}

// =============================================================================
// Global Shutter Registry
// =============================================================================

/// Global registry of all currently open shutters for emergency shutdown.
///
/// This registry allows signal handlers and panic hooks to close all shutters
/// when the process is terminating unexpectedly.
static SHUTTER_REGISTRY: OnceLock<ShutterRegistry> = OnceLock::new();

/// Counter for generating unique guard IDs
static GUARD_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Registry of open shutters for emergency shutdown
pub struct ShutterRegistry {
    /// Map of guard ID to weak reference to shutter driver
    shutters: Mutex<HashMap<u64, Weak<dyn ShutterControl>>>,
    /// Flag indicating if signal handlers are installed
    handlers_installed: AtomicBool,
    /// Optional reference to hardware registry for emergency motor stop and DAQ zeroing
    /// This is set via install_panic_hook_with_hardware()
    hardware_registry: Mutex<Option<Weak<hardware::registry::DeviceRegistry>>>,
    /// Flag ensuring emergency close runs only once
    emergency_closed: AtomicBool,
}

impl ShutterRegistry {
    /// Get or create the global registry
    pub fn global() -> &'static ShutterRegistry {
        SHUTTER_REGISTRY.get_or_init(|| {
            // Initialize emergency runtime at startup (fail-fast if creation fails)
            init_emergency_runtime();

            ShutterRegistry {
                shutters: Mutex::new(HashMap::new()),
                handlers_installed: AtomicBool::new(false),
                hardware_registry: Mutex::new(None),
                emergency_closed: AtomicBool::new(false),
            }
        })
    }

    /// Register a shutter for emergency shutdown
    pub fn register(driver: &Arc<dyn ShutterControl>) -> u64 {
        let id = GUARD_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        let weak = Arc::downgrade(driver);

        if let Ok(mut shutters) = Self::global().shutters.lock() {
            shutters.insert(id, weak);
            info!(
                guard_id = id,
                total_shutters = shutters.len(),
                "Registered shutter for emergency shutdown"
            );
        }

        id
    }

    /// Unregister a shutter
    pub fn unregister(id: u64) {
        if let Ok(mut shutters) = Self::global().shutters.lock() {
            if shutters.remove(&id).is_some() {
                info!(
                    guard_id = id,
                    remaining = shutters.len(),
                    "Unregistered shutter from emergency registry"
                );
            }
        }
    }

    /// Get or create a tokio runtime handle for emergency operations
    ///
    /// First tries to use the current runtime, then falls back to the pre-allocated
    /// emergency runtime. Returns None only if the emergency runtime was never initialized.
    fn get_or_create_runtime() -> Option<Handle> {
        // Try to use existing runtime first
        if let Ok(handle) = Handle::try_current() {
            return Some(handle);
        }

        // Fallback: use pre-allocated emergency runtime
        if let Some(rt) = EMERGENCY_RUNTIME.get() {
            return Some(rt.handle().clone());
        }

        // This should never happen if init_emergency_runtime() was called at startup
        error!("Emergency runtime requested but not initialized");
        None
    }

    /// Emergency close all registered shutters
    ///
    /// This is called by signal handlers and panic hooks.
    /// It attempts to close all shutters but cannot guarantee success
    /// (e.g., if hardware is unresponsive).
    pub fn emergency_close_all() {
        // Idempotency check - ensure emergency close runs only once
        if Self::global().emergency_closed.swap(true, Ordering::SeqCst) {
            info!("Emergency close already executed, skipping");
            return;
        }

        warn!("EMERGENCY: Closing all registered shutters");

        let shutters: Vec<Arc<dyn ShutterControl>> = {
            match Self::global().shutters.try_lock() {
                Ok(guard) => guard.values().filter_map(|weak| weak.upgrade()).collect(),
                Err(_) => {
                    error!("Failed to acquire shutter registry lock during emergency close (deadlock risk)");
                    return;
                }
            }
        };

        if shutters.is_empty() {
            info!("No shutters registered for emergency close");
            return;
        }

        info!("Attempting to close {} registered shutters", shutters.len());

        // Try to get a runtime handle once (with fallback)
        let handle = Self::get_or_create_runtime();

        // Use single bridge thread pattern for all shutters
        if let Some(handle) = handle {
            let result = std::thread::spawn(move || {
                handle.block_on(async {
                    // Spawn parallel tasks for each shutter
                    let tasks: Vec<_> = shutters
                        .into_iter()
                        .enumerate()
                        .map(|(i, shutter)| {
                            tokio::spawn(async move {
                                match tokio::time::timeout(
                                    Duration::from_secs(2),
                                    shutter.close_shutter(),
                                )
                                .await
                                {
                                    Ok(Ok(())) => {
                                        info!(
                                            shutter_index = i,
                                            "Emergency shutter close: SUCCESS"
                                        );
                                        true
                                    }
                                    Ok(Err(e)) => {
                                        error!(
                                            shutter_index = i,
                                            error = %e,
                                            "Emergency shutter close: FAILED"
                                        );
                                        false
                                    }
                                    Err(_) => {
                                        error!(
                                            shutter_index = i,
                                            "Emergency shutter close: TIMEOUT (2s)"
                                        );
                                        false
                                    }
                                }
                            })
                        })
                        .collect();

                    // Wait for all tasks to complete
                    for (i, task) in tasks.into_iter().enumerate() {
                        if let Err(e) = task.await {
                            error!(shutter_index = i, error = ?e, "Emergency close task panicked");
                        }
                    }
                })
            })
            .join();

            if let Err(e) = result {
                error!(error = ?e, "Emergency close bridge thread panicked");
            }
        } else {
            error!(
                "No tokio runtime available for emergency shutter close (runtime creation failed)"
            );
        }
    }

    /// Emergency close ALL ShutterControl devices from the hardware registry.
    ///
    /// Unlike `emergency_close_all()` which only closes shutters registered via
    /// `HeartbeatShutterGuard`, this method queries the `DeviceRegistry` for ALL
    /// devices with `ShutterControl` capability and closes them. This covers
    /// shutters opened via gRPC/direct API that were never registered with the
    /// shutter guard system.
    ///
    /// Best-effort - does not panic if shutdown fails.
    pub fn emergency_close_all_shutters_from_registry() {
        warn!("EMERGENCY: Closing all ShutterControl devices from registry");

        let registry: Option<Arc<hardware::registry::DeviceRegistry>> = {
            match Self::global().hardware_registry.try_lock() {
                Ok(guard) => guard.as_ref().and_then(|weak| weak.upgrade()),
                Err(_) => {
                    error!("Failed to acquire hardware registry lock during emergency shutter close (deadlock risk)");
                    return;
                }
            }
        };

        let Some(registry) = registry else {
            info!("No hardware registry available for emergency shutter close");
            return;
        };

        let shutter_device_ids = registry.devices_with_capability(Capability::ShutterControl);

        if shutter_device_ids.is_empty() {
            info!("No ShutterControl devices registered for emergency close");
            return;
        }

        info!(
            "Attempting to close {} ShutterControl devices from registry",
            shutter_device_ids.len()
        );

        // Try to get a runtime handle once (with fallback)
        let handle = Self::get_or_create_runtime();

        for (i, device_id) in shutter_device_ids.iter().enumerate() {
            if let Some(shutter) = registry.get_shutter_control(device_id) {
                if let Some(ref handle) = handle {
                    let shutter = shutter.clone();
                    let id_for_thread = device_id.clone();
                    let handle = handle.clone();
                    let result = std::thread::spawn(move || {
                        handle.block_on(async {
                            match tokio::time::timeout(
                                Duration::from_secs(2),
                                shutter.close_shutter(),
                            )
                            .await
                            {
                                Ok(Ok(())) => {
                                    info!(device_id = %id_for_thread, shutter_index = i, "Emergency registry shutter close: SUCCESS");
                                    true
                                }
                                Ok(Err(e)) => {
                                    error!(
                                        device_id = %id_for_thread,
                                        shutter_index = i,
                                        error = %e,
                                        "Emergency registry shutter close: FAILED"
                                    );
                                    false
                                }
                                Err(_) => {
                                    error!(device_id = %id_for_thread, shutter_index = i, "Emergency registry shutter close: TIMEOUT (2s)");
                                    false
                                }
                            }
                        })
                    })
                    .join();

                    if let Err(e) = result {
                        error!(device_id = %device_id, shutter_index = i, error = ?e, "Emergency registry shutter close thread panicked");
                    }
                } else {
                    error!(
                        device_id = %device_id,
                        shutter_index = i,
                        "No tokio runtime available for emergency registry shutter close (runtime creation failed)"
                    );
                }
            }
        }
    }

    /// Emergency disable ALL EmissionControl devices from the hardware registry.
    ///
    /// Queries the `DeviceRegistry` for all devices with `EmissionControl` capability
    /// and calls `disable_emission()` on each. This turns off laser sources to prevent
    /// uncontrolled beam exposure.
    ///
    /// Best-effort - does not panic if shutdown fails.
    pub fn emergency_disable_all_emission() {
        warn!("EMERGENCY: Disabling all EmissionControl devices");

        let registry: Option<Arc<hardware::registry::DeviceRegistry>> = {
            match Self::global().hardware_registry.try_lock() {
                Ok(guard) => guard.as_ref().and_then(|weak| weak.upgrade()),
                Err(_) => {
                    error!("Failed to acquire hardware registry lock during emergency emission disable (deadlock risk)");
                    return;
                }
            }
        };

        let Some(registry) = registry else {
            info!("No hardware registry available for emergency emission disable");
            return;
        };

        let emission_device_ids = registry.devices_with_capability(Capability::EmissionControl);

        if emission_device_ids.is_empty() {
            info!("No EmissionControl devices registered for emergency disable");
            return;
        }

        info!(
            "Attempting to disable {} EmissionControl devices",
            emission_device_ids.len()
        );

        // Try to get a runtime handle once (with fallback)
        let handle = Self::get_or_create_runtime();

        for (i, device_id) in emission_device_ids.iter().enumerate() {
            if let Some(emission) = registry.get_emission_control(device_id) {
                if let Some(ref handle) = handle {
                    let emission = emission.clone();
                    let id_for_thread = device_id.clone();
                    let handle = handle.clone();
                    let result = std::thread::spawn(move || {
                        handle.block_on(async {
                            match tokio::time::timeout(
                                Duration::from_secs(2),
                                emission.disable_emission(),
                            )
                            .await
                            {
                                Ok(Ok(())) => {
                                    info!(device_id = %id_for_thread, emission_index = i, "Emergency emission disable: SUCCESS");
                                    true
                                }
                                Ok(Err(e)) => {
                                    error!(
                                        device_id = %id_for_thread,
                                        emission_index = i,
                                        error = %e,
                                        "Emergency emission disable: FAILED"
                                    );
                                    false
                                }
                                Err(_) => {
                                    error!(device_id = %id_for_thread, emission_index = i, "Emergency emission disable: TIMEOUT (2s)");
                                    false
                                }
                            }
                        })
                    })
                    .join();

                    if let Err(e) = result {
                        error!(device_id = %device_id, emission_index = i, error = ?e, "Emergency emission disable thread panicked");
                    }
                } else {
                    error!(
                        device_id = %device_id,
                        emission_index = i,
                        "No tokio runtime available for emergency emission disable (runtime creation failed)"
                    );
                }
            }
        }
    }

    /// Emergency stop all motors (Movable devices)
    ///
    /// This is called by panic hooks to halt all motion.
    /// Best-effort - does not panic if shutdown fails.
    pub fn emergency_stop_motors() {
        warn!("EMERGENCY: Stopping all motors");

        let registry: Option<Arc<hardware::registry::DeviceRegistry>> = {
            match Self::global().hardware_registry.try_lock() {
                Ok(guard) => guard.as_ref().and_then(|weak| weak.upgrade()),
                Err(_) => {
                    error!("Failed to acquire hardware registry lock during emergency stop (deadlock risk)");
                    return;
                }
            }
        };

        let Some(registry) = registry else {
            info!("No hardware registry available for emergency motor stop");
            return;
        };

        let devices = registry.list_devices();
        let movable_devices: Vec<_> = devices
            .iter()
            .filter(|d| d.capabilities.contains(&Capability::Movable))
            .collect();

        if movable_devices.is_empty() {
            info!("No movable devices registered for emergency stop");
            return;
        }

        info!("Attempting to stop {} motors", movable_devices.len());

        // Try to get a runtime handle once (with fallback)
        let handle = Self::get_or_create_runtime();

        for (i, device_info) in movable_devices.iter().enumerate() {
            if let Some(motor) = registry.get_movable(&device_info.id) {
                if let Some(ref handle) = handle {
                    let motor = motor.clone();
                    let device_id = device_info.id.clone();
                    let handle = handle.clone();
                    let result = std::thread::spawn(move || {
                        handle.block_on(async {
                            match tokio::time::timeout(Duration::from_secs(2), motor.stop()).await
                            {
                                Ok(Ok(())) => {
                                    info!(device_id = %device_id, motor_index = i, "Emergency motor stop: SUCCESS");
                                    true
                                }
                                Ok(Err(e)) => {
                                    error!(
                                        device_id = %device_id,
                                        motor_index = i,
                                        error = %e,
                                        "Emergency motor stop: FAILED"
                                    );
                                    false
                                }
                                Err(_) => {
                                    error!(device_id = %device_id, motor_index = i, "Emergency motor stop: TIMEOUT (2s)");
                                    false
                                }
                            }
                        })
                    })
                    .join();

                    if let Err(e) = result {
                        error!(device_id = %device_info.id, motor_index = i, error = ?e, "Emergency stop thread panicked");
                    }
                } else {
                    error!(
                        device_id = %device_info.id,
                        motor_index = i,
                        "No tokio runtime available for emergency motor stop (runtime creation failed)"
                    );
                }
            }
        }
    }

    /// Emergency zero all DAQ analog outputs (Settable devices)
    ///
    /// This is called by panic hooks to set critical outputs (e.g., EOM control) to zero.
    /// Best-effort - does not panic if shutdown fails.
    pub fn emergency_zero_outputs() {
        warn!("EMERGENCY: Zeroing DAQ analog outputs");

        let registry: Option<Arc<hardware::registry::DeviceRegistry>> = {
            match Self::global().hardware_registry.try_lock() {
                Ok(guard) => guard.as_ref().and_then(|weak| weak.upgrade()),
                Err(_) => {
                    error!("Failed to acquire hardware registry lock during emergency zero (deadlock risk)");
                    return;
                }
            }
        };

        let Some(registry) = registry else {
            info!("No hardware registry available for emergency DAQ zeroing");
            return;
        };

        let devices = registry.list_devices();
        let settable_devices: Vec<_> = devices
            .iter()
            .filter(|d| d.capabilities.contains(&Capability::Settable))
            .collect();

        if settable_devices.is_empty() {
            info!("No settable devices registered for emergency zero");
            return;
        }

        info!("Attempting to zero {} DAQ outputs", settable_devices.len());

        // Try to get a runtime handle once (with fallback)
        let handle = Self::get_or_create_runtime();

        for (i, device_info) in settable_devices.iter().enumerate() {
            if let Some(output) = registry.get_settable(&device_info.id) {
                if let Some(ref handle) = handle {
                    let output = output.clone();
                    let device_id = device_info.id.clone();
                    let handle = handle.clone();
                    let result = std::thread::spawn(move || {
                        handle.block_on(async {
                            // Try to set "value" parameter to 0.0 (common convention for DAQ outputs)
                            match tokio::time::timeout(
                                Duration::from_secs(2),
                                output.set_value("value", ::serde_json::json!(0.0)),
                            )
                            .await
                            {
                                Ok(Ok(())) => {
                                    info!(device_id = %device_id, output_index = i, "Emergency DAQ zero: SUCCESS");
                                    true
                                }
                                Ok(Err(e)) => {
                                    error!(
                                        device_id = %device_id,
                                        output_index = i,
                                        error = %e,
                                        "Emergency DAQ zero: FAILED"
                                    );
                                    false
                                }
                                Err(_) => {
                                    error!(device_id = %device_id, output_index = i, "Emergency DAQ zero: TIMEOUT (2s)");
                                    false
                                }
                            }
                        })
                    })
                    .join();

                    if let Err(e) = result {
                        error!(device_id = %device_info.id, output_index = i, error = ?e, "Emergency zero thread panicked");
                    }
                } else {
                    error!(
                        device_id = %device_info.id,
                        output_index = i,
                        "No tokio runtime available for emergency DAQ zero (runtime creation failed)"
                    );
                }
            }
        }
    }

    /// Install signal handlers for graceful shutdown
    ///
    /// This installs handlers for SIGTERM and SIGINT that will attempt
    /// to close all shutters before the process exits.
    ///
    /// # Platform Support
    /// - Unix: SIGTERM, SIGINT
    /// - Windows: Ctrl+C, Ctrl+Break
    #[cfg(unix)]
    pub fn install_signal_handlers() {
        use std::sync::atomic::Ordering;

        let registry = Self::global();
        if registry
            .handlers_installed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            info!("Signal handlers already installed");
            return;
        }

        info!("Installing shutter safety signal handlers");

        // Spawn a task to handle signals
        std::thread::spawn(|| {
            use tokio::signal::unix::{signal, SignalKind};

            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to create signal handler runtime: {}", e);
                    return;
                }
            };

            rt.block_on(async {
                let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
                let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT handler");

                tokio::select! {
                    _ = sigterm.recv() => {
                        warn!("Received SIGTERM - closing all shutters");
                        Self::emergency_close_all();
                    }
                    _ = sigint.recv() => {
                        warn!("Received SIGINT - closing all shutters");
                        Self::emergency_close_all();
                    }
                }
            });
        });
    }

    /// Install signal handlers (Windows version)
    #[cfg(windows)]
    pub fn install_signal_handlers() {
        let registry = Self::global();
        if registry
            .handlers_installed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            info!("Signal handlers already installed");
            return;
        }

        info!("Installing shutter safety signal handlers (Windows)");

        std::thread::spawn(|| {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to create signal handler runtime: {}", e);
                    return;
                }
            };

            rt.block_on(async {
                let mut ctrl_c = tokio::signal::ctrl_c();

                ctrl_c.await.expect("Ctrl+C handler");
                warn!("Received Ctrl+C - closing all shutters");
                Self::emergency_close_all();
            });
        });
    }

    /// Install a panic hook that closes all shutters
    ///
    /// DEPRECATED: Use `install_panic_hook_with_hardware()` instead for full hardware safety.
    ///
    /// This should be called once during application startup.
    pub fn install_panic_hook() {
        let default_hook = std::panic::take_hook();

        std::panic::set_hook(Box::new(move |info| {
            error!("PANIC detected - attempting emergency shutter close");
            Self::emergency_close_all();

            // Call the default hook to print the panic message
            default_hook(info);
        }));

        info!("Installed panic hook for shutter safety (shutters only)");
    }

    /// Install a panic hook with full hardware emergency shutdown
    ///
    /// This registers a panic hook that will:
    /// 1. Close all scripting-registered laser shutters
    /// 2. Close ALL ShutterControl devices from the hardware registry
    /// 3. Disable ALL EmissionControl devices (laser sources)
    /// 4. Stop all motors (Movable devices)
    /// 5. Zero critical DAQ outputs (Settable devices)
    ///
    /// # Arguments
    /// * `registry` - Reference to the hardware device registry
    ///
    /// # Safety
    /// This is best-effort emergency shutdown. Cannot protect against:
    /// - SIGKILL (kill -9)
    /// - Power failure
    /// - Hardware crashes
    ///
    /// Always use hardware interlocks for production laser labs.
    ///
    /// # Example
    /// ```rust,ignore
    /// use scripting::shutter_safety::ShutterRegistry;
    /// use hardware::registry::DeviceRegistry;
    /// use std::sync::Arc;
    ///
    /// let registry = Arc::new(DeviceRegistry::new());
    /// // ... register devices ...
    ///
    /// ShutterRegistry::install_panic_hook_with_hardware(&registry);
    /// ```
    pub fn install_panic_hook_with_hardware(registry: &Arc<hardware::registry::DeviceRegistry>) {
        // Store weak reference to hardware registry
        if let Ok(mut hw_guard) = Self::global().hardware_registry.lock() {
            *hw_guard = Some(Arc::downgrade(registry));
        } else {
            error!("Failed to store hardware registry reference for panic hook");
            return;
        }

        let default_hook = std::panic::take_hook();

        std::panic::set_hook(Box::new(move |info| {
            error!("PANIC detected - attempting emergency hardware shutdown");

            // Best-effort hardware shutdown sequence (don't panic in panic handler)
            // Order: close shutters first (block beam), then disable emission,
            // then stop motors, then zero DAQ outputs
            Self::emergency_close_all();
            Self::emergency_close_all_shutters_from_registry();
            Self::emergency_disable_all_emission();
            Self::emergency_stop_motors();
            Self::emergency_zero_outputs();

            // Call the default hook to print the panic message
            default_hook(info);
        }));

        info!("Installed panic hook for full hardware safety (shutters + emission + motors + DAQ)");
    }
}

// =============================================================================
// Heartbeat-Based Shutter Guard
// =============================================================================

/// A shutter guard that requires periodic heartbeats to keep the shutter open.
///
/// If no heartbeat is received within the timeout period, the shutter is
/// automatically closed. This protects against script hangs and deadlocks.
///
/// # Example
///
/// ```rust,ignore
/// let guard = HeartbeatShutterGuard::new(driver, Duration::from_secs(5)).await?;
///
/// for i in 0..100 {
///     guard.heartbeat(); // Must call periodically!
///     expensive_operation();
/// }
///
/// // Guard closes shutter on drop
/// ```
pub struct HeartbeatShutterGuard {
    /// Unique ID for this guard
    id: u64,
    /// Shutter driver
    driver: Arc<dyn ShutterControl>,
    /// Channel to send heartbeats
    heartbeat_tx: mpsc::Sender<()>,
    /// Watchdog task handle
    _watchdog_handle: JoinHandle<()>,
    /// Flag indicating if shutter was opened successfully
    is_open: AtomicBool,
}

impl HeartbeatShutterGuard {
    /// Create a new heartbeat-based shutter guard.
    ///
    /// Opens the shutter immediately and starts a watchdog task that will
    /// close the shutter if no heartbeat is received within the timeout.
    ///
    /// # Arguments
    ///
    /// * `driver` - The shutter control driver
    /// * `timeout` - Maximum time between heartbeats before auto-close
    ///
    /// # Errors
    ///
    /// Returns an error if the shutter cannot be opened.
    pub async fn new(driver: Arc<dyn ShutterControl>, timeout: Duration) -> anyhow::Result<Self> {
        // Clamp timeout to reasonable bounds
        let timeout = timeout.clamp(Duration::from_millis(500), MAX_HEARTBEAT_TIMEOUT);

        // Open the shutter
        driver.open_shutter().await?;
        info!(
            timeout_secs = timeout.as_secs_f32(),
            "HeartbeatShutterGuard: Shutter opened with heartbeat watchdog"
        );

        // Register with global registry
        let id = ShutterRegistry::register(&driver);

        // Create heartbeat channel
        let (heartbeat_tx, mut heartbeat_rx) = mpsc::channel::<()>(1);

        // Spawn watchdog task
        let watchdog_driver = driver.clone();
        let watchdog_handle = tokio::spawn(async move {
            let mut last_heartbeat = Instant::now();

            loop {
                tokio::select! {
                    // Wait for heartbeat or timeout
                    result = tokio::time::timeout(timeout, heartbeat_rx.recv()) => {
                        match result {
                            Ok(Some(())) => {
                                // Heartbeat received
                                last_heartbeat = Instant::now();
                            }
                            Ok(None) => {
                                // Channel closed (guard dropped)
                                info!("HeartbeatShutterGuard: Channel closed, watchdog exiting");
                                return;
                            }
                            Err(_) => {
                                // Timeout! No heartbeat received
                                let elapsed = last_heartbeat.elapsed();
                                error!(
                                    elapsed_secs = elapsed.as_secs_f32(),
                                    timeout_secs = timeout.as_secs_f32(),
                                    "HeartbeatShutterGuard: TIMEOUT - no heartbeat! Closing shutter"
                                );

                                // Attempt to close shutter
                                match watchdog_driver.close_shutter().await {
                                    Ok(()) => {
                                        warn!("HeartbeatShutterGuard: Shutter closed due to timeout");
                                    }
                                    Err(e) => {
                                        error!(
                                            error = %e,
                                            "HeartbeatShutterGuard: CRITICAL - Failed to close shutter on timeout!"
                                        );
                                    }
                                }
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            id,
            driver,
            heartbeat_tx,
            _watchdog_handle: watchdog_handle,
            is_open: AtomicBool::new(true),
        })
    }

    /// Send a heartbeat to keep the shutter open.
    ///
    /// This must be called periodically (more frequently than the timeout)
    /// to prevent the watchdog from closing the shutter.
    ///
    /// Returns `true` if the heartbeat was sent successfully.
    pub fn heartbeat(&self) -> bool {
        if !self.is_open.load(Ordering::SeqCst) {
            return false;
        }

        match self.heartbeat_tx.try_send(()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Channel is full, but that's okay - a heartbeat is pending
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("HeartbeatShutterGuard: Heartbeat channel closed");
                false
            }
        }
    }

    /// Check if the shutter is still open.
    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::SeqCst)
    }

    /// Get the guard ID (for debugging/logging)
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Manually close the shutter and mark the guard as closed.
    pub async fn close(&self) -> anyhow::Result<()> {
        if self
            .is_open
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.driver.close_shutter().await?;
            info!(guard_id = self.id, "HeartbeatShutterGuard: Shutter closed");
        }
        Ok(())
    }
}

impl Drop for HeartbeatShutterGuard {
    fn drop(&mut self) {
        // Mark as closed to stop watchdog from trying to close again
        let was_open = self.is_open.swap(false, Ordering::SeqCst);

        // Unregister from global registry
        ShutterRegistry::unregister(self.id);

        if was_open {
            // Attempt to close shutter on drop
            // This is best-effort since we can't do async in Drop
            if let Ok(handle) = Handle::try_current() {
                let driver = self.driver.clone();
                let id = self.id;

                // Use spawn_blocking to close the shutter
                // This is fire-and-forget since we're in Drop
                handle.spawn(async move {
                    match tokio::time::timeout(Duration::from_secs(2), driver.close_shutter()).await
                    {
                        Ok(Ok(())) => {
                            info!(guard_id = id, "HeartbeatShutterGuard drop: Shutter closed");
                        }
                        Ok(Err(e)) => {
                            error!(
                                guard_id = id,
                                error = %e,
                                "HeartbeatShutterGuard drop: Failed to close shutter"
                            );
                        }
                        Err(_) => {
                            error!(
                                guard_id = id,
                                "HeartbeatShutterGuard drop: Timeout closing shutter"
                            );
                        }
                    }
                });
            } else {
                warn!(
                    guard_id = self.id,
                    "HeartbeatShutterGuard drop: No runtime available, cannot close shutter!"
                );
            }
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicBool;
    use tokio::time::sleep;

    /// Mock shutter for testing
    struct MockShutter {
        is_open: AtomicBool,
        close_count: std::sync::atomic::AtomicU32,
    }

    impl MockShutter {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                is_open: AtomicBool::new(false),
                close_count: std::sync::atomic::AtomicU32::new(0),
            })
        }

        fn close_count(&self) -> u32 {
            self.close_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ShutterControl for MockShutter {
        async fn open_shutter(&self) -> anyhow::Result<()> {
            self.is_open.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn close_shutter(&self) -> anyhow::Result<()> {
            self.is_open.store(false, Ordering::SeqCst);
            self.close_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn is_shutter_open(&self) -> anyhow::Result<bool> {
            Ok(self.is_open.load(Ordering::SeqCst))
        }
    }

    #[tokio::test]
    async fn test_heartbeat_guard_opens_shutter() {
        let mock = MockShutter::new();
        let guard = HeartbeatShutterGuard::new(mock.clone(), Duration::from_secs(5))
            .await
            .unwrap();

        assert!(mock.is_open.load(Ordering::SeqCst));
        assert!(guard.is_open());

        drop(guard);
        // Give the drop task time to complete
        sleep(Duration::from_millis(100)).await;
        assert!(!mock.is_open.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_heartbeat_prevents_timeout() {
        let mock = MockShutter::new();
        let guard = HeartbeatShutterGuard::new(mock.clone(), Duration::from_millis(100))
            .await
            .unwrap();

        // Send heartbeats faster than timeout
        for _ in 0..5 {
            assert!(guard.heartbeat());
            sleep(Duration::from_millis(50)).await;
        }

        // Shutter should still be open
        assert!(mock.is_open.load(Ordering::SeqCst));
        assert_eq!(mock.close_count(), 0);

        drop(guard);
    }

    #[tokio::test]
    async fn test_timeout_closes_shutter() {
        let mock = MockShutter::new();
        // Use 600ms timeout (min is 500ms due to clamping)
        let guard = HeartbeatShutterGuard::new(mock.clone(), Duration::from_millis(600))
            .await
            .unwrap();

        // Don't send any heartbeats - wait for timeout plus buffer
        // The watchdog needs time to detect timeout AND close the shutter
        // 600ms timeout + 200ms buffer = 800ms wait
        sleep(Duration::from_millis(900)).await;

        // Watchdog should have closed the shutter
        assert!(!mock.is_open.load(Ordering::SeqCst));
        assert!(mock.close_count() >= 1);

        // Guard's is_open should also reflect the closed state
        // (though it may still think it's open since it didn't initiate the close)
        drop(guard);
    }

    #[test]
    fn test_registry_register_unregister() {
        let mock = MockShutter::new();
        let id = ShutterRegistry::register(&(mock.clone() as Arc<dyn ShutterControl>));

        assert!(id > 0);

        ShutterRegistry::unregister(id);
        // Should not panic on double unregister
        ShutterRegistry::unregister(id);
    }

    /// Helper to create a DeviceRegistry with mock laser devices for testing
    /// emergency shutdown methods.
    async fn create_registry_with_lasers(count: usize) -> Arc<hardware::registry::DeviceRegistry> {
        use driver_mock::MockLaserFactory;
        use hardware::registry::DeviceRegistry;

        let registry = DeviceRegistry::new();
        registry.register_factory(Box::new(MockLaserFactory));

        for i in 0..count {
            registry
                .register_from_toml(
                    &format!("laser_{i}"),
                    &format!("Mock Laser {i}"),
                    "mock_laser",
                    toml::Value::Table(Default::default()),
                )
                .await
                .expect("Failed to register mock laser");
        }

        Arc::new(registry)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_emergency_close_all_shutters_from_registry() {
        use common::driver::Capability;

        let registry = create_registry_with_lasers(2).await;

        // Store registry in ShutterRegistry so emergency methods can find it
        if let Ok(mut hw_guard) = ShutterRegistry::global().hardware_registry.lock() {
            *hw_guard = Some(Arc::downgrade(&registry));
        }

        // Open shutters on both lasers
        for id in registry.devices_with_capability(Capability::ShutterControl) {
            let shutter = registry.get_shutter_control(&id).expect("shutter exists");
            shutter.open_shutter().await.expect("open shutter");
            assert!(shutter.is_shutter_open().await.expect("query shutter"));
        }

        // Emergency close all from registry
        ShutterRegistry::emergency_close_all_shutters_from_registry();

        // Verify all shutters are closed
        for id in registry.devices_with_capability(Capability::ShutterControl) {
            let shutter = registry.get_shutter_control(&id).expect("shutter exists");
            assert!(
                !shutter.is_shutter_open().await.expect("query shutter"),
                "Shutter {id} should be closed after emergency close"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_emergency_disable_all_emission() {
        use common::driver::Capability;

        let registry = create_registry_with_lasers(2).await;

        // Store registry in ShutterRegistry
        if let Ok(mut hw_guard) = ShutterRegistry::global().hardware_registry.lock() {
            *hw_guard = Some(Arc::downgrade(&registry));
        }

        // Enable emission on both lasers
        for id in registry.devices_with_capability(Capability::EmissionControl) {
            let emission = registry.get_emission_control(&id).expect("emission exists");
            emission.enable_emission().await.expect("enable emission");
            assert!(emission
                .is_emission_enabled()
                .await
                .expect("query emission"));
        }

        // Emergency disable all
        ShutterRegistry::emergency_disable_all_emission();

        // Verify all emission is disabled
        for id in registry.devices_with_capability(Capability::EmissionControl) {
            let emission = registry.get_emission_control(&id).expect("emission exists");
            assert!(
                !emission
                    .is_emission_enabled()
                    .await
                    .expect("query emission"),
                "Emission on {id} should be disabled after emergency disable"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_emergency_methods_no_registry_does_not_panic() {
        // Clear registry reference to simulate no hardware
        if let Ok(mut hw_guard) = ShutterRegistry::global().hardware_registry.lock() {
            *hw_guard = None;
        }

        // These should return early without panicking
        ShutterRegistry::emergency_close_all_shutters_from_registry();
        ShutterRegistry::emergency_disable_all_emission();
        ShutterRegistry::emergency_stop_motors();
        ShutterRegistry::emergency_zero_outputs();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_emergency_methods_empty_registry_does_not_panic() {
        use hardware::registry::DeviceRegistry;

        let registry = Arc::new(DeviceRegistry::new());
        if let Ok(mut hw_guard) = ShutterRegistry::global().hardware_registry.lock() {
            *hw_guard = Some(Arc::downgrade(&registry));
        }

        // No devices registered — should return early without panicking
        ShutterRegistry::emergency_close_all_shutters_from_registry();
        ShutterRegistry::emergency_disable_all_emission();
        ShutterRegistry::emergency_stop_motors();
        ShutterRegistry::emergency_zero_outputs();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_emergency_shutter_close_idempotent() {
        let registry = create_registry_with_lasers(1).await;
        if let Ok(mut hw_guard) = ShutterRegistry::global().hardware_registry.lock() {
            *hw_guard = Some(Arc::downgrade(&registry));
        }

        // Open shutter
        let shutter = registry
            .get_shutter_control("laser_0")
            .expect("shutter exists");
        shutter.open_shutter().await.expect("open");

        // Close twice — should not panic
        ShutterRegistry::emergency_close_all_shutters_from_registry();
        ShutterRegistry::emergency_close_all_shutters_from_registry();

        assert!(!shutter.is_shutter_open().await.expect("query"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_emergency_emission_disable_idempotent() {
        let registry = create_registry_with_lasers(1).await;
        if let Ok(mut hw_guard) = ShutterRegistry::global().hardware_registry.lock() {
            *hw_guard = Some(Arc::downgrade(&registry));
        }

        // Enable emission
        let emission = registry
            .get_emission_control("laser_0")
            .expect("emission exists");
        emission.enable_emission().await.expect("enable");

        // Disable twice — should not panic
        ShutterRegistry::emergency_disable_all_emission();
        ShutterRegistry::emergency_disable_all_emission();

        assert!(!emission.is_emission_enabled().await.expect("query"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_one_failure_does_not_prevent_others() {
        // Create registry with multiple lasers
        let registry = create_registry_with_lasers(3).await;
        if let Ok(mut hw_guard) = ShutterRegistry::global().hardware_registry.lock() {
            *hw_guard = Some(Arc::downgrade(&registry));
        }

        // Open all shutters
        for id in registry.devices_with_capability(Capability::ShutterControl) {
            let shutter = registry.get_shutter_control(&id).expect("shutter exists");
            shutter.open_shutter().await.expect("open shutter");
        }

        // Emergency close — even if mock devices don't fail, this exercises the
        // iteration-continues-on-failure code path (all devices get visited)
        ShutterRegistry::emergency_close_all_shutters_from_registry();

        // All shutters should be closed
        for id in registry.devices_with_capability(Capability::ShutterControl) {
            let shutter = registry.get_shutter_control(&id).expect("shutter exists");
            assert!(
                !shutter.is_shutter_open().await.expect("query"),
                "Shutter {id} should be closed"
            );
        }
    }
}
