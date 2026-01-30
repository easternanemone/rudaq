# Repo Cleanup Candidates (2026-01)

**Method:** Octocode `localSearchCode`, `localViewStructure`, `localFindFiles`, grepai trace, manual inspection.

---

## 1. Delete immediately (root cruft)

| Item | Reason |
|------|--------|
| **`daq_acquisition.rbl`**, **`daq_timeseries_only.rbl`**, **`daq_camera_only.rbl`**, **`daq_default.rbl`** (repo root) | Duplicate blueprint outputs. Canonical location: `crates/server/blueprints/`. Server loads from there. Root copies are untracked and ~2–5× larger (different gen); regenerate script writes to `crates/server/blueprints/`. |
| **`zellij`** (repo root) | 51MB ELF binary; accidentally committed. Not a session file. Remove from git and delete. |
| **`repomix-rerun.xml`** (repo root) | ~30MB repomix config/cache; no code references. Local tooling artifact. Remove from git and delete. |

---

## 2. Already ignored (optional disk cleanup)

These are in `.gitignore`; safe to delete from filesystem if present:

- `check*.log`, `check_log.txt`, `check_output.txt`
- `CLAUDE.md.backup-*`
- `*.log` (root-level)

---

## 3. Keep but monitor

| Item | Notes |
|------|--------|
| **`crates/common/examples/review_check.rs`** | No references in Cargo.toml, CI, or docs. Diagnostic example (bd-ga2o) for Observable/Parameter. Keep as documented example unless we consolidate examples. |
| **`docs/archive/`** | In `.gitignore`. Use for moving superseded docs (e.g. old phase plans) if we want to archive rather than delete. |

---

## 4. Planning / prompts (archive candidates)

- **`.planning/phases/*`**: Phase plans (01–08) and RESEARCH/VERIFICATION. Many are completed. Consider moving older phases to `docs/archive/` or `.planning/archive/` if we create it, or trim to CONTEXT + SUMMARY only.
- **`.prompts/001–006`**: Driver-plugins research and phase prompts. Historical; could archive after verification no active references.

---

## 5. Call-graph / dead-code notes

- **grepai trace** pointed at `crates/daq-core/` (removed during rename). Index may be stale.
- **`review_check`**: no callers; standalone example.
- No systematic dead-code sweep performed; consider `cargo udeps` / `cargo machete` and grepai `trace callers` on rarely-used pub items for follow-up.

---

## 6. Actions taken

- [x] Delete root `daq_*.rbl` (4 files).
- [x] `git rm zellij repomix-rerun.xml` and delete from disk.
- [x] Add `/*.rbl` to `.gitignore` to avoid re-adding root blueprint cruft.
- [ ] (Optional) Delete ignored root logs/backups from disk.
