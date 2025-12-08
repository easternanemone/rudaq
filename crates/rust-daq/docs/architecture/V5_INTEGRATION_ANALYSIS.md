# V5 Architecture Integration Analysis
**Date**: 2025-12-06
**Analysis Tool**: gemini-2.5-pro thinkdeep + expert validation
**Status**: RESOLVED - V5 Transition Complete

## Resolution Status (2025-12-06)

**Split-brain issues have been RESOLVED.**
- Unified `Parameter<T>` and `Observable<T>` (bd-si19 completed)
- Parameterized trait implemented (bd-9clg completed)
- Drivers migrated to Parameter system (bd-dili completed)
- gRPC integration fixed (bd-gmwv completed)

See [V5_IMPLEMENTATION_STATUS.md](V5_IMPLEMENTATION_STATUS.md) for current state.

## Executive Summary

The V5 "headless-first" rewrite has **all the right architectural components** but they exist **in complete isolation**. This is not a missing features problem - it's an **integration failure**. The recent work (Stageable trait bd-7aq6, parameter streaming bd-f5yh) has been **superficial band-aids** that don't address the root cause.

### Key Finding
rust-daq has TWO separate reactive parameter systems that should be ONE:
- `Parameter<T>` (742 lines) - Command pattern for hardware writes
- `Observable<T>` (345 lines) - Notification pattern for broadcasts

**NO hardware drivers use either**. They all use raw `Arc<RwLock<T>>`.

## Comparison to Mature Frameworks

### What PyMoDAQ/ScopeFoundry/Qudi Got Right

| Pattern | Mature Frameworks | rust-daq V5 |
|---------|-------------------|-------------|
| **Reactive State** | ScopeFoundry `LoggedQuantity`, PyMoDAQ `ParameterTree` - central parameter system with auto-sync | ✅ EXISTS (`Parameter<T>`) ❌ UNUSED by drivers |
| **Logic/Hardware Split** | Qudi Logic Modules - high-level algorithms separate from drivers | ✅ EXISTS (`Module` trait) ❌ NOT CONNECTED to driver params |
| **Config Snapshots** | Serialize entire runtime state for reproducibility | ✅ EXISTS (`ParameterSet`) ❌ NO centralized registry |
| **Live Data Tapping** | PyMoDAQ DataGrabbers - broadcast data to multiple consumers | ⚠️ PARTIAL (ring buffer) ❌ NO tap mechanism |
| **System Health** | Heartbeat monitoring for headless operation | ❌ MISSING |
| **Experiment Manifests** | Auto-inject all params to HDF5 | ⚠️ PARTIAL (`metadata.rs` exists) ❌ NO auto-collection |

## Critical Architectural Gaps

### 1. Fragmented Reactive Primitives (P0 - BLOCKING)

**Problem**: Two incompatible patterns for the same concept
```rust
// Parameter<T> - Hardware write action
pub struct Parameter<T> {
    name: String,
    value_tx: watch::Sender<T>,  // ← Has subscription support!
    hardware_writer: Option<Arc<dyn Fn(T) -> Result<()>>>,  // ← Hardware callback
    // ...but drivers don't use this
}

// Observable<T> - Multi-subscriber notifications
pub struct Observable<T> {
    sender: watch::Sender<T>,  // ← Same mechanism!
    // ...modules use this, drivers don't
}

// Drivers - Raw state management
pub struct MockCamera {
    exposure_s: Arc<RwLock<f64>>,  // ← Bypasses both systems!
    // NO automatic notifications, NO parameter registry integration
}
```

**Impact**:
- Driver state changes are invisible to gRPC clients
- Modules can't observe hardware parameters
- Recent parameter streaming (bd-f5yh) duplicated `Parameter<T>.subscribe()` functionality
- Presets can't snapshot driver state

