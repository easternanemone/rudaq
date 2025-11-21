# Jules Setup Complete ✅

**Date**: 2025-11-21
**Status**: Ready for development

## What Was Created

### 1. `.jules/setup.sh` (Committed)
Automated setup script that Jules runs on session start:
- ✅ Verifies Rust toolchain (rustfmt, clippy)
- ✅ Checks protobuf compiler (required for gRPC)
- ✅ Builds with `networking` feature
- ✅ Runs sanity tests (116 tests expected)
- ✅ Fast: Uses cached artifacts (~2-3 seconds)

### 2. `.jules/agents.md` (Committed)
Comprehensive architecture guide for Jules agents:
- V5 headless-first architecture overview
- Phase 3 Network Layer completion status
- Development commands and feature flags
- Common patterns (broadcast channels, DataPoint flow)
- Debugging tips and pitfalls to avoid
- Phase 4 Arrow batching roadmap

### 3. `.jules/.gitignore` (Committed)
Keeps Jules session files out of git

## Environment Variables for Jules UI

Configure these in the Jules web interface (shown in your screenshot):

### Required Variables

| Key | Value | Description |
|-----|-------|-------------|
| `RUST_LOG` | `info` | Logging level (use `debug` for debugging) |
| `RUST_BACKTRACE` | `1` | Enable backtraces for error diagnosis |
| `CARGO_INCREMENTAL` | `1` | Enable incremental compilation |

### Optional Variables (Advanced)

| Key | Value | Description |
|-----|-------|-------------|
| `DAQ_DAEMON_PORT` | `50051` | gRPC daemon port (if testing networking) |
| `RINGBUFFER_SIZE` | `100` | Ring buffer size for data streaming |
| `HDF5_DIR` | `/usr/local` | HDF5 installation path (if custom location) |

### For Remote Hardware Testing

| Key | Value | Description |
|-----|-------|-------------|
| `MAITAI_HOST` | `maitai@100.117.5.12` | Remote hardware test machine |
| `MAITAI_DAQ_PATH` | `~/rust-daq` | rust-daq installation path on maitai |

## How to Use in Jules UI

Based on your screenshot:

1. **Setup Script Field**: Enter `echo do_setup` (or leave as shown)
2. **Environment Variables Section**:
   - Click the "+" button to add each variable
   - Enter Key, Value, and optional Description
   - Click "Save" after adding all variables

### Example Configuration

```
Key: RUST_LOG
Value: info
Description: Control logging verbosity for rust-daq

Key: RUST_BACKTRACE
Value: 1
Description: Enable stack traces for debugging

Key: CARGO_INCREMENTAL
Value: 1
Description: Speed up recompilation during development
```

3. **Click "Run and snapshot"** to test the setup

## Verification

After setup, Jules should display:
```
✅ rust-daq environment ready!

📚 Architecture: V5 Headless-First (see .jules/agents.md)
🚀 Phase 3 Complete: Network Layer with gRPC + CLI client
🔜 Phase 4 Next: Arrow batching (PR #104)
```

## What Jules Agents Now Know

When Jules agents start working on PRs, they will automatically:

1. **Run `.jules/setup.sh`** → Environment verified and built
2. **Read `.jules/agents.md`** → Understand V5 architecture and Phase 3 completion
3. **Have access to env vars** → Proper logging and build configuration

### Key Context Provided

- ✅ V5 headless-first architecture (not GUI-focused)
- ✅ Phase 3 Network Layer complete (gRPC, broadcast channels, CLI client)
- ✅ Phase 4 next: Arrow batching (PR #104 needs rebase)
- ✅ Feature flag usage: Always build with `--features networking`
- ✅ Module patterns: Use `measurement_types::DataPoint` (not `grpc::server::DataPoint`)
- ✅ Test expectations: 116 tests should pass
- ✅ Common pitfalls: Avoid circular dependencies, don't modify V2 code

## Benefits

### For Jules Agents Working on PRs:

1. **Faster Onboarding**: Setup runs automatically, no manual configuration
2. **Architecture Awareness**: Agents know about Phase 3 completion and V5 design
3. **Correct Build Commands**: Always uses `--features networking`
4. **Better PRs**: Follows established patterns (broadcast channels, DataPoint flow)
5. **Fewer Errors**: Knows common pitfalls (module visibility, feature flags)

### For Your Workflow:

1. **Consistent Environment**: All Jules sessions start with verified build
2. **Self-Documenting**: Architecture guide lives with the code
3. **Easier Reviews**: PRs follow documented patterns
4. **Scalable**: New contributors get same context via Jules

## Next Steps

1. ✅ **Configure Jules UI** with the environment variables above
2. ✅ **Click "Run and snapshot"** to verify setup works
3. ✅ **Create a test PR** with Jules to verify agent behavior
4. 🔄 **Monitor PR #104** (Arrow batching) - Jules agents now know this is Phase 4 work

## Testing the Setup

Create a test Jules session and verify:

```bash
# Jules should automatically run .jules/setup.sh
# Then you can test commands:

cargo build --lib --features networking
# Should complete quickly (uses cache)

cargo test --lib --features networking
# Should show: "test result: ok. 116 passed; 0 failed"

# Jules agents will see .jules/agents.md content
cat .jules/agents.md
# Should show architecture guide
```

## Troubleshooting

### Setup Script Fails

**Problem**: "protoc not found"
**Solution**: Install protobuf compiler:
- macOS: `brew install protobuf`
- Linux: `apt-get install protobuf-compiler`

**Problem**: "Build failed"
**Solution**: Check that HDF5 is installed (if using storage feature)
- macOS: `brew install hdf5`
- Linux: `apt-get install libhdf5-dev`

### Jules Agent Confusion

**Problem**: Agent tries to modify V2 code
**Solution**: `.jules/agents.md` explicitly lists V2 modules as REMOVED - agent should see this

**Problem**: Agent uses wrong DataPoint import
**Solution**: Guide shows correct pattern: `crate::measurement_types::DataPoint`

## Maintenance

### When Architecture Changes:

Update `.jules/agents.md` with:
- New phase completions
- Changed module structure
- New patterns or best practices

### When Dependencies Change:

Update `.jules/setup.sh` with:
- New system dependencies
- Changed build commands
- Additional verification steps

---

**Committed to**: Both TheFermiSea and easternanemone repos
**Commit**: dc580e81 - feat(jules): add Jules AI assistant setup and architecture guide
**Files Created**: 3 (setup.sh, agents.md, .gitignore)
**Total Lines**: 242 lines of Jules configuration
