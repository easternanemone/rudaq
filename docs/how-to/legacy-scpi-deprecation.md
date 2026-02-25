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

## Runtime Validation Behavior

At daemon startup, driver types are classified as universal (`universal_*`), native-exception (PVCAM, Andor, Comedi, Dover), or unrecognized:

1. In `universal` / `hybrid-db` modes: unrecognized driver types cause a startup error.
2. In `native` / `custom` modes: unrecognized driver types emit a warning but startup continues.
3. Runtime policy summary prints driver classification counts.

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
3. Phase 3 (complete): legacy SCPI/TCP required explicit opt-in for one release cycle.
4. Phase 4 (complete): legacy SCPI/TCP drivers removed. Only `universal_*` and native-exception drivers (PVCAM, Andor, Comedi, Dover) are recognized.

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
