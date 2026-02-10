# rust-daq Claude Guide

This file is a concise operational supplement for Claude.
`AGENTS.md` is the canonical policy document for this repository.
If there is any conflict, follow `AGENTS.md`.

## Fast Start

```bash
# Build / test / lint (local)
cargo build
cargo nextest run
cargo test --doc
cargo fmt --all
cargo clippy --all-targets

# Beads quick check (worktree-safe)
BD_SAFE='bd --no-daemon --no-auto-import --db .beads/beads.db'
$BD_SAFE where --json
$BD_SAFE ready --json
$BD_SAFE dep cycles --json
```

## Maitai Hardware Build (Critical)

For real hardware on maitai, use the dedicated build script.

```bash
bash scripts/build-maitai.sh
```

Why this matters:
- Building without `--features maitai` can silently select mock PVCAM paths.
- The script sources environment, cleans stale artifacts, and builds the daemon with the full hardware profile.

Quick verification after launch:
- Daemon log should show `pvcam_sdk feature enabled: true`.
- Device registration should include expected physical devices (camera, laser, power meter, rotators, motion, DAQ).

## Beads Workflow

Use `bd` for all task tracking and prefer JSON output for scripting/automation.

```bash
bd ready --json
bd update <id> --status in_progress --json
bd close <id> --reason "Completed" --json
```

Canonical statuses:
- `open`
- `in_progress`
- `blocked`
- `closed`

## Hook Layout

Configured in `.claude/settings.json`.

- `PreToolUse`:
- Bash commands are routed by `.claude/hooks/pretool-dispatch.sh` to run only relevant checks (`bd close`, `git push`).
- Read operations run `.claude/hooks/check-file-size.sh`.
- `SessionStart`:
- `.claude/hooks/session-start.sh` shows task context and tool health.

Quality gate split:
- `bd close`: lightweight gate (`fmt --check` + ast-grep structural check when available).
- `git push`: full gate (`fmt --check` + `clippy` + tests).

## grepai Requirement

Primary code exploration tool is `grepai`.

```bash
grepai search "intent query" --json --compact
grepai trace callers "SymbolName" --json
grepai trace callees "SymbolName" --json
```

If grepai backend is unavailable, fall back to `rg`/`grep` and report the fallback.

## LSP

Rust LSP is enabled in `.claude/settings.json`:
- `rust-analyzer-lsp@claude-plugins-official: true`
- `ENABLE_LSP_TOOL=1`

`pyright` remains disabled by default.

## References

- Canonical agent policy: `AGENTS.md`
- Testing details: `docs/guides/testing.md`
- Feature flags: `config/feature_flags.toml`
