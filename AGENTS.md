# Repository Guidelines

## Current Project Structure

**Status**: Single-crate application (flat structure)

```
rust-daq/
├── Cargo.toml          # Single crate configuration
├── src/                # Application source code
│   ├── main.rs         # Entry point
│   ├── lib.rs          # Library exports
│   ├── app.rs          # Core application state
│   ├── core.rs         # Core traits (Instrument, DataProcessor)
│   ├── data/           # Data processors (FFT, IIR, Trigger, Storage)
│   ├── instrument/     # Instrument implementations (ESP300, MaiTai, etc.)
│   └── gui/            # GUI components (egui)
├── config/             # Configuration files
├── tests/              # Integration tests
└── target/             # Build artifacts (do not commit)
```

**Future Architecture** (planned with Python integration):
- Workspace structure with separate `rust_daq/` GUI crate
- Plugin system in `plugins/` directory for modular instrument drivers
- PyO3 bindings in `python/` for high-level scripting (see [README.md – Performance Architecture](README.md#performance-architecture) and [FINAL_CONSENSUS_REPORT.md](FINAL_CONSENSUS_REPORT.md))

## Build, Test, and Development Commands
- `cargo check` — Fast compile-time validation before opening a PR
- `cargo run` — Launches the desktop application with default features
- `cargo run --features full` — Run with all optional features (HDF5, Arrow, VISA)
- `cargo fmt` — Format code using `rustfmt`; required prior to commits
- `cargo clippy --all-targets --all-features` — Static analysis for common Rust pitfalls
- `cargo test --all-features` — Run all tests with optional feature backends

## Coding Style & Naming Conventions
We rely on standard Rust 4-space indentation and `rustfmt` defaults. Use `snake_case` for functions and files, `CamelCase` for types, and SCREAMING_SNAKE_CASE for constants. Keep modules cohesive—instrument drivers live in `src/instrument/`, data processors in `src/data/`, and GUI components in `src/gui/`. Document intent with concise comments when logic spans multiple async tasks or channels.

## Testing Guidelines
Unit tests live beside their modules; integration coverage belongs in `tests/`, e.g., `tests/integration_test.rs`. Run `cargo test --all-features` before pushing to ensure optional backends compile. When adding hardware integrations, provide mocked pathways or feature flags so tests run in CI without devices attached.

## Commit & Pull Request Guidelines
Follow the Conventional Commits pattern already in history (`feat:`, `fix:`, etc.), referencing issue IDs where relevant. Each PR should summarise the change scope, note impacted subsystems (UI, pipeline, plugin), and include screenshots or logs when UI or acquisition behavior changes. Link configuration updates to the matching sample in `config/` so reviewers can reproduce the scenario.

## Multi-Agent Workflow & Tooling
- **Jules Working Directory**: Jules AI agents work in a separate `rust_daq/` directory to avoid conflicts. This is NOT the main project structure—it's a Jules-specific workspace.
- **Main Project**: All production code lives in `src/` at the repository root (single-crate structure).
- Always request a dedicated `git worktree` before starting. The default repo hosts active automation; parallel agents working in the same directory risk clobbering each other's changes.
- Run `BEADS_DB=.beads/daq.db bd …` whenever you interact with the beads tracker; creating `$HOME/.beads` is disallowed in the sandbox and will fail.
- Before handing the repo back, run `cargo check` and `git status -sb` to verify the workspace is clean. Our recent error-handling updates live in `src/error.rs` and `src/app.rs`; confirm they remain intact if you touch those files.

## Issue Tracking with bd (beads)

**IMPORTANT**: This project uses **bd (beads)** for ALL issue tracking. Do NOT use markdown TODOs, task lists, or other tracking methods.

### Why bd?

- Dependency-aware: Track blockers and relationships between issues
- Git-friendly: Auto-syncs to JSONL for version control
- Agent-optimized: JSON output, ready work detection, discovered-from links
- Prevents duplicate tracking systems and confusion

### Quick Start

**Check for ready work:**
```bash
bd ready --json
```

**Create new issues:**
```bash
bd create "Issue title" -t bug|feature|task -p 0-4 --json
bd create "Issue title" -p 1 --deps discovered-from:bd-123 --json
```

**Claim and update:**
```bash
bd update bd-42 --status in_progress --json
bd update bd-42 --priority 1 --json
```

**Complete work:**
```bash
bd close bd-42 --reason "Completed" --json
```

### Issue Types

- `bug` - Something broken
- `feature` - New functionality
- `task` - Work item (tests, docs, refactoring)
- `epic` - Large feature with subtasks
- `chore` - Maintenance (dependencies, tooling)

### Priorities

- `0` - Critical (security, data loss, broken builds)
- `1` - High (major features, important bugs)
- `2` - Medium (default, nice-to-have)
- `3` - Low (polish, optimization)
- `4` - Backlog (future ideas)

### Workflow for AI Agents

1. **Check ready work**: `bd ready` shows unblocked issues
2. **Claim your task**: `bd update <id> --status in_progress`
3. **Work on it**: Implement, test, document
4. **Discover new work?** Create linked issue:
   - `bd create "Found bug" -p 1 --deps discovered-from:<parent-id>`
5. **Complete**: `bd close <id> --reason "Done"`
6. **Commit together**: Always commit the `.beads/issues.jsonl` file together with the code changes so issue state stays in sync with code state

### Auto-Sync

bd automatically syncs with git:
- Exports to `.beads/issues.jsonl` after changes (5s debounce)
- Imports from JSONL when newer (e.g., after `git pull`)
- No manual export/import needed!

### MCP Server (Recommended)

If using Claude or MCP-compatible clients, install the beads MCP server:

```bash
pip install beads-mcp
```

Add to MCP config (e.g., `~/.config/claude/config.json`):
```json
{
  "beads": {
    "command": "beads-mcp",
    "args": []
  }
}
```

Then use `mcp__beads__*` functions instead of CLI commands.

### Important Rules

- ✅ Use bd for ALL task tracking
- ✅ Always use `--json` flag for programmatic use
- ✅ Link discovered work with `discovered-from` dependencies
- ✅ Check `bd ready` before asking "what should I work on?"
- ❌ Do NOT create markdown TODO lists
- ❌ Do NOT use external issue trackers
- ❌ Do NOT duplicate tracking systems

For more details, see README.md and QUICKSTART.md.
