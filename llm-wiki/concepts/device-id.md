# `DeviceId` — identity

<!--
last-ingested: 2026-04-19
sources:
  - crates/common/src/ (DeviceId type)
  - CLAUDE.md §Architecture (key abstractions)
see-also:
  - ./driver-registry.md
  - ./parameter.md
-->

`Arc<str>`-backed device identity. Cheap to clone, stable across the run.

## Why `Arc<str>` not `String`

- Cheap clone (pointer bump instead of allocation + memcpy).
- Immutable by construction — no accidental mutation.
- `Hash + Eq` by string content so it works as a `HashMap` key.

## Usage conventions

- Strings are lower-snake-case: `mock_camera`, `stage_x`, `laser_1`.
- Must match the `id` field in TOML device configs.
- Unique within a `DeviceRegistry`. Duplicate registration is an error, not a silent shadow.
- Cloning in hot paths is safe and expected.

## Don't

- Don't `.to_string()` on a `DeviceId` unless crossing a boundary that truly needs `String` (protobuf serialization is the common case).
- Don't build ad-hoc strings from `DeviceId + suffix` — if you need a namespaced key, introduce a distinct typed key, don't overload the id.
- Don't compare by `as_ref()` pointer equality — always content equality.

## Related

- [`driver-registry.md`](./driver-registry.md) — the `DeviceRegistry` maps `DeviceId` → `Arc<dyn Capability>`.
- [`parameter.md`](./parameter.md) — every `Parameter<T>` carries the owning `DeviceId` in its metadata for logging and snapshotting.
