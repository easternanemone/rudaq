# crate: `driver-dover-motion`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-dover-motion/
  - docs/reference/dover-motion-api.md
  - docs/reference/driver-capability-matrix.md
see-also:
  - ../drivers/dover-motion.md
  - ./dover-motion-sys.md
-->

**Role:** Dover Motion SmartStage driver via MotionSynergyAPI FFI.

**Status: EXPERIMENTAL.** Not wired into `driver-registry` as of the
2026-03-13 capability matrix. Requires the vendor SDK (Windows /
specialized targets). Wiring + feature gate are open work.

**Factory:** `DoverAxisFactory` (`driver_type = "dover_axis"`).

**Capabilities:** `Movable`, `Parameterized`, `TriggerOnPosition`.

**Deployment target:** **Windows** (Dover Motion SDK; USB / Ethernet).

**Paired sys crate:** `dover-motion-sys`.

**Note:** `TriggerOnPosition` is only provided by this driver — no mock
or universal equivalent exists.
