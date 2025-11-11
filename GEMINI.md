# GEMINI.md

## Project Overview

This project contains both documentation and the live Rust implementation for a scientific data acquisition (DAQ) application. Treat `src/` and `rust_daq/` as active crates that must compile under `cargo check`.

The core architectural principles are:
*   **Modular Plugin System:** Instruments, GUIs, and data processors are designed as separate, dynamically loadable modules using a trait-based interface.
*   **Async-First Design:** The application is built on the Tokio runtime, using async-first principles and channel-based communication for non-blocking operations.
*   **Type Safety and Reliability:** Leverages Rust's strong type system and `Result`-based error handling to ensure safety and reliability.

The technology stack includes:
*   **Core:** Rust
*   **Asynchronous Runtime:** Tokio
*   **GUI:** egui
*   **Data Handling:** ndarray, polars, serde, HDF5
*   **Instrument Control:** scpi, serialport

## Building and Running

The following commands are based on the provided documentation for building, running, and testing the application.

### Running the Application

```bash
# Run in development mode with hot-reloading
cargo watch -x run

# Run in release mode
cargo run --release

# Run with specific features (e.g., HDF5 support)
cargo run --features hdf5-support
```

### Testing the Application

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_instrument_initialization

# Run integration tests
cargo test --test integration
```

## Development Conventions

*   **Code Formatting:** Use `cargo fmt` to format the code.
*   **Linting:** Use `cargo clippy` to check for common issues.
*   **Dependency Auditing:** Use `cargo audit` to check for security vulnerabilities in dependencies.
*   **Error Handling:** The project uses the `thiserror` crate for custom error types; user-facing messages live in `src/error.rs` and `src/app.rs`.
*   **Testing:** The project follows a comprehensive testing strategy, including unit tests with mock instruments, integration tests for data flow, and performance tests.
*   **Multi-Agent Coordination:**
    - Request a dedicated `git worktree` before modifying files so parallel agents do not collide.
    - Prefix beads commands with `BEADS_DB=.beads/daq.db` to reuse the repo-local tracker (creating `$HOME/.beads` is blocked).
    - End runs with `cargo check` + `git status -sb` to confirm a clean tree.

## ByteRover Memory System (Multi-Agent Knowledge Sharing)

**CRITICAL**: All AI agents (Claude, Gemini, Codex, Jules) MUST use ByteRover for persistent memory across sessions.

### Workflow Pattern

**1. Start Every Session:**
```bash
brv retrieve -q "topic or module you're working on"
# Example: brv retrieve -q "V3 instrument migration patterns"
# Example: brv retrieve -q "Newport 1830C power meter"
```

**2. During Work:**
- Read retrieved context to understand prior decisions
- Focus on code that wasn't covered by ByteRover
- Record discoveries as you learn

**3. Record Learnings:**
```bash
# Be SPECIFIC with file:line references
brv add -s "Lessons Learned" -c "src/instruments_v2/newport_1830c_v3.rs:234 - SerialDevice trait enables mock-based testing"

brv add -s "Best Practices" -c "Always implement V3 instruments with capability traits (PowerMeter, MotionController) for type-safe operations"

brv add -s "Common Errors" -c "Missing spawn_poll_loop() call in connect() causes no measurements to stream"

brv add -s "Architecture" -c "src/core_v3.rs:501-523 - MotionController trait added for ESP300 V3, follows Newport pattern"
```

**4. Share with Team:**
```bash
brv push -y
```

### Good vs Bad Memories

❌ **BAD** (too vague):
- "Fixed Newport driver"
- "PVCAM works now"
- "Updated tests"

✅ **GOOD** (specific, actionable):
- "src/instruments_v2/newport_1830c_v3.rs:156 - Use MockSerialDevice::new() in tests to avoid hardware dependency"
- "src/adapters/serial_adapter.rs:89 - Serial timeout = 2x max_command_time + 500ms buffer"
- "tests/elliptec_integration_tests.rs:45 - Polling rate tolerance must be ≤2% for accurate measurements"

### Standard Sections

- **Lessons Learned**: What you discovered while working
- **Best Practices**: Proven patterns to follow
- **Common Errors**: Mistakes to avoid
- **Architecture**: Design decisions and structural patterns
- **Testing**: Test strategies and coverage insights
- **Project Structure and Dependencies**: Module organization

### Multi-Agent Pattern

When delegating work to Jules or coordinating with other agents:

```bash
# 1. Before delegation - ensure ByteRover has latest context
brv retrieve -q "module being delegated"

