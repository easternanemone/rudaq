# Plugin System P0 Fixes

**Date:** 2025-11-29
**Issues:** bd-22si.2.4, bd-22si.2.5, bd-22si.4.4, bd-22si.6.3, bd-22si.1.4
**Status:** Completed

## Summary

This document records the P0 fixes applied to the plugin system to make it compile. These fixes addressed fundamental Rust patterns that were incorrectly applied in the initial implementation.

## Changes Made

### 1. Trait Object Fix (bd-22si.2.4)

**Problem:** The original code used `Box<dyn AsyncRead + AsyncWrite + Send + Sync + Unpin>` which Rust doesn't allow - only auto traits (Send/Sync) can be combined with non-auto traits in trait objects.

**Solution:** Replaced with concrete type `tokio_serial::SerialStream` wrapped in `Mutex`:

```rust
// Before (broken):
port: Box<dyn AsyncInstrumentPort>

// After (working):
port: Mutex<SerialStream>
```

**Rationale:** Using a concrete type is simpler and matches the current use case. When TCP support is added in Phase 5, we can introduce generics or an enum.

### 2. Borrow Checker Fix (bd-22si.2.5)

**Problem:** All capability methods took `&mut self` because `execute_command` required mutable access to the port. This conflicted with config lookups that borrowed `self.config` immutably.

**Solution:** Changed all methods from `&mut self` to `&self` using interior mutability:

```rust
// Before:
async fn execute_command(&mut self, command: &str) -> Result<String> {
    self.port.write_all(...).await??;  // Needs &mut self
}

// After:
async fn execute_command(&self, command: &str) -> Result<String> {
    let mut port = self.port.lock().await;  // Interior mutability
    port.write_all(...).await??;
}
```

**Benefits:**
- Config lookups and command execution no longer conflict
- `GenericDriver` can be shared via `Arc<GenericDriver>` for capability handles
- Matches V5 architecture patterns used elsewhere in rust-daq

### 3. Feature Flag (bd-22si.4.4)

**Problem:** The plugin module depended on `tokio_serial` which is behind a feature flag, but the module was always compiled.

**Solution:** Added feature gate to `src/hardware/mod.rs`:

```rust
#[cfg(feature = "tokio_serial")]
pub mod plugin;
```

**Additional fixes:**
- Fixed `Duration` import in registry.rs
- Removed unused `PathBuf` import

### 4. UI Removal (bd-22si.6.3)

**Problem:** `ui.rs` implemented egui widgets, which contradicts V5's headless-first architecture. It also had fundamental async/sync mismatch issues.

**Solution:** Deleted `ui.rs` entirely. UI will be implemented via gRPC metadata exposure in Phase 6, allowing the remote Slint GUI to render controls dynamically.

### 5. Import Cleanup (bd-22si.1.4)

**Changes:**
- Removed unused `std::collections::HashMap` from schema.rs
- Removed unused `std::sync::Arc` from driver.rs (will be re-added for Handle pattern)
- Added module-level documentation to all plugin files

## Files Modified

| File | Changes |
|------|---------|
| `src/hardware/plugin/driver.rs` | Complete rewrite of struct and all methods |
| `src/hardware/plugin/registry.rs` | Updated to match new GenericDriver API |
| `src/hardware/plugin/schema.rs` | Removed unused import, added docs |
| `src/hardware/plugin/mod.rs` | Removed ui module, added docs |
| `src/hardware/mod.rs` | Added feature flag |
| `src/hardware/plugin/ui.rs` | **Deleted** |

## Verification

```bash
# Compiles successfully with tokio_serial feature
cargo check --features tokio_serial
```

## Next Steps

The following work remains for the plugin system (see bd-22si):

1. **Phase 3: Capability Trait Implementations** (bd-22si.3)
   - Implement Handle pattern (bd-22si.3.6)
   - Create `PluginAxisHandle`, `PluginSensorHandle`, etc.
   - Implement `Movable`, `Readable`, `Settable` traits on handles

2. **Phase 5: Advanced Features** (bd-22si.5)
   - TCP/IP protocol support
   - Rhai scripting for complex sequences
   - Plugin hot-reload (development mode)

3. **Phase 6: UI Integration** (bd-22si.6)
   - Expose plugin metadata via gRPC
   - Remote GUI rendering

4. **Phase 7: Testing & Documentation** (bd-22si.7)
   - Unit tests with mock serial
   - Plugin creation user guide

## Architecture Decisions

### Why Concrete Type Over Trait Object?

Trait objects with `AsyncRead + AsyncWrite` are notoriously difficult due to `Pin<&mut Self>` requirements. Using `SerialStream` directly:
- Simplifies the code
- Avoids `Pin` complexity
- Can be generalized later with an enum when TCP is needed

### Why Interior Mutability?

The `Mutex<SerialStream>` pattern:
- Allows `&self` methods throughout
- Enables `Arc<GenericDriver>` sharing for capability handles
- Matches existing V5 patterns in rust-daq
- Prevents borrow conflicts between config reads and port writes

### Why Delete UI Instead of Fix?

1. V5 architecture is headless-first
2. egui is being deprecated in favor of remote Slint GUI
3. The async/sync mismatch was fundamental, not fixable with minor changes
4. gRPC-based approach is more flexible and aligns with architecture

## References

- [Plugin System Research](plugin_system_research.md)
- [V5 Architecture](ARCHITECTURAL_FLAW_ANALYSIS.md)
- Gemini CLI consultation (verified fixes on 2025-11-29)
