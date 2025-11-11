# JULES.md - Jules Agent Context

**General context for Jules AI coding agents** working on the rust-daq project.

## Project Goals

Build a high-performance scientific data acquisition (DAQ) system in Rust as a modular alternative to Python frameworks like PyMoDAQ. Core architecture: async-first with Tokio runtime, egui GUI, trait-based plugin system for instruments and data processors.

## Autonomous Swarm Development

**CRITICAL**: You are part of a **self-learning, autonomous swarm** of Jules agents working in parallel on this project.

### Your Autonomy

- **Modify project tools as needed**: If ast-grep rules need updates for new patterns you're developing, update `rust_daq_ast_grep_rules.yml`
- **Evolve documentation**: Freely add/modify documentation files to guide swarm development
- **Share knowledge**: Document patterns, discoveries, and gotchas so other Jules agents benefit
- **Coordinate through artifacts**: Other agents learn from your commits, documentation, tests, and code patterns

### Swarm Coordination Mechanisms

1. **Documentation is Living**: Add to CLAUDE.md, AGENTS.md, docs/ as you discover patterns
2. **ByteRover (Optional)**: Share specific learnings via `brv add -s "Section" -c "file:line - details"`
3. **ast-grep Rules**: Update rules to enforce or guide patterns you establish
4. **Git Commits**: Your commit messages and diffs teach other agents
5. **Tests as Examples**: Your tests demonstrate correct usage patterns

### Self-Learning Pattern

```bash
# When you discover a new pattern worth enforcing:
# 1. Implement the pattern in your code
# 2. Add an ast-grep rule to detect/enforce it
# 3. Document why in rust_daq_ast_grep_rules.yml comments
# 4. Update relevant docs (CLAUDE.md, etc.)

# When you encounter obstacles:
# 1. Document the issue and solution
# 2. Add to docs/ or update existing guides
# 3. Create tests to prevent regression
```

## Available Tools & Resources

### Hardware Access via Tailscale

Some tasks require physical hardware (cameras, motion controllers, lasers, power meters) connected to the **maitai-eos** machine.

**Environment Variables** (pre-configured in Jules VM):
- `TAILSCALE_AUTHKEY` - Authentication for Tailscale
- `TAILSCALE_DOMAIN` - Your tailnet domain
- `HARDWARE_MACHINE` - Set to `maitai-eos`

**Usage**:
```bash
# Verify connection
ping maitai-eos

# Run commands on hardware machine
ssh user@maitai-eos "command"
```

### Issue Tracking with Beads

