# crate: `driver-pvcam`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-pvcam/
  - crates/driver-pvcam/pvcam-sys/
  - docs/reference/pvcam-sdk.md
  - docs/explanation/pvcam-integration-map.md
  - docs/reference/driver-capability-matrix.md
  - .claude/handoffs/2026-03-23-pvcam-epic.md
see-also:
  - ../drivers/pvcam.md
  - ./pvcam-sys.md
-->

**Role:** Photometrics PVCAM cameras (Prime 95B, Prime BSI). Safe Rust
layer over the PVCAM C SDK.

**Feature gates:** `pvcam` / `pvcam_sdk` / `pvcam_hardware`.

**Capabilities exposed:** `FrameProducer`, `Triggerable`, `ExposureControl`,
`Commandable`, `Parameterized`.

**Deployment target:** **maitai** (Linux x86_64 + PVCAM SDK).

**Module layout (decomposed 2026-02):**

| Path | Role |
|------|------|
| `lib.rs` | `PvcamDriver` struct + trait impls + entry. |
| `macros.rs` | Macro-generated parameter bindings. |
| `components/acquisition/mod.rs` | Frame acquisition loop, callback handling. |
| `components/acquisition/buffer.rs` | Frame buffer mgmt. |
| `components/acquisition/callback_context.rs` | FFI callback context. |
| `components/acquisition/ffi_safe.rs` | FFI-safe type wrappers. |
| `components/features/mod.rs` | PVCAM feature enumeration + parameter mapping. |
| `components/features/enums.rs` | Feature enum definitions. |
| `components/features/types.rs` | Feature type mappings. |
| `components/connection.rs` | Camera connection lifecycle. |
| `components/frame_pool.rs` | `Pool<T>` integration. |
| `components/speed_table.rs` | Readout speed table. |
| `components/taps.rs` | Camera tap configuration. |

**Current constructor:** `PvcamDriver::new_async(camera_name)` at
`crates/driver-pvcam/src/lib.rs:353` (returns `Result<Self>` via
`async`). The `PvcamFactory` (line 118) is what the registry calls from
a TOML `InstrumentConfig`. There is **no** `PvcamDriver::from_config`
method and **no** `PvcamDriver::new` method — earlier docs claimed a
`new → from_config` deprecation path; grep confirms neither exists. Fix
older docs when you see them.

**Nested sys crate:** `driver-pvcam/pvcam-sys` — see [`pvcam-sys.md`](./pvcam-sys.md).
