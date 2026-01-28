# ADR: Capability System Consolidation

**Status:** Proposed  
**Date:** 2026-01-27  
**Authors:** Gemini 3 Pro Analysis, Brian Squires  
**Epic:** bd-4myc

## Context

The rust-daq system has a Hardware Abstraction Layer (HAL) that bridges two paradigms:

1. **Static Rust Type System** - Uses traits (`Movable`, `Readable`, `Parameterized`) to enforce thread safety and compile-time guarantees
2. **Dynamic Plugin System** - TOML-based declarative drivers that can extend the system without recompilation

This duality has led to capability definitions being spread across multiple layers, creating maintenance burden and silent API failures.

## Problem Statement

Currently, adding a new capability (e.g., "Spectrometer") requires touching **6 files** across 4 architectural layers:

| Layer | File | Change Required |
|-------|------|-----------------|
| Core | `daq-core/src/driver.rs` | Add to `Capability` enum |
| Schema | `daq-hardware/src/plugin/schema.rs` | Add to `CapabilitiesConfig` struct |
| Registry (struct) | `daq-hardware/src/registry.rs` | Add `Option<Arc<dyn Trait>>` field |
| Registry (introspection) | `daq-hardware/src/registry.rs` | Update `capabilities()` method |
| Proto | `daq-proto/proto/daq.proto` | Add `bool is_X = N;` flag |
| gRPC | `daq-server/src/grpc/hardware_service.rs` | Add manual mapping logic |

This violates DRY and creates risk of silent API failures when proto/gRPC layers are not updated.

## Current Architecture: 4-Layer Data Flow

### Layer 1: Configuration (Dynamic)

**Source:** TOML files in `config/devices/`

```toml
# ell14.toml
[device]
name = "ELL14 Rotation Stage"
capabilities = ["Movable", "Parameterized"]  # Metadata hint

[trait_mapping.Movable]
# Structural config - THIS drives behavior
move_abs_cmd = "ma{addr}{pos}"
```

**Key Insight:** The string list `capabilities = [...]` is mostly metadata. Actual behavior is driven by the *presence* of structural sections like `[trait_mapping.Movable]` which populate `Option<MovableCapability>` in `CapabilitiesConfig`.

### Layer 2: Driver Factory (Runtime Instantiation)

**Source:** `daq-hardware/src/registry.rs`

The registry inspects structural config and instantiates Rust trait objects. `Parameterized` is ALWAYS wired for plugins.

### Layer 3: Registry Introspection (Internal API)

**Source:** `daq-hardware/src/registry.rs`

This layer ignores the TOML string list entirely. It checks which Rust trait objects are actually present, outputting a dynamic `Vec<Capability>`.

### Layer 4: gRPC Service (External API)

**Source:** `daq-server/src/grpc/hardware_service.rs`

Manual mapping from `Vec<Capability>` to boolean proto flags. **This is where gaps occur.**

## Identified Gaps

### Gap 1: Missing `is_parameterized` Flag

- **TOML:** Declares `capabilities = ["Movable", "Parameterized"]`
- **Registry:** `GenericDriver` always implements `Parameterized`
- **Proto:** No `is_parameterized` field exists
- **Impact:** Clients cannot discover if `ListParameters` RPC will return data

### Gap 2: Proto Inconsistency Between Services

| Service | Message | Capability Flags |
|---------|---------|------------------|
| HardwareService | `DeviceInfo` | 8 boolean flags, missing Parameterized |
| PluginService | `PluginSummary` | `has_settable`, `has_scriptable`, `has_loggable` |

### Gap 3: Terminology Mismatch

TOML uses "Parameterized", PluginService uses "has_settable", HardwareService is missing it entirely.

## Risks of Current Approach

1. **Silent API Failure** - Adding capability in TOML + Rust does not expose it to clients
2. **Schema Bloat** - `DeviceInfo` grows indefinitely with boolean flags
3. **Client Breakage** - Clients need `.proto` regeneration to see new capabilities
4. **Maintenance Burden** - 6 files across 4 layers for each new capability

## Proposed Solution: String-Based Capability List

Consolidate to a single source of truth: the `Capability` enum in Rust.

### Phase 1: Immediate Fix (bd-4myc.1)

Add missing flag to unblock current work:

```protobuf
// daq.proto
message DeviceInfo {
    bool is_parameterized = 18;  // NEW
}
```

### Phase 2: Architecture Consolidation (bd-4myc.2, bd-4myc.3, bd-4myc.4)

1. Add `repeated string capabilities = 100` to DeviceInfo
2. Mark existing boolean flags as `[deprecated = true]`
3. Implement `Capability::as_str()` for stable serialization
4. Update clients to use string list

## Canonical Capability Strings

| Rust Variant | Canonical String | Proto Legacy Flag | Notes |
|--------------|------------------|-------------------|-------|
| `Movable` | `"Movable"` | `is_movable` | |
| `Readable` | `"Readable"` | `is_readable` | |
| `Triggerable` | `"Triggerable"` | `is_triggerable` | |
| `FrameProducer` | `"FrameProducer"` | `is_frame_producer` | |
| `ExposureControllable` | `"ExposureControllable"` | `is_exposure_controllable` | |
| `ShutterControllable` | `"ShutterControllable"` | `is_shutter_controllable` | |
| `WavelengthTunable` | `"WavelengthTunable"` | `is_wavelength_tunable` | |
| `EmissionControllable` | `"EmissionControllable"` | `is_emission_controllable` | |
| `Parameterized` | `"Parameterized"` | `is_parameterized` (new) | Maps to Plugin "Settable" |
| `Scriptable` | `"Scriptable"` | (none) | Plugin-only |
| `Loggable` | `"Loggable"` | (none) | Plugin-only |

**Casing Contract:** PascalCase. Clients normalize to lowercase for comparison.

## Implementation Warnings

1. **Do NOT use `format!("{:?}", c)`** - Use explicit `as_str()` for stable API contract
2. **Protobuf strings are case-sensitive** - Document canonical casing
3. **Maintain backward compatibility** - Keep deprecated boolean flags during migration

## Benefits of Consolidation

| Aspect | Before | After |
|--------|--------|-------|
| Files to touch for new capability | 6 | 2 (enum + trait) |
| Proto changes needed | Yes | No |
| Silent API failures | Possible | Impossible |

## Decision

**Recommended:** Proceed with phased consolidation to string-based capabilities.

## References

- `crates/daq-core/src/driver.rs` - Capability enum
- `crates/daq-hardware/src/registry.rs` - Registry introspection
- `crates/daq-hardware/src/plugin/schema.rs` - TOML schema
- `crates/daq-server/src/grpc/hardware_service.rs` - Proto translation
- `crates/daq-proto/proto/daq.proto` - DeviceInfo message
