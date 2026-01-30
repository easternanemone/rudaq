# bd-* / bd-bd-* Branch Review

**Date:** 2026-01-29  
**Purpose:** Identify stale `bd-*` and `bd-bd-*` branches for cleanup.

---

## 1. Methodology

- **ahead** = commits on branch not in `main`
- **behind** = commits on `main` not in branch
- **ahead=0** → branch is fully contained in `main` (merged or redundant) → **stale**
- **PR MERGED** → work in `main` → **stale**
- **PR CLOSED** (not merged) → abandoned/superseded → **stale**
- **bd-bd-X** often = worktree variant of **bd-X**; if **bd-X** was merged, **bd-bd-X** is **stale**

---

## 2. Summary Table

| Branch | Last commit | Behind | Ahead | PR / Notes | Verdict |
|--------|-------------|--------|-------|------------|---------|
| **bd-1gdn.5** | 2026-01-25 | 172 | **0** | — | **STALE** (ahead=0) |
| **bd-51l7** | 2026-01-25 | 247 | 1 | Planning ref (07-04-SUMMARY) | Likely stale |
| **bd-bd-4088.1** | 2026-01-27 | 90 | 38 | No PR | Unclear (unique commits) |
| **bd-bd-51l7** | 2026-01-25 | 246 | 1 | — | Likely stale |
| **bd-bd-al9v** | 2026-01-27 | 90 | 41 | bd-al9v MERGED #235 | **STALE** (worktree of merged) |
| **bd-bd-gb19** | 2026-01-25 | 200 | **0** | — | **STALE** (ahead=0) |
| **bd-bd-i8wj** | 2026-01-26 | 141 | 1 | bd-i8wj MERGED #214 | **STALE** (worktree of merged) |
| **bd-bd-js4b** | 2026-01-25 | 247 | **0** | — | **STALE** (ahead=0) |
| **bd-bd-o7tj** | 2026-01-27 | 90 | 14 | bd-o7tj MERGED #227/228 | **STALE** (worktree of merged) |
| **bd-bd-ohww** | 2026-01-25 | 199 | **0** | — | **STALE** (ahead=0) |
| **bd-bd-pmdr** | 2026-01-27 | 90 | 3 | No PR | Unclear |
| **bd-bd-q7l1.1.1** | 2026-01-28 | 8 | 1 | CLOSED #251 | **STALE** |
| **bd-bd-q7l1.1.2** | 2026-01-28 | 8 | 2 | CLOSED #252 | **STALE** |
| **bd-bd-tz85** | 2026-01-26 | 155 | 1 | — | Likely stale |
| **bd-bd-zp00** | 2026-01-26 | 139 | 1 | bd-zp00 MERGED #216 | **STALE** (worktree of merged) |
| **bd-f87d** | 2026-01-26 | 142 | 1 | bd-bd-f87d MERGED #213 | **STALE** (variant of merged) |
| **bd-g8e0** | 2026-01-25 | 173 | 1 | bd-bd-g8e0 MERGED #201 | **STALE** (variant of merged) |
| **bd-izdj-hardening-review** | 2026-01-29 | **0** | **1** | Production hardening epic | **KEEP** (active) |
| **bd-o31v** | 2026-01-26 | 115 | 1 | No PR | Unclear |
| **bd-q7l1.1.1** | 2026-01-28 | 8 | 1 | Same as bd-bd-q7l1.1.1 | **STALE** |
| **bd-q7l1.1.2** | 2026-01-28 | 8 | 2 | Same as bd-bd-q7l1.1.2 | **STALE** |
| **bd-q7l1.1.3** | 2026-01-28 | 8 | 3 | MERGED #253 | **STALE** (merged) |
| **bd-xlc6** | 2026-01-27 | 90 | 14 | No PR | Unclear |

---

## 3. Verdict Summary

| Verdict | Count | Branches |
|---------|-------|----------|
| **KEEP** | 1 | `bd-izdj-hardening-review` |
| **STALE** (safe to delete) | 18 | All others except “Unclear” |
| **Unclear** | 4 | `bd-bd-4088.1`, `bd-bd-pmdr`, `bd-o31v`, `bd-xlc6` |

---

## 4. STALE Branches (recommended to delete)

**ahead=0 (fully in main):**  
`bd-1gdn.5`, `bd-bd-gb19`, `bd-bd-js4b`, `bd-bd-ohww`

**Worktree variants of merged branches:**  
`bd-bd-al9v`, `bd-bd-i8wj`, `bd-bd-o7tj`, `bd-bd-zp00`, `bd-f87d`, `bd-g8e0`

**Closed or superseded PRs:**  
`bd-bd-q7l1.1.1`, `bd-bd-q7l1.1.2`, `bd-q7l1.1.1`, `bd-q7l1.1.2`

**Merged:**  
`bd-q7l1.1.3`

**Likely stale (no PR, small ahead):**  
`bd-51l7`, `bd-bd-51l7`, `bd-bd-tz85`

---

## 5. Thorough Review of the 4 Unclear Branches

### 5.1 bd-bd-4088.1

| Metric | Value |
|--------|-------|
| Ahead of main | 38 commits |
| Behind main | 90 commits |
| Last commit | 2026-01-27 |
| PR | None |

**Commits (sample):** Mix of merged work (bd-icyx, bd-xrnw, bd-o1nl.\*, bd-ck2j, hooks, etc.) plus **unique** work:

