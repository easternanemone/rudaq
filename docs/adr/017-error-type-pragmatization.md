# ADR-017: Error Type Pragmatization — DaqError Enum vs MS Struct Pattern

## Status

Evaluation complete. Partial adoption recommended.

## Context

Microsoft's Pragmatic Rust Guidelines (M-ERRORS-CANONICAL-STRUCTS) recommend that
library errors be **structs** containing a `Backtrace`, an optional upstream cause,
and `is_xxx()` accessor methods — not enums. ErrorKind enums, if used, should be
`pub(crate)` to avoid exposing internal failure modes.

rust-daq currently uses `DaqError`, a 30-variant `thiserror` enum, as its canonical
error type. Three related guidelines are relevant:

- **M-ERRORS-CANONICAL-STRUCTS**: Errors should be structs, not enums
- **M-APP-ERROR**: Only applications may use `anyhow`; libraries must define own types
- **M-PANIC-IS-STOP**: Panics are not errors; 2,816 unwrap/expect in library code

## Current State

### Error types
- `DaqError` — 30-variant enum (public), created via `#[derive(Error)]`
- `DriverError` — struct with `driver_type`, `kind`, `message` (already MS-compliant)
- `StorageError` — struct with `kind`, `message` (already MS-compliant)
- `ErrorKind` — 33-variant public enum for gRPC metadata

### Usage footprint
- 57 `Err(DaqError::Variant(...))` construction sites across 15 crates
- 40 match arms that destructure DaqError variants
- 65 test assertions on specific error variants
- `anyhow::Result` used in all 24 capability trait methods (intentional design)

### What works well today
1. `DriverError` and `StorageError` are already MS-compliant structs
2. Error mapping at the gRPC boundary (`error_mapping.rs`) is well-designed
3. The `ErrorKind` enum provides typed gRPC metadata round-tripping
4. `anyhow::Result` in capability traits gives driver authors ergonomic flexibility

## Analysis

### Full migration cost (DaqError enum → struct)

A complete migration would require:
- Changing 57 error construction sites to `DaqError::new(ErrorKind::Config, "...")`
- Rewriting 40 match arms to use `if err.is_config()` or `err.kind()` patterns
- Updating 65+ test assertions
- Converting all `#[from]` impls to manual `From` impls that capture Backtraces
- Touching 15+ crates

**Estimated effort**: 2-3 days of focused work, high risk of regressions.

### Incremental approach (recommended)

The MS guidelines are about **preventing new technical debt**, not mandating a
rewrite. A pragmatic path:

#### Phase 1: Add Backtrace to existing types (low risk)
- Add `backtrace: Backtrace` field to `DriverError` and `StorageError` structs
- These are already structs, so this is additive
- Implement `std::error::Error::provide()` for backtrace propagation
- DaqError variants that wrap these structs get backtraces "for free"

#### Phase 2: Make ErrorKind pub(crate) (medium risk)
- Currently `ErrorKind` is public for the gRPC metadata round-trip
- Add `is_config()`, `is_driver()`, `is_storage()` etc. methods to `DaqError`
- Add `kind() -> ErrorKind` accessor (keeps the enum internal but queryable)
- Downstream code migrates from `match err.kind { ... }` to `err.is_xxx()`
- **Caution**: `ClientError::daq_error_kind()` returns `Option<ErrorKind>` for
  the gRPC round-trip — this needs an alternative (e.g., return string)

#### Phase 3: Evaluate DaqError struct migration (future)
- After Phase 1-2, assess whether full struct migration adds enough value
- The thiserror enum pattern is well-understood in the Rust ecosystem
- The MS struct recommendation is strongest for libraries consumed externally;
  rust-daq's error types cross an internal gRPC boundary, not a crate API

### anyhow in capability traits (keep as-is)

The `anyhow::Result` return type in capability traits is documented and intentional
(error.rs:23-46). Driver authors need flexibility to use their own error types
internally. The gRPC boundary is where `anyhow::Error` gets downcast to structured
types. This is the correct boundary — library code doesn't surface `anyhow` to
end users.

MS guideline M-APP-ERROR says "crates in your own repository exclusively used from
your application may use anyhow." Since all rust-daq crates are internal to this
repository and consumed only by the daemon binary, this is within spec.

## Decision

1. **Adopt Phase 1** (Backtrace) — bd-6co3g, unblocked now
2. **Defer Phase 2** (pub(crate) ErrorKind) — defer until after unwrap reduction
3. **Defer Phase 3** (struct migration) — evaluate after Phase 1-2 experience
4. **Keep anyhow in traits** — within M-APP-ERROR scope for internal crates

## Consequences

- Backtraces will be available for DriverError/StorageError chains (aids debugging)
- ErrorKind remains public for now (pragmatic: gRPC round-trip needs it)
- DaqError remains an enum (ecosystem-standard thiserror pattern)
- No breaking API changes needed for Phase 1

## References

- MS Rust Guidelines: M-ERRORS-CANONICAL-STRUCTS, M-APP-ERROR
- `crates/common-traits/src/error.rs` — error hierarchy
- `crates/server/src/grpc/error_mapping.rs` — gRPC conversion
- `crates/client/src/error.rs` — client-side extraction
