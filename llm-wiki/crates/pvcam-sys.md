# crate: `pvcam-sys` (nested under `driver-pvcam`)

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-pvcam/pvcam-sys/Cargo.toml
see-also:
  - ./driver-pvcam.md
-->

**Role:** Raw FFI bindings to the PVCAM C SDK.

**Location:** `crates/driver-pvcam/pvcam-sys/` (nested, not a top-level
sibling).

**Edition:** `2021` (still — the sys crates haven't been migrated to
edition 2024 alongside the rest of the workspace).

**Feature-gated** by the parent `driver-pvcam` crate's flags. Only
compiled when PVCAM SDK paths are available.

**Rules:**

- No business logic here — binding surface only.
- `unsafe` is expected; safety is established in the parent crate.
- Keep in sync with vendor header versions via `build.rs` + `bindgen`.
