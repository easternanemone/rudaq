# crate: `common-traits`

<!--
last-ingested: 2026-04-19
sources:
  - crates/common-traits/Cargo.toml
  - crates/common-traits/src/lib.rs
see-also:
  - ../concepts/capability-traits.md
  - ../concepts/driver-registry.md
-->

**Role:** Capability traits, `DriverFactory` API, reactive-parameter trait
surfaces. Extracted from `common` so driver crates don't pull the full
`common` dependency tree.

**Key exports:**

- Capability traits (see full list in [`../concepts/capability-traits.md`](../concepts/capability-traits.md)).
- `DriverFactory` — the plugin API each `driver-*` crate implements.
- `Parameterized` trait (intermediate between `common::Parameter<T>` and capability introspection).

**Dependents:** all `driver-*` crates, `hardware`, `driver-registry`,
`experiment`.

**Conventions:**

- Capability traits use `async_trait` (required for `Arc<dyn Trait>`; see [`../invariants.md`](../invariants.md)).
- Trait methods return `Result<T, <crate>::Error>` using `thiserror`.
- Adding a new capability: define here, update `common/capabilities.rs`
  re-exports, bump the capability matrix in
  `docs/reference/driver-capability-matrix.md`.
