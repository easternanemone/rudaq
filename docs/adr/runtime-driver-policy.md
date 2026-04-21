# Runtime Driver Policy: Universal vs Native vs DB-backed

Status: Superseded for DB details
Date: 2026-02-19
Owner: rust-daq maintainers

> **Current status (April 2026):** The universal-vs-native driver policy
> remains useful, but every SurrealDB-specific statement below is historical.
> The current control plane is SQLite-only behind the `db` feature.

## Context

The project currently supports multiple runtime paths:

- Universal TOML drivers (`universal_*`) for SCPI/TCP/serial classes.
- Native Rust drivers for hardware that requires SDK-specific behavior (especially cameras).
- Optional SQLite control-plane persistence and reconciliation.

Without an explicit policy, runtime behavior can drift between launch modes and profiles.

## Decision

1. SCPI/TCP/serial instrument classes default to `driver-universal` TOML manifests.
2. Native-exception devices remain native SDK drivers (PVCAM cameras via `driver-pvcam`, Andor cameras via `driver-andor-sdk3`, Comedi DAQ via `driver-comedi`, Dover Motion stages via `driver-dover-motion`).
3. SQLite is the control-plane persistence layer when enabled, but startup from TOML remains supported.
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

## DB Maturity Stages

- Stage A (current default): TOML remains source for startup, SQLite mirrors desired state and enables reconciliation.
- Stage B (transitional): DB-managed updates are primary for mutable desired state, TOML remains bootstrapping/rollback.
- Stage C (future): DB can be authoritative in production profiles with documented restore/export path.

## Startup Logging Contract

Daemon startup must log:

- Config source/path.
- Counts by runtime policy class:
  - `driver-universal` manifest count
  - native SDK driver count (`driver-pvcam`, `driver-andor-sdk3`, `driver-comedi`, `driver-dover-motion`)
  - deprecated native count
- Legacy SCPI/TCP native driver warnings with `driver-universal` manifest replacement hints.

## Backward Compatibility and Rollback

- Existing configs continue to load.
- Native SCPI/TCP driver types emit warnings, not hard failures.
- Rollback from universal/hybrid mode:
  1. Restart with `--runtime-mode native` or `--hardware-config config/maitai_universal.toml`.
  2. Re-import known-good TOML into SQLite if DB-backed mode was used.
  3. Validate device list and command metadata before operations resume.

## Team Sign-Off Checklist

- [ ] Runtime policy logs appear in daemon startup output.
- [ ] UI/CLI mode labels map to expected config profiles.
- [ ] Native SDK driver path is verified (`driver-mock`, `driver-pvcam`, `driver-andor-sdk3`, `driver-comedi`, `driver-dover-motion`).
- [ ] Legacy driver warnings and migration docs are visible to operators.
- [ ] SQLite DB behavior is covered by integration matrix tests.
