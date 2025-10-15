# bd + Jules Integration Workflow

This document explains how to integrate bd (beads) issue tracking with Jules AI coding sessions for optimal workflow.

## ⚠️ Important: Jules Working Directory vs Main Project

**Jules Working Directory**: Jules AI agents work in a separate `rust_daq/` directory structure that is NOT the main project layout. This is Jules-specific workspace isolation.

**Main Project Structure**: Production code lives in `src/` at repository root (flat single-crate structure). See [AGENTS.md](AGENTS.md) for current project structure.

**Path References in This Document**: Examples below use Jules-specific `rust_daq/src/` paths. When working directly in the main repository, use `src/` paths instead.

## Why Integrate bd and Jules?

- **bd**: Tracks work queue, priorities, and dependencies
- **Jules**: Implements the actual code changes
- **Together**: bd provides context and tracking, Jules provides implementation

## Workflow Overview

```
1. bd ready          → See what's ready to work on
2. bd show <id>      → Get issue details
3. Create Jules      → With bd context included
4. Jules works       → Creates PR with changes
5. Review & merge    → Merge the PR
6. bd done <id>      → Mark issue complete
```

## Step-by-Step Integration

### 1. Check Ready Work

```bash
cd /Users/briansquires/code/rust-daq
bd ready
```

Example output:
```
📋 Ready work (6 issues with no blockers):

1. [P0] daq-8: Fix test infrastructure - tests hang indefinitely
2. [P0] daq-7: Implement HDF5 data persistence in experiment.rs
3. [P1] daq-9: Reduce unwrap/expect calls to prevent panics
...
```

### 2. Get Issue Details

```bash
bd show daq-7
```

This shows:
- Full description
- Priority level
- Dependencies (blocks/blocked-by)
- Created/updated timestamps

### 3. Create Jules Session with bd Context

When creating a Jules session, include bd information:

**Title Format**: `[Priority] [Issue-ID] Issue Title`
```
[P0] [daq-7] Implement HDF5 data persistence in experiment.rs
```

**Prompt Format**: Include all bd context
```markdown
**bd Issue**: daq-7 (P0)
**Blocks**: (list any issues this blocks)
**Blocked By**: (list any dependencies)

## Problem Description
[bd issue description]

## Current State
[specific file locations in NEW workspace structure]

## Required Changes
[detailed implementation steps]

## Success Criteria
[how to verify completion]
```

### 4. Example: Creating Jules Session from bd Issue

```python
# For daq-7 (HDF5 implementation)
Source: sources/github/TheFermiSea/rust-daq
Title: [P0] [daq-7] Implement HDF5 data persistence in experiment.rs

Prompt:
**bd Issue**: daq-7 (P0 - Critical)
**Path**: /Users/briansquires/code/rust-daq
**Jules Working Directory**: rust_daq/src/experiment.rs (Jules-specific path)
**Main Project Path**: src/experiment.rs (actual repository structure)

## Problem Description
6 TODO comments in src/experiment.rs lines 687-795. HDF5 file initialization, data writing, and finalization are stubbed out. Experiments claim to save data but don't. Wire up existing Hdf5ExperimentWriter and implement actual I/O operations.

## Current State
The Elliptec scan experiment in src/experiment.rs has:
- Line 687: "TODO: Initialize HDF5 writer"
- Line 769: "TODO: Save image and metadata to HDF5"
- Line 774: Debug logging for HDF5 saves
- Line 793: "TODO: Finalize HDF5 file"

The Hdf5ExperimentWriter struct exists but is never used.

## Required Changes
1. At line 687: Initialize Hdf5ExperimentWriter with config.hdf5_filepath
2. At line 769: Implement actual data writing (save images and metadata)
3. Add proper error handling for HDF5 operations using DaqError
4. At line 793: Properly finalize and close the HDF5 file
5. Wire up the existing storage infrastructure in src/data/storage.rs

## Success Criteria
- All 6 TODO comments removed
- Data actually persists to HDF5 files at specified paths
- Proper error handling with DaqError propagation
- Tests can verify data was written correctly
- cargo test passes with storage_hdf5 feature enabled

## Important Notes for Jules Sessions
- Jules works in isolated rust_daq/ directory (NOT main project structure)
- Main project uses flat structure: code in src/ at repository root
- HDF5 feature flag: storage_hdf5
- Related files: src/data/storage.rs, src/experiment.rs (main project paths)
```

### 5. Track Jules Session in bd

Add Jules session URL to bd issue:
```bash
# After creating Jules session, add a note
bd comment daq-7 "Jules session: https://jules.google.com/session/<id>"
```

### 6. Monitor Progress

Check Jules activities:
```bash
# In Claude Code, use Jules tools
mcp__jules__list-activities sessionId: <session-id>
```

### 7. After Jules Completes

When Jules creates a PR:

1. **Review the PR**:
   ```bash
   gh pr view <number>
   gh pr checks <number>
   ```

2. **Merge if good**:
   ```bash
   gh pr merge <number> --squash
   ```

3. **Mark bd issue complete**:
   ```bash
   bd done daq-7
   ```

4. **Check what's now ready** (dependencies unblocked):
   ```bash
   bd ready
   ```

## Best Practices

### ✅ DO

- Always include `[bd-issue-id]` in Jules session titles
- Include bd priority levels (P0/P1/P2) in titles
- Reference blocking/blocked issues in prompts
- Specify WORKSPACE structure paths (rust_daq/src/...)
- Update bd issue when Jules creates PR
- Mark bd issue done only after PR is merged

### ❌ DON'T

- Create Jules sessions for blocked issues (wait for dependencies)
- Use old flat structure paths (src/ instead of rust_daq/src/)
- Start multiple Jules sessions on dependent issues
- Mark bd issues done before PR is merged
- Skip including bd context in Jules prompts

## Dependency-Aware Workflow

bd tracks dependencies. Use this to avoid conflicts:

```bash
# Check what blocks an issue
bd show daq-2
# Output: Blocked by: daq-8

# Don't start daq-2 until daq-8 is done!
# Instead, work on daq-8 first
bd ready  # Shows only unblocked issues
```

## Example Full Workflow

```bash
# 1. Start of session
cd /Users/briansquires/code/rust-daq
bd ready

# 2. Pick highest priority ready issue
bd show daq-8  # P0, no blockers

# 3. Create Jules session with full bd context
# [Use Claude Code Jules tools with prompt including bd info]

# 4. Wait for Jules to complete
# Jules creates PR #17

# 5. Review and merge
gh pr view 17
gh pr merge 17 --squash

# 6. Update bd
bd done daq-8

# 7. Check newly unblocked work
bd ready  # Now daq-2 and daq-3 are ready!
bd dep tree daq-6  # See the full chain
```

## Summary

**bd** = Work queue + dependencies + priorities
**Jules** = Implementation worker
**Integration** = bd context → Jules prompts → Merged PRs → bd tracking

This gives you:
- Clear work prioritization (bd ready)
- Dependency management (bd dep tree)
- AI implementation (Jules)
- Progress tracking (bd status)
- Clean workflow (ready → implement → merge → done)
