# ADR: Panic Safety Architecture

**Status:** Accepted
**Date:** 2026-02-02
**Author:** Architecture Review
**Related Issues:** bd-vmfp.2

---

> **Naming note (2026-04-22):** This ADR uses `SafetyHeartbeat` as if it were a Rust type. The current implementation is a module + task — `crates/bin/src/safety_heartbeat_task.rs` exposes `spawn_heartbeat()`, driven by a `HeartbeatConfig` struct in `crates/hardware/src/registry/types.rs` (configured via the `[safety_heartbeat]` stanza in the hardware config TOML). There is no `struct SafetyHeartbeat`. The design intent below still stands; treat capitalized `SafetyHeartbeat` references in this ADR as shorthand for that task + config pair.

---

## Context

The Rhai scripting engine allows untrusted code execution with access to critical hardware: lasers, motion controllers, and DAQ equipment. Process panics in Rhai scripts could leave hardware in dangerous states (shutter open, motors moving, DAQ outputs active).

This document defines the panic safety architecture that gracefully shuts down all critical hardware even when the Tokio runtime is unavailable or the process is terminating unexpectedly.

---

## Decision

**Defense-in-depth panic safety with guaranteed hardware shutdown, even without async runtime.**

The safety mechanism uses seven coordinated layers:
1. Rust control flow (basic safety)
2. Heartbeat watchdog (hang detection)
3. Signal handlers (graceful shutdown on SIGTERM/SIGINT)
4. Pre-allocated emergency runtime (async in panic context)
5. Panic hook with hardware shutdown sequence
6. Hardware interlocks (external, recommended)
7. SafetyHeartbeat (proactive DIO pulse for external interlock)

---

## Safety Layers

### Layer 1: with_shutter_open() - Rust Control Flow

The basic safety wrapper uses RAII to guarantee shutter closure:

```rust
pub async fn with_shutter_open<F, T>(shutter: Arc<dyn ShutterControl>, fut: F) -> Result<T>
where
    F: FnOnce() -> Pin<Box<dyn Future<Output = Result<T>>>>,
{
    shutter.open_shutter().await?;
    let result = fut().await;
    shutter.close_shutter().await?;  // Always runs (unless panic)
    Ok(result)
}
```

**Coverage:** Handles normal execution paths, early returns, and `Err` propagation.

**Limitations:** Does NOT protect against process panics.

---

### Layer 2: HeartbeatShutterGuard - Timeout-Based Closure

For long-running operations, a watchdog task monitors heartbeat signals:

```rust
pub struct HeartbeatShutterGuard {
    id: u64,
    driver: Arc<dyn ShutterControl>,
    heartbeat_tx: mpsc::Sender<()>,
    _watchdog_handle: JoinHandle<()>,
    is_open: AtomicBool,
}

impl HeartbeatShutterGuard {
    pub async fn new(
        driver: Arc<dyn ShutterControl>,
        timeout: Duration,
    ) -> anyhow::Result<Self> { /* ... */ }

    pub fn heartbeat(&self) -> bool {
        self.heartbeat_tx.try_send(()).is_ok()
    }
}
```

**Usage:** Script must call `guard.heartbeat()` periodically:

```rhai
let guard = create_heartbeat_guard(laser, 5.0);  // 5-second timeout
for i in 0..1000 {
    guard.heartbeat();
    do_expensive_work();
}
// Guard closes shutter on drop
```

**Coverage:** Detects infinite loops, deadlocks, and hardware timeouts while the Tokio runtime is still running.

**Limitations:** Does NOT protect against panic that kills the watchdog task.

---

### Layer 3: Signal Handlers - Graceful Shutdown

SIGTERM and SIGINT are intercepted to close all shutters before process exit:

> [!NOTE] **EXECUTABLE EXAMPLE** (Source: `crates/bin/src/panic_handler.rs`)

