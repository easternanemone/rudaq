# Fresh-session entry-point prompt for bd-g22gu.2.2 DTW work

Paste the block below into the next `/clear`'d Claude Code session as your
first message. The memory note `project_bd_g22gu_2_status.md` is auto-loaded
via `MEMORY.md`; the prompt here adds the immediate action and expected
artefacts so the fresh session doesn't have to re-derive context.

---

Continue bd-g22gu.2.2 (DTW for echelle line identification) against the
HgAr DoE fixtures in `debug/phase5/hgar_matrix/`.

**Before starting, confirm state:**
1. `gh pr view 614 --json state,mergedAt` — should be MERGED. If still OPEN,
   ask me whether to merge it first or rebase the DTW branch on top of
   `feat/bd-g22gu22-hgar-capture-matrix`.
2. `bd show bd-g22gu.2.2.2` — the task with the plan.
3. `cat debug/phase5/hgar_matrix/matrix.json` — fixture metadata (9
   frames, MCP gain 0-500, ~10x bright_px dynamic range).
4. Read `debug/phase5/memo-dtw-line-id.md` — the research memo whose
   "don't implement" verdict we're now validating empirically.

**Plan (from bd-g22gu.2.2.2):**

1. **Baseline measurement** — small integration test that runs the current
   `echelle::wavelength_fitting::detect_arc_lines` + `match_lines_to_atlas`
   on each of the 9 DoE fixtures. Write per-frame
   (lines_detected, atlas_matched, rms_nm) to
   `debug/phase5/hgar_matrix/baseline_matches.json`. This is the bar DTW
   must clear.

2. **DTW implementation** — `crates/echelle/src/dtw_wavelength.rs`, feature
   `dtw_line_matching` (non-default). Hand-rolled DTW primitive (the memo
   estimates ~100 LOC) or vendor `shshemi/dtw-rs` (~200 LOC, MIT).
   Public entry: drop-in replacement for `match_lines_to_atlas` behind
   the feature flag.

3. **Comparison test** — same harness as step 1 but with
   `--features dtw_line_matching`. Assert DTW >= current method on the
   `g350_t10000ms` / `g500_t3000ms` DoE frames (the memo's "DTW sweet
   spot" regime).

4. **Decision** — land behind the flag if DTW wins; close bd-g22gu.2.2
   with empirical evidence otherwise. Either outcome is a valid finish.

**Gotchas carried forward from bd-g22gu.2.2.1 (also in the project memory
note):**

- `Rhai create_andor_camera()` returns a mock — script-based MCP control
  is inert against a running daemon. Use `snapshot --set` for real-
  hardware param control.
- `SetParameter` RPC uses serde enum variant names (`CWOn`), not the
  driver's `TryFrom` aliases (`CW On`).
- MCP gain <100 is on the flat region of the iStar response curve.
  Meaningful density gradient starts at gain ≥200.
- Detector saturation at 4195 ADU regardless of gain (detector internal
  12-bit ADC ceiling).

**Hardware safety (if you need to capture more fixtures):**

- MCP gain ≤ 500 absolute ceiling.
- Total MCP>100 dwell ≤ 60 s per session.
- `--after-set mcp_gain=0` on every snapshot that raises gain.
- Verify end-of-session cleanup log shows `MCP gain = 0 confirmed`
  before disconnect.

**Branch discipline:** new feature branch off current main (`feat/bd-g22gu22-dtw-line-id`
or similar). Do not work on main directly. bd-g22gu.4 regression test on
main gates every echelle change — it'll catch any DTW regression
automatically when enabled by default, and you can assert its
bit-exactness is preserved when DTW is OFF-by-default.
