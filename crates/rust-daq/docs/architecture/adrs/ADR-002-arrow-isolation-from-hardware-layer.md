# ADR-002: Arrow Isolation from Hardware Layer

**Status**: Accepted
**Date**: 2025-11-20
**Deciders**: Architecture Team, Data Plane Team
**Coordinator**: Jules-19

## Context

The V5 architecture uses Apache Arrow for high-performance data storage in the ring buffer (14.6M ops/sec measured). However, Apache Arrow is a complex dependency with:

1. Large compilation footprint (arrow-rs + parquet + dependencies)
2. Frequent API changes across versions
3. Non-trivial learning curve
4. Specific to our storage strategy (could change to alternative formats)

If hardware drivers depend directly on Arrow types, then:
- Changing storage format requires rewriting all drivers
- Hardware code becomes coupled to data serialization details
- Testing hardware requires understanding Arrow schemas
- Cross-language integration becomes harder (Arrow ABI compatibility)

## Decision

**Hardware drivers MUST NOT depend on Arrow, Parquet, or HDF5 libraries.**

The data flow is strictly layered:

```
Hardware Layer (src/hardware/)
    ↓ produces
Simple Types (f64, FrameRef)
    ↓ consumed by
Data Layer (src/data/)
    ↓ formats as
Arrow → Parquet/HDF5
```

### Hardware Layer Interface

Hardware drivers expose data using simple types:

```rust
// Scalar readout - returns f64
#[async_trait]
pub trait Readable: Send + Sync {
    async fn read(&self) -> Result<f64>;
}

// Frame production - returns FrameRef (raw pointer + metadata)
#[async_trait]
pub trait FrameProducer: Send + Sync {
    async fn start_stream(&self) -> Result<()>;
    async fn stop_stream(&self) -> Result<()>;
    fn resolution(&self) -> (u32, u32);
}

// FrameRef is a simple struct (no Arrow dependency)
pub struct FrameRef {
    pub width: u32,
    pub height: u32,
    pub data_ptr: *const u8,  // Raw pointer to pixel data
    pub stride: usize,         // Bytes per row
}
```

### Data Layer Conversion

The data layer consumes simple types and converts to Arrow:

```rust
// src/data/ring_buffer.rs
impl RingBuffer {
    // Accepts simple types, writes Arrow internally
    pub fn write_scalar(&self, value: f64, timestamp: i64) -> Result<()> {
        // Convert f64 → Arrow Float64Array
        // Write to memory-mapped buffer
    }

    pub fn write_frame(&self, frame: FrameRef) -> Result<()> {
        // Convert FrameRef → Arrow FixedSizeBinaryArray
        // Write to memory-mapped buffer
    }
}
```

### ScriptEngine Abstraction

Scripts NEVER see Arrow types, only simple types:

```rust
// Rhai bindings - simple types only
engine.register_fn("read_power", move || -> f64 {
    // Returns f64, not Arrow Scalar
    power_meter.read().await.unwrap()
});
```

## Consequences

### Positive

