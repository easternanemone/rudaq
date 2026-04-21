# driver: PVCAM

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-pvcam/
  - docs/reference/pvcam-sdk.md
  - docs/explanation/pvcam-integration-map.md
  - docs/reference/driver-capability-matrix.md
  - .claude/handoffs/2026-03-23-pvcam-epic.md
see-also:
  - ../crates/driver-pvcam.md
  - ../hardware/maitai.md
-->

**Vendor:** Photometrics. **SDK:** PVCAM (C library).
**Crate:** `driver-pvcam` + nested `pvcam-sys` FFI.
**Factory:** `PvcamFactory` (`driver_type = "pvcam"`).
**Feature flags:** `pvcam` / `pvcam_sdk` / `pvcam_hardware`.
**Host:** maitai only.

## Capabilities

- `FrameProducer`
- `Triggerable`
- `ExposureControl`
- `Commandable` (only SDK driver that exposes this)
- `Parameterized`

## Tested cameras

- Prime 95B.
- Prime BSI.

## Construction

```rust
// crates/driver-pvcam/src/lib.rs:353
let driver = Arc::new(PvcamDriver::new_async(camera_name).await?);
```

The factory `PvcamFactory` at line 118 is what the registry invokes from
a TOML `InstrumentConfig` — call the driver directly only in tests or
bespoke integrations. Neither `PvcamDriver::from_config` nor
`PvcamDriver::new` exists in the code; older docs listing them as a
migration path are stale.

## Module map (decomposed 2026-02)

- `lib.rs` — struct + trait impls.
- `components/acquisition/` — frame loop, buffer mgmt, FFI callback context, FFI-safe wrappers.
- `components/features/` — PVCAM feature enumeration + parameter mapping.
- `components/connection.rs` — connection lifecycle.
- `components/frame_pool.rs` — `pool::Pool<T>` integration.
- `components/speed_table.rs` — readout speed table.
- `components/taps.rs` — tap configuration.

## Integration notes

- Uses `pool::Pool<Frame>` for zero-alloc frame delivery.
- Feeds the mmap ring buffer (`storage::RingBuffer`).
- `BorrowGuard` / `BorrowCount` protect against slot reclamation while
  foreign consumers hold frame references.

## Known issues / open work

- See the `.claude/handoffs/2026-03-23-pvcam-epic.md` handoff for the
  latest epic-level status (not yet fully ingested into this page).
- `ANDOR_SDK_FIXES.md` at repo root is Andor-specific despite similar
  SDK-layer concerns; PVCAM-specific follow-ups belong in a bead.
