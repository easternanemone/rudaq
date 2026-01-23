# Handoff: Technical Debt Epic bd-4asi

**Project:** rust-daq (Rust-based data acquisition system for scientific instrumentation)
**Epic:** bd-4asi - Technical Debt: Driver & Scripting Infrastructure
**Created:** 2026-01-23
**Validated by:** Codex code review

## Quick Start for Claude.ai

Paste this document into a new Claude.ai conversation along with relevant source files. This epic is self-contained and requires no session context.

---

## Goal

Fix 15 technical debt issues in the driver and scripting infrastructure, validated by external AI code reviewers (Codex and Gemini).

## Repository Structure

```
rust-daq/
├── crates/
│   ├── daq-core/              # Foundation types, DriverFactory trait
│   ├── daq-hardware/
│   │   ├── src/drivers/
│   │   │   └── generic_serial.rs  # H1, M4 issues
│   │   └── src/factory.rs         # H3, M1, M5, L2 issues
│   ├── daq-scripting/
│   │   ├── src/bindings.rs        # M2, L1 issues
│   │   └── src/shutter_safety.rs  # M3, L4 issues
│   ├── daq-driver-spectra-physics/
│   │   └── src/maitai.rs          # H2 issue
│   ├── daq-driver-newport/
│   │   └── src/newport_1830c.rs   # M6 issue
│   └── daq-driver-thorlabs/
│       └── src/ell14.rs           # M7 issue
└── config/devices/                # L3 issue
```

---

## Implementation Order (Critical Dependencies)

```
H1 (timeout fix) ──────────────────────────────────> R1 (refactor)
                                                      │
H3 (capability gating) ──> M4 (config params)         │
                                                      │
M2, M3 (shutter safety) ──────────────────────────────┘ (independent)
M1 (serial settings) ─────────────────────────────────┘ (independent)
```

---

## Issues by Priority

### HIGH PRIORITY

#### H1: GenericSerialDriver Silent Timeout Failures
**File:** `crates/daq-hardware/src/drivers/generic_serial.rs`
**Lines:** 600-669, 827-907
**Problem:** `transaction()` and `transaction_with_timeout()` return `Ok("")` on timeout instead of error
**Impact:** Masks hardware disconnects, causes downstream parsing errors
**Fix:**
```rust
// Before: returns Ok("") on timeout
// After: return timeout error when response buffer is empty
if response.is_empty() {
    return Err(anyhow!("Timeout: no response from device"));
}
```
**Tests to update:** Any tests that expect `Ok("")` on timeout

#### H3: DeviceComponents/Capabilities Mismatch
**File:** `crates/daq-hardware/src/factory.rs`
**Lines:** 520-536, 656-667
**Problem:** Builds DeviceComponents with all traits unconditionally regardless of config
**Impact:** Registry advertises unsupported capabilities, runtime errors on invocation
**Fix:** Construct DeviceComponents based on declared `device.capabilities` or presence of `trait_mapping.*`:
```rust
let mut components = DeviceComponents::new();
if config.capabilities.contains(&Capability::Movable) || config.trait_mapping.movable.is_some() {
    components = components.with_movable(driver.clone());
}
// ... repeat for other capabilities
```

---

### MEDIUM PRIORITY

