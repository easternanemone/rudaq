> [!WARNING] **ARCHIVAL / HISTORICAL**
> This document is a historical snapshot and is preserved for context. It does not represent current operational guidance or source-of-truth architecture.

# Test Suite Overhaul Plan

> Authored via 3 Codex planning sessions (2026-02-24).
> Targets the hybrid-db architecture that is becoming the default runtime.

## Prerequisites

This plan assumes the following PRs are merged first:
- **PR #379** (bd-9n9k.7): Adds CI matrix rows `runtime / universal-smoke`,
  `runtime / hybrid-db-mem-smoke`, `runtime / hybrid-db-rocksdb-smoke`, and
  creates `runtime_mode_smoke.rs` (the existing smoke test file).
- **PR #381** (bd-9n9k.2): Flips the default runtime from `mock` to `hybrid-db`.

**Naming clarification:** `runtime_mode_smoke.rs` is an *existing* file from
PR #379. `runtime_mode_resolution.rs` is a *new* file proposed below. They
are distinct files with different purposes.

## Motivation

The default daemon runtime is flipping from `mock` to `hybrid-db` (PR #381).
The existing test suite was written around the legacy serial-driver architecture
and has significant gaps in hybrid-db, ConfigService, and watch-reconciler
coverage. This plan specifies concrete files, tests, and retirement actions.

---

## 1. New Files to Create

### `crates/bin/tests/runtime_mode_resolution.rs`

**Purpose:** Process-level runtime-mode resolution precedence tests.
**Feature gate:** None for mock/env tests; `#[cfg(feature = "db-surreal-mem")]`
for hybrid/default-start success tests.

| Test Function | Description |
|---------------|-------------|
| `test_runtime_mode_cli_overrides_env` | Spawn daemon with `--runtime-mode mock` + `RUSTDAQ_RUNTIME_MODE=hybrid-db`; assert mock path wins |
| `test_runtime_mode_env_override_applies_when_cli_absent` | Spawn daemon with env only; assert selected mode reflects env |
| `test_runtime_mode_hardware_config_implies_custom_without_cli_or_env` | Spawn with `--hardware-config <temp profile>`; assert custom path |
| `test_runtime_mode_invalid_env_falls_back_to_default` | Invalid env value; assert warning/fallback behavior |
| `test_runtime_mode_default_hybrid_db_starts_from_workspace_root` | (db-surreal-mem) No runtime flag, CWD=workspace root; assert hybrid-db startup succeeds |

**Implementation notes:**
- Reuse `start_daemon` helper pattern from `golden_lifecycle.rs`
- Must set `.current_dir(workspace_root())` for hybrid-db default tests
  since `config/maitai_universal.toml` resolves relative to CWD

### `crates/integration-tests/tests/grpc_config_service_e2e.rs`

**Purpose:** Standalone integration tests for ConfigService gRPC surface
(split from the 37KB `surrealdb_daemon_e2e.rs` monolith).
**Feature gate:** `#![cfg(all(feature = "db-surreal-mem", feature = "server"))]`

| Test Function | Description |
|---------------|-------------|
| `test_grpc_config_service_list_drivers_after_shadow_write` | Seed DB via shadow-write, assert `ListDrivers` returns enriched metadata |
| `test_grpc_config_service_instrument_upsert_get_roundtrip` | `UpsertInstrument` then `GetInstrument`; assert config/enablement parity |
| `test_grpc_config_service_db_info_health_counts` | Assert `GetDbInfo` health + driver/instrument counts |
| `test_grpc_config_service_import_export_roundtrip` | Export TOML, import into fresh DB; assert instrument count restored |
| `test_grpc_config_service_subscribe_config_changes_emits_upsert` | Subscribe stream emits mutation event after upsert |

**Implementation notes:**
- ConfigService has no direct driver-upsert RPC; use shadow-write seed + `ListDrivers`
- Existing unit suite in `crates/server/src/grpc/config_service.rs` covers
  internal logic; these tests target the full gRPC wire path

### `crates/bin/tests/hybrid_db_watch_reconciler_e2e.rs`

**Purpose:** Daemon subprocess + gRPC E2E for SurrealDB LIVE SELECT watch reconciler.
**Feature gate:** `#![cfg(feature = "db-surreal-mem")]`

| Test Function | Description |
|---------------|-------------|
| `test_watch_reconciler_adds_device_after_configservice_upsert` | Upsert via ConfigService; assert device appears via `ListDevices` |
| `test_watch_reconciler_removes_device_after_configservice_delete` | Delete via ConfigService; assert device disappears |
| `test_watch_reconciler_restarts_device_after_config_change` | Modify config JSON; assert device remains with updated metadata |
| `test_watch_reconciler_ignores_unknown_driver_type` | Invalid driver upsert ignored; baseline device count unchanged |

**Implementation notes:**
- The watch reconciler uses SurrealDB LIVE SELECT (not filesystem inotify/kqueue)
- Must allow short polling interval for change propagation in tests

---

## 2. Existing Files to Modify

### `crates/bin/tests/golden_lifecycle.rs`

- Add workspace-root CWD helper to `start_daemon` spawn
- Add hybrid/default golden process smoke tests (gated `db-surreal-mem`):
  - `test_golden_default_runtime_hybrid_db_startup_shutdown` — no `--runtime-mode`, assert startup + graceful SIGINT shutdown
  - `test_golden_default_runtime_registers_devices` — no `--runtime-mode`, assert device registration logs present

### `crates/integration-tests/tests/runtime_metadata_matrix.rs`

Keep as metadata-focused suite (do NOT merge into `runtime_mode_smoke.rs`). Add:
- `matrix_universal_factory_info_matches_device_commands` — `factory_info().available_commands` == device metadata command catalog
- `matrix_universal_device_capabilities_match_factory_info` — top-level `device.capabilities` matches factory capabilities set
- `matrix_hybrid_universal_factory_info_present_for_universal_devices` — all universal devices in hybrid profile have factory info + populated commands

### `crates/integration-tests/tests/surrealdb_daemon_e2e.rs`

Shrink the 37KB monolith:
- Move T4 and T5-T8 ConfigService cases into new `grpc_config_service_e2e.rs`
- Retain T1-T3, T9, T10-T13 and shared helpers until common helper extraction lands

### `crates/integration-tests/tests/common/mod.rs`

Add shared helpers reused by new ConfigService/watch tests:
- `workspace_root()` — resolve workspace root from `CARGO_MANIFEST_DIR`
- `mock_maitai_lab_path()` — path to mock maitai lab profile
- `load_mock_maitai_config()` — parse mock maitai profile
- `shadow_write_mock_maitai_config()` — seed DB with mock config
- `setup_config_service_with_mock_maitai_db()` — full ConfigService harness

### `crates/integration-tests/tests/hardware_serial_tests.rs`

Split out legacy MaiTai protocol blocks; keep generic serial transport tests only.

### `crates/integration-tests/tests/grpc_parameter_integration.rs`

Replace legacy MaiTai-specific parameter PTY test with universal
manifest-backed parameterized device case.

### `crates/integration-tests/tests/module_exports_test.rs`

Move legacy serial export assertions to compat section/file;
add universal registry/factory export assertions.

---

## 3. Legacy Files to Retire

These files test the legacy serial-driver crate path that has been superseded
by `driver-universal` TOML manifests. Each will be replaced by coverage in
`hardware_universal_driver_validation.rs` + `runtime_metadata_matrix.rs`.

| File | Size | Reason | Replacement |
|------|------|--------|-------------|
| `hardware_elliptec_validation.rs` | 88KB | Legacy ELL/elliptec crate path | `hardware_universal_driver_validation.rs` + `runtime_metadata_matrix.rs` |
| `hardware_ell14_protocol_features.rs` | 48KB | Legacy ELL14 protocol-specific tests | `hardware_universal_driver_validation.rs` (manifest + mock transport) |
| `hardware_esp300_validation.rs` | 27KB | Legacy ESP300 driver crate path | `hardware_universal_driver_validation.rs` |
| `hardware_maitai_validation.rs` | 32KB | Legacy Spectra Physics/MaiTai driver path | `hardware_universal_driver_validation.rs` |
| `hardware_newport1830c_validation.rs` | 33KB | Legacy Newport power-meter serial driver | `hardware_universal_driver_validation.rs` |

**Total legacy code to retire: ~228KB**

> **Strategy:** Do not delete until universal replacements are green and merged.
> Tag each with `#[deprecated]` attribute or `// LEGACY:` marker first.

---

## 4. Feature Gate Mapping

| File | No gate | `db-surreal-mem` | `server` | `universal` |
|------|---------|-------------------|----------|-------------|
| `runtime_mode_resolution.rs` | CLI/env precedence tests | default/hybrid startup | | |
| `grpc_config_service_e2e.rs` | | x | x | |
| `hybrid_db_watch_reconciler_e2e.rs` | | x | | |
| `golden_lifecycle.rs` (new tests) | | x | | |
| `runtime_metadata_matrix.rs` (new tests) | x | | | |

---

## 5. CI Matrix Impact

### Required: Add 1 new row

```yaml
- name: "bin / hybrid-db-mem-e2e"
  package: bin
  features: db-surreal-mem
  cache-suffix: db-surreal-mem
```

This runs:
- `hybrid_db_watch_reconciler_e2e.rs`
- Gated golden default/hybrid tests in `golden_lifecycle.rs`
- Runtime mode resolution tests that need DB

### No new integration-tests rows needed

Existing rows cover the new tests automatically:
- `runtime / hybrid-db-mem-smoke` compiles `grpc_config_service_e2e.rs`
- `runtime / universal-smoke` compiles metadata enrichment tests

### Optional

```yaml
- name: "bin / runtime-resolution"
  package: bin
  features: ""
  cache-suffix: runtime-resolution
```

Only needed if runtime precedence tests aren't covered by the default
`Format, lint, and tests` workflow.

---

## Execution Order

1. **Phase 1 (P0):** Create `runtime_mode_resolution.rs` + golden_lifecycle additions
2. **Phase 2 (P1):** Create `grpc_config_service_e2e.rs` + extract from surrealdb monolith
3. **Phase 3 (P1):** Create `hybrid_db_watch_reconciler_e2e.rs`
4. **Phase 4 (P2):** Add metadata enrichment assertions to `runtime_metadata_matrix.rs`
5. **Phase 5 (P3):** Retire legacy files (after P1-P4 are green)

## Risks

1. **CWD dependency:** Default hybrid-db daemon tests fail if subprocess CWD
   is not workspace root (`config/maitai_universal.toml` resolves relatively)
2. **No driver-upsert RPC:** ConfigService tests must seed via shadow-write +
   verify through `ListDrivers`
3. **Watch reconciler timing:** LIVE SELECT propagation may need short sleep/retry
   in tests to avoid flaky assertions
4. **Legacy retirement scope:** 228KB of legacy tests is a large deletion; must
   ensure universal coverage is comprehensive before removal
