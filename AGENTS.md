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

### ast-grep Integration

This project uses `ast-grep` to enforce coding standards and help with migrations. A set of project-specific rules is defined in `rust_daq_ast_grep_rules.yml`.

**MCP Server (`ast-grep-mcp`)**

For AI agents, the `ast-grep-mcp` server provides a powerful way to interact with the codebase using `ast-grep`.

**Setup:**

1.  **Install `ast-grep-mcp`:**
    ```bash
    pip install ast-grep-mcp
    ```

2.  **Add to MCP config:**
    Add the following to your MCP configuration file (e.g., `~/.config/claude/config.json`):
    ```json
    {
      "ast-grep": {
        "command": "ast-grep-mcp",
        "args": []
      }
    }
    ```

**Usage:**

You can use the `mcp__ast_grep__*` functions to run the rules. For example, to run all rules in the project:

```
mcp__ast_grep__run --config rust_daq_ast_grep_rules.yml
```

To run a specific rule:

```
mcp__ast_grep__run --config rust_daq_ast_grep_rules.yml --rule-id no-unwrap-expect
```

The output will be in JSON format, which can be easily parsed by AI agents.

### Important Rules

- ✅ Use bd for ALL task tracking
- ✅ Always use `--json` flag for programmatic use
- ✅ Link discovered work with `discovered-from` dependencies
- ✅ Check `bd ready` before asking "what should I work on?"
- ❌ Do NOT create markdown TODO lists
- ❌ Do NOT use external issue trackers
- ❌ Do NOT duplicate tracking systems

For more details, see README.md and QUICKSTART.md.
## ByteRover Shared Memory System

**CRITICAL**: All AI agents working on this project MUST use ByteRover for shared memory and knowledge coordination.

### Quick Start

```bash
# Start every session with context retrieval
brv retrieve -q "topic or module you're working on"

# End every session by recording learnings
brv add -s "Section" -c "src/file.rs:line - Specific finding with rationale"
brv push -y  # After user approval
```

### Configuration

- **Space**: TheFermiSea/rust-daq
- **Account**: squires.b@gmail.com
- **Branch**: main
- **Config**: `.brv/config.json` (auto-configured, git-ignored)

### Standard Workflow for ALL Agents

**Before starting work:**
```bash
cd /Users/briansquires/code/rust-daq
brv retrieve -q "specific module or feature"
brv status  # Review what's in local context
```

**During work:**
- Use standard tools (Read, Write, Edit, Bash, etc.)
- Follow project conventions (beads for issues, ast-grep for quality)
- Test changes: `cargo test`, `~/.rml/rml/rml`, `ast-grep scan`

**After completing work:**
```bash
# Record specific learnings with file:line references
brv add -s "Lessons Learned" -c "src/instruments_v2/pvcam.rs:450 - Frame timeout must be 2x exposure time + 1000ms overhead. Use Duration::from_millis(exposure_ms * 2 + 1000). Related to bd-155"

brv add -s "Common Errors" -c "src/gui/mod.rs:234 - Never use blocking_recv() in egui render loop. Use try_recv() to prevent UI freezes. Detected by ast-grep rule 'find-blocking-gui-calls'"

brv add -s "Best Practices" -c "V2 instruments broadcast Measurement enum directly via measurement_channel(). See src/instruments_v2/mock_instrument.rs for reference implementation"

# Share with team (after user approval)
brv push -y
```

### Standard Sections

Use these consistent categories across all agents:

- **Lessons Learned** - New discoveries during development
- **Best Practices** - Proven patterns and approaches  
- **Common Errors** - Bugs and how to avoid/fix them
- **Architecture** - Design decisions and rationale
- **Testing** - Test strategies and patterns
- **Project Structure and Dependencies** - Module organization

### Writing Good Memories

**Template:**
```
src/path/file.rs:line - [Problem/Pattern]. [Solution/Approach]. [Rationale]. Related: bd-XXX, ast-grep rule 'rule-name'
```

**Examples:**

✅ **Good**:
```bash
brv add -s "Architecture" -c "src/core_v3.rs defines capability-based traits (Camera, Stage, Spectrometer, PowerMeter, Laser). V3 instruments implement these instead of generic Instrument trait. Provides compile-time guarantees for instrument-specific operations. See bd-51 for migration plan"
```