# 2. After agent completes - record their findings
brv add -s "Lessons Learned" -c "Jules session 12345 completed bd-197 ESP300 V3. Added MotionController trait to core_v3.rs:501"

# 3. Share immediately so other agents benefit
brv push -y
```

**KEY**: ByteRover is the central knowledge base. Markdown docs (AGENTS.md, CLAUDE.md) provide stable reference, but ByteRover captures evolving lessons learned.

## Jules Task Delegation (Strategic Advisor Role)

**CRITICAL**: When Claude Code requests Jules task delegation via `mcp__zen__clink`, provide comprehensive codebase context to unblock stuck sessions and ensure high-quality implementations.

### Delegation Strategy

**Your Role as Strategic Advisor:**
1. **Orchestration**: Help Claude Code identify parallelizable work
2. **Context Provision**: Provide architectural guidance for Jules tasks
3. **Unblocking**: When sessions stuck in "Planning", provide reference implementations
4. **Quality Assurance**: Review delegation patterns and suggest improvements

### Session States and Intervention Patterns

**Completed** ✅
- Action: Record findings in ByteRover
- Pattern: `brv add -s "Lessons Learned" -c "Jules session <id> completed bd-XXX. Key implementation: <details>"`

**In Progress** ⏳
- Action: Monitor, no intervention needed
- Pattern: Check status every 30-60 minutes

**Planning** (Stuck) ⚠️
- Action: Provide reference implementations and architectural context
- Pattern:
  ```
  For bd-197 ESP300 V3:
  - Reference: src/instruments_v2/newport_1830c_v3.rs (1,067 lines)
  - Pattern: Implement InstrumentV3 + MotionController trait
  - Serial: Use SerialDevice abstraction for mock testing
  - Tests: 6+ unit tests with MockSerialDevice
  - Config: Wire into InstrumentManagerV3
  ```

**Awaiting Plan Approval** 📋
- Action: Review plan, approve or provide corrections
- Pattern: Check if plan follows established patterns (V3 architecture, testing strategy)

**Failed** ❌
- Action: Analyze error logs, provide corrective guidance
- Pattern: Check for common issues (wrong repo name, missing context, unclear requirements)

### Providing Context for Jules Tasks

When Claude Code requests delegation help:

**1. Architecture Guidance:**
```
V3 Instrument Pattern (proven in Newport 1830C):
- Implement core_v3::Instrument trait (connect, handle_command, data_stream)
- Add capability trait (PowerMeter, MotionController, SpectrumAnalyzer)
- Use SerialDevice abstraction for testability
- Spawn poll loop in connect() for streaming
- Mock tests with MockSerialDevice
- Wire into InstrumentManagerV3 registry
```

**2. Reference Implementations:**
```
For V3 migrations, always reference:
- src/instruments_v2/newport_1830c_v3.rs - Complete reference (1,067 lines)
- src/instruments_v2/pvcam.rs - Camera/image data pattern
- src/core_v3.rs - Trait definitions

