# driver: Dover Motion (SmartStage)

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-dover-motion/
  - docs/reference/dover-motion-api.md
  - docs/reference/driver-capability-matrix.md
see-also:
  - ../crates/driver-dover-motion.md
-->

**Vendor:** Dover Motion. **SDK:** MotionSynergyAPI (C++).
**Crate:** `driver-dover-motion` + paired `dover-motion-sys`.
**Feature flags:** *(not yet wired)*.
**Host:** Windows (Dover Motion SDK not available on Linux).

## Capabilities

- `Movable`
- `Parameterized`
- `TriggerOnPosition` — **only driver providing this capability**.

## Status: EXPERIMENTAL

As of the 2026-03-13 capability matrix, `driver-dover-motion` is **not
wired** into `driver-registry`. Integration requires:

1. Wire `DoverAxisFactory` into `driver-registry/src/lib.rs` behind a new feature gate (`dover` / `dover_sdk` / `dover_hardware`, by analogy).
2. Expose the feature from `bin` / workspace `Cargo.toml`.
3. Provide a mock or universal equivalent for `TriggerOnPosition` so CI can run tests without the real SDK.
4. Update the capability matrix.

File a bead before starting this work.

## Connection

USB or Ethernet (depending on controller firmware). API details in
`docs/reference/dover-motion-api.md`.
