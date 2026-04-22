# Live Echelle Calibration Session (leabs-dev, Mechelle 5000 + Andor iSTAR)

This runbook is the practical, operator-facing sequence for a live calibration
session on `leabs-dev` using:

- Andor iSTAR camera
- Mechelle 5000 spectrograph
- Ocean Optics HG-2 Hg-Ar calibration lamp

It assumes the rust-daq echelle MVP and calibration workspace are available in
the GUI (Image Viewer side panel).

## Session Goals (Single Lab Session)

Minimum acceptable outcomes for one session:

- confirm live stream is a real echellegram (not the prior ramp-pattern failure)
- capture a reproducible flat frame set (continuum source) and Hg-Ar arc frame set
- run the offline 3-pass calibration pipeline to generate a full calibration profile
- load the generated profile in the Image Viewer for visual validation
- verify trace overlays align with visible echelle orders
- verify wavelength solution is plausibly calibrated
- save and export evidence (screenshots, profile file, extracted preview)

Stretch goals:

- manually refine the 3-pass output profile in the GUI (trace/wavelength tweaks)
- generate blaze preview CSV artifact from flat-derived preview
- curate a new golden dataset for repo regression
- compare against external reference tool output (within agreed tolerance)

## Known Andor SDK Caveats (Read First)

Documented SDK quirks observed on this setup are tracked in:

- `ANDOR_SDK_FIXES.md` (from repo root)

Important for this session:

- Treat streamed `FrameData.width/height/bit_depth` as canonical.
- Do not trust `AOIWidth` / `AOIHeight` parameter values alone.
- ROI/binning fields may be absent in `FrameData` on some Andor SDK paths.

## Roles (Recommended)

- Bench operator:
  - lamp control
  - spectrograph/camera physical setup
  - confirms illumination path and source stability
- rust-daq operator (GUI):
  - streaming, capture checks, calibration workspace edits
- Recorder (can be same person):
  - filenames, session notes, anomalies, screenshots

## Pre-Session Checklist (5-10 min)

- Camera cooled and stable
- Mechelle slit / optics configuration recorded
- HG-2 lamp warmed up and stable
- Continuum/flat source available (if doing blaze/flat work)
- `leabs-dev` reachable over SSH
- Local machine has `grpcurl`
- Working tree clean enough for session notes (optional)

Quick connectivity check:

```bash
ssh leabs-dev 'echo ok && hostname && date'
```

## Start rust-daq on leabs-dev

Use the deploy helper (recommended):

```bash
cd <path-to-rust-daq>
bash scripts/deploy/deploy-leabs.sh --skip-build
```

Useful variants:

- daemon only:

```bash
bash scripts/deploy/deploy-leabs.sh --skip-build --daemon-only
```

- GUI only (daemon already running):

```bash
bash scripts/deploy/deploy-leabs.sh --gui-only
```

If you do not use the helper, ensure a daemon is running on `leabs-dev` and a
local tunnel or direct network path is available for the GUI.

## Verify Daemon and Device Availability

If using local GUI + local tunnel, check local daemon endpoint:

```bash
grpcurl -plaintext localhost:50051 daq.HardwareService/ListDevices
```

If probing directly on `leabs-dev`:

```bash
ssh leabs-dev 'grpcurl -plaintext localhost:50051 daq.HardwareService/ListDevices'
```

Expected device IDs include the iSTAR camera (for example `istar_camera`).

List camera parameters:

```bash
grpcurl -plaintext -d '{"device_id":"istar_camera"}' \
  localhost:50051 daq.HardwareService/ListParameters
```

Check exposure via typed RPC:

```bash
grpcurl -plaintext -d '{"device_id":"istar_camera"}' \
  localhost:50051 daq.HardwareService/GetExposure
```

Check a specific observable parameter (example):

```bash
grpcurl -plaintext -d '{"device_id":"istar_camera","parameter_name":"AOIWidth"}' \
  localhost:50051 daq.HardwareService/GetParameter
```

## Session Directory and Naming (Recommended)

Do not commit raw live captures immediately. Capture into a session directory
first, then curate a smaller fixture subset into `testdata/` later.

Suggested local session directory:

```text
<lab-data-root>/echelle/leabs-dev/YYYY-MM-DD-hg2-session-01/
```

Suggested files to record:

- `session_notes.md`
- `device_list.json`
- `camera_parameters_start.json`
- `flat_*.json` / `flat_*_payload_lz4.bin` (if exporting raw frames)
- `arc_hg2_*.json` / `arc_hg2_*_payload_lz4.bin`
- `profile_draft_v1.toml`
- `preview_*.json`, `preview_*.csv`
- screenshots (`.png`)

Suggested mode IDs in notes:

- `mechelle5000-istar-fullframe-bin1x1-hg2-<date>`

## Capture Matrix (Practical Starting Point)

Start with a small matrix and expand only if needed.

### Arc (HG-2)

Capture at least:

- `1 ms`
- `10 ms`
- `100 ms`

If lines saturate or are too dim, add:

- `0.2 ms`, `0.5 ms`, `2 ms`, `5 ms`, `20 ms`, `50 ms`

### Flat / Blaze (continuum source)

Capture enough signal without clipping:

- short exposure (avoid saturation)
- nominal exposure
- near-saturation but not clipped (for diagnostics only)

### Dark / Background (optional but useful)

- same exposure(s) as arc/flat when possible
- shutter/illumination blocked

## Live Sanity Check Before Calibration Editing (Critical)

Before spending time tracing/fitting, verify the frame content is physically
plausible.

### What a Good Arc Frame Should Look Like

- visible curved/tilted echelle orders across the sensor
- line-rich structure within each order
- frame content changes with exposure time (not bitwise-identical)
- no global ramp-only pattern

### Ramp-Pattern Triage (Previous Failure Mode)

Stop calibration work if you see any of the following:

- `10 ms` and `100 ms` frames are visually identical
- only a monotonic ramp / gradient pattern is present
- `1 ms` differs by only a constant offset from longer exposures

If observed:

1. Verify real hardware path (not simulator / test pattern mode).
2. Confirm camera trigger mode and acquisition mode.
3. Confirm intensifier / readout mode state if applicable.
4. Re-check exposure changes are accepted (`GetExposure` and/or `SetExposureResponse` actual value).
5. Document the behavior in:
   - `ANDOR_SDK_FIXES.md`

## Optional gRPC Capture Commands (CLI Spot Checks)

Set exposure:

```bash
grpcurl -plaintext -d '{"device_id":"istar_camera","exposure_ms":10.0}' \
  localhost:50051 daq.HardwareService/SetExposure
```

Start one-frame acquisition:

```bash
grpcurl -plaintext -d '{"device_id":"istar_camera","frame_count":1}' \
  localhost:50051 daq.HardwareService/StartStream
```

Read one streamed frame JSON (manual spot check):

```bash
grpcurl -plaintext -d '{"device_id":"istar_camera","max_fps":0,"quality":"STREAM_QUALITY_FULL"}' \
  localhost:50051 daq.HardwareService/StreamFrames
```

Stop stream if needed:

```bash
grpcurl -plaintext -d '{"device_id":"istar_camera"}' \
  localhost:50051 daq.HardwareService/StopStream
```

Notes:

- `FrameData.data` may be LZ4-compressed and base64-encoded in `grpcurl` output.
- For calibration work, the GUI Image Viewer is the primary interface.

## Offline Calibration Pipeline (Automated)

Before opening the GUI, run the 3-pass calibration pipeline:

```bash
rust-daq-daemon calibrate \
  --frame arc_hg2_capture.tiff \
  --flat flat_halogen_capture.tiff \
  --config config/calibration/mechelle_5000.toml \
  --output session_calibration_profile.toml
```

> **Note (bd-cpph3/bd-lj4g4, Apr 2026):** halogen-alone has replaced DH3P
> as the preferred flat for post-slit-swap calibration (142 traces vs 17,
> 0.27 nm RMS vs 0.40 nm). The blaze fitter is lamp-agnostic, so DH3P
> or D2-alone still work if halogen is unavailable.

This automatically:
1. Detects traces from the flat frame
2. Performs echelle equation seeding + atlas matching (Pass 1)
3. Re-seeds failed orders via quadratic regression (Pass 2)
4. Bootstraps uncalibrated orders via physics model (Pass 3)

Expected result: 115/115 orders calibrated (42 arc-matched + 73 bootstrapped), covering 230–844nm.