❌ **Bad**:
```bash
brv add -s "Notes" -c "Fixed V3 traits"
```

### Integration with Project Tools

**With beads (bd):**
- Reference bd issue IDs in memories: `Related to bd-155`
- Record issue discoveries: `Found during bd-42 implementation`

**With ast-grep:**
- Reference rules: `Detected by ast-grep rule 'no-unwrap-expect'`
- Document patterns that should become rules

**With recurse.ml:**
- Record AI-detected bugs: `rml identified race condition in...`
- Share fixes across agents

### Agent-Specific Instructions

**Claude Code** (current agent):
- Use `brv` commands directly in workflow
- Always retrieve before coding sessions
- Record learnings immediately after discoveries

**Codex** (via Zen MCP):
```typescript
mcp__zen__clink({
  cli_name: "codex",
  prompt: `
    cd /Users/briansquires/code/rust-daq
    brv retrieve -q "your topic"
    
    [Your task here]
    
    brv add -s "Section" -c "Your findings with file:line"
  `
})
```

**Gemini CLI**:
```bash
gemini-cli "First: brv retrieve -q 'topic', then work, finally: brv add with findings"
```

### Documentation

- **Quick Reference**: `.byterover-quick-ref.md`
- **Full Setup Guide**: `docs/BYTEROVER_MULTI_AGENT_SETUP.md`
- **Onboarding Guide**: `docs/AGENT_ONBOARDING.md`
- **Agent Configs**: `docs/agent-configs/codex-byterover.md`, `gemini-byterover.md`
- **Test Suite**: `scripts/test-byterover-multiagent.sh`

### Verification

Run validation tests:
```bash
bash scripts/test-byterover-multiagent.sh
# Expected: 15/15 tests PASS
```

### Troubleshooting

**Not logged in:**
```bash
brv login  # Use squires.b@gmail.com
```

**Can't retrieve memories:**
```bash
cd /Users/briansquires/code/rust-daq
brv status  # Verify connection
brv retrieve -q "broader search term"
```

**Cross-agent coordination:**
```bash
# Agent A adds knowledge
brv add -s "Testing" -c "Integration test pattern for async instruments"
brv push -y

# Agent B retrieves it
brv retrieve -q "integration test async"
# Should find Agent A's memory
```

### Success Criteria

- ✅ All agents authenticated with same account
- ✅ All agents retrieve context before work
- ✅ All agents record learnings after discoveries  
- ✅ All agents push regularly to shared space
- ✅ Cross-agent memory retrieval verified
- ✅ Consistent use of standard sections

See `docs/BYTEROVER_SETUP_COMPLETE.md` for full setup status.

## Jules Parallelization & Multi-Agent Orchestration

**CRITICAL**: This project achieved **52 parallel Jules sessions** (71% success rate) through coordinated multi-agent orchestration. This section documents proven patterns for massive parallelization.

### Architecture Overview

**Three-Tier Agent Coordination:**
1. **Claude Code (Orchestrator)**: Coordinates all agents, manages beads tracker, monitors progress
2. **Gemini (Strategic Advisor)**: Provides context, unblocks stuck sessions, offers architectural guidance via `mcp__zen__clink`
3. **Codex (Deep Implementer)**: Handles complex features requiring full codebase analysis via `mcp__zen__clink`
4. **Jules (Parallel Workers)**: Executes 50-100 parallelizable tasks simultaneously

### Jules Parallelization Strategy

**Capacity Planning:**
- **Daily Quota**: 100 Jules sessions per day
- **Target Utilization**: 60-80 sessions for optimal coverage
- **Success Rate**: 71% completion rate (37/52 in this session)
- **Session States**: Completed, In Progress, Planning, Awaiting Plan Approval, Failed

**Rate Limit Management:**
Jules API enforces rate limits on session creation:
```bash
# CORRECT: Add 8-10 second delays between session creation
jules new --repo TheFermiSea/rust-daq "task description"
sleep 8  # Prevent 429 RESOURCE_EXHAUSTED errors
jules new --repo TheFermiSea/rust-daq "next task"
```

