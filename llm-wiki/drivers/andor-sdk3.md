# driver: Andor SDK3 (iStar + Shamrock)

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-andor-sdk3/
  - docs/reference/driver-capability-matrix.md
  - ANDOR_SDK_FIXES.md
see-also:
  - ../crates/driver-andor-sdk3.md
  - ../hardware/leabs-dev.md
-->

**Vendor:** Andor. **SDK:** SDK3 (C library).
**Crate:** `driver-andor-sdk3` + paired `andor-sdk3-sys`.
**Feature flags:** `andor` / `andor_hardware`.
**Host:** leabs-dev.

## Two factories

| Factory | `driver_type` | Capabilities |
|---------|---------------|--------------|
| `AndorCameraFactory` | `andor_istar` | `FrameProducer`, `Triggerable`, `ExposureControl`, `Parameterized` |
| `AndorSpectrographFactory` | `andor_shamrock` | `WavelengthTunable`, `ShutterControl`, `Parameterized` |

The iStar camera additionally implements `GatedCamera` (DDG + MCP gain) —
the **only** production source of this capability.

## Integration notes

- Camera + spectrograph share the SDK3 runtime; initialize in a
  coordinated order.
- Echelle-mode acquisitions pair an Andor iStar + Shamrock with the
  `echelle` crate's extraction pipeline.

## Known SDK-layer issues

- See `ANDOR_SDK_FIXES.md` at repo root. Ingest into this page when
  addressing or closing items.