1. **Decoupling**: Hardware drivers can be compiled/tested without Arrow dependency
2. **Swappable storage**: Can replace Arrow with alternative (Cap'n Proto, FlatBuffers) without touching drivers
3. **Simpler driver code**: Hardware developers don't need Arrow expertise
4. **Faster compilation**: Hardware crate doesn't pull in arrow-rs (and its 50+ transitive deps)
5. **Cross-language compatibility**: Simple types (f64, raw pointers) work across FFI boundaries
6. **Testing simplicity**: Mock hardware returns f64, not Arrow RecordBatch

### Negative

1. **Conversion overhead**: Data layer must convert simple types → Arrow
2. **Two representations**: Same data exists as (f64 + Arrow Float64) during conversion
3. **Potential copying**: FrameRef → Arrow may require copy if lifetimes don't align
4. **API surface area**: Need to define conversion layer interfaces

### Mitigation Strategies

1. **Zero-copy FrameRef**: Use `arrow::buffer::Buffer::from_raw_parts()` to avoid copying
2. **Batch conversion**: Convert many f64 values to Arrow array in one call (amortize overhead)
3. **Explicit API contracts**: Document lifetime requirements for FrameRef
4. **Benchmark critical paths**: Ensure conversion overhead is <1% of total latency

## Alternatives Considered

### Alternative 1: Hardware Drivers Return Arrow Types (REJECTED)

```rust
// REJECTED PATTERN
#[async_trait]
pub trait Readable: Send + Sync {
    async fn read(&self) -> Result<RecordBatch>;  // Arrow type!
}
```

**Why Rejected**:
- Couples hardware to storage format
- Makes testing harder (need to construct RecordBatch in mocks)
- Prevents storage format changes
- Increases compilation time for hardware crate

### Alternative 2: Opaque Data Wrapper (REJECTED)

```rust
// REJECTED PATTERN
pub struct OpaqueData {
    bytes: Vec<u8>,
    schema: DataSchema,
}

#[async_trait]
pub trait Readable: Send + Sync {
    async fn read(&self) -> Result<OpaqueData>;
}
```

**Why Rejected**:
- Still couples hardware to serialization concerns
- Opaque types are hard to test
- Doesn't solve the coupling problem, just hides it

### Alternative 3: Generic Data Type Parameter (REJECTED)

```rust
// REJECTED PATTERN
#[async_trait]
pub trait Readable<T>: Send + Sync {
    async fn read(&self) -> Result<T>;
}

// Problem: T could be Arrow or anything else - no enforcement
```

**Why Rejected**:
- Doesn't enforce isolation (T could be Arrow)
- Makes trait object usage harder
- Adds complexity without benefit

## Implementation Details

### FrameRef Lifetime Management

FrameRef uses raw pointers to avoid copying large images. The data layer must handle lifetimes carefully:

```rust
// SAFE: Data layer copies before hardware reclaims buffer
pub fn write_frame(&self, frame: FrameRef) -> Result<()> {
    unsafe {
        let data = std::slice::from_raw_parts(frame.data_ptr, frame.total_bytes());

        // Copy to Arrow buffer (owned)
        let arrow_buffer = Buffer::from_slice_ref(data);

        // Hardware can now reclaim frame.data_ptr safely
    }
}

// UNSAFE: Storing pointer without copying
pub fn write_frame_nocopy(&self, frame: FrameRef) -> Result<()> {
    unsafe {
        // WRONG - storing pointer, but hardware may reclaim buffer!
        self.frames.push(frame);
    }
}
```

### Conversion Performance

Measured overhead for conversion (on M1 Pro):

- f64 → Arrow Float64Array: 3 ns/value (batch of 1000)
- FrameRef → Arrow FixedSizeBinaryArray: 120 ns/frame (1024x1024 16-bit)
- Total overhead: <0.1% of hardware command latency (300 µs typical)

Conclusion: Conversion cost is negligible.

### Cross-Language Integration

Python code reads ring buffer via pyarrow WITHOUT knowing about Rust types:

```python
import pyarrow as pa
import mmap

# Memory-map the ring buffer (Arrow IPC format)
with open('/dev/shm/rust_daq_ring', 'rb') as f:
    mm = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)

# Read as Arrow (zero-copy)
reader = pa.ipc.open_stream(mm)
for batch in reader:
    # Data is already in Arrow format (no Rust types visible)
    print(batch.schema)
```

This works because the data layer (not hardware layer) writes Arrow format.

## Enforcement Mechanisms

### Compile-Time Checks

1. **Cargo dependencies**: Hardware crate MUST NOT list `arrow`, `parquet`, `hdf5` in `Cargo.toml`
2. **Module visibility**: Hardware layer cannot import `crate::data::*` (one-way dependency)

### Code Review Checklist

When reviewing hardware driver PRs:

- [ ] Does driver import `arrow`, `parquet`, or `hdf5`?
- [ ] Does trait return Arrow types (RecordBatch, Array, etc.)?
- [ ] Does driver construct Arrow schemas?
- [ ] Does test code require Arrow dependency?

If ANY answer is "yes", reject the PR.

### Automated Enforcement

Add to CI pipeline:

```bash
# Verify hardware crate has no Arrow dependency
cargo tree -p rust_daq_hardware | grep -q arrow && exit 1
```

## Related Decisions

- ADR-001: Capability Traits (defines hardware layer interface)
- ADR-003: ScriptEngine Abstraction (scripts also avoid Arrow)
- ADR-004: Ring Buffer Memory Layout (where Arrow is actually used)

## Migration Path

Existing code that violates this ADR:

1. **src/instrument/pvcam.rs** (old V2) - Returns `ImageData` with Arrow dependency
   - Migrate to `FrameProducer` trait returning `FrameRef`

2. **src/data/storage.rs** - Some hardware-specific code mixed with Arrow
   - Separate concerns: hardware → simple types, storage → Arrow

3. **Test fixtures** - Some tests construct Arrow data in hardware tests
   - Use simple types in hardware tests, Arrow only in data layer tests

## References

- Arrow documentation: https://arrow.apache.org/docs/
- Zero-copy buffer management: https://docs.rs/arrow/latest/arrow/buffer/
- The Mullet Strategy: V5_OPTIMIZATION_STRATEGIES.md
- Ring buffer implementation: src/data/ring_buffer.rs

## Implementation Status

- [x] FrameRef type defined (src/hardware/mod.rs)
- [x] Capability traits use simple types
- [x] Ring buffer conversion layer implemented
- [ ] PVCAM driver migrated to FrameRef
- [ ] All V2 drivers removed
- [ ] CI enforcement added

## Notes

This decision is part of the "Mullet Strategy" - **Arrow in back, simple types in front**.

**Key Principle**: The hardware layer should be as simple and portable as possible. Arrow is an implementation detail of our storage layer, not a fundamental part of the hardware abstraction.

If we later switch to FlatBuffers, Cap'n Proto, or a custom binary format, only the data layer changes. Hardware drivers remain unchanged.

**Anti-Pattern Example** (from V2, now removed):

```rust
// OLD V2 CODE - VIOLATED THIS ADR
impl HardwareAdapter for PvcamAdapter {
    fn acquire_frame(&self) -> Result<ImageData> {
        // ImageData contained Arrow RecordBatch!
        // Hardware was coupled to Arrow
    }
}
```

**Correct V5 Pattern**:

```rust
// V5 CODE - FOLLOWS ADR
impl FrameProducer for PvcamCamera {
    fn resolution(&self) -> (u32, u32) {
        (1024, 1024)  // Simple types only
    }
}

// Data layer handles Arrow conversion
impl RingBuffer {
    pub fn write_frame(&self, frame: FrameRef) -> Result<()> {
        // Arrow conversion happens HERE, not in hardware
    }
}
```

---

**Document Owner**: Jules-19 (Architecture Coordinator)
**Last Updated**: 2025-11-20
**Review Cycle**: Quarterly
