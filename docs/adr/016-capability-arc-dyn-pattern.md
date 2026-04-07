# ADR-016: Capability-Based Dynamic Dispatch via Arc\<dyn Trait\>

## Status

Accepted

## Context

Microsoft's Pragmatic Rust Guidelines (M-AVOID-WRAPPERS) recommend avoiding smart
pointers like `Arc<T>`, `Box<T>`, and `Rc<T>` in public APIs. The guidelines argue
these are implementation details that introduce infectious complexity.

rust-daq's `DeviceComponents` struct exposes 24 capability traits as
`Option<Arc<dyn Trait>>` fields (e.g., `pub movable: Option<Arc<dyn Movable>>`),
and `DeviceRegistry` returns `Option<Arc<dyn Trait>>` from typed getter methods.
This is a deliberate architectural choice, not an oversight.

## Decision

We accept the `Arc<dyn Trait>` pattern for device capabilities as an **intentional
deviation** from M-AVOID-WRAPPERS, documented here per guideline requirements.

## Rationale

### Why concrete types don't work

The set of devices is unknown at compile time. A `DeviceRegistry` may contain a
PVCAM camera, an Andor spectrograph, a Comedi DAQ card, and a mock stage — all
discovered from TOML config files at runtime. No single concrete type can represent
"something that can move" across this heterogeneous set.

### Why generics don't work

Generics would infect every consumer with type parameters:

```rust
// This would require every function touching devices to be generic
// over every possible capability combination — combinatorial explosion.
struct RunEngine<M: Movable, R: Readable, F: FrameProducer, ...> { ... }
```

With 24 capability traits, this is not viable. The MS guidelines themselves
acknowledge this: "Once generics become a nesting problem, `dyn Trait` can be
considered" (M-DI-HIERARCHY).

### Why Arc specifically

`Arc` (not `Box` or `Rc`) is required because:

1. **Multi-holder sharing**: The same camera may be accessed by RunEngine (for
   acquisition), the gRPC server (for parameter queries), and the safety sentinel
   (for emergency shutdown) — concurrently, from different tasks.
2. **Send + Sync**: Tokio tasks require `Send + Sync`. `Arc<dyn Trait>` satisfies
   both; `Rc` does not, and `Box` prevents sharing.
3. **Cheap cloning**: `DeviceComponents` is cloned when building `StreamConfig`
   and other transient structures. Arc clone is O(1).

### Mitigations for wrapper complexity

1. **Typed accessors hide Arc**: Most consumers use `registry.get_movable("stage_1")`
   which returns `Option<Arc<dyn Movable>>` — the Arc is visible but unavoidable.
   Consumers call methods directly on `&*arc` without unwrapping nested types.
2. **Builder pattern**: `DeviceComponents::new().with_movable(arc).with_readable(arc)`
   keeps construction clean.
3. **No nesting**: The pattern is `Arc<dyn Trait>` (one level), never
   `Arc<Mutex<Box<dyn Trait>>>` or similar.

## Consequences

- Consumers must handle `Option<Arc<dyn Trait>>` when accessing capabilities.
- Adding a new capability requires a new field on `DeviceComponents` and a new
  getter on `DeviceRegistry`.
- The `async_trait` crate is retained for capability traits because native async
  fn in trait only supports static dispatch, but `Arc<dyn Trait>` requires dynamic
  dispatch via boxed futures.

## Alternatives Considered

1. **Enum dispatch**: A `Device` enum with variants for each driver type. Rejected:
   new drivers would require modifying the enum (not extensible for plugins).
2. **Type-erased map**: `HashMap<TypeId, Box<dyn Any>>`. Rejected: loses
   compile-time type safety, requires downcasting at every access.
3. **ECS-style component storage**: Store capabilities in typed columns. Rejected:
   excessive complexity for ~50 devices; ECS shines at thousands of entities.

## References

- MS Rust Guidelines: M-AVOID-WRAPPERS, M-DI-HIERARCHY
- `crates/common-traits/src/driver.rs` — `DeviceComponents` definition
- `crates/hardware/src/registry.rs` — `DeviceRegistry` typed getters