**Solution (bd-si19)**:
Embed `Observable<T>` inside `Parameter<T>` to create ONE unified abstraction:
```rust
pub struct Parameter<T> {
    name: String,
    observable: Observable<T>,  // ← Handles subscriptions
    write_action: Option<Box<dyn Fn(&T) + Send + Sync>>,  // ← Hardware write

    pub fn set_value(&mut self, value: T) {
        if let Some(action) = &self.write_action {
            action(&value);  // 1. Execute hardware command
        }
        self.observable.set(value);  // 2. Broadcast to subscribers
    }

    pub fn subscribe(&mut self, callback: fn(&T)) {
        self.observable.subscribe(callback);  // Delegate
    }
}
```

### 2. Driver Migration Never Happened (P0 - BLOCKING)

**Problem**: V5 architecture built, V4 patterns never removed

All drivers use legacy state management:
- ❌ `MockCamera`: `Arc<RwLock<f64>>` for exposure
- ❌ `MockStage`: `Arc<RwLock<f64>>` for position
- ❌ `PVCAM`: raw state structs
- ❌ `ELL14`, `ESP300`, `MaiTai`, `Newport1830C`: manual state

**Why This Matters**:
```rust
// Current: Changes are invisible
impl ExposureControl for MockCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        *self.exposure_s.write().await = seconds;
        // ← NO ONE KNOWS THIS CHANGED
        Ok(())
    }
}

// Target: Automatic notifications
impl ExposureControl for MockCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        self.exposure.set(seconds)?;
        // ← Parameter<T> automatically:
        //    1. Writes to hardware (via callback)
        //    2. Notifies all subscribers (gRPC, modules, logger)
        //    3. Updates parameter registry
        Ok(())
    }
}
```

**Solution (bd-dili)**:
Refactor all drivers to use `Parameter<T>` for mutable state.

### 3. No Centralized Parameter Registry (P1)

**Problem**: Parameters are isolated within driver structs

```rust
// Current: No way to enumerate or access parameters
let camera = MockCamera::new(640, 480);
// How do you list its parameters? You can't.
// How does a Module get exposure? It can't.
// How does gRPC list_parameters work? It doesn't.
```

**Impact**:
- `list_parameters` RPC returns empty (no central registry)
- Modules can't observe driver params (need cross-layer access)
- Presets can't snapshot full system state
- Experiment manifests can't auto-collect metadata

**Solution (bd-gajr)**:
`DeviceRegistry` maintains `ParameterSet` for each device:
```rust
impl DeviceRegistry {
    pub fn get_parameters(&self, device_id: &str) -> Option<&ParameterSet> {
        self.devices.get(device_id).map(|d| &d.parameters)
    }
}

// Drivers populate on construction:
impl MockCamera {
    pub fn new(...) -> (Self, ParameterSet) {
        let mut params = ParameterSet::new();
        let exposure = Parameter::new("exposure_s", 0.1)
            .with_unit("s")
            .with_range(0.001, 10.0);
        params.register(exposure.clone());
        (Self { exposure, ... }, params)
    }
}
```

### 4. gRPC Integration Is Ad-Hoc (P1)

**Problem**: bd-f5yh added raw `broadcast::Sender<ParameterChange>` to `HardwareServiceImpl`

This duplicates `Parameter<T>.subscribe()` functionality and only captures RPC-initiated changes:
```rust
// bd-f5yh implementation (WRONG):
async fn set_parameter(...) {
    settable.set_value(...)?;  // Hardware write
    self.param_change_tx.send(ParameterChange { ... })?;  // Manual broadcast
}

// Hardware-initiated changes DON'T broadcast:
async fn read_from_hardware(...) {
    let value = hardware.get_exposure()?;
    // ← NO broadcast! gRPC clients never see this
}
```

**Solution (bd-o5n9)**:
Replace with proper `Parameter<T>` integration:
```rust
async fn stream_parameter_changes(...) {
    let params = registry.get_parameters(&device_id)?;
    for param in params.iter() {
        let mut rx = param.subscribe();  // ← Use existing mechanism
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                tx.send(ParameterChange::from(rx.borrow())).await?;
            }
        });
    }
}
```

### 5. Missing "Headless" Safety Features (P2)

**Silent Failure Risk**: Errors in background tasks don't surface
```rust
// Current: Module crashes silently
tokio::spawn(async move {
    loop {
        match sensor.read().await {
            Err(e) => {
                // ← ERROR LOST! Experiment continues with stale data
            }
        }
    }
});
```

**Solutions**:
- **SystemHealthMonitor** (bd-pauy): Heartbeat from all modules, error propagation
- **Ring buffer tap** (bd-dqic): Remote visualization without disrupting HDF5 writer
- **Auto-manifest injection** (bd-ej44): Snapshot all params to HDF5 at experiment start

## Work Completed vs Actual Needs

### Recent Work (Questionable Value)
- ✅ bd-7aq6: Stageable trait - Good pattern, but modules already had stage/unstage
- ⚠️ bd-f5yh: Parameter streaming - Duplicates `Parameter<T>.subscribe()`, only works for RPC changes

### Critical Path (What Actually Matters)
1. **bd-si19** (P0): Unify Parameter/Observable
2. **bd-dili** (P0): Migrate drivers to Parameter<T>
3. **bd-gajr** (P1): ParameterRegistry in DeviceRegistry
4. **bd-o5n9** (P1): Wire Parameter subscriptions to gRPC
5. **bd-pauy** (P2): System health monitoring
6. **bd-dqic** (P2): Ring buffer tap mechanism
7. **bd-ej44** (P2): Auto-manifest injection

## Why This Happened (Postmortem)

The V5 "headless-first" rewrite followed a classic **incomplete migration** pattern:
1. ✅ Designed beautiful new architecture (Parameter<T>, Observable<T>, Modules)
2. ✅ Implemented core components
3. ❌ **Never migrated drivers** to use new patterns
4. ❌ **Never removed old patterns** (raw Arc<RwLock>)
5. ❌ **No integration testing** between layers

Result: Two architectures coexist, neither fully functional.

## Recommendations

### Immediate (P0 - Weeks)
1. Unify reactive primitives (bd-si19)
2. Migrate MockCamera and MockStage as pilot (bd-dili phase 1)
3. Add ParameterRegistry to DeviceRegistry (bd-gajr)

### Short-term (P1 - Month)
4. Complete driver migration (bd-dili phase 2)
5. Replace bd-f5yh with proper integration (bd-o5n9)
6. Validate with end-to-end test: gRPC client observes hardware-initiated change

### Medium-term (P2 - Months)
7. System health monitoring (bd-pauy)
8. Ring buffer tapping (bd-dqic)
9. Experiment manifest auto-injection (bd-ej44)

### Long-term (Future)
- Logic module library (PID control, auto-focus, beam alignment)
- Plugin system for third-party modules
- Distributed RunEngine for multi-instrument experiments

## Success Criteria

The V5 architecture will be "complete" when:
- ✅ All driver state is `Parameter<T>`-based (no raw Mutex)
- ✅ gRPC clients can list and subscribe to all hardware parameters
- ✅ Modules can observe and react to driver parameter changes
- ✅ Presets can snapshot and restore entire system state
- ✅ Headless experiments run for hours without silent failures
- ✅ HDF5 files contain complete parameter manifests

## References
- gemini-2.5-pro analysis: "rust-daq is structurally sound but architecturally incomplete"
- Expert validation: "Unify Parameter<T> and Observable<T> by embedding one in the other"
- PyMoDAQ LoggedQuantity: https://github.com/PyMoDAQ/PyMoDAQ
- ScopeFoundry architecture: https://github.com/ScopeFoundry/ScopeFoundry
- Qudi modules: https://github.com/Ulm-IQO/qudi
