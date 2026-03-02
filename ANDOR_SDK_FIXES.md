# ANDOR SDK Fixes / Vendor Requests

This file is a running log of Andor SDK (and SDK-facing integration) quirks observed while
using Andor cameras in `rust-daq`.

Purpose:

- preserve concrete repro details for vendor support requests
- separate confirmed observations from speculation
- track current workarounds in `rust-daq`

When adding a new entry, include:

- observation date (YYYY-MM-DD)
- camera model / host / acquisition mode
- exact observed values
- expected behavior
- impact on acquisition/reduction
- current workaround
- evidence file(s)

## Confirmed Issues

### 1. `AOIWidth` / `AOIHeight` report `1` while streamed frames are full-frame (`2048x2048`)

- Observation date: `2026-02-25`
- Environment:
  - Host: `leabs-dev`
  - Camera: Andor iSTAR sCMOS (`istar_camera`)
  - Spectrograph: Mechelle 5000
  - Illumination: Ocean Optics HG-2 Hg-Ar lamp
  - Acquisition path: `rust-daq-daemon` gRPC stream (`StreamFrames`)
- Observed behavior:
  - `GetParameter(AOIWidth)` returned `"1"`
  - `GetParameter(AOIHeight)` returned `"1"`
  - streamed `FrameData` payloads decoded to `2048 x 2048` frames
  - streamed frame summaries also reported `bit_depth=12`, `COMPRESSION_LZ4`, `uncompressed_size=8388608`
- Expected behavior:
  - AOI parameters should reflect the effective acquisition geometry currently used for streamed frames.
  - If a parameter is unavailable/stale in the current mode, the SDK should report that explicitly (not a plausible but incorrect numeric value).
- Impact:
  - Any client that trusts `AOIWidth` / `AOIHeight` for buffer sizing, calibration profile selection, or axis reconstruction can mis-handle data.
  - Makes automated calibration pipelines brittle because parameter RPC state and frame stream state disagree.
- Current `rust-daq` workaround:
  - treat streamed `FrameData.width` / `FrameData.height` as canonical for captured frames
  - treat AOI parameter values as advisory only when they conflict with streamed frame metadata
- Vendor request:
  - ensure AOI parameter queries return the effective geometry in active acquisition modes
  - or expose an authoritative "effective acquisition configuration" API separate from editable parameters
  - document any mode-dependent semantics where AOI parameters may be placeholders/stale
- Evidence:
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/testdata/echelle/leabs-dev/2026-02-25-hg2/parameters_raw.json`
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/testdata/echelle/leabs-dev/2026-02-25-hg2/manifest.json`
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/testdata/echelle/leabs-dev/2026-02-25-hg2/README.md`

### 2. Streamed frame metadata omits ROI/binning fields needed to interpret geometry

- Observation date: `2026-02-25`
- Environment:
  - Same capture session as issue #1 (`leabs-dev`, iSTAR + Mechelle 5000, Hg-Ar lamp)
- Observed behavior:
  - streamed `FrameData` in the captured session omitted ROI and binning metadata fields:
    - `roi_x`
    - `roi_y`
    - `binning_x`
    - `binning_y`
  - these fields were absent (`null` in our captured JSON artifacts)
- Expected behavior:
  - Every streamed frame (or acquisition start metadata packet) should include the effective ROI origin and binning values used to produce that frame.
- Impact:
  - prevents reliable reconstruction of pixel-to-sensor mapping from the frame stream alone
  - complicates echelle calibration/profile matching (trace and wavelength solutions depend on ROI/binning)
  - makes offline replay/fixtures less self-describing
- Current `rust-daq` workaround:
  - infer geometry from frame dimensions + local capture context when possible
  - document the missing fields in fixture metadata and treat assumptions as provisional
- Vendor request:
  - populate ROI/binning metadata on streamed frames (or an accompanying acquisition metadata structure) for all acquisition modes
  - guarantee consistency between streamed metadata and parameter query values
- Evidence:
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/testdata/echelle/leabs-dev/2026-02-25-hg2/hg2_001ms_frame.json`
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/testdata/echelle/leabs-dev/2026-02-25-hg2/hg2_010ms_frame.json`
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/testdata/echelle/leabs-dev/2026-02-25-hg2/hg2_100ms_frame.json`
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/testdata/echelle/leabs-dev/2026-02-25-hg2/README.md`

### 3. Long-running iSTAR streaming session can terminate daemon with kernel general protection fault

- Observation date: `2026-02-26`
- Environment:
  - Host: `leabs-dev`
  - Camera: Andor iSTAR sCMOS (`istar_camera`)
  - Spectrograph: Mechelle 5000
  - Acquisition path: `rust-daq-daemon` gRPC `StartStream` + `StreamFrames` from local GUI via SSH tunnel
  - GUI behavior at failure: after streaming for some time, UI reports:
    - `Failed to start hardware stream: status: Unknown, message: "transport error", ...`
- Observed behavior:
  - `rust-daq-daemon` process exits unexpectedly during/after streaming activity
  - `/tmp/rust-daq-daemon.log` contains only daemon startup text (no Rust panic message)
  - `journalctl` on `leabs-dev` reports a kernel crash entry:
    - `kernel: traps: tokio-runtime-w[...] general protection fault ... in rust-daq-daemon[...]`
  - After the daemon exits, SSH local-forward attempts log repeated:
    - `sshd[...]: error: connect_to localhost port 50051: failed.`
- Expected behavior:
  - Camera streaming should not terminate the daemon process.
  - SDK/driver faults should be surfaced as recoverable camera errors instead of process-wide memory faults.
- Impact:
  - kills all DAQ services, not just camera streaming
  - UI reconnect logic later surfaces transport errors that obscure the original root cause
  - interrupts live calibration sessions and risks loss of unsaved calibration work
- Current `rust-daq` workaround:
  - manually rebuild/restart daemon on `leabs-dev`
  - treat UI `transport error` after a period of streaming as a possible daemon crash and confirm with `journalctl`
  - capture system logs (`journalctl`) because Rust daemon log may not contain crash details
- Vendor request:
  - investigate iSTAR streaming SDK calls for memory safety / invalid pointer / callback lifetime issues under sustained acquisition
  - provide guidance on thread-safety requirements and callback ownership for long-running streaming
  - provide a reproducible debug mode/logging option for SDK-side streaming faults
- Evidence:
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/ANDOR_SDK_FIXES.md` (this entry; includes exact observed kernel message text)
  - `journalctl` on `leabs-dev` (`2026-02-26 21:09:59`) showing `general protection fault` in `rust-daq-daemon`
  - `/tmp/rust-daq-daemon.log` on `leabs-dev` (startup-only log with no Rust panic output)
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/scripts/leabs-daemon-crash-wrapper.sh` (wrapped daemon crash-capture harness)
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/scripts/repro-istar-stream-crash.sh` (local repro driver with health polling + artifact collection)

## Notes

- This file records observed behavior at the SDK/API boundary from the `rust-daq` integration perspective.
- Root cause may be in the Andor SDK, camera firmware, or mode-specific SDK semantics; vendor confirmation is needed.
- If a future capture disproves an entry, keep the old entry and add a dated follow-up rather than deleting history.
