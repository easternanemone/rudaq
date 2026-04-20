# NIST ASD integration: shared internal crate with generated const data

**Decision date:** 2026-04-20
**Context:** bd-3yb8.30.1.5 (A0)
**Epic:** bd-3yb8.30 (HgAr wavelength calibration precision for LIBS spectroscopy)

## Question

The NIST Atomic Spectra Database (ASD) data needed for Mechelle 5000 HgAr
wavelength calibration is already curated in the sibling project
`~/code/CF-LIBS-improved/ASD_da/libs_production.db` (SQLite, 28,135 rows
covering Hg I/II, Ar I/II, Ne I, Th I/II, etc. at 200–975 nm with `aki`,
energy levels, degeneracies, Stark parameters, and accuracy grades).

rust-daq's `load_hgar_atlas()` currently returns only 29 lines from a
hand-curated subset. Expanding to ~900 HgAr lines + metadata requires
deciding how rust-daq consumes the CF-LIBS data.

## Options considered

### (a) Runtime rusqlite query against CF-LIBS DB
- **Pro:** Zero duplication, single source of truth.
- **Con:** rust-daq daemon now requires CF-LIBS to be checked out at a
  configured filesystem path. Deployments on maitai / leabs-dev break if
  CF-LIBS isn't installed. Drags SQLite (C dep) into every rust-daq
  target build. Couples rust-daq's runtime to CF-LIBS's file layout.

### (b) Vendor a filtered SQLite DB copy + rusqlite at runtime
- **Pro:** Self-contained deployment, rusqlite query flexibility.
- **Con:** Committed binary blob is opaque to `git diff` and `git blame`.
  Still drags SQLite C dep into all target builds. Drift risk from
  CF-LIBS unless a re-extraction script is committed and regularly run.

### (c) Vendor as CSV
- **Rejected by user 2026-04-20.** Loses type safety; loses the rich
  metadata (aki, energy levels, degeneracies, Stark) that downstream
  LIBS work (`bd-3yb8`) needs; no enforced schema.

### (d) **Chosen:** Shared internal crate with generated Rust const data
- New workspace crate `crates/atomic-reference/`.
- `scripts/data/regenerate_atomic_reference.py` reads CF-LIBS's SQLite
  DB, filters to species rust-daq + LIBS care about (Hg, Ar, Ne, Th in
  the 200–975 nm Mechelle bandpass), and writes a Rust source file
  `crates/atomic-reference/src/lines_data.rs` with `pub const LINES:
  &[AtomicLine] = &[...]`.
- The generated file is committed; builds never touch CF-LIBS or the
  Python script. Updating data = developer reruns the script, commits
  the diff.
- Public API: `atomic_reference::lines_for_element(...)`,
  `atomic_reference::atlas::hgar()`, convenience constructors for the
  existing `AtlasLine` shape (so `echelle::load_hgar_atlas()` becomes a
  thin wrapper without breaking its existing callers).

## Why (d)

- **Pure Rust, zero runtime deps.** No SQLite, no filesystem lookup,
  no cross-repo path assumption.
- **Compile-time type safety.** Misspelled element names, out-of-range
  wavelengths, malformed entries — all caught by the compiler.
- **Fully `git diff`-able.** A generated file of `AtomicLine {...}`
  entries is a text diff any reviewer can read. Binary SQLite blobs
  hide schema drift until runtime.
- **Matches repo convention `M-SMALLER-CRATES`** (see
  `.claude/skills/ms-rust/08_universal_guidelines.md`): if a submodule
  can be used independently, it should be a crate. Atomic-reference is
  the canonical example — pure data, read-only, usable by any crate
  that does spectroscopy.
- **Promotable.** The crate can later be extracted to its own repo and
  shared with CF-LIBS, fulfilling option (c)'s "shared crate" spirit
  without the up-front coordination cost today.
- **Mirrors existing repo pattern.** `andor-sdk3-sys` uses the same
  generator-pattern for `bindgen` output: generator script is
  reproducible, committed generated file is the build input.

## Implementation plan

1. Create `crates/atomic-reference/` as a workspace member.
2. Define `pub struct AtomicLine` with fields needed for calibration
   today (`wavelength_nm`, `element`, `sp_num`, `aki`) and reserved
   slots for downstream LIBS (`ei_ev`, `ek_ev`, `gi`, `gk`,
   `accuracy_grade`) so future expansion is additive.
3. Write `scripts/data/regenerate_atomic_reference.py` that queries the
   CF-LIBS DB and emits the generated file.
4. Run the script once, commit the generated file.
5. Expose convenience functions for the existing echelle use case:
   `pub fn atlas_hgar() -> Vec<AtlasLine>` returning the same shape
   `echelle::wavelength_fitting::AtlasLine` currently provides.
6. Rewire `echelle::wavelength_fitting::load_hgar_atlas()` to delegate.
7. Unit tests preserve the current 29-line atlas as a strict subset
   (no existing line dropped, all present lines match NIST wavelengths
   to 4 decimal places).

## Non-goals for bd-3yb8.30.1

- Not publishing to crates.io.
- Not porting CF-LIBS to consume this crate (that's a CF-LIBS-side
  follow-up after this epic validates the design).
- Not filling Stark / partition-function fields — those stay for the
  LIBS integration parent `bd-3yb8`. This task extracts only the
  subset needed for wavelength calibration.
