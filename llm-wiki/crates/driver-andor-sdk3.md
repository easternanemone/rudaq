# crate: `driver-andor-sdk3`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-andor-sdk3/
  - docs/reference/driver-capability-matrix.md
see-also:
  - ../drivers/andor-sdk3.md
  - ./andor-sdk3-sys.md
-->

**Role:** Safe Rust driver for Andor iStar camera (ICCD) and Shamrock
spectrograph via Andor SDK3.

**Feature gates:** `andor` / `andor_hardware`.

**Factories:**

- `AndorCameraFactory` (`driver_type = "andor_istar"`) — FrameProducer, Triggerable, ExposureControl, Parameterized.
- `AndorSpectrographFactory` (`driver_type = "andor_shamrock"`) — WavelengthTunable, ShutterControl, Parameterized.

**Deployment target:** **leabs-dev** (Linux x86_64 + Andor SDK3).

**Paired sys crate:** `andor-sdk3-sys`.

**Notable:** only source of `GatedCamera` in production (iStar ICCD with
DDG + MCP gain).
