# Legacy SCPI/TCP Driver Migration — COMPLETE

This document archives the completed migration from legacy native SCPI/TCP drivers to universal TOML drivers.

## Scope (Archived)

Migration scope was limited to SCPI/TCP-style serial drivers. All such drivers have been replaced
with `driver-universal` TOML manifests. Native SDK drivers (PVCAM, Andor, Comedi, Dover) were not
in scope and remain as dedicated crates with FFI bindings.

## Legacy -> Universal Mapping (Archived)

All legacy driver types have been replaced. For reference:

| Legacy driver type (removed) | Universal replacement (active) | Manifest location |
|---|---|---|
| `ell14` | `universal_thorlabs_ell14` | `config/devices/ell14.toml` |
| `maitai` | `universal_spectra-physics_maitai` | `config/devices/maitai.toml` |
| `newport1830_c` | `universal_newport_1830-c` | `config/devices/newport_1830c.toml` |
| `esp300` | `universal_newport_esp300` | `config/devices/esp300.toml` |
| `thorlabs_pm400` | `universal_thorlabs_pm400` | `config/devices/thorlabs_pm400.toml` |
| `red_pitaya_pid` | `universal_red_pitaya_pid` | `config/devices/red_pitaya_pid.toml` |

**Note:** The `driver-thorlabs`, `driver-newport`, `driver-spectra-physics`, `driver-red-pitaya`,
`driver-generic`, and `drivers` metacrate have been permanently removed from the workspace.

## Runtime Validation Behavior

At daemon startup, driver types are classified as universal (`universal_*`), native-exception (PVCAM, Andor, Comedi, Dover), or unrecognized:

1. In `universal` / `hybrid-db` modes: unrecognized driver types cause a startup error.
2. In `native` / `custom` modes: unrecognized driver types emit a warning but startup continues.
3. Runtime policy summary prints driver classification counts.

## Migration Workflow (Archived — Completed)

The following workflow was used to complete the migration:

1. ✅ Switched profile to universal equivalent (`--runtime-mode universal` or universal TOML config).
2. ✅ Validated command/status parity in advanced panel widgets.
3. ✅ Ran matrix/integration checks:
   - no-db universal-only
   - hybrid camera-native + universal
   - db-on metadata parity
4. ✅ Completed hardware runbook signoff on maitai before defaulting production launchers.
5. ✅ Deleted legacy serial driver crates from workspace.

## Migration Timeline (COMPLETE)

**All 5 phases completed:**

1. **Phase 1 (complete)**: warning-only, migration documentation and runbook in place.
2. **Phase 2 (complete)**: universal is the recommended default in operator workflows.
3. **Phase 3 (complete)**: legacy SCPI/TCP required explicit opt-in for one release cycle.
4. **Phase 4 (complete)**: in `universal` / `hybrid-db` modes, legacy SCPI/TCP drivers are no longer
   accepted. Only `universal_*` and native-exception drivers (PVCAM, Andor, Comedi, Dover) are
   recognized in these modes. A `--runtime-mode native` rollback path remains for exceptional cases.
5. **Phase 5 (complete)**: Legacy serial driver crates (`driver-thorlabs`, `driver-newport`,
   `driver-spectra-physics`, `driver-red-pitaya`, `driver-generic`, `drivers` metacrate) fully
   deleted from workspace. All serial/TCP/SCPI devices now use `driver-universal` TOML manifests
   exclusively. Only native SDK drivers (PVCAM, Andor, Comedi, Dover) retain dedicated crates with
   FFI bindings.

## Rollback Policy (Archived — No Longer Applicable)

Legacy rollback path has been removed. If issues occur with universal drivers:

1. Debug/fix the TOML manifest in `config/devices/`.
2. File issue with:
   - affected device manifest path
   - startup logs
   - command/response traces
3. Use `driver-mock` for temporary workarounds if device unavailable.

**No legacy native mode available** — all serial/TCP/SCPI devices use `driver-universal` exclusively.

## GUI Panel Dispatch for Universal Drivers

Universal drivers serve their GUI control panel configuration over gRPC via the `ui_schema_json` field in `DeviceMetadata`. This is the **Priority 0** dispatch path — when present, it overrides both local TOML config cache (Priority 1) and hardcoded capability-based panels (Priority 2).