- `50b32677` feat(daq-labeled): create Scipp-like labeled array crate with Arrow tensor integration
- `45fecbbf` fix(daq-labeled): complete test_3d_array_to_arrow metadata validation
- `bd4f655e` fix: apply cargo fmt formatting to arrow_tensor.rs

**Diff vs main:** 85 files, +8,133 / −2,705 lines.

**Unique vs main:**

- **`crates/daq-labeled/`** – **entire new crate** (Scipp-like labeled arrays, Arrow tensors). **Not present on main.** Modules: `lib`, `arrow_tensor`, `dimension`, `labeled_array`, `labeled_frame`.
- Reconnect logic moved from `ui` into `daq_client` (re-export in egui). Main already has `crates/client` with `reconnect`; this work landed via the daq-client extraction.
- `scripting` GenericDriver bindings + tests, ELL14 refactor, etc. – **main already has these** (scripting, hardware).

**Verdict:** **Keep – unique unmerged work.** The **daq-labeled** crate exists only on this branch. **Update:** Branch was renamed to **`daq-labeled`** (old name `bd-bd-4088.1` deleted).

---

### 5.2 bd-bd-pmdr

| Metric | Value |
|--------|-------|
| Ahead of main | 3 commits |
| Behind main | 90 commits |
| Last commit | 2026-01-27 |
| PR | None |

**Commits:**

1. `32eb54b2` refactor: improve ELL14 power safety, format scripting code  
2. `b3196bd6` feat(core): add log scrubbing utilities for sensitive data  
3. `4798a1f4` chore: add Claude Code workflow enforcement hooks  

**Diff vs main:** 15 files, +972 / −167. Touches: `.claude/hooks/*`, `daq-core/log_scrubbing`, `ell14`, `.beads`.

**Main today:**

- **`.claude/hooks/`** – present (check-file-size, enforce-bead, validate-completion, etc.).
- **Log scrubbing** – in `common::log_scrubbing` (likely migrated from `daq-core`).
- **ELL14** – main has ongoing ELL14 changes (e.g. bd-2m11.5).

**Verdict:** **STALE – safe to delete.** Hooks, log scrubbing, and ELL14 work have been superseded on main. No unique unmerged content.

---

### 5.3 bd-o31v

| Metric | Value |
|--------|-------|
| Ahead of main | 1 commit |
| Behind main | 115 commits |
| Last commit | 2026-01-26 |
| PR | None |

**Commit:** `194bef68` chore: worktree verification for bd-o31v  

**Diff vs main:** 1 file, +1 line: **`.worktree-verify`** only.

**Verdict:** **STALE – safe to delete.** Purely administrative; no code or config. No reason to keep.

---

### 5.4 bd-xlc6

| Metric | Value |
|--------|-------|
| Ahead of main | 14 commits |
| Behind main | 90 commits |
| Last commit | 2026-01-27 |
| PR | None |

**Commits (sample):** GenericDriver bindings, comedi_discover, Comedi calibration, hooks, log scrubbing, ELL14, plus:

- `a49e37f7` docs(scripting): add safety warning to transaction() method  
- `229fe6b7` feat(comedi): add comedi_discover for automatic device scanning  

**Diff vs main:** 29 files, +3,935 / −189. Includes Comedi `calibration.rs`, `device` changes, `generic_driver_bindings`, scripting tests.

**Main today:**

- **`comedi_discover`** – in `driver-comedi` (device.rs).
- **`calibration.rs`** – in `driver-comedi`.
- **`generic_driver_bindings`** and scripting tests – in `crates/scripting`.
- **Hooks, log scrubbing** – as above.

The only potentially unique piece is the **transaction() safety warning** in scripting docs. Everything else exists on main (via other PRs).

**Verdict:** **STALE – safe to delete.** No meaningful unique work. The extra safety doc is minor; can be re-added if desired.

---

### 5.5 Summary: Unclear Branches After Review

| Branch | Verdict | Action |
|--------|---------|--------|
| **bd-bd-4088.1** → **daq-labeled** | **Keep** | Renamed to `daq-labeled`. Unique `daq-labeled` crate. |
| **bd-bd-pmdr** | **STALE** | Safe to delete. |
| **bd-o31v** | **STALE** | Safe to delete. |
| **bd-xlc6** | **STALE** | Safe to delete. |

---

## 6. Keep

- **bd-izdj-hardening-review** – 0 behind, 1 ahead; aligns with bd-izdj Production Hardening epic. Active.
- **daq-labeled** – renamed from `bd-bd-4088.1`; holds unique `daq-labeled` crate (Scipp-like labeled arrays, Arrow). See §5.1.

---

## 7. Commands to Delete STALE Branches

```bash
# Only STALE (excludes Unclear + KEEP)
for b in bd-1gdn.5 bd-51l7 bd-bd-51l7 bd-bd-al9v bd-bd-gb19 bd-bd-i8wj \
  bd-bd-js4b bd-bd-o7tj bd-bd-ohww bd-bd-q7l1.1.1 bd-bd-q7l1.1.2 \
  bd-bd-tz85 bd-bd-zp00 bd-f87d bd-g8e0 bd-q7l1.1.1 bd-q7l1.1.2 \
  bd-q7l1.1.3; do
  git push origin --delete "$b"
done
```

**After thorough review (Unclear → resolved):**  
- **Deleted:** `bd-bd-pmdr`, `bd-o31v`, `bd-xlc6` (all STALE; see §5).  
- **Renamed:** `bd-bd-4088.1` → **`daq-labeled`** (unique `daq-labeled` crate preserved on new branch).
