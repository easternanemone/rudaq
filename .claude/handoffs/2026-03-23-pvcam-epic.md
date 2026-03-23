# PVCAM Prime BSI Epic (bd-oqo7) — Handoff

## Date: 2026-03-23
## Branch: main (HEAD: 0a1813ba)
## Epic: bd-oqo7 (2/10 complete, 1 deferred)
## Plan: .claude/plans/typed-enchanting-pixel.md

---

## What Was Done This Session

### Completed and merged to main:
- **bd-oqo7.5** (trigger/timing): Closed as already-implemented — driver already has 47 Parameter<T> fields including all trigger/timing params. NotebookLM confirmed PARAM_EDGE_TRIGGER (ST133-only) and PARAM_SHTR_GATE_MODE (intensifier-only) don't apply to Prime BSI.
- **bd-oqo7.6** (I/O & diagnostics): Merged and hardware-verified on maitai. Added 7 new FeatureManager methods (get_io_bitdepth, get_io_type, get/set_logic_output, get/set_logic_output_invert, get_controller_alive, get_ccs_status, get_exp_min_time), 2 new enums (IoType, LogicOutput), 5 manual PARAM constants in pvcam-sys.
- **bd-oqo7.10** (PrimeLocate): Deferred to 2026-06-01 — not needed for LIBS.

### Beads triage completed:
- All 8 remaining tasks have corrected descriptions reflecting the true scope (most are 30-80% done)
- bd-oqo7.8 unblocked by removing wrong dependencies on .1 and .2
- Priorities adjusted: .4 and .7 downgraded to P3, .9 to P3

### Key discoveries:
1. **The driver is far more complete than task descriptions suggest** — 47 Parameter<T> fields, full speed table management, temperature control, Smart Streaming mode enum, metadata state, host summing params all exist
2. **PVCAM SDK PARAM constants are complex C macros** that bindgen can't parse — must define manually in `pvcam-sys/src/lib.rs` using encoding `type << 24 | class << 16 | ordinal`
3. **`Parameter<T>::with_write_callback` requires `T: Into<f64> + From<f64>`** — bool and u32 params must use Parameter<f64> with conventions (0.0/1.0 for bool)
4. **NotebookLM "rust-daq manuals" notebook** (id: 207c009c-830c-4edb-871f-1fe4ded98e65) has the PVCAM SDK Programming Manual — use it for parameter details, enum values, and SDK patterns

### Agent work completed but not integrated:
- **bd-oqo7.3** (PrimeEnhance): Agent completed full implementation in worktree but code was lost on cleanup. Agent output transcript preserved at `/private/tmp/claude-501/-Users-briansquires-code-rust-daq/1a56405d-96e4-4ce3-a072-45922754567b/tasks/a564646ba8df89d2a.output` (may be lost on reboot). **However**, the PP infrastructure already exists in `features/mod.rs` (list_pp_features, set_pp_param, set_pp_feature_enabled, reset_pp_features) — the agent was duplicating existing work. The real task is just wiring these into Parameters + config.

---

## Implementation Plan (approved, in .claude/plans/typed-enchanting-pixel.md)

### Phase 1: Quick Wins (parallel, ~4 hours each)
- **bd-oqo7.8** — SurrealDB persistence: Connect Parameter writes to `param_change_tx` broadcast channel. Server already has `spawn_parameter_state_writer()` and `DeviceParamState` schema. Add recovery on startup.
- **bd-oqo7.7** — Host summing: Include `summing_count` in Frame metadata when summing enabled. Parameters already exist.

### Phase 2: P1 Features (sequential, ~8-12 hours each)
- **bd-oqo7.2** — Frame metadata: Wire `pl_md_frame_decode` into frame loop. FFI methods exist in `ffi_safe.rs`. `MetadataState` exists with atomic flag.
- **bd-oqo7.1** — SMART Streaming: Expose exposure time list. `smart_stream_enabled`, `smart_stream_mode`, `SmartStreamMode` enum all exist. Need exposure list API.

### Phase 3: PrimeEnhance (~8 hours)
- **bd-oqo7.3** — Wire `list_pp_features()` + `set_pp_feature_enabled()` into Parameter system at init. Add `PrimeEnhanceConfig` to TOML.

### Phase 4: Multi-ROI (~12 hours, can defer)
- **bd-oqo7.4** — `pl_exp_setup_cont_multi_roi()` FFI exists. Need MultiRoi config, buffer sizing, frame loop switch.

### Phase 5: State Machine (~6 hours)
- **bd-oqo7.9** — Use `get_controller_alive()` + `get_ccs_status()` for SurrealDB lifecycle tracking.

---

## Hardware Context

- **maitai** (Tailscale: 100.117.5.12, SSH: `maitai@100.117.5.12`)
  - Prime BSI camera (PVCAM)
  - Daemon running: `./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_universal.toml`
  - Build: `bash scripts/ops/build-maitai.sh`
  - Tests: `cargo nextest run -p driver-pvcam --features pvcam_sdk` (stop daemon first for hardware tests)
  - 22/22 non-hardware tests pass; 8 hardware tests fail with LIBUSB_ERROR_BUSY when daemon is running

---

## Critical Files

| File | Purpose |
|------|---------|
| `crates/driver-pvcam/src/lib.rs` (2239 lines) | Main driver, PvcamCamera struct, Parameter fields, constructor |
| `crates/driver-pvcam/src/components/features/mod.rs` (3343 lines) | FeatureManager with all SDK methods: get/set for every param category |
| `crates/driver-pvcam/src/components/features/enums.rs` (830 lines) | All PVCAM enum types with from_pvcam()/to_pvcam() |
| `crates/driver-pvcam/src/components/acquisition/frame_loop.rs` | Frame acquisition loop — where metadata decode + summing would wire in |
| `crates/driver-pvcam/src/components/acquisition/ffi_safe.rs` | FFI safe wrappers including pl_md_frame_decode, multi-ROI setup |
| `crates/driver-pvcam/src/components/acquisition/metadata.rs` | MetadataState with atomic enable flag |
| `crates/driver-pvcam/src/macros.rs` (424 lines) | `pvcam_parameters!` declarative macro for Parameter generation |
| `crates/driver-pvcam/pvcam-sys/src/lib.rs` | FFI bindings + manual PARAM constants (bindgen can't parse PVCAM macros) |
| `crates/driver-pvcam/src/config.rs` | PvcamConfig TOML deserialization |
| `crates/db/src/config_store.rs` | DeviceParamState, batch_upsert, get_device_state for SurrealDB persistence |
| `crates/server/src/grpc/hardware_service/mod.rs` | spawn_parameter_state_writer() pattern |

---

## Verification Checklist (for each feature)

```bash
cargo check -p driver-pvcam                                              # Local build
cargo clippy -p driver-pvcam --all-targets -- -D warnings                # Clippy
cargo nextest run -p driver-pvcam                                        # Local tests
cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web  # WASM
ssh maitai@100.117.5.12 "cd ~/code/rust-daq && git pull && bash scripts/ops/build-maitai.sh"  # SDK build
ssh maitai@100.117.5.12 "cd ~/code/rust-daq && cargo nextest run -p driver-pvcam --features pvcam_sdk"  # SDK tests
```

---

## Gotchas

1. **pvcam-sys constants**: When adding new PARAM_ constants, they must be manually defined in `pvcam-sys/src/lib.rs` with `#[cfg(feature = "pvcam-sdk")]` guard. Use encoding: `type << 24 | class << 16 | ordinal`. Types: 1=i16, 4=f64, 6=u16, 7=u32, 9=enum, 11=bool, 13=char_ptr.

2. **Parameter<T> write callbacks**: `with_write_callback` requires `T: Into<f64> + From<f64>`. For bool params, use `Parameter<f64>` with 0.0/1.0. For u32, use `Parameter<f64>` and cast.

3. **Feature patterns**: The FeatureManager uses explicit getter/setter methods with `#[cfg(feature = "pvcam_sdk")]` guards and mock fallbacks, NOT the `pvcam_parameters!` macro (which is used in lib.rs for Parameter struct fields).

4. **Hardware tests**: The daemon holds the camera USB device. Stop it before running hardware tests: `ssh maitai "pkill rust-daq"`. Restart after: `ssh maitai "cd ~/code/rust-daq && nohup ./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_universal.toml &"`.

5. **Agent worktree isolation**: Always use `isolation: "worktree"` for parallel agents. The post-commit hook at `.git/hooks/post-commit` auto-pushes agent branches to origin. BUT worktree cleanup can delete commits before they're pushed — check `git branch -r` after agent completion.