### How it works

1. **TOML manifest** (`config/devices/*.toml`): The `[ui.control_panel]` section defines panel layout and sections (sensor, parameter, custom_action, status_display, etc.)
2. **UniversalDriverFactory** (`factory.rs`): Serializes the `[ui]` section to JSON and stores it in `DeviceMetadata.ui_schema_json`
3. **gRPC transport**: The `ui_schema_json` field flows through the registry to `HardwareService/ListDevices` responses
4. **GUI dispatch** (`dispatch.rs:try_grpc_ui_config`): Deserializes the JSON back to `ControlPanelConfig` and renders a `ConfigDrivenPanel`

### Command-driven parameters

Universal drivers don't implement the `Parameterized` trait (their parameters are defined declaratively in TOML, not as typed `Parameter<T>` with compile-time registries). For parameter sections that need read/write, use `read_command` and `write_command` fields:

```toml
[[ui.control_panel.sections]]
type = "parameter"
label = "Repetition Rate"
parameter = "rep_rate"
widget = "spinner"
read_command = "read_rep_rate"   # Uses ExecuteDeviceCommand RPC
write_command = "set_rep_rate"   # Uses ExecuteDeviceCommand RPC
```

These bypass `GetParameter`/`SetParameter` RPCs and route through `ExecuteDeviceCommand` instead, using the command names defined in the manifest's `[commands]` section.

### Validation lessons learned (2026-02-25)

End-to-end pipeline validation across 4 layers (TOML files, factory serialization, gRPC transport, GUI deserialization + dispatch) revealed that:

- **Stale daemon processes**: Always check for existing daemon processes before starting new ones during validation. A previous debug-build daemon holding port 50051 served outdated `ui_schema_json` without `read_command`/`write_command` fields, despite the TOML and binary being correct. Use `ps aux | grep rust-daq-daemon` or `lsof -i :50051` to verify.
- **Unit tests are necessary but not sufficient**: The TOML-to-JSON roundtrip test passed locally, the dispatch tests passed, but the stale process issue was only discoverable by querying the running daemon via `grpcurl`.
- **Validate the running system**: After deploying changes, always query the actual gRPC response to confirm the full pipeline works end-to-end.

Validation command:
```bash
grpcurl -plaintext -import-path crates/protocol/proto -proto daq.proto \
  localhost:50051 daq.HardwareService/ListDevices | \
  python3 -c "import json,sys; [print(json.dumps(json.loads(d['metadata']['uiSchemaJson']),indent=2)) for d in json.load(sys.stdin)['devices'] if 'uiSchemaJson' in d.get('metadata',{})]"
```

## Current State (Post-Migration)

**Workspace structure (26 crates):**
- **Native SDK drivers (4):** `driver-pvcam` (+`pvcam-sys`), `driver-andor-sdk3` (+`andor-sdk3-sys`), `driver-comedi` (+`comedi-sys`), `driver-dover-motion` (+`dover-motion-sys`)
- **Universal/mock drivers (3):** `driver-universal`, `driver-mock`, `driver-registry`
- **Removed crates (6):** `driver-thorlabs`, `driver-newport`, `driver-spectra-physics`, `driver-red-pitaya`, `driver-generic`, `drivers` metacrate

**Device manifests (config/devices/*.toml):**
- `ell14.toml` (replaces `driver-thorlabs`)
- `maitai.toml` (replaces `driver-spectra-physics`)
- `newport_1830c.toml`, `esp300.toml` (replaces `driver-newport`)
- `red_pitaya_pid.toml` (replaces `driver-red-pitaya`)
- `thorlabs_pm400.toml` (declarative power meter)

## Support Policy (Post-Migration)

- **New SCPI/TCP devices:** MUST use `driver-universal` TOML manifests in `config/devices/`.
- **Native SDK drivers:** PVCAM, Andor, Comedi, Dover retain dedicated crates (cannot be replaced by manifests).
- **Rollback:** Legacy native mode removed; no rollback path available.
- **Bug fixes:** Universal path only; legacy crates deleted.
