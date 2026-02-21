# Runtime Driver Policy: Universal vs Native vs SurrealDB

Status: Accepted  
Date: 2026-02-19  
Owner: rust-daq maintainers

## Context

The project currently supports multiple runtime paths:

- Universal TOML drivers (`universal_*`) for SCPI/TCP/serial classes.
- Native Rust drivers for hardware that requires SDK-specific behavior (especially cameras).
- Optional SurrealDB control-plane persistence and reconciliation.

Without an explicit policy, runtime behavior can drift between launch modes and profiles.

## Decision

1. SCPI/TCP/serial instrument classes default to universal TOML drivers.
2. Native-exception devices remain native drivers (PVCAM/Andor cameras, Comedi DAQ, Dover Motion stages).
3. SurrealDB is the control-plane persistence layer when enabled, but startup from TOML remains supported.
4. Runtime mode is explicit at launch:
   - `mock`
   - `native`
   - `universal`
   - `hybrid-db`

## Runtime Selection Precedence

1. Explicit CLI/UI mode selection (`--runtime-mode`).
2. Explicit `--hardware-config <path>` when provided.
3. Legacy shorthand (`--lab-hardware`) for native maitai profile.
4. Fallback: mock runtime.

## SurrealDB Maturity Stages

- Stage A (current default): TOML remains source for startup, DB mirrors desired state and enables reconciliation.
- Stage B (transitional): DB-managed updates are primary for mutable desired state, TOML remains bootstrapping/rollback.
- Stage C (future): DB can be authoritative in production profiles with documented restore/export path.

## Startup Logging Contract

Daemon startup must log:

- Config source/path.
- Counts by runtime policy class:
  - universal driver count
  - native exception count (PVCAM, Andor, Comedi, Dover Motion)
  - deprecated native count
- Legacy SCPI/TCP native driver warnings with universal replacement hints.

## Backward Compatibility and Rollback

- Existing configs continue to load.
- Native SCPI/TCP driver types emit warnings, not hard failures.
- Rollback from universal/hybrid mode:
  1. Restart with `--runtime-mode native` or `--hardware-config config/maitai_hardware.toml`.
  2. Re-import known-good TOML into DB if `hybrid-db` was used.
  3. Validate device list and command metadata before operations resume.

## Team Sign-Off Checklist

- [ ] Runtime policy logs appear in daemon startup output.
- [ ] UI/CLI mode labels map to expected config profiles.
- [ ] Native exception path is verified (mock/PVCAM/Andor/Comedi/Dover).
- [ ] Legacy driver warnings and migration docs are visible to operators.
- [ ] SurrealDB mode behavior is covered by integration matrix tests.
