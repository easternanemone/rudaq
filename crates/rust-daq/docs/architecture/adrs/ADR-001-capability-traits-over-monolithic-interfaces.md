# ADR-001: Capability Traits Over Monolithic Interfaces

**Status**: Accepted
**Date**: 2025-11-20
**Deciders**: Architecture Team
**Coordinator**: Jules-19

## Context

The rust-daq project historically suffered from monolithic hardware interfaces that forced all devices into a single trait contract (V1 `Instrument` trait, V2 `HardwareAdapter` trait). This led to:

1. Runtime errors when unsupported operations were called
2. Type-unsafe downcasting (`as_any()` anti-pattern)
3. Tight coupling between instrument types and application code
4. Difficulty testing individual device capabilities
5. Inability to compose hardware in generic experiment code

Reference frameworks (DynExp, PyMODAQ, ScopeFoundry) also suffer from this design flaw, using runtime checks for capability detection.

## Decision

We adopt **atomic capability traits** as the fundamental hardware abstraction. Each trait represents a single, well-defined capability:

- `Movable` - Motion control (stages, actuators, goniometers)
- `Triggerable` - External triggering (cameras, detectors)
- `Readable` - Scalar readout (power meters, sensors)
- `ExposureControl` - Integration time (cameras, spectrometers)
- `FrameProducer` - Image acquisition (cameras, beam profilers)

Devices implement only the capabilities they support. Generic code uses trait bounds to express requirements at compile time.

### Implementation Pattern

```rust
// Hardware driver implements relevant capabilities
pub struct PvcamCamera { /* ... */ }

#[async_trait]
impl Triggerable for PvcamCamera {
    async fn arm(&self) -> Result<()> { /* ... */ }
    async fn trigger(&self) -> Result<()> { /* ... */ }
}

#[async_trait]
impl ExposureControl for PvcamCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> { /* ... */ }
    async fn get_exposure(&self) -> Result<f64> { /* ... */ }
}

#[async_trait]
impl FrameProducer for PvcamCamera {
    async fn start_stream(&self) -> Result<()> { /* ... */ }
    async fn stop_stream(&self) -> Result<()> { /* ... */ }
    fn resolution(&self) -> (u32, u32) { /* ... */ }
}

// Generic experiment code - compiler enforces capabilities
async fn triggered_acquisition<C>(camera: &C) -> Result<()>
where
    C: Triggerable + ExposureControl + FrameProducer
{
    camera.set_exposure(0.1).await?;
    camera.arm().await?;
    camera.trigger().await?;
    // Compiler guarantees camera supports all three operations
    Ok(())
}
```

### Trait Design Principles

Each capability trait MUST:

1. **Be async** - Use `#[async_trait]` for non-blocking I/O
2. **Be thread-safe** - Require `Send + Sync` bounds
3. **Use anyhow::Result** - Consistent error handling
4. **Focus on ONE capability** - Single Responsibility Principle
5. **Take &self** - Immutable references (interior mutability via Mutex)

## Consequences

### Positive

1. **Compile-time safety**: Cannot call unsupported operations (checked by type system)
2. **Hardware agnostic code**: Experiments generic over trait bounds
3. **Easy testing**: Mock individual capabilities without full device simulation
4. **Clear contracts**: Small, focused traits are easier to understand and implement
5. **Composability**: Devices can mix capabilities (triggered camera = Triggerable + FrameProducer)
6. **No runtime type checks**: Eliminates `as_any()` anti-pattern from V2

### Negative

1. **More traits to manage**: 5+ capability traits vs 1 monolithic interface
2. **Trait object complexity**: Need blanket impls for combined traits (e.g., `Camera`)
3. **Learning curve**: Developers must understand trait composition
4. **Boilerplate for common combinations**: Repeated `where` clauses

### Mitigation Strategies

For trait object usage, provide combined traits with blanket implementations:

```rust
// Combined trait for cameras (trait object support)
pub trait Camera: Triggerable + FrameProducer {}

// Blanket impl - any type with both capabilities gets Camera for free
impl<T: Triggerable + FrameProducer> Camera for T {}

// Use in trait objects
fn use_camera(camera: Arc<dyn Camera>) { /* ... */ }
```

For common capability combinations, provide type aliases:

```rust
// Common combinations
pub trait TriggeredCamera: Triggerable + ExposureControl + FrameProducer {}
pub trait MotionStage: Movable + Triggerable {}
pub trait ScalarSensor: Readable {}
```

## Alternatives Considered

### Alternative 1: Monolithic Instrument Trait (V1/V2 Pattern)

**Rejected** - Requires runtime capability detection and unsafe downcasting.

```rust
// V2 pattern - REJECTED
pub trait Instrument {
    fn read(&self) -> Result<f64>;  // Not all instruments have scalar read
    fn trigger(&self) -> Result<()>; // Not all instruments are triggerable
    fn as_any(&self) -> &dyn Any;   // Type-unsafe escape hatch
}
```

### Alternative 2: Enum-Based Capability Discovery (DynExp Pattern)

**Rejected** - Still requires runtime checks, loses compile-time safety.

```rust
// DynExp pattern - REJECTED
pub enum Capability {
    Movable,
    Triggerable,
    Readable,
}

pub trait Instrument {
    fn supports(&self, cap: Capability) -> bool;
    fn as_movable(&self) -> Option<&dyn Movable>;  // Still requires downcasting
}
```

### Alternative 3: Associated Types (Generic Instrument)

**Rejected** - Too rigid, cannot express "implements A or B or both".

```rust
// Generic pattern - REJECTED
pub trait Instrument {
    type Control: MotionControl;
    type Acquisition: DataAcquisition;
}
// Problem: Cannot express "camera without motion control"
```

## Related Decisions

- ADR-002: Async Trait Methods (all capabilities are async)
- ADR-003: Hardware Layer Isolation (no Arrow/HDF5 dependencies)
- ADR-004: ScriptEngine Abstraction (capabilities exposed to scripts)

## References

- DynExp architecture: Monolithic instrument base class with runtime capability checks
- PyMODAQ: Plugin system with capability discovery at runtime
- ScopeFoundry: Measurement classes with runtime feature detection
- Rust async traits: async-trait crate for async methods in traits
- src/hardware/capabilities.rs: V5 implementation

## Implementation Status

- [x] Capability traits defined (src/hardware/capabilities.rs)
- [x] Mock implementations (src/hardware/mock.rs)
- [x] Tests for trait composition
- [ ] ESP300 driver migration (in progress)
- [ ] PVCAM driver migration (in progress)
- [ ] MaiTai driver migration (in progress)
- [ ] All legacy V2 drivers removed

## Notes

This decision fundamentally changes how hardware is abstracted in rust-daq. It aligns with Rust's preference for compile-time guarantees over runtime flexibility. The learning curve for developers is offset by the safety and clarity benefits.

**Key Insight**: By making capabilities explicit in the type system, we push errors from runtime to compile time. A experiment that calls `camera.move_abs()` will fail to compile if the camera doesn't implement `Movable`, rather than failing at runtime with "unsupported operation".

This is a **foundational** decision that affects all future hardware integrations. Any new device must implement the appropriate capability traits, not a monolithic `Instrument` interface.

---

**Document Owner**: Jules-19 (Architecture Coordinator)
**Last Updated**: 2025-11-20
**Review Cycle**: Quarterly