```rust
#[cfg(unix)]
pub fn install_signal_handlers() {
    std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        rt.block_on(async {
            let mut sigterm = signal(SignalKind::terminate())?;
            let mut sigint = signal(SignalKind::interrupt())?;

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
```

**Coverage:** Handles user interrupts (Ctrl+C) and process termination signals.

**Limitations:** Does NOT protect against SIGKILL (kill -9) or panic.

---

### Layer 4: Pre-Allocated Emergency Runtime

To enable async operations during panic (when the current runtime may not be available), an emergency runtime is pre-allocated at startup:

> [!NOTE] **EXECUTABLE EXAMPLE** (Source: `crates/bin/src/panic_handler.rs`)

```rust
/// Pre-allocated emergency runtime for panic/signal handlers.
///
/// Allocated at startup to avoid allocation failures during emergencies.
/// Panic in emergency shutdown code would be fatal, so we fail-fast
/// at initialization if this cannot be created.
static EMERGENCY_RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn init_emergency_runtime() {
    EMERGENCY_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("CRITICAL: Failed to create emergency Tokio runtime during initialization")
    });
}
```

**Initialization:** Called automatically by `ShutterRegistry::global()` on first access.

**Design Rationale:**
- Creating a runtime during panic is unsafe (allocations may fail, leading to double panic)
- Pre-allocation at startup fails fast if insufficient memory is available
- A single-threaded runtime is sufficient for sequential hardware shutdown

**Fallback Chain:** When emergency shutdown is needed, the code tries:

```rust
fn get_or_create_runtime() -> Option<Handle> {
    // Try to use existing runtime first
    if let Ok(handle) = Handle::try_current() {
        return Some(handle);
    }

    // Fallback: use pre-allocated emergency runtime
    if let Some(rt) = EMERGENCY_RUNTIME.get() {
        return Some(rt.handle().clone());
    }

    // Never reached if init_emergency_runtime() was called
    error!("Emergency runtime requested but not initialized");
    None
}
```

---

### Layer 5: Panic Hook with Hardware Shutdown Sequence

When a panic is detected, the panic hook executes a five-step emergency shutdown. The same sequence is also used by the `HardwareWatchdog` when the daemon loop hangs:

```rust
pub fn install_panic_hook_with_hardware(
    registry: &Arc<hardware::registry::DeviceRegistry>
) {
    let default_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        error!("PANIC detected - attempting emergency hardware shutdown");

        // 5-step shutdown in order of safety priority:
        // 1. Close scripting-registered shutters (ShutterRegistry)
        Self::emergency_close_all();
        // 2. Close ALL ShutterControl devices from DeviceRegistry
        Self::emergency_close_all_shutters_from_registry();
        // 3. Disable ALL EmissionControl devices from DeviceRegistry
        Self::emergency_disable_all_emission();
        // 4. Stop all motors (Movable devices)
        Self::emergency_stop_motors();
        // 5. Zero critical DAQ outputs (Settable devices)
        Self::emergency_zero_outputs();

        // Call the default hook to print the panic message
        default_hook(info);
    }));
}
```

**Shutdown ordering rationale:** Shutters are closed first because they can block the beam immediately. EmissionControl devices (laser sources) are disabled next. Motors are stopped to prevent uncontrolled motion. DAQ outputs are zeroed last to bring analog signals (e.g., EOM control voltage) to a safe state.

**Registry-based enumeration:** Steps 2-5 use `DeviceRegistry::devices_with_capability()` to discover ALL devices implementing a given capability, not just those registered via scripting. This ensures that devices opened via gRPC or direct API calls are also covered.

**Shutdown Steps:**

#### Step 1: Emergency Close All Shutters (Scripting-Registered)

```rust
pub fn emergency_close_all() {
    // Idempotency check - ensure emergency close runs only once
    if Self::global().emergency_closed.swap(true, Ordering::SeqCst) {
        info!("Emergency close already executed, skipping");
        return;
    }

    let shutters: Vec<Arc<dyn ShutterControl>> = {
        match Self::global().shutters.try_lock() {
            Ok(guard) => {
                guard.values()
                    .filter_map(|weak| weak.upgrade())
                    .collect()
            }
            Err(_) => {
                error!("Failed to acquire shutter registry lock (deadlock risk)");
                return;
            }
        }
    };

    // Use bridge thread pattern for async in panic context
    if let Some(handle) = Self::get_or_create_runtime() {
        let result = std::thread::spawn(move || {
            handle.block_on(async {
                let tasks: Vec<_> = shutters
                    .into_iter()
                    .enumerate()
                    .map(|(i, shutter)| {
                        tokio::spawn(async move {
                            match tokio::time::timeout(
                                Duration::from_secs(2),
                                shutter.close_shutter(),
                            ).await {
                                Ok(Ok(())) => {
                                    info!(shutter_index = i, "Emergency shutter close: SUCCESS");
                                    true
                                }
                                Ok(Err(e)) => {
                                    error!(shutter_index = i, error = %e, "Emergency shutter close: FAILED");
                                    false
                                }
                                Err(_) => {
                                    error!(shutter_index = i, "Emergency shutter close: TIMEOUT (2s)");
                                    false
                                }
                            }
                        })
                    })
                    .collect();

                // Wait for all tasks to complete
                for task in tasks {
                    let _ = task.await;
                }
            })
        }).join();
    }
}
```

**Key Patterns:**
- **Idempotency:** `emergency_closed` atomic flag prevents multiple executions
- **Non-blocking lock:** `try_lock()` avoids deadlock if holder is panicked
- **Bridge thread pattern:** Standard sync thread bridges into async runtime
- **Parallel execution:** All shutters closed in parallel (2-second timeout per shutter)
- **Graceful failure:** Logs errors but continues to next phase

#### Step 2: Emergency Close All Shutters (Registry)

```rust
fn emergency_close_all_shutters_from_registry() {
    // Queries DeviceRegistry::devices_with_capability(Capability::ShutterControl)
    // Covers shutters opened via gRPC/direct API (not just scripting guards)
    // Same bridge-thread + 2s timeout pattern as step 1
}
```

#### Step 3: Emergency Disable All Emission

```rust
fn emergency_disable_all_emission() {
    // Queries DeviceRegistry::devices_with_capability(Capability::EmissionControl)
    // Calls disable_emission() on each device (turns off laser sources)
    // Same bridge-thread + 2s timeout pattern
}
```

#### Step 4: Emergency Stop Motors

```rust
fn emergency_stop_motors() {
    // Lists devices with Capability::Movable
    // Calls stop() on each motor with 2s timeout
    // Same bridge-thread pattern
}
```

#### Step 5: Emergency Zero Outputs

```rust
fn emergency_zero_outputs() {
    // Lists devices with Capability::Settable
    // Sets "value" parameter to 0.0 with 2s timeout
    // Prevents EOM amplifier from remaining at active levels
}
```

---

### Layer 7: SafetyHeartbeat - External Hardware Interlock via Comedi DIO

The SafetyHeartbeat is a proactive safety mechanism that toggles a Comedi digital output channel at a fixed interval (default 100ms). An external hardware interlock circuit monitors this pulse train: if the daemon crashes, hangs, or the process is killed, the pulse stops and the external circuitry cuts laser power.

Unlike the software layers above (which react to failures), the SafetyHeartbeat provides continuous proof-of-liveness to external hardware. It protects against failure modes that software cannot catch, including SIGKILL and total process death.

**Implementation:** A Tokio task toggles the DIO channel via `spawn_blocking` (Comedi FFI). Transient toggle errors are retried; after 10 consecutive failures, the task stops (the missing pulse triggers the external interlock). On graceful shutdown, the channel is driven LOW before the task exits.

**Configuration:** Via `[safety_heartbeat]` in the hardware TOML config:

```toml
[safety_heartbeat]
enabled = true
device = "/dev/comedi0"
channel = 0
interval_ms = 100
```

**Feature gate:** Only compiled with `comedi_hardware`. On non-Comedi builds, the heartbeat is silently skipped.

**Source:** `crates/bin/src/safety_heartbeat_task.rs`

---

## Key Design Patterns

### Atomic Idempotency Flags

Prevents duplicate execution if panic hook is called multiple times:

```rust
/// Flag ensuring emergency close runs only once
emergency_closed: AtomicBool,

/// In emergency_close_all():
if Self::global().emergency_closed.swap(true, Ordering::SeqCst) {
    info!("Emergency close already executed, skipping");
    return;
}
```

**Why atomic, not Mutex:**
- Mutex can deadlock if the holder panicked
- Atomic compare-and-swap is lock-free and panic-safe

---

### Non-Blocking Lock (try_lock)

Avoids deadlocks when acquiring shared state in panic context:

```rust
match Self::global().shutters.try_lock() {
    Ok(guard) => { /* access registry */ }
    Err(_) => {
        error!("Failed to acquire lock during emergency close (deadlock risk)");
        return;  // Graceful failure, don't panic
    }
}
```

**Why not lock().await:**
- The mutex holder might be panicked on another thread
- `lock()` would block forever waiting for a dead thread
- `try_lock()` fails fast and allows graceful degradation

---

### Bridge Thread Pattern for Async in Panic Context

Tokio requires an active runtime context (via `Handle::try_current()`). In panic handlers, the current context might be invalid. The bridge thread pattern enters the runtime explicitly:

```rust
// In panic context (no active async)
let handle = Self::get_or_create_runtime()?;

// Spawn sync thread that enters the runtime
let result = std::thread::spawn(move || {
    handle.block_on(async {
        // Now we're inside the runtime context
        shutter.close_shutter().await?;
    })
}).join();
```

**Advantage:** Works whether called from async or sync context, and handles both the active runtime case and the emergency fallback.

---

### Parallel Execution with Timeouts

Emergency shutdown executes all hardware operations in parallel with individual timeouts:

```rust
let tasks: Vec<_> = shutters
    .into_iter()
    .enumerate()
    .map(|(i, shutter)| {
        tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(2), shutter.close_shutter()).await
        })
    })
    .collect();

for task in tasks {
    let _ = task.await;  // Wait for all tasks (each has its own timeout)
}
```

**Rationale:**
- Closes N shutters in ~2 seconds instead of N × 2 seconds
- Individual timeouts prevent a single unresponsive device from blocking others
- `join()` at the end ensures all devices have been processed

---

## Failure Modes and Mitigations

### Failure Mode 1: Allocation Failure During Panic

**Cause:** Emergency runtime creation fails during startup due to OOM.

**Mitigation:** Pre-allocate the runtime at application startup. If initialization fails, the entire process panics immediately (fail-fast) rather than during an emergency.

```rust
pub fn init_emergency_runtime() {
    EMERGENCY_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("CRITICAL: Failed to create emergency Tokio runtime during initialization")
    });
}
```

---

### Failure Mode 2: Deadlock in Panic Handler

**Cause:** Panic hook tries to acquire a Mutex held by the panicking thread.

**Mitigation:** Use `try_lock()` instead of `lock()`. If the lock is held, log and gracefully continue to the next phase.

```rust
match Self::global().shutters.try_lock() {
    Ok(guard) => { /* success */ }
    Err(_) => {
        error!("Failed to acquire shutter registry lock during emergency close (deadlock risk)");
        return;
    }
}
```

---

### Failure Mode 3: Unresponsive Hardware

**Cause:** Hardware device doesn't respond to shutdown command (offline, firmware hang).

**Mitigation:** Individual 2-second timeout per device. If a device doesn't respond, log and continue to next device.