This project uses **[beads](https://github.com/steveyegge/beads)** for issue tracking (NOT GitHub issues or markdown TODOs).

**Environment Variables**:
- `BEADS_DB` - Already set to `.beads/daq.db` (project-local tracker)
- `BD_ACTOR` - Your Jules session ID (for audit trail)

**Key Commands**:
```bash
# Find ready work
bd ready --json

# Show issue details
bd show bd-XXX

# Update status as you work
bd update bd-XXX --status in_progress
bd update bd-XXX --notes "Progress details"

# Create new issues when discovering work
bd create "Issue title" -t feature -p 1
bd dep add <new-id> <parent-id> --type discovered-from
```

**Important**:
- Update issues to `in_progress` when starting work
- Add `--notes` to document your progress
- Commit `.beads/issues.jsonl` with your code changes
- The orchestrator (Claude Code) will close issues after reviewing your work

### ByteRover Memory System

Share knowledge with other agents via ByteRover:

```bash
# Retrieve context at start
brv retrieve -q "topic you're working on"

# Record learnings (be specific with file:line references)
brv add -s "Lessons Learned" -c "src/file.rs:123 - Specific learning"
brv add -s "Common Errors" -c "src/file.rs:456 - Error to avoid"

# Share with team when done
brv push -y
```

**Standard Sections**: Lessons Learned, Best Practices, Common Errors, Architecture, Testing

## Hierarchical Task Delegation

Jules sessions can spawn other Jules sessions for complex tasks:

```bash
# Create child sessions for subtasks
jules new --repo TheFermiSea/rust-daq "Subtask description"

# Monitor child sessions
jules remote list --session | grep "Completed"

# Pull completed work
jules pull <child-session-id>
```

**Use when**: Your task is complex with multiple independent subtasks that can be parallelized.

## Project Documentation

**Read these files** to understand the codebase:

### Architecture & Patterns
- `docs/rust-daq-app-architecture.md` - Overall architecture
- `src/core.rs` - Core traits (Instrument, DataProcessor, StorageWriter)
- `src/core_v3.rs` - V3 instrument architecture with capability traits
- `src/instruments_v2/newport_1830c_v3.rs` - **Reference V3 implementation** (1,067 lines)

### Development Guides
- `CLAUDE.md` - Development conventions and commands
- `AGENTS.md` - Multi-agent coordination patterns
- `docs/BEADS_INSTALLATION.md` - Beads workflow details
- `docs/BYTEROVER_MULTI_AGENT_SETUP.md` - ByteRover usage

### Domain-Specific
- `docs/rust-daq-instrument-guide.md` - Instrument control patterns
- `docs/rust-daq-data-guide.md` - Data management
- `docs/rust-daq-gui-guide.md` - GUI development

## Common Commands

### Development
```bash
cargo check              # Fast compile check
cargo test               # Run unit tests
cargo test -- --nocapture # Show test output
cargo fmt                # Format code
cargo clippy             # Lint checking
```

### Testing Strategy
- Unit tests: Mock all I/O, test logic
- Integration tests: Test with real hardware if needed
- Target: 85% coverage for new code

### Before Submitting PR
```bash
cargo test
cargo fmt
cargo clippy
ast-grep scan --config rust_daq_ast_grep_rules.yml
git status  # Verify .beads/issues.jsonl committed
```

## Environment Notes

- **Async runtime**: Tokio (use `tokio::time::sleep`, not `std::thread::sleep`)
- **Error handling**: Use `DaqError` enum (defined in `src/error.rs`), not `anyhow!`
- **Configuration**: Load from TOML via `config/default.toml`
- **Testing**: Use `mockall` crate for mocking traits

## Making Decisions

You have full autonomy to:
- Choose implementation approaches based on documentation
- Decide when to use mocks vs hardware testing
- Break complex tasks into child Jules sessions
- Discover and create new beads issues
- Ask questions when requirements are unclear

**Key principle**: Read the documentation thoroughly, understand the existing patterns (especially the Newport V3 reference), and make informed engineering decisions. The orchestrator trusts your judgment.

## Success Criteria

- Code compiles (`cargo check` passes)
- Tests pass (`cargo test` succeeds)
- Clippy warnings addressed
- Beads issue updated with progress
- ByteRover updated with learnings (file:line specificity)
- PR description explains what/why/how

---

**Remember**: You're an autonomous engineer. Use the tools and documentation to make good decisions. The orchestrator will help with integration and coordination, but the implementation approach is yours to determine.

## Massive Parallel Execution (71-95 Concurrent Sessions)

**CRITICAL**: To maximize Jules utilization (71-95/100 sessions), you MUST use **parallel session creation**, not sequential delays.

### Parallel Session Creation Best Practices

**Key Insight from Gemini**: The 100-session quota applies to **concurrently executing** sessions (`In Progress` state), NOT total active sessions. Your goal is to keep the execution queue full by creating sessions faster than Jules can schedule them.

**Correct Approach** - Parallel Creation with Background Processes:
```bash
#!/bin/bash
# Create 20 concurrent sessions at a time

create_session_bg() {
    local task="$1"
    local repo="TheFermiSea/rust-daq"
    local max_retries=3
    local delay=1

    for ((i=0; i<max_retries; i++)); do
        output=$(jules new --repo "$repo" "$task" 2>&1)

        if [[ $? -eq 0 ]]; then
            sid=$(echo "$output" | grep "ID:" | awk '{print $2}')
            echo "[SUCCESS] Created $sid"
            return 0
        elif [[ "$output" == *"429"* ]]; then
            # Exponential backoff with jitter on rate limits
            jitter=$(echo "scale=1; $delay + $RANDOM % 2" | bc)
            sleep "$jitter"
            delay=$((delay * 2))
        fi
    done
    return 1
}

export -f create_session_bg

# Process in batches of 20 concurrent
MAX_PARALLEL=20
COUNTER=0

while IFS= read -r task; do
    create_session_bg "$task" &
    ((COUNTER++))

    if (( COUNTER % MAX_PARALLEL == 0 )); then
        wait  # Wait for batch to complete
    fi
done < tasks.txt

wait  # Wait for final batch
```

**WRONG Approach** - Sequential with Delays:
```bash
# DON'T DO THIS - takes 12-15 minutes for 93 tasks
for task in "${tasks[@]}"; do
    jules new --repo TheFermiSea/rust-daq "$task"
    sleep 8-10  # TOO SLOW!
done
```

### Session States and Lifecycle

**All Session States**:
- **Planning**: Agent is analyzing task and creating implementation plan
- **Awaiting Plan Approval**: Plan created, waiting for user/system approval
- **In Progress**: Actively executing (consumes 1 of 100 execution slots)
- **Completed**: Successfully finished
- **Failed**: Encountered unrecoverable error
- **Cancelled**: Manually terminated
- **TimedOut**: Exceeded execution time limit

**Orchestrator Intervention Points**:
1. **Stuck in Planning (>1 hour)**: Provide reference implementations and architectural context
2. **Failed**: Analyze logs, determine if retriable (network/hardware) or needs manual fix (compilation/test failures)
3. **Awaiting Plan Approval**: Review plan quality, approve if reasonable

### Session Management at Scale (50-100 Sessions)

**High-Frequency Monitoring** (run every 60 seconds):
```bash
#!/bin/bash
# Monitor session states and track progress

while true; do
    echo "=== Session State Summary ($(date)) ==="
    jules remote list --session | awk '{print $2}' | sort | uniq -c | sort -nr

    # Count sessions by state
    in_progress=$(jules remote list --session | grep "In Progress" | wc -l)
    planning=$(jules remote list --session | grep "Planning" | wc -l)
    completed=$(jules remote list --session | grep "Completed" | wc -l)

    echo "Utilization: $in_progress/100 executing | $planning queued | $completed finished"

    sleep 60
done
```

**Automated PR Review Workflow**:
```bash
#!/bin/bash
# Check CI status on open PRs and comment with @jules for failures

for pr in $(gh pr list --json number --jq '.[].number'); do
    status=$(gh pr view $pr --json statusCheckRollup --jq '.statusCheckRollup[0].state')

    if [[ "$status" == "FAILURE" ]]; then
        # Find associated beads issue from branch name
        branch=$(gh pr view $pr --json headRefName --jq '.headRefName')
        bead_id=$(echo "$branch" | grep -oP 'bd-[a-f0-9]+')

        # Comment with @jules
        gh pr comment $pr --body "@jules CI is failing. Please:
1. Rebase on latest main: git fetch origin && git rebase origin/main
2. Fix failing tests shown in CI logs
3. Run \`cargo test && cargo clippy\` locally before pushing

Issue: $bead_id
Reference: docs/rust-daq-app-architecture.md"
    fi
done
```

### Orchestrator Role (Claude Code)

**Your Focus as Orchestrator**:
1. **Strategic Decomposition**: Break epics into small, independent tasks (beads issues)
2. **Resource Allocation**: Feed Jules swarm with ready tasks to keep 71-95/100 utilization
3. **Exception Handling**: Manage stuck sessions, failed tasks, merge conflicts
4. **Integration & QC**: Review PRs not auto-merged, perform final merge
5. **Swarm Monitoring**: Watch overall health and throughput

**Jules's Autonomy**: Jules handles **entire implementation** (coding, testing, documentation) for each task.

**The Line**: You define **WHAT** (the task). Jules decides **HOW** (the implementation). Don't intervene unless stuck/failed.

### Rate Limits and Quotas

**Session Count Discrepancy Explained**:
- `jules remote list --session` count (e.g., 52): **Total Active Sessions** (Planning + Awaiting Approval + In Progress)
- Dashboard utilization (e.g., 21/100): **Concurrent Executing Sessions** (In Progress only)

**Maximizing Utilization**: Always have MORE tasks in Planning/Awaiting than available execution slots. When one of 100 In Progress completes, a Planning task immediately schedules.

**Your bottleneck is NOT the 100-session limit - it's the rate of session creation filling the queue.**