# crate: `hardware`

<!--
last-ingested: 2026-04-19
sources:
  - crates/hardware/Cargo.toml
  - crates/hardware/src/
  - docs/explanation/newcomer-guide.md §Driver/Factory Pattern
see-also:
  - ../concepts/driver-registry.md
  - ../concepts/capability-traits.md
  - ./driver-registry.md
-->

**Role:** Hardware Abstraction Layer. Owns `DeviceRegistry`, config /
schema loading, and the capability-lookup surface.

**Key exports:**

- `DeviceRegistry` — `DashMap<DeviceId, Arc<dyn Capability>>`. Accessors per capability: `get_movable`, `get_readable`, `get_frame_producer`, …
- Config loading for declarative drivers (reads `config/devices/*.toml` via `driver-universal`).
- Schema validation.

**Not here:** concrete driver crates. Feature-gated driver registration
moved to [`driver-registry`](./driver-registry.md) after a refactor.

**Dependents:** `server`, `experiment`, `bin`, `scripting`, `integration-tests`.

**Notes:**

- Removed legacy items: `Ell14Driver` constructors (serial drivers moved to `driver-universal`); `execute_script` free function; `InstrumentConfigV3` type alias.
- `GenericDriver` constructors are `new_serial` / `new_tcp` / `new_mock` in `manifest_driver/driver.rs`; there is no bare `GenericDriver::new`. Older docs listing a `new → new_serial` migration are stale.
- Actually carrying `#[deprecated(since = "0.3.0")]`: `DeviceConfig` (schema v2) → `UniversalDriverConfig` (schema v3); `hardware::registry::create_mock_registry` → `driver_registry::create_canonical_mock_registry(workspace_root)`.