#### H2: MaiTai Wavelength Tuning Race Condition
**File:** `crates/daq-driver-spectra-physics/src/maitai.rs`
**Lines:** 252-308 (attach_wavelength_callbacks), 576-577 (set_wavelength)
**Problem:** Returns after 50ms without verifying laser reached target wavelength
**Impact:** Scripts take data at wrong wavelength during fast scans
**Fix:** Add polling-based settle confirmation:
```rust
pub async fn set_wavelength_and_wait(&self, target: f64, tolerance: f64, timeout: Duration) -> Result<()> {
    self.set_wavelength(target).await?;
    let deadline = Instant::now() + timeout;
    loop {
        let current = self.query_wavelength().await?;
        if (current - target).abs() < tolerance {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(anyhow!("Wavelength settle timeout"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

#### M1: Serial Settings Ignored in Factory
**File:** `crates/daq-hardware/src/factory.rs`
**Lines:** 618-624
**Problem:** Hardcodes 8N1/no flow control, ignores config values like `data_bits`, `parity`
**Fix:** Map `DeviceConfig.connection` into tokio_serial builder

#### M2: Shutter Registry Lifecycle Gaps
**File:** `crates/daq-scripting/src/bindings.rs`
**Lines:** 743-758
**Problem:** Registers in ShutterRegistry before opening; doesn't unregister on failure
**Fix:** Register AFTER successful `open_shutter()` or use scope guard to unregister on failure

#### M3: HeartbeatShutterGuard State Inconsistency
**File:** `crates/daq-scripting/src/shutter_safety.rs`
**Lines:** 381-419, 470-479
**Problem:** Watchdog closes shutter but doesn't update `is_open`; `close()` doesn't stop watchdog
**Fix:** Set `is_open = false` when watchdog closes; have `close()` abort watchdog task

#### M4: Non-Numeric Config Parameters Ignored
**File:** `crates/daq-hardware/src/drivers/generic_serial.rs`
**Lines:** 201-206
**Problem:** Only loads numeric defaults via `as_f64()`, drops strings/booleans
**Fix:** Store parameters as `toml::Value` or `evalexpr::Value`, or validate and reject unsupported types

#### M5: Memory Leak in Factory Introspection
**File:** `crates/daq-hardware/src/factory.rs`
**Lines:** 556-569
**Problem:** `Box::leak` on every `driver_type()`, `name()`, `capabilities()` call
**Fix:** Use `OnceLock` or leak once in constructor, cache `&'static` references

---

### LOW PRIORITY

#### M6: Newport 1830-C Performance Bottleneck
**File:** `crates/daq-driver-newport/src/newport_1830c.rs`
**Lines:** 348-357
**Problem:** Sends `U1` before every `D?` read (~100ms overhead)
**Note:** Intentional for robustness against front-panel unit changes
**Fix:** Make configurable or reassert only on cadence/failure

#### M7: ELL14 Fragile Bus Logic
**File:** `crates/daq-driver-thorlabs/src/ell14.rs`
**Lines:** 817-934
**Problem:** "3x silence" drain adds ~15ms latency per command
**Note:** RS-485 handling is well-designed for noisy buses
**Fix:** Make drain behavior configurable, gate behind "shared bus" flag

#### L1: Blocking Sleep in Scripting
**File:** `crates/daq-scripting/src/bindings.rs`
**Lines:** 662-666
**Problem:** Uses `std::thread::sleep` in `read_averaged`
**Fix:** Use `tokio::time::sleep` in async helper

#### L2: Duplicate DriverFactory Concepts
**Files:** `factory.rs:140-348` vs `daq-core/src/driver.rs:473-520`
**Fix:** Rename hardware struct or factor shared mapping into single helper

#### L3: Binary Commands Not Implemented
**File:** `config/devices/modbus_example.toml:66-140`
**Fix:** Mark as unsupported in docs or gate under feature flag

#### L4: Flaky Timing Tests
**File:** `crates/daq-scripting/src/shutter_safety.rs:580+`
**Fix:** Use `#[tokio::test(start_paused = true)]` with `tokio::time::advance()`

---

### REFACTORING

#### R1: Extract Shared Serial Buffer Draining Logic
**Problem:** "Aggressive buffer draining" duplicated in 3/4 drivers
**Fix:** Extract into shared `SerialUtils` trait or `Rs485Bus` utility
**Dependency:** Complete H1 first

---

## Positives to Preserve

- Clear safety documentation in shutter_safety.rs
- Defense-in-depth layering in scripting bindings
- Guarded `run_blocking` to avoid runtime deadlocks
- Correct signed integer handling in ELL14
- Proper `spawn_blocking` for serial port opens
- Strong protocol docs/logging in MaiTai and Newport drivers
- Robust RS-485 handling (prefix scanning, response framing)

---

## Build & Test Commands

```bash
# Build
cargo build

# Test all
cargo nextest run

# Test specific crate
cargo nextest run -p daq-hardware
cargo nextest run -p daq-scripting

# Lint
cargo clippy --all-targets

# Format
cargo fmt --all
```

---

## Beads Tracking

Issues in this epic should be tracked in beads:
```bash
bd show bd-4asi           # View epic
bd update <id> --status in_progress  # Start work
bd close <id>             # Complete work
bd sync                   # Push changes
```
