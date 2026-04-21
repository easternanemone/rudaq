# crate: `common`

<!--
last-ingested: 2026-04-19
sources:
  - crates/common/Cargo.toml
  - crates/common/src/
see-also:
  - ../concepts/parameter.md
  - ../concepts/device-id.md
-->

**Role:** Foundation. Shared types, error handling, reactive parameters,
size limits, module domain types.

**Key exports (verify against `lib.rs` on use):**

- `Parameter<T>`, `Observable<T>` — reactive state. See [`../concepts/parameter.md`](../concepts/parameter.md).
- `DeviceId` — `Arc<str>`-backed identity. See [`../concepts/device-id.md`](../concepts/device-id.md).
- Error types (library errors as `thiserror` variants).
- `limits.rs` — size / rate caps.

**Dependents:** ~every other crate in the workspace. Changes here have broad
blast radius — prefer extending in `common-traits` first when possible.

**Notes:**

- Previously contained capability traits; those have been extracted to `common-traits` but re-exported.
- Removed items (replaced): `DataPoint` (→ `Observable<T>` / `Parameter<T>`).