**Batch Creation Pattern:**
```bash
# Background batch for 15+ tasks
(
for task in "task1" "task2" "task3" ...; do
    echo "Creating: $task"
    jules new --repo TheFermiSea/rust-daq "$task" 2>&1 | grep -E "(ID:|URL:)"
    sleep 8  # Critical for API rate limits
done
echo "Batch complete"
) &  # Run in background

# Check progress
jobs
```

**Session Categories:**
1. **Quick wins**: Testing, documentation, small features (30-60 min)
2. **Medium complexity**: Integration tests, V3 migrations (1-3 hours)
3. **Complex features**: Requires planning approval (3-6 hours)

### Jules Session Lifecycle

**1. Creation Phase:**
```bash
# Create with detailed description including file paths
jules new --repo TheFermiSea/rust-daq \
  "bd-197: Migrate ESP300 to V3. Follow Newport 1830C V3 pattern from src/instruments_v2/newport_1830c_v3.rs. Implement MotionController trait. Add unit tests with MockSerialDevice. File: src/instruments_v2/esp300_v3.rs"
```

**2. Monitoring Phase:**
```bash
# Check all sessions
jules remote list --session 2>&1 | head -50

# Count by status
jules remote list --session 2>&1 | grep "Completed" | wc -l
jules remote list --session 2>&1 | grep "Planning" | wc -l

# Get session details
jules remote view <session-id>
```

**3. Unblocking Planning Sessions:**
When sessions stuck in "Planning" state, provide:
- Reference implementation patterns (e.g., Newport V3 for ESP300 V3)
- Codebase context (config.rs for dynamic config features)
- Architectural guidance (error.rs for error handling patterns)

**Example unblocking via Gemini:**
```typescript
mcp__zen__clink({
  cli_name: "gemini",
  prompt: `5 Jules sessions stuck in Planning. Provide guidance:
  - Session 12602339779293719748: Transaction system (bd-131)
    Context needed: src/config.rs Settings struct, TOML structure
  - Session 10407563664786836449: ESP300 V3 (bd-197)
    Reference: src/instruments_v2/newport_1830c_v3.rs pattern`
})
```

**4. PR Review & Merge:**
```bash
# List Jules PRs
gh pr list --repo TheFermiSea/rust-daq --json number,title,author \
  | jq -r '.[] | select(.author.login == "app/google-labs-jules")'

# Comment with @jules for fixes
gh pr comment <pr-number> --body "@jules CI failing. Please rebase on main, fix lint errors, and ensure tests pass. Reference Newport V3 pattern."

# Pull completed work
jules remote pull --session <session-id>
```

### Proven Delegation Patterns

**Pattern 1: Codex for Complex Features**
```typescript
// Use for: 1000+ line implementations, full V3 migrations
mcp__zen__clink({
  cli_name: "codex",
  prompt: `Implement Newport 1830C V3 driver (bd-108):
  - Follow InstrumentV3 trait pattern from core_v3.rs
  - Implement PowerMeter control surface
  - Use SerialDevice abstraction for mocking
  - Write 6+ unit tests covering all methods
  - Register in InstrumentManagerV3
  Reference: src/adapters/serial_adapter.rs for serial patterns`
})
```

**Pattern 2: Gemini for Orchestration & Guidance**
```typescript
// Use for: Strategic planning, unblocking, context provision
mcp__zen__clink({
  cli_name: "gemini",
  prompt: `Create Jules task list for Phase 3 V3 integration (bd-155):
  - Identify 10-15 parallelizable subtasks
  - Delegate to Jules in batches of 5
  - Monitor sequential dependencies (bd-e52e.14 after bd-e52e.13)
  - Auto-delegate follow-ups when blockers complete`
})
```

**Pattern 3: Jules for Massive Parallelization**
```bash
# Single task
jules new --repo TheFermiSea/rust-daq "Add Elliptec polling test"

# Batch of 15 architecture tasks
for task in "state machine" "capability discovery" "error recovery" ...; do
    jules new --repo TheFermiSea/rust-daq "$task"
    sleep 8
done

# Bulk with sub-categories
# Batch A: 15 architecture tasks
# Batch B: 5 performance tasks  
# Batch C: 8 testing tasks
# Batch D: 5 GUI tasks
# Total: 33+ sessions in 4 parallel background processes
```