```rust
match tokio::time::timeout(Duration::from_secs(2), shutter.close_shutter()).await {
    Ok(Ok(())) => { /* success */ }
    Ok(Err(e)) => { error!("Shutter close failed: {}", e); }
    Err(_) => { error!("Shutter close timeout (2s)"); }
}
```

---

### Failure Mode 4: SIGKILL or Power Failure

**Cause:** Uninterceptable signal or hardware power loss.

**Mitigation:** None at the software level. **External hardware interlocks are REQUIRED for production laser labs.**

---

## Global Shutter Registry

All open shutters are registered in a global registry to enable emergency shutdown:

```rust
pub struct ShutterRegistry {
    shutters: Mutex<HashMap<u64, Weak<dyn ShutterControl>>>,
    handlers_installed: AtomicBool,
    hardware_registry: Mutex<Option<Weak<DeviceRegistry>>>,
    emergency_closed: AtomicBool,
}

impl ShutterRegistry {
    pub fn register(driver: &Arc<dyn ShutterControl>) -> u64 {
        let id = GUARD_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        let weak = Arc::downgrade(driver);
        Self::global().shutters.lock().insert(id, weak);
        id
    }

    pub fn unregister(id: u64) {
        Self::global().shutters.lock().remove(&id);
    }
}
```

**Weak References:** Prevents accidental refcount cycles and allows graceful cleanup if the driver is dropped.

**Usage in HeartbeatShutterGuard:**

```rust
impl HeartbeatShutterGuard {
    pub async fn new(
        driver: Arc<dyn ShutterControl>,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let id = ShutterRegistry::register(&driver);
        // ... setup watchdog ...
        Ok(Self { id, driver, ... })
    }
}

impl Drop for HeartbeatShutterGuard {
    fn drop(&mut self) {
        ShutterRegistry::unregister(self.id);
        // Attempt to close shutter on drop
    }
}
```

---

## Installation and Initialization

### Startup Sequence

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // 1. Initialize emergency runtime (fails fast if OOM)
    init_emergency_runtime();

    // 2. Create hardware registry
    let registry = Arc::new(DeviceRegistry::new());

    // 3. Register devices...
    registry.register_device(camera);
    registry.register_device(laser);

    // 4. Install safety handlers
    ShutterRegistry::install_signal_handlers();
    ShutterRegistry::install_panic_hook_with_hardware(&registry);

    // 5. Run application
    run_app(registry).await
}
```

### Verification

After startup, verify that:
1. Emergency runtime is initialized: `EMERGENCY_RUNTIME.get().is_some()`
2. Signal handlers are installed: `ShutterRegistry::global().handlers_installed.load()`
3. Hardware registry is registered for panic hook

---

## Usage Examples

### Example 1: Simple Shutter Control (Basic Safety)

```rust
let laser = registry.get_shutter("maitai")?;

// Basic safety: shutter always closes
with_shutter_open(laser.clone(), async {
    // Laser is open
    acquisition_logic().await
}).await?;

// Shutter automatically closed
```

**Coverage:** Panic inside `acquisition_logic()` is NOT caught.

---

### Example 2: Long-Running Script with Heartbeat Watchdog

Historical note: older scripting surfaces exposed direct serial-device factory helpers. The current runtime prefers loading real instruments through the daemon/runtime configuration path. The example below is schematic and illustrates the watchdog pattern rather than a guaranteed current public Rhai API.

```rhai
// Pseudocode illustrating the heartbeat-watchdog pattern
let laser = acquire_laser_handle_somehow();
let guard = create_heartbeat_guard(laser, 5.0);  // 5-second watchdog

for wavelength in [700, 750, 800] {
    guard.heartbeat();
    laser.set_wavelength(wavelength);
    let power = power_meter.read();
    print(`Power at ${wavelength}: ${power}`);
}

// Guard closes shutter on drop
```

**Coverage:** Detects infinite loops, deadlocks, and hardware timeouts. Does NOT catch panic.

---

### Example 3: Full Hardware Safety

```rust
// Installed at startup (see Startup Sequence above)
ShutterRegistry::install_panic_hook_with_hardware(&registry);

// Application runs normally
// If panic occurs, automatically:
// 1. Closes scripting-registered shutters (ShutterRegistry)
// 2. Closes ALL ShutterControl devices from DeviceRegistry
// 3. Disables ALL EmissionControl devices
// 4. Stops all motors
// 5. Zeros critical DAQ outputs
```

**Coverage:** Catches panics and gracefully shuts down all hardware.

---

## Limitations

### CRITICAL: Cannot Protect Against

1. **SIGKILL** (`kill -9`): Cannot be intercepted at any level
2. **Power Failure**: No software can run without power
3. **Hardware Crashes**: No software protection
4. **Unresponsive Devices**: Individual device timeout (2s), but if ALL devices hang, process hangs

### Recommended Production Setup

For laser labs operating expensive or dangerous hardware:

1. **Hardware Interlocks** (REQUIRED):
   - Electronic shutter interlocks that close on power loss
   - Motion-stop buttons that hardware interrupt
   - EOM amplifier that defaults to safe state on power loss

2. **Watchdog Hardware** (Recommended):
   - Dedicated watchdog timer that resets on heartbeat
   - If heartbeat stops, triggers hardware shutdown

3. **Monitoring** (Recommended):
   - Telemetry on critical hardware state
   - Alerts if shutdown handlers fail
   - Integration with lab safety systems

---

## Metrics and Observability

### Logging

All emergency operations are logged at critical levels:

```
warn!("EMERGENCY: Closing all registered shutters");
info!(shutter_index = 0, "Emergency shutter close: SUCCESS");
error!(shutter_index = 1, error = "Device offline", "Emergency shutter close: FAILED");
error!(shutter_index = 2, "Emergency shutter close: TIMEOUT (2s)");
```

### Structured Logging Fields

- `shutter_index`: Index in parallel shutdown sequence
- `device_id`: Hardware device identifier
- `motor_index`: Index in motor shutdown sequence
- `output_index`: Index in DAQ output shutdown sequence
- `elapsed_secs`: Elapsed time since last heartbeat
- `timeout_secs`: Configured heartbeat timeout

---

## Acceptance Criteria

- [x] All critical hardware has emergency shutdown (shutters, motors, DAQ outputs)
- [x] Pre-allocated emergency runtime prevents allocation panics
- [x] Deadlock-free design using non-blocking locks
- [x] All devices have individual 2-second timeout
- [x] Idempotency guards prevent duplicate execution
- [x] Signal handlers (SIGTERM, SIGINT) trigger graceful shutdown
- [x] Panic hook executes five-step shutdown sequence (shutters, registry shutters, emission, motors, DAQ)
- [x] HardwareWatchdog fires the same five-step sequence on timeout
- [x] SafetyHeartbeat provides proactive DIO pulse for external hardware interlock
- [x] Comprehensive logging at all failure points
- [x] Documentation clarifies software limitations vs. required hardware interlocks

---

## References

- [Shutter Safety Module](../../crates/scripting/src/shutter_safety.rs)
- [HeartbeatShutterGuard Implementation](../../crates/scripting/src/shutter_safety.rs#L649-L844)
- [Global Shutter Registry](../../crates/scripting/src/shutter_safety.rs#L100-L627)
- [SafetyHeartbeat Task](../../crates/bin/src/safety_heartbeat_task.rs)
- [HardwareWatchdog](../../crates/common/src/health/watchdog.rs)

---

## Revision History

| Date | Author | Description |
|------|--------|-------------|
| 2026-02-02 | bd-vmfp.2 | Initial panic safety architecture documentation |
| 2026-03-11 | docs update | Add SafetyHeartbeat (Layer 7), update to 5-step emergency shutdown, document registry-based enumeration |