## GUI Workflow: Image Viewer Calibration Workspace (Validation + Refinement)

Open the Image Viewer for `istar_camera` and use the `Echelle Spectrum (MVP Preview)`
side panel to validate and optionally refine the auto-generated profile.

### 1. Load Generated Profile (Profile tab)

- Click `Load Editor`
- Enter path to the auto-generated profile from the pipeline
- Click `Load`
- Review provenance (should show `creator_tool`, timestamp, source frames)

Recommended file path:

```text
<lab-data-root>/echelle/leabs-dev/YYYY-MM-DD-hg2-session-01/session_calibration_profile.toml
```

### 2. Validate Traces (Trace tab)

With the arc or flat frame visible:

- Enable `Show trace overlays on image`
- Inspect overlays visually
- Verify overlays follow the visible order centers across the field
- Optional: select individual orders to refine centerline or sample range if needed
  - `sample_start` / `sample_end`
  - trace coefficients (`c0`, `c1`, ...)
  - `Nudge +/-Y` for quick centerline shifts

### 3. Inspect Wavelength Solution (Wavelength Fit tab)

- Review any notes or metadata about which orders are arc-matched vs bootstrapped
- For arc-matched orders (42 of them):
  - wavelength comes directly from atlas matching
  - residuals should be < 0.5nm
- For bootstrapped orders (73 of them):
  - wavelength predicted from physics model + 2D Chebyshev residual surface
  - marked as "bootstrapped" in order notes
  - expected accuracy ±0.3–0.5nm within each order family

### 4. Preview and Validate (Extraction panel + plot)

After profile loads:

- Confirm live extracted preview renders without errors
- Compare selected-order and merged views
- Use hover cross-link to verify plotted sample maps to the expected image order location
- Wavelength axis should span 230–844nm (full Mechelle 5000 range)

### 5. Optional GUI Refinement (Arc/Points + Wavelength Fit tabs)

If manual refinement is desired:

- Add manual calibration points to individual orders if needed
- Click `Fit Selected Order (LSQ)` to improve specific order wavelength fits
- Re-save the profile with `Save + Activate`

Current GUI fit scope:

- selected-order polynomial least-squares refit (manual points)
- full global re-fitting across all orders is a future enhancement

### 6. Blaze / Flat (Blaze/Flat tab)

With a suitable flat-derived preview visible:

- Compare `Uncorrected` vs `Preview corrected` curves
- Optionally generate a normalized preview artifact:
  - set `Blaze export CSV` path
  - click `Generate From Selected Order Preview`

This updates the editor profile’s blaze artifact reference and provides a
placeholder artifact for later full pipeline integration.

## Evidence to Save Before Ending Session

- profile file (`.toml` or `.json`)
- at least one screenshot of:
  - raw image with trace overlays
  - selected-order residual plot
  - extracted spectrum preview
- exported preview JSON and merged CSV
- line list JSON and calibration points JSON (if used)
- session notes:
  - source used (HG-2 / flat source)
  - exposure matrix
  - gain/readout/trigger mode
  - anomalies or SDK quirks

## Post-Session: Curate Repo Fixture (Optional)

Once a real Hg-Ar echellegram capture is confirmed:

1. Select a small representative subset (do not dump all raw lab data into git).
2. Copy into a new dataset folder under:
   - `testdata/echelle/leabs-dev/` (repo-relative)
3. Generate reference outputs (update or extend):
   - `scripts/echelle/reference_extract_hg2.py`
   - or a new script for the new capture naming/mode
4. Update regression tolerances and harness inputs.

## Fast Failure Conditions (Abort and Reconfigure)

Abort calibration fitting and debug acquisition first if any of these occur:

- ramp-like frames instead of echelle structure
- exposure changes do not materially alter the frame
- frames saturate broadly across many orders
- profile validation errors persist after obvious corrections
- trace overlays cannot be aligned visually despite repeated edits

## Session Notes Template (Copy/Paste)

```text
Session ID:
Date/Time (UTC):
Operator(s):
Device ID:
Camera mode / readout:
Exposure matrix:
Lamp(s):
Flat source:
Frame dimensions / ROI / binning (from FrameData):
Observed SDK quirks:
Trace alignment status:
Wavelength fit status (orders completed, RMS):
Blaze preview artifact generated:
Files exported:
Next actions:
```
