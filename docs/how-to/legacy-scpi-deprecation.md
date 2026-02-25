# Legacy SCPI/TCP Driver Deprecation Plan

This guide defines the migration path from legacy native SCPI/TCP drivers to universal TOML drivers.

## Scope

Deprecation scope is limited to SCPI/TCP-style drivers where universal equivalents exist.
Camera-native and SDK-bound drivers are not in scope.

## Legacy -> Universal Mapping

| Legacy driver type | Universal replacement |
|---|---|
| `ell14` | `universal_thorlabs_ell14` |
| `maitai` | `universal_spectra-physics_maitai` |
| `newport1830_c` | `universal_newport_1830-c` |
| `esp300` | `universal_newport_esp300` |
| `thorlabs_pm400` | `universal_thorlabs_pm400` |

## Runtime Warning Behavior

At daemon startup, when a deprecated legacy SCPI/TCP driver type is present:

1. A warning is logged with the replacement driver type.
2. Runtime policy summary prints native non-camera counts.
3. Startup continues (non-breaking).

## Migration Workflow

1. Switch profile to universal equivalent (`--runtime-mode universal` or universal TOML config).
2. Validate command/status parity in advanced panel widgets.
3. Run matrix/integration checks:
   - no-db universal-only
   - hybrid camera-native + universal
   - db-on metadata parity
4. Complete hardware runbook signoff on maitai before defaulting production launchers.

## Proposed Timeline

1. Phase 1 (complete): warning-only, migration documentation and runbook in place.
2. Phase 2 (complete): universal is the recommended default in operator workflows.
3. Phase 3 (current): legacy SCPI/TCP requires explicit opt-in (`--allow-legacy-drivers` or `RUSTDAQ_ALLOW_LEGACY_DRIVERS=1`).
4. Phase 4: remove legacy SCPI/TCP path after hardware signoff and release communication.

## Rollback Policy

If universal/hybrid rollout regresses operations:

1. Relaunch daemon with `--runtime-mode native` or explicit native `--hardware-config`.
2. Export/backup DB state if `hybrid-db` was active.
3. File regression issue with:
   - runtime mode used
   - startup policy log lines
   - affected devices and commands

## Support Policy

- During warning-only phase, legacy path remains supported for operational continuity.
- New development for SCPI/TCP instruments should target universal manifests.
- Bug fixes prioritize universal path first, legacy path best-effort.
