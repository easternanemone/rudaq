# Beads (bd) — Issue Tracker

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md §Beads Issue Tracker
  - CLAUDE.md §Landing the Plane
see-also:
  - ../invariants.md
  - ./pr-workflow.md
-->

`bd` is the **only** task tracker for this repo. See
[`invariants.md`](../invariants.md) — do not use `TodoWrite`, `TaskCreate`,
or markdown TODO lists.

## Quick reference

```
bd prime                       # Full workflow context + commands (run at session start)
bd ready                       # Find available work
bd show <id>                   # View issue details
bd update <id> --claim         # Claim work
bd update <id> --note "..."    # Progress note
bd close <id>                  # Mark complete
bd dolt push                   # Sync beads remote (Dolt-backed)
bd remember "..."              # Persistent knowledge (do NOT create MEMORY.md)
```

## Data location

`.beads/` at repo root. Dolt-backed SQL. Do not edit directly.

If `bd dolt push` complains about a missing `origin` remote:

```
bash scripts/ops/setup-beads-dolt-remote.sh
```

## Issue ID convention

- Issue IDs look like `bd-47p2`, `bd-123`, etc.
- Reference in code: `TODO(bd-xxxx)`. Grep target.
- Reference in commit messages: `(bd-xxxx)`.
- Reference in branches: `feat/bd-xxxx-description`.

## When to file a bead

- **Pre-existing test failure** you encountered but could not fix cleanly → file before closing your work.
- **Pre-existing warning** in a file you did not touch → file so someone can sweep.
- **Follow-up work** discovered during an implementation → file before closing the parent.
- **Lint findings** from a wiki lint pass (see [`../schema.md`](../schema.md) §Lint).

## Session-close protocol (MANDATORY)

Work is not complete until `git push` succeeds:

```
bd close <id>            # Close finished issues
bd dolt push             # Sync beads
git pull --rebase        # Avoid diverging
git push                 # Push code
git status               # MUST show "up to date with origin"
```