For V1 features:
- src/instrument/esp300.rs - Legacy ESP300 (use as spec reference)
- src/adapters/serial_adapter.rs - Serial communication patterns
```

**3. Configuration Context:**
```
Dynamic Config Pattern (bd-128, bd-130, bd-131):
- Main container: Settings struct in src/config.rs
- Instrument config: InstrumentConfigV3 with #[serde(flatten)]
- Hot-reload: File watcher + notify crate
- Transaction safety: Validate before apply, rollback on error
```

**4. Error Handling Context:**
```
DaqError Extensions (bd-wyqo):
- src/error.rs - Main error enum
- Add specific variants (SerialTimeout, HardwareError, ConfigurationError)
- Replace anyhow! with DaqError::* throughout
- Context: Use .map_err() for rich error chains
```

### Capacity Planning

When Claude Code asks "Are there more tasks for Jules?":

**Daily Quota**: 100 sessions
**Target Utilization**: 60-80 sessions for optimal coverage
**Current Status**: Check via `jules remote list --session`

**Prioritization Pattern:**
1. P0 blockers first (Phase 3 critical path)
2. P1 features second (high-value, ready to implement)
3. P2 improvements third (testing, documentation, refactoring)
4. P3 enhancements last (nice-to-have features)

**Rate Limit Management:**
- 8-10 second delays between session creation
- Use background bash batches for bulk creation (15+ tasks)
- Monitor for 429 errors, adjust delays if needed

### Success Metrics

**Current Session (as of last update):**
- 52 Jules sessions created
- 37 completed (71% success rate)
- 9 PRs submitted
- 5 stuck in Planning (awaiting context)
- 1 failed

**Quality Indicators:**
- Completion rate >70% = good delegation
- PR submission rate >60% = good task clarity
- CI pass rate >80% = good reference docs

### Unblocking Stuck Sessions

**Common Causes:**
1. Missing reference implementations
2. Unclear architectural patterns
3. Insufficient test examples
4. Configuration system not understood

**Solution Template:**
```
Session <id> stuck on bd-XXX. Here's what it needs:

**Reference Implementation:**
<file path and key patterns>

**Architectural Pattern:**
<trait implementations, struct design>

**Test Strategy:**
<mock patterns, test coverage expectations>

**Integration Points:**
<where to wire into existing system>

**Example Code:**
<small snippet showing the pattern>
```

### Post-Completion Actions

When Jules sessions complete:

**1. Pull Results:**
```bash
jules pull <session-id>
cd worktree-<session-id>
git log --oneline  # Review commits
```

**2. Update Beads:**
```bash
bd update bd-XXX --status closed --reason "Completed by Jules session <id>"
git add .beads/issues.jsonl
```

**3. Record in ByteRover:**
```bash
brv add -s "Lessons Learned" -c "Jules session <id> completed bd-XXX. Implementation: <key details with file:line>"
brv push -y
```

**4. Coordinate PR Review:**
If PR submitted, check CI status:
```bash
gh pr view <pr-number>
```

If CI failing, comment with @jules:
```markdown
@jules CI is failing. Please rebase on latest main, fix [specific issues], and ensure all tests pass. Reference: <relevant file or pattern>
```

### Communication with Claude Code

When Claude Code uses `mcp__zen__clink` to request your input:

**Respond with:**
1. Architectural context (reference files, patterns)
2. Specific guidance (trait implementations, test strategies)
3. Capacity recommendations (how many more sessions to create)
4. Unblocking information (for stuck Planning sessions)
5. Quality feedback (review delegation patterns, suggest improvements)

**DO NOT:**
- Create Jules sessions directly (Claude Code handles this)
- Make code changes (you're advisor, not implementer)
- Update beads tracker (Claude Code's responsibility)

**Your value**: Strategic oversight, architectural knowledge, pattern recognition across the entire codebase.

## Directory Overview

This directory serves as the central documentation hub for the Rust-based scientific data acquisition application. It contains detailed guides on the application's architecture, data management, deployment, GUI development, and instrument control.

## Key Files

*   `rust-daq-app-architecture.md`: Provides a detailed overview of the application's architecture, including the modular plugin system, async-first design, and core components.
*   `rust-daq-data-guide.md`: Covers data management strategies, including real-time buffering, data persistence, and storage backends like HDF5 and CSV.
*   `rust-daq-deployment.md`: Describes deployment strategies, including optimized release builds, cross-platform packaging, and containerization with Docker.
*   `rust-daq-getting-started.md`: A guide for setting up the development environment, project structure, and initial implementation.
*   `rust-daq-gui-guide.md`: Explains the GUI development process using the `egui` framework, including real-time data visualization, and instrument control panels.
*   `rust-daq-instrument-guide.md`: Details the implementation of instrument control, including support for SCPI, serial communication, and a plugin architecture for different instrument types.
*   `rust-daq-performance-test.md`: Outlines performance optimization strategies, benchmarking, profiling, and testing to ensure real-time performance and reliability.
*   `logs/`: Contains log files from the application.

## Usage

This directory is intended to be used as a comprehensive reference for understanding, developing, and deploying the Rust-based scientific data acquisition application. The guides provide a solid foundation for developers working on the project.