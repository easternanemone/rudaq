# Hook Tests

This directory contains functional tests for Claude Code hooks used in the rust-daq project.

## Test Coverage

### pre-commit-checks.sh Tests
- ✓ Non-commit commands pass through without triggering hooks
- ✓ Commits with --no-verify flag skip checks
- ✓ Regular git commits trigger cargo fmt, ast-grep, and clippy checks
- ✓ Detection of git commit in piped/chained commands
- ✓ Script exists and is executable
- ✓ Proper shebang line

**Hook Purpose:** Tier 2 lint gate - validates code formatting and clippy warnings before commit.

### pre-push-checks.sh Tests
- ✓ Non-push commands pass through without triggering hooks
- ✓ git push commands trigger test execution
- ✓ Detection of git push in piped/chained commands
- ✓ Script exists and is executable
- ✓ Proper shebang line
- ✓ Uses cargo-nextest or cargo test for running tests
- ✓ Excludes ui crate from test runs

**Hook Purpose:** Tier 3 test gate - ensures all tests pass before pushing to remote.

### rustfmt-on-save.sh Tests
- ✓ Non-Rust files are skipped
- ✓ Rust files trigger rustfmt formatting
- ✓ Non-existent files are handled gracefully
- ✓ Script exists and is executable
- ✓ Proper shebang line
- ✓ Hook is non-blocking (always exits 0)
- ✓ Checks for rustfmt availability

**Hook Purpose:** Tier 1 auto-format - formats Rust files after Edit/Write operations.

### session-start.sh Tests
- ✓ Script exists and is executable
- ✓ Proper shebang line
- ✓ Subagents exit early (no status output for explore/plan agents)
- ✓ Main session shows task status
- ✓ Checks for .beads directory
- ✓ Checks for bd command availability
- ✓ Includes task status sections (in_progress, ready, blocked, stale)
- ✓ Handles missing bd command gracefully
- ✓ Creates marker file for subagents

**Hook Purpose:** SessionStart hook - displays beads task status and PR reminders at session start.

## Running Tests

### Run All Hook Tests
```bash
bash tests/hooks/run_all_tests.sh
```

### Run Individual Test Suites
```bash
bash tests/hooks/test_pre_commit_checks.sh
bash tests/hooks/test_pre_push_checks.sh
bash tests/hooks/test_rustfmt_on_save.sh
bash tests/hooks/test_session_start.sh
```

## Test Structure

Each test script:
1. Uses colored output (green ✓ for pass, red ✗ for fail)
2. Provides detailed error messages on failure
3. Tracks test count and pass/fail statistics
4. Exits with code 0 on success, 1 on any failure
5. Uses mock JSON input to simulate Claude Code tool invocations

## Dependencies

- bash
- jq (used by hooks for JSON parsing)
- Standard Unix utilities (grep, head, mktemp)

## Adding New Tests

To add tests for a new hook:

1. Create `test_<hook_name>.sh` in this directory
2. Follow the existing test structure (see any test file for template)
3. Make the script executable: `chmod +x test_<hook_name>.sh`
4. Add the test to `run_all_tests.sh`
5. Update this README with test coverage information

## CI Integration

These tests are designed to run in CI environments without requiring cargo/rustc.
They validate hook behavior, structure, and error handling independently of Rust toolchain availability.