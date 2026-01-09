# Driver Plugins Research Summary

**One-liner:** Use TOML-based protocol configs with enum_dispatch factory pattern for 10x performance over trait objects, reserving scripting for edge cases only.

**Version:** v1

**Date:** 2026-01-08

## Key Findings

1. **Hybrid architecture is optimal:** Fully config-driven state machines lose type safety; fully coded drivers lack flexibility. The sweet spot is TOML protocol definitions + code-generated trait implementations.

2. **enum_dispatch provides 10x performance over trait objects:** Verified via crate benchmarks. For a closed set of known drivers, enum-based dispatch eliminates vtable lookups and enables compiler optimizations.

3. **No existing Rust crate supports pure config-driven FSM generation:** smlang, statig, and rust-fsm all require Rust macro invocations. Config can define structure, but code generation is needed.

4. **PyMeasure and ophyd provide proven patterns:** Property-based APIs with validators (PyMeasure) and component composition with lifecycle hooks (ophyd) translate well to Rust traits.

5. **TOML preferred over YAML:** Better type safety, native Rust ecosystem support (Cargo, figment), and simpler parsing rules. Already used in rust-daq.

## Recommended Architecture

```
TOML Config Files (devices/*.toml)
         |
         v
[Compile-time proc-macro or runtime parser]
         |
         v
GenericSerialDriver implements Movable, Readable, etc.
         |
         v
enum_dispatch factory creates typed drivers
         |
         v
DeviceRegistry holds heterogeneous driver collection
```

## Decisions Needed

1. **Scope of MVP:** Should Phase 1 support only ELL14 migration, or include a second driver (ESP300) for pattern validation?

2. **Config file location:** `config/devices/*.toml` or `drivers/*.toml`?

3. **Scripting support:** Include Rhai fallback in Phase 1, or defer to Phase 4?

4. **Binary protocol priority:** Address Modbus/binary protocols in Phase 1, or defer?

## Blockers

- None identified. All recommended crates are actively maintained and async-compatible.

## Next Step

Create `driver-plugins-plan.md` with detailed implementation plan, including:
- TOML schema specification
- GenericSerialDriver struct design
- Factory and registry interfaces
- Migration path for existing ELL14 driver
- Test strategy

## Research Document

Full findings: [driver-plugins-research.md](./driver-plugins-research.md)
