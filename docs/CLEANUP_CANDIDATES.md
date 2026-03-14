# Aggressive Cleanup Candidates

This document tracks files and directories that may be candidates for future cleanup. Review and remove items as appropriate after verification.

## Last Updated: 2026-03-14

---

## Documentation Files

### `HISTORY.md` (21KB)
- **Location:** `/HISTORY.md`
- **Description:** Large historical changelog/notes file
- **Risk:** May contain valuable historical context for project evolution
- **Recommendation:** Review for relevance; consider archiving or moving to `docs/archive/`

### `ANDOR_SDK_FIXES.md` (7.7KB)
- **Location:** `/ANDOR_SDK_FIXES.md`
- **Description:** Documentation of SDK fixes and workarounds
- **Risk:** May still be relevant for Andor camera integration
- **Recommendation:** Verify if issues are resolved; move to `docs/reference/` if still relevant

---

## AI Tool Directories (Caches & Temp)

### `.beadhub-cache/`
- **Location:** `/.beadhub-cache/`
- **Description:** BeadHub cache directory
- **Risk:** Cache can be regenerated, but may slow down initial operations
- **Recommendation:** Safe to remove if disk space needed

### `.brv/`
- **Location:** `/.brv/`
- **Description:** ByteRover memory/agent coordination storage
- **Risk:** Contains persistent memory across sessions
- **Recommendation:** Review contents; may contain valuable cross-session context

### `.codemachine/`
- **Location:** `/.codemachine/`
- **Description:** CodeMachine AI tool data directory
- **Risk:** Unknown usage status
- **Recommendation:** Verify if actively used before removal

### `.gemini-clipboard/`
- **Location:** `/.gemini-clipboard/`
- **Description:** Gemini clipboard data
- **Risk:** Temporary clipboard storage, likely safe to remove
- **Recommendation:** Safe to remove if not actively using Gemini

### `.ralph-tui/`
- **Location:** `/.ralph-tui/`
- **Description:** Ralph TUI progress tracking
- **Risk:** Contains `progress.md` - may have active task tracking
- **Recommendation:** Check `progress.md` for active work before removal

---

## Python Artifacts

### `.venv/` and `.pal_venv/`
- **Location:** `/.venv/`, `/.pal_venv/`
- **Description:** Python virtual environments
- **Risk:** May be actively used for Python scripts/tools
- **Recommendation:** Verify if Python tools are still needed; can be recreated

### `__pycache__/` directories
- **Location:** Various throughout codebase
- **Description:** Python bytecode cache
- **Risk:** None - can be regenerated
- **Recommendation:** Safe to remove with: `find . -type d -name "__pycache__" -exec rm -rf {} +`

---

## GitHub Files

### `.github/TAILSCALE_FEATURE_REQUEST.md`
- **Location:** `/.github/TAILSCALE_FEATURE_REQUEST.md`
- **Description:** Feature request document
- **Risk:** May be tracking an open feature request
- **Recommendation:** Check if feature has been implemented or if issue exists elsewhere

---

## Other Files

### `.history/`
- **Location:** `/.history/`
- **Description:** Shell history directory
- **Risk:** None - personal shell history
- **Recommendation:** Safe to remove

### `CF-LIBS.code-workspace`
- **Location:** `/CF-LIBS.code-workspace`
- **Description:** VS Code workspace configuration
- **Risk:** May be used by team members
- **Recommendation:** Check if workspace file is actively used

### `.pre-commit-config.yaml` vs `.pre-commit-quick.yaml`
- **Location:** Root directory
- **Description:** Two pre-commit configurations
- **Risk:** May cause confusion
- **Recommendation:** Consider consolidating or documenting the difference

---

## Cleanup Commands Reference

```bash
# Remove Python cache directories
find . -type d -name "__pycache__" -exec rm -rf {} +

# Remove .venv directories (after verification)
rm -rf .venv .pal_venv

# Remove AI tool caches (after verification)
rm -rf .beadhub-cache .gemini-clipboard

# Remove shell history
rm -rf .history
```

---

## Review Schedule

- **Quarterly:** Review this document and remove items that are no longer relevant
- **Before major releases:** Consider cleanup of outdated documentation
- **After tool migrations:** Remove directories for tools no longer in use
