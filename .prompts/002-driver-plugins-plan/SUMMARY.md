# Driver Plugins Plan Summary

**One-liner:** TOML-driven hardware protocol definitions with enum_dispatch factory pattern, enabling user-extensible serial drivers without Rust code changes.

**Version:** v1

---

## Phase Overview

| Phase | Objective | Duration | Key Deliverable |
|-------|-----------|----------|-----------------|
| **1. Core Infrastructure** | Config schema, validation, loading | 1-2 weeks | `DeviceConfig` structs + serde_valid + schemars |
| **2. GenericSerialDriver + ELL14** | Working config-driven driver MVP | 2-4 weeks | `GenericSerialDriver` + `config/devices/ell14.toml` |
| **3. ESP300 + Capabilities** | Pattern validation across devices | 1-2 weeks | ESP300, Newport1830C, MaiTai configs |
| **4. State Machines** | Init sequences, error recovery | 2-4 weeks | smlang integration, `[init_sequence]` support |
| **5. Scripted Extensions** | Rhai fallback for edge cases | 1-2 weeks | Script sandbox, custom parsers |
| **6. Binary Protocols** | Modbus and custom framing (future) | 3-4 weeks | Binary frame builder |

---

## Key Decisions Made

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Config format** | TOML | Type safety, Rust ecosystem alignment, already used in rust-daq |
| **Dispatch method** | `enum_dispatch` | 10x faster than `dyn Trait`, zero heap allocation |
| **Validation** | `serde_valid` + `schemars` | Integrated validation + IDE schema support |
| **State machines** | `smlang` (config-defined structure) | Declarative structure with compile-time safety |
| **Expression evaluation** | `evalexpr` crate | Mature, well-tested for unit conversions |
| **MVP scope** | ELL14 first, ESP300 second | Different protocol styles validate pattern generality |
| **Config location** | `config/devices/*.toml` | Consistent with existing `config/config.v4.toml` |
| **Scripting** | Rhai (deferred to Phase 5) | Most devices don't need it; provides escape hatch |
| **Binary protocols** | Deferred to Phase 6 | No current hardware requires Modbus |

---

## Assumptions Needing Validation

1. **enum_dispatch + async_trait compatibility** - Research indicates compatibility, verify in Phase 2
2. **Regex performance for response parsing** - Assumed < 1ms per parse; benchmark if issues arise
3. **evalexpr adequacy for conversions** - All existing drivers use simple formulas; may need Rhai for complex cases
4. **User comfort with TOML** - Schema validation + IDE support should help non-programmers

---

## Blockers

- None identified. All dependencies are available crates.

---

## Success Criteria

- [ ] User can add a new serial device by creating `config/devices/<device>.toml` only
- [ ] Existing capability traits (Movable, Readable, etc.) work with config-based drivers
- [ ] No Rust code changes required for standard serial protocols
- [ ] Compile-time schema validation via schemars
- [ ] Clear error messages when config is malformed
- [ ] No performance regression vs hand-coded drivers

---

## Next Step

**Execute Phase 1 - Core Infrastructure**

1. Add `serde_valid`, `schemars`, `enum_dispatch` to `daq-hardware/Cargo.toml`
2. Create `crates/daq-hardware/src/config/mod.rs` with `DeviceConfig` schema structs
3. Implement config loader and validation
4. Generate `config/schemas/device.schema.json`
5. Write unit tests for config parsing

See `driver-plugins-plan.md` for detailed Phase 1 tasks.