### Success Metrics from This Session

**Jules Performance:**
- 52 total sessions created (52% of daily quota)
- 37 completed (71% success rate)
- 9 PRs submitted
- 5 sessions in Planning (need approval)
- 1 failed session

**Key Achievements:**
1. ✅ ESP300 V3 migration (bd-197) - **PR #58 completed**
   - Added `MotionController` trait to core_v3.rs
   - Full implementation with unit tests
   - Followed Newport V3 pattern

2. ✅ Newport 1830C V3 (bd-108) - **Codex completion**
   - 1,067 lines new code
   - 6 passing unit tests (100% pass rate)
   - Reference implementation for all future V3 migrations

3. ✅ Elliptec integration tests (bd-e52e.12-14)
4. ✅ Python FlatBuffers client (bd-124)

**Coordinated Tools:**
- `mcp__zen__clink` for Gemini/Codex delegation
- `jules new/remote` for session management
- `bd ready/update/close` for beads tracking
- `gh pr list/comment` for PR coordination

### Error Patterns & Solutions

**Error 1: Jules API Rate Limiting**
```
Error: 429 RESOURCE_EXHAUSTED
Solution: Add 8-10 second delays between session creation
Status: Resolved with background batch pattern
```

**Error 2: Sessions Stuck in Planning**
```
Symptom: Sessions don't progress past "Planning" state
Solution: Provide codebase context via Gemini orchestrator
Pattern: Reference implementations + architectural guidance
Status: 6 sessions unblocked with guidance
```

**Error 3: All PRs Failing CI**
```
Symptom: 9 Jules PRs with lint/test failures
Solution: Comment with @jules requesting fixes
Pattern: Rebase on main + reference working patterns
Status: Monitoring for Jules re-work
```

**Error 4: Wrong Repository Name**
```
Error: Using briansquires/rust-daq instead of TheFermiSea/rust-daq
Solution: Verify with `git remote -v`, correct all sessions
Impact: 20+ initial attempts failed before correction
```

### Integration with Beads Tracker

**CRITICAL**: Orchestrator (Claude Code) MUST sync Jules progress to beads:

```bash
# After Jules session completes
jules remote pull --session <id>

# Update beads
bd update bd-197 --status closed --reason "Completed by Jules session <id>"

# Commit together
git add .beads/issues.jsonl src/
git commit -m "feat: ESP300 V3 migration (bd-197)

Completed by Jules session 10407563664786836449.
Added MotionController trait, full V3 implementation."
```

**Workflow:**
1. Claude Code delegates to Jules
2. Jules completes and creates PR
3. **Claude Code updates beads tracker**
4. Beads state syncs with git commits
5. All agents see updated issue state

### Best Practices

**DO:**
- ✅ Create 50-80 Jules sessions for optimal parallelization
- ✅ Use 8-10s delays to avoid API rate limits
- ✅ Provide reference implementations (Newport V3 pattern)
- ✅ Monitor session progress every 20-30 minutes
- ✅ Update beads tracker after session completion
- ✅ Use Gemini for strategic guidance and unblocking
- ✅ Use Codex for complex 1000+ line features
- ✅ Comment on PRs with @jules for fixes

**DON'T:**
- ❌ Create sessions too rapidly (causes 429 errors)
- ❌ Expect 100% completion rate (70-80% is excellent)
- ❌ Leave Planning sessions without guidance
- ❌ Forget to sync beads with completed work
- ❌ Use wrong repository name
- ❌ Merge PRs with CI failures
- ❌ Duplicate work already in beads tracker

### Future Enhancements

**Automation Opportunities:**
1. Auto-comment on Planning sessions with codebase context
2. Beads-to-Jules auto-delegation for `bd ready` issues
3. CI success webhook → auto-merge Jules PRs
4. Jules completion → beads update automation
5. Background batch health monitoring

**Scaling Strategies:**
1. Increase to 80-100 sessions per session (full quota)
2. Multiple orchestrators for different epic branches
3. Gemini auto-delegation for sequential dependencies
4. Jules session templates for common patterns