# Capability-Based Architecture Migration Guide

**Task**: bd-bm03
**Date**: 2025-11-18
**Status**: Traits defined, full migration pending

## What Was Done

### 1. Created `/src/hardware/capabilities.rs`

Defined 5 atomic capability traits:

- **`Movable`**: Motion control (stages, actuators)
  - `move_abs()`, `move_rel()`, `position()`, `wait_settled()`

- **`Triggerable`**: External triggering support
  - `arm()`, `trigger()`

- **`ExposureControl`**: Integration time control
  - `set_exposure()`, `get_exposure()`

- **`FrameProducer`**: Image/frame generation
  - `start_stream()`, `stop_stream()`, `resolution()`

- **`Readable`**: Scalar measurement readout
  - `read()`

All traits are:
- Async (`#[async_trait]`)
- Thread-safe (`Send + Sync`)
- Use `anyhow::Result<T>` for errors

### 2. Updated `/src/hardware/mod.rs`

Added data types:

- **`FrameRef`**: Zero-copy frame reference (pointer + metadata)
- **`Roi`**: Region of Interest for cameras (migrated from `core_v3`)

Re-exported capability traits for easy use.

### 3. Migration from `core_v3.rs`

Useful types migrated:
- `Roi` struct with validation methods
- Image metadata concepts (now in `FrameRef`)

Types **NOT** migrated (obsolete with new design):
- `Instrument` trait (monolithic, replaced by capability composition)
- `Camera`, `Stage`, `Spectrometer` meta-traits (too coarse)
- `InstrumentState`, `Command`, `Response` (actor model, being replaced)
- `Measurement` enum (broadcast system, being replaced)

## Why This Design

### Old Architecture (core_v3)
```rust
// Monolithic traits
trait Camera: Instrument {
    async fn set_exposure(&mut self, ms: f64) -> Result<()>;
    async fn set_roi(&mut self, roi: Roi) -> Result<()>;
    async fn start_acquisition(&mut self) -> Result<()>;
    // ... 10 more methods
}

// Device must implement entire trait even if only 2 methods are used
struct SimpleCamera { /* ... */ }
impl Instrument for SimpleCamera { /* ... */ }
impl Camera for SimpleCamera { /* ... 12 methods */ }
```

### New Architecture (capabilities)
```rust
// Atomic capabilities
struct SimpleCamera { /* ... */ }

impl ExposureControl for SimpleCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> { /* ... */ }
    async fn get_exposure(&self) -> Result<f64> { /* ... */ }
}

impl FrameProducer for SimpleCamera {
    async fn start_stream(&self) -> Result<()> { /* ... */ }
    async fn stop_stream(&self) -> Result<()> { /* ... */ }
    fn resolution(&self) -> (u32, u32) { (1024, 1024) }
}

// Generic code uses trait bounds
async fn triggered_scan<T>(device: &T) -> Result<()>
where
    T: Triggerable + ExposureControl + FrameProducer
{
    device.set_exposure(0.1).await?;
    device.arm().await?;
    device.trigger().await?;
    Ok(())
}
```

Benefits:
- **Composable**: Devices implement only what they support
- **Testable**: Mock individual capabilities
- **Clear**: Each trait has one job
- **Flexible**: Functions work with any device implementing required capabilities

## Next Steps (NOT DONE - Blocked)

### Step 1: Fix Compilation Errors

The codebase currently has errors due to:
1. Missing `daq_core` crate (should be in `crates/daq-core`)
2. References to removed `app_actor` module
3. Imports from `core_v3` that reference removed traits

**Action Required**:
```bash
# These errors must be fixed before removing core_v3.rs:
cargo check 2>&1 | grep "error\[E"
```

Current errors:
- `unresolved import crate::app_actor` (src/app.rs)
- `unresolved module daq_core` (src/core.rs, src/error.rs, etc.)
- `unresolved import InstrumentRegistryV2` (src/gui/mod.rs)
- `unresolved import core_v3::MotionController` (src/instruments_v2/esp300.rs)

### Step 2: Migrate Concrete Implementations

Once compilation is fixed, migrate existing instruments:

**Example: ESP300 Stage**
```rust
// OLD (core_v3.rs)
impl Stage for ESP300 {
    async fn move_absolute(&mut self, position_mm: f64) -> Result<()> { /* ... */ }
    async fn position(&self) -> Result<f64> { /* ... */ }
    // ... 6 more methods
}

// NEW (capabilities.rs)
impl Movable for ESP300 {
    async fn move_abs(&self, position: f64) -> Result<()> { /* ... */ }
    async fn position(&self) -> Result<f64> { /* ... */ }
    async fn move_rel(&self, distance: f64) -> Result<()> { /* ... */ }
    async fn wait_settled(&self) -> Result<()> { /* ... */ }
}

impl Triggerable for ESP300 {
    async fn arm(&self) -> Result<()> { /* ... */ }
    async fn trigger(&self) -> Result<()> { /* ... */ }
}
```

**Files to migrate**:
- [ ] `src/instruments_v2/esp300.rs` (Stage → Movable)
- [ ] `src/instruments_v2/newport_1830c_v3.rs` (PowerMeter → Readable)
- [ ] `src/instrument/mock_v3.rs` (Camera → ExposureControl + FrameProducer)
- [ ] Any other `core_v3::*` trait implementations

### Step 3: Remove `core_v3.rs`

**ONLY AFTER** all references are migrated:

```bash
# 1. Verify no imports remain
rg "use.*core_v3" --type rust

# 2. Delete file
rm src/core_v3.rs

# 3. Remove from lib.rs
# Edit src/lib.rs and remove: pub mod core_v3;

# 4. Verify compilation
cargo check
```

### Step 4: Update High-Level Code

Experiment/scan code using old traits:

```rust
// OLD
async fn run_scan<C: Camera>(camera: &C) -> Result<()> {
    camera.set_exposure(100.0).await?;  // milliseconds
    camera.start_acquisition().await?;
    Ok(())
}

// NEW
async fn run_scan<C>(camera: &C) -> Result<()>
where
    C: ExposureControl + FrameProducer
{
    camera.set_exposure(0.1).await?;  // seconds
    camera.start_stream().await?;
    Ok(())
}
```

## Trait Composition Patterns

### Pattern 1: Triggered Camera
```rust
fn setup_triggered_camera<T>(camera: &T) -> Result<()>
where
    T: Triggerable + ExposureControl + FrameProducer
{
    // Use all three capabilities
}
```

### Pattern 2: Motion Stage with Triggering
```rust
fn synchronized_scan<S>(stage: &S) -> Result<()>
where
    S: Movable + Triggerable
{
    // Stage triggers camera at each position
}
```

### Pattern 3: Simple Readout Device
```rust
fn monitor_power<R>(meter: &R) -> Result<Vec<f64>>
where
    R: Readable
{
    // Just read values
}
```

## Acceptance Criteria (PARTIAL)

- [x] `src/hardware/capabilities.rs` created with 5 traits
- [x] All traits are `Send + Sync + async_trait`
- [x] `FrameRef` and `Roi` defined in `mod.rs`
- [x] Documentation shows trait composition patterns
- [ ] **BLOCKED**: cargo check passes (requires daq_core crate fix)
- [ ] **BLOCKED**: core_v3.rs deleted (requires migration of all imports)
- [ ] **PENDING**: Example implementations for 2+ real devices

## Architecture Decision Record

**Decision**: Use fine-grained capability traits instead of monolithic `Instrument` trait.

**Rationale**:
1. Devices vary widely in capabilities (not all cameras support ROI, not all stages are triggerable)
2. Monolithic traits force boilerplate "unsupported" implementations
3. Trait composition enables generic code that works with any compatible device
4. Testing is easier with small, focused traits

**Trade-offs**:
- More traits to define (5 instead of 1)
- Type bounds can become complex (`T: A + B + C`)
- No single "Instrument" type for heterogeneous collections

**Mitigations**:
- Use type aliases for common combinations: `type TriggeredCamera = dyn Triggerable + ExposureControl + FrameProducer;`
- Provide helper functions for common patterns
- Document expected trait combinations for each use case

## References

- **Task Tracker**: `bd-bm03` in `bd.yaml`
- **Related Tasks**:
  - bd-bm02 (Phase 1E coordination - blocked by this)
  - bd-rir3 (V2/V4 runtime coexistence - needs capability-based devices)
- **Design Docs**:
  - `docs/ARCHITECTURAL_REDESIGN_2025.md` (old approach with monolithic traits)
  - `docs/PHASE_1E_RUNTIME_INTEGRATION.md` (runtime system expecting capabilities)
