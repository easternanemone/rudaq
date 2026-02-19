# Device Metadata Contract for Advanced Control Panels

This document defines the runtime metadata contract used by advanced control panel composition.

## Canonical Fields

1. `DeviceInfo.capabilities` (repeated string)
   - Canonical capability list.
   - UI and services must gate behavior from this list, not deprecated boolean flags.

2. `DeviceInfo.metadata.available_commands` (repeated string)
   - Catalog of command names exposed by the device driver/factory.
   - Used by advanced command widgets to build action/status controls.

3. `DeviceInfo.metadata.ui_schema_json` (optional string)
   - JSON serialization of a driver-defined UI schema.
   - For universal TOML drivers, this is sourced from the manifest `[ui]` section.
   - UI may use this for quick-add hints (for example `status_display.summary_params`).

## Contract Rules

1. Capability gating
   - Command execution UI must require `commandable` capability.
   - Publishing `available_commands` does not override capability requirements.

2. Command catalog stability
   - Command names should be stable across driver versions where possible.
   - Drivers should publish all command names that can be executed via `execute_device_command`.

3. UI schema behavior
   - `ui_schema_json` is advisory metadata.
   - Missing or malformed schema must not break panel rendering.
   - Runtime behavior remains capability-driven even when UI schema is absent.

## Driver Responsibilities

1. Universal TOML drivers
   - Publish `available_commands` from manifest command keys.
   - Publish `ui_schema_json` from `[ui]` when present.

2. Native drivers (camera, SDK-backed)
   - Publish `available_commands` when `Commandable` is implemented.
   - Keep command names aligned with actual command handler match arms.

## Integration Points

- Factory introspection: `hardware::registry::FactoryInfo`
- Runtime metadata: `common::driver::DeviceMetadata` -> `hardware::registry::DeviceMetadata`
- gRPC exposure: `protocol::daq::DeviceMetadata`
- UI consumption: advanced command/status widget panel in `crates/ui/src/app.rs`
