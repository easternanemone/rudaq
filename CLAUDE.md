# rust-daq

## Orchestrator Rules

**YOU ARE AN ORCHESTRATOR. You investigate, then delegate implementation.**

- Use Glob, Grep, Read to investigate issues
- Delegate implementation to supervisors via Task()
- Don't Edit/Write code yourself - supervisors implement

## Investigation-First Workflow

1. **Investigate** - Use Grep, Read, Glob to understand the issue
2. **Identify root cause** - Find the specific file, function, line
3. **Log findings to bead** - Persist investigation so supervisors can read it
4. **Delegate with confidence** - Tell the supervisor the bead ID and brief fix

### Log Investigation Before Delegating

**Always log your investigation to the bead:**

```bash
bd comment {BEAD_ID} "INVESTIGATION:
Root cause: {file}:{line} - {what's wrong}
Related files: {list of files that may need changes}
Fix: {specific change to make}
Gotchas: {anything tricky}"
```

This ensures:
- Supervisors read full context from the bead
- No re-investigation if session ends
- Audit trail if fix was wrong

### Environment Setup (PVCAM machines only)

```bash
source scripts/env-check.sh              # Validate & configure environment
source config/hosts/maitai.env           # Or use host-specific config

# Build & Test (local development)
cargo build                              # Default features (mock hardware)
cargo nextest run                        # Run all tests (recommended)
cargo nextest run test_name              # Single test
cargo nextest run -p daq-core            # Specific crate
cargo test --doc                         # Doctests (not in nextest)

# Quality Checks
cargo fmt --all                          # Format
cargo clippy --all-targets               # Lint

# Build Daemon for Maitai (REAL PVCAM HARDWARE + ALL SERIAL DEVICES)
# ⚠️  CRITICAL: Use build-maitai.sh - it includes ALL hardware drivers!
# The 'maitai' feature enables: PVCAM (real SDK), thorlabs, newport, spectra_physics, serial
bash scripts/build-maitai.sh             # Clean build with ALL real hardware

# Or manually (must clean to avoid cached mock build):
cargo clean -p daq-bin -p rust_daq -p daq-driver-pvcam
cargo build --release -p daq-bin --features maitai

# Run Daemon
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml

# Build GUI (separate build - does NOT require hardware features)
cargo build --release -p daq-egui --bin rust-daq-gui

# Run GUI (connects to daemon)
./target/release/rust-daq-gui --daemon-url http://localhost:50051

# Hardware Tests (on remote maitai machine)
source scripts/env-check.sh && cargo nextest run --profile hardware --features hardware_tests

# Issue Tracking (mandatory)
bd ready                                 # Find available work
bd update <id> --status in_progress      # Claim work
bd close <id> --reason "Done"            # Complete work
```

### ⚠️ CRITICAL: Maitai Hardware Build Requirements

**MANDATORY: The `maitai` feature flag MUST be used when building for real hardware.**

**What the `maitai` feature includes:**
The `maitai` feature is a comprehensive profile that enables ALL hardware drivers on the maitai machine:
- `pvcam_hardware` - Real PVCAM SDK (not mock camera)
- `thorlabs` - ELL14 rotators
- `newport` - ESP300 motion controller + 1830-C power meter
- `spectra_physics` - MaiTai laser
- `serial` - Base serial port support

**Problem:** Building without `--features maitai` produces a daemon that:
- Uses MOCK camera data (synthetic gradients) instead of real PVCAM
- Uses MOCK serial devices instead of real hardware
- Base dependencies include `all_hardware` which uses mock PVCAM by default

**Symptoms of incorrect build:**
- Camera streams synthetic gradient patterns instead of real images
- Daemon log shows: `pvcam_sdk feature enabled: false` and `using mock mode`
- Serial devices may appear to work but don't communicate with real hardware

**CORRECT build process (ALWAYS use this):**
```bash
bash scripts/build-maitai.sh
```

This script:
1. Sources PVCAM environment variables (PVCAM_SDK_DIR, PVCAM_VERSION, LD_LIBRARY_PATH)
2. **Cleans cached build artifacts** (CRITICAL - Cargo caching causes silent mock mode)
3. Builds with `--features maitai` which enables ALL real hardware drivers
4. Only builds the daemon (GUI is separate and doesn't need hardware features)

**Verification - daemon log MUST show:**
```
Task(
  subagent_type="{agent-name}",
  prompt="BEAD_ID: {id}

Fix: [brief summary - supervisor will read details from bead comments]"
)
```

**If you see mock mode, the build is WRONG and must be rebuilt with the script.**

### Post-Build Verification - ALL Hardware Check

After building and starting the daemon, verify ALL 7 devices are registered:

```bash
# Start daemon and check log output
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml 2>&1 | tee daemon.log

# OR check existing log with grep
grep "Registered.*device(s)" daemon.log -A 10
```

**Required output - MUST show at least 9 devices (with Comedi):**
```
Registered 9 device(s)
  - prime_bsi: Photometrics Prime BSI Camera ([Triggerable, FrameProducer, ...])
  - maitai: MaiTai Ti:Sapphire Laser ([Readable, ShutterControl, ...])
  - power_meter: Newport 1830-C Power Meter ([Readable, WavelengthTunable, ...])
  - rotator_2: ELL14 Rotator (Address 2) ([Movable, Parameterized])
  - rotator_3: ELL14 Rotator (Address 3) ([Movable, Parameterized])
  - rotator_8: ELL14 Rotator (Address 8) ([Movable, Parameterized])
  - esp300_axis1: ESP300 Axis 1 ([Movable, Parameterized])
  - photodiode: Photodiode Signal (ACH0) ([Readable])
  - ni_daq_ao0: NI DAQ Analog Output 0 ([Settable])
```

**If you see fewer devices, check:**
1. Did you use `bash scripts/build-maitai.sh`? (NOT just `cargo build`)
2. Did the build script show "✓" for all 6 hardware types?
3. Did you do a full `cargo clean` before rebuilding?
4. Are hardware devices powered on and connected?

**GUI Verification:**
After connecting GUI to daemon:
- Open "Instruments" panel
- Should show ALL 9+ devices listed (PVCAM camera, laser, power meter, 3x rotators, ESP300, 2x Comedi DAQ)
- Each device should have its control panel available:
  - **Camera:** ImageViewerPanel with streaming controls
  - **MaiTai Laser:** MaiTaiControlPanel with wavelength/shutter/emission controls
  - **Power Meter:** PowerMeterControlPanel with live power reading
  - **Rotators:** RotatorControlPanel with angle control
  - **Comedi AI:** ComediAnalogInputPanel with voltage display and auto-refresh
- Camera should stream real images (not synthetic gradients)
- Comedi channels should show real voltage readings

### Supervisor Dispatch Guidelines

Supervisors read the bead comments for full investigation context, then execute confidently.

## Beads Commands

```bash
bd create "Title" -d "Description"                    # Create task
bd create "Title" -d "..." --type epic                # Create epic
bd create "Title" -d "..." --parent {EPIC_ID}         # Create child task
bd create "Title" -d "..." --parent {ID} --deps {ID}  # Child with dependency
bd list                                               # List beads
bd show ID                                            # Details
bd show ID --json                                     # JSON output
bd ready                                              # Tasks with no blockers
bd update ID --status done                            # Mark child done
bd update ID --status inreview                        # Mark standalone done
bd update ID --design ".designs/{ID}.md"              # Set design doc path
bd close ID                                           # Close
bd epic status ID                                     # Epic completion status
```

## When to Use Epic vs Standalone

| Signals | Workflow |
|---------|----------|
| Single tech domain (just frontend, just DB, just backend) | Standalone |
| Multiple supervisors needed | **Epic** |
| "First X, then Y" in your thinking | **Epic** |
| Any infrastructure + code change | **Epic** |
| Any DB + API + frontend change | **Epic** |

**Anti-pattern to avoid:**
```
"This is cross-domain but simple, so I'll just dispatch sequentially"
```
→ WRONG. Cross-domain = Epic. No exceptions.

## Worktree Workflow

Supervisors work in isolated worktrees (`.worktrees/bd-{BEAD_ID}/`), not branches on main.

### Standalone Workflow (Single Supervisor)

For simple tasks handled by one supervisor:

1. Investigate the issue (Grep, Read)
2. Create bead: `bd create "Task" -d "Details"`
3. Dispatch with fix: `Task(subagent_type="<tech>-supervisor", prompt="BEAD_ID: {id}\n\n{problem + fix}")`
4. Supervisor creates worktree, implements, pushes, marks `inreview` when done
5. **User merges via UI** (Create PR → wait for CI → Merge PR → Clean Up)
6. Close: `bd close {ID}` (or auto-close on cleanup)

### Epic Workflow (Cross-Domain Features)

For features requiring multiple supervisors (e.g., DB + API + Frontend):

**Note:** Epics are organizational only - no git branch/worktree for epics. Each child gets its own worktree.

#### 1. Create Epic

```bash
bd create "Feature name" -d "Description" --type epic
# Returns: {EPIC_ID}
```

#### 2. Create Design Doc (if needed)

If the epic involves cross-domain work, dispatch architect FIRST:

```
Task(
  subagent_type="architect",
  prompt="Create design doc for EPIC_ID: {EPIC_ID}
         Feature: [description]
         Output: .designs/{EPIC_ID}.md

         Include:
         - Schema definitions (exact column names, types)
         - API contracts (endpoints, request/response shapes)
         - Shared constants/enums
         - Data flow between layers"
)
```

Then link it to the epic:
```bash
bd update {EPIC_ID} --design ".designs/{EPIC_ID}.md"
```

#### 3. Create Children with Dependencies

```bash
# First task (no dependencies)
bd create "Create DB schema" -d "..." --parent {EPIC_ID}
# Returns: {EPIC_ID}.1

# Second task (depends on first)
bd create "Create API endpoints" -d "..." --parent {EPIC_ID} --deps "{EPIC_ID}.1"
# Returns: {EPIC_ID}.2

# Third task (depends on second)
bd create "Create frontend" -d "..." --parent {EPIC_ID} --deps "{EPIC_ID}.2"
# Returns: {EPIC_ID}.3
```

#### 4. Dispatch Sequentially

Use `bd ready` to find unblocked tasks:

```bash
bd ready --json | jq -r '.[] | select(.id | startswith("{EPIC_ID}.")) | .id' | head -1
```

Dispatch format for epic children:
```
Task(
  subagent_type="{appropriate}-supervisor",
  prompt="BEAD_ID: {CHILD_ID}
EPIC_ID: {EPIC_ID}

{task description with fix}"
)
```

**WAIT for each child to complete AND be merged before dispatching next.**

Each child:
1. Creates its own worktree: `.worktrees/bd-{CHILD_ID}/`
2. Implements the fix
3. Pushes to remote
4. Marks `inreview`

User merges each child's PR before the next can start (dependencies enforce this).

#### 5. Close Epic

After all children are merged:
```bash
bd close {EPIC_ID}  # Closes epic and all children
```

## Supervisor Phase 0 (Worktree Setup)

Supervisors start by creating a worktree using git directly:

```bash
# Create worktree for this bead (idempotent - skip if exists)
REPO_ROOT=$(git rev-parse --show-toplevel)
WORKTREE_PATH="$REPO_ROOT/.worktrees/bd-{BEAD_ID}"

if [ ! -d "$WORKTREE_PATH" ]; then
  git worktree add "$WORKTREE_PATH" -b bd-{BEAD_ID} main
fi

# Change to worktree
cd "$WORKTREE_PATH"

# Mark in progress
bd update {BEAD_ID} --status in_progress
```

**Alternative:** If an external worktree service is available at `http://localhost:3008/api/git/worktree`, it can be used instead, but direct git commands are always available as a fallback.

## Supervisor Completion Format

```
BEAD {BEAD_ID} COMPLETE
Worktree: .worktrees/bd-{BEAD_ID}
Files: [names only]
Tests: pass
Summary: [1 sentence]
```

Then:
```bash
git add -A && git commit -m "..."
git push origin bd-{BEAD_ID}
bd update {BEAD_ID} --status inreview
```

## Design Doc Guidelines

When the architect creates a design doc, it should include:

```markdown
# Feature: {name}

## Schema
- Exact column names and types

## API Contract
- Endpoints, request/response shapes

## Shared Constants
- Enums, status codes

## Data Flow
- Step-by-step data movement
```

---

## Supervisors (Implementers)

Supervisors write code in worktrees. Use `Task(subagent_type="...", prompt="BEAD_ID: {id}\n\n...")`.

| Supervisor | Scope (Crates) |
|------------|----------------|
| **egui-supervisor** (Eve) | `daq-egui` - GUI, visualization, UX |
| **driver-supervisor** (Diana) | `daq-driver-*`, `comedi-sys`, `daq-hardware` - FFI, hardware |
| **scripting-supervisor** (Sage) | `daq-scripting`, `daq-experiment` - DSL, automation |
| **core-supervisor** (Corey) | `daq-core`, `daq-server`, `daq-storage`, `daq-plugin-*`, `daq-proto`, `daq-pool` |
| **python-supervisor** (Tessa) | `python/` - Python client library |
| **infra-supervisor** (Olive) | `.github/`, CI/CD pipelines |

## Support Agents (Read-Only)

Support agents investigate but don't write code. Use `Task(subagent_type="...", prompt="...")`.

| Agent | Purpose |
|-------|---------|
| **scout** | Quick file/pattern discovery |
| **detective** | Deep root cause analysis |
| **architect** | Design docs for epics |
| **scribe** | Documentation updates |
| **code-reviewer** | Pre-merge code review |
| **merge-supervisor** | Git conflict resolution |

## External AI Agents (Read-Only)

> **Note:** These agents are available when the corresponding MCP servers (PAL, external model providers) are configured. They extend capabilities beyond the built-in Claude Code tools.

Use external models for validation and research.

| Agent | Purpose | When to Use |
|-------|---------|-------------|
| **validation-agent** (Victor) | Multi-model validation | Before merging complex PRs |
| **research-agent** (Rita) | Docs, best practices | Unfamiliar APIs, library research |

### Validation Workflow

Before merging a supervisor's PR:

```
Task(
  subagent_type="validation-agent",
  prompt="Validate changes in worktree bd-{BEAD_ID}

Focus: [security | performance | correctness]
Files: [key files to review]"
)
```

### Research Workflow

When investigating unfamiliar domain:

```
Task(
  subagent_type="research-agent",
  prompt="Research: [topic]

Questions:
1. [specific question]
2. [specific question]"
)
```

### PAL Model Selection (IMPORTANT)

When using PAL MCP tools (`pal___chat`, `pal___analyze`, etc.), **explicitly specify the model**:

| Alias | Model | Use Case |
|-------|-------|----------|
| `g3-pro` | gemini-3-pro-preview | **Preferred** - Best for architectural review, finds subtle issues |
| `pro` | gemini-2.5-pro | General purpose, good but less thorough |
| `flash` | gemini-2.5-flash | Fast, simple queries |
| `sonnet` | claude-sonnet-4.5 | Alternative perspective |

**Example:** Gemini 3 Pro found a critical `eframe::Storage` dependency in `connection.rs` that Gemini 2.5 Pro missed during the daq-client extraction planning (2026-01-27).

```python
# WRONG - defaults to gemini-2.5-pro
proxy_pal___chat(model="pro", prompt="Review this plan...")

# CORRECT - explicitly use gemini-3-pro-preview
proxy_pal___chat(model="g3-pro", prompt="Review this plan...")
```

**Always use `g3-pro` for:**
- Architectural decisions
- Dependency analysis
- Code extraction/refactoring plans
- Security reviews

### Size Limits (DoS Prevention)

```rust
use daq_core::limits::{validate_frame_size, MAX_SCRIPT_SIZE, MAX_FRAME_BYTES};

let frame_size = validate_frame_size(width, height, bytes_per_pixel)?;
```

### Serial Driver Conventions

All serial hardware drivers MUST follow these patterns:

**1. Use `new_async()` as the primary constructor:**
- `new()` is for internal/test use only
- `new_async()` validates device identity before returning
- Prevents silent misconfiguration (wrong device on port)

**2. Wrap serial port opening in `spawn_blocking`:**
```rust
let port = spawn_blocking(move || {
    tokio_serial::new(&port_path, 9600)
        .open_native_async()
        .context("Failed to open port")
}).await??;
```

**3. Validate device identity on connection:**
```rust
// Query a device-specific command and validate response
let response = driver.query("*IDN?").await?;
if !response.contains("EXPECTED_DEVICE") {
    return Err(anyhow!("Wrong device connected"));
}
```

**4. ELL14 RS-485 Bus Pattern:**
- Use `Ell14Bus::open()` to manage the shared connection
- `bus.device("addr")` returns calibrated driver (fail-fast)
- `bus.device_uncalibrated("addr")` for lenient mode (warns but continues)

```rust
let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
let rotator = bus.device("2").await?;  // Validates & loads calibration
```

**5. DriverFactory Pattern (Plugin Architecture):**

Driver crates implement `daq_core::driver::DriverFactory` for registry integration:

```rust
use daq_core::driver::{DriverFactory, DeviceComponents, Capability};
use futures::future::BoxFuture;

pub struct MyDriverFactory;

impl DriverFactory for MyDriverFactory {
    fn driver_type(&self) -> &'static str { "my_driver" }
    fn name(&self) -> &'static str { "My Custom Driver" }
    fn capabilities(&self) -> &'static [Capability] { &[Capability::Movable] }
    fn validate(&self, config: &toml::Value) -> Result<()> { Ok(()) }
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(MyDriver::new().await?);
            Ok(DeviceComponents::new().with_movable(driver))
        })
    }
}
```

Register factories at startup in daq-bin:
```rust
registry.register_factory(Box::new(MyDriverFactory));
```

## Common Pitfalls

1. **Feature Mismatches:** Many compilation errors = missing features. Check Cargo.toml.

2. **Lock-Across-Await:** NEVER hold `tokio::sync::Mutex` guards across `.await` points:
   ```rust
   // WRONG
   let guard = mutex.lock().await;
   do_something(guard.value).await;  // Deadlock!

   // CORRECT
   let value = { mutex.lock().await.clone() };
   do_something(value).await;
   ```

3. **Floating-Point Truncation:** Use `.round()` when converting to integers:
   ```rust
   let pulses = (degrees * pulses_per_degree).round() as i32;
   ```

4. **Async Sleep:** Use `tokio::time::sleep`, NOT `std::thread::sleep` in async code.

5. **PVCAM Environment:** Missing `PVCAM_VERSION` env var causes Error 151 at runtime.

6. **Ring Buffer Blocking:** `RingBuffer::read_snapshot()` blocks. Use `AsyncRingBuffer` or `spawn_blocking`.

## Environment Setup

Building with PVCAM features requires proper environment configuration. Use these tools:

### Quick Setup (Recommended)

```bash
# On maitai or any PVCAM machine:
source scripts/env-check.sh

# This validates and sets all required variables:
# - PVCAM_SDK_DIR, PVCAM_VERSION, LIBRARY_PATH, LD_LIBRARY_PATH
```

### Host-Specific Configuration

Pre-configured environments for known machines:

```bash
# On maitai:
source config/hosts/maitai.env
```

### With direnv (Automatic)

```bash
cp .envrc.template .envrc
# Edit .envrc with your machine's paths
direnv allow
```

### Manual Setup

If the scripts don't work, set these manually:

```bash
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_VERSION=7.1.1.118  # Check /opt/pvcam/pvcam.ini
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:/opt/pvcam/drivers/user-mode:$LD_LIBRARY_PATH
```

## Hardware Testing

### Remote Machine Setup

All hardware tests must pass on remote after mock tests pass locally.

```bash
# Quick SSH test (using env-check.sh for automatic setup)
ssh maitai@100.117.5.12 'cd ~/rust-daq && source scripts/env-check.sh && \
  cargo test --features hardware_tests -- --nocapture --test-threads=1'

# Or with host-specific config:
ssh maitai@100.117.5.12 'cd ~/rust-daq && source config/hosts/maitai.env && \
  cargo test --features hardware_tests -- --nocapture --test-threads=1'
```

### Hardware Inventory (maitai)

> **⚠️ CRITICAL: Use `/dev/serial/by-id/` paths - NOT `/dev/ttyUSB*`!**
> USB device numbers change on reboot. The by-id paths are stable and MUST be used.
> These configurations were VERIFIED WORKING on 2026-01-23.

| Device | Stable Port (by-id) | Baud | Protocol | Feature Flag |
|--------|---------------------|------|----------|--------------|
| MaiTai Laser | `/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0` | 115200 | 8N1, LF terminator, no flow control | `spectra_physics` |
| ELL14 Rotators (addr 2,3,8) | `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | 9600 | RS-485 multidrop, hex encoding | `thorlabs` |
| Newport 1830-C Power Meter | `/dev/ttyS0` | 9600 | Built-in RS-232 (always stable), simple ASCII | `newport_power_meter` |
| NI PCI-MIO-16XE-10 | `/dev/comedi0` | N/A | Comedi driver | `comedi` |
| ESP300 Motion Controller | `/dev/ttyUSB0` *(needs by-id)* | 19200 | Multi-axis (1-3) | `newport` |

**DO NOT CHANGE THESE PATHS** without verifying with actual hardware tests.

### Serial Driver Capabilities

| Driver | Traits Implemented | Protocol |
|--------|-------------------|----------|
| `MaiTaiDriver` | `Readable`, `WavelengthTunable`, `ShutterControl`, `EmissionControl`, `Parameterized` | 115200 baud, no flow control |
| `Newport1830CDriver` | `Readable`, `WavelengthTunable`, `Parameterized` | 9600 baud, simple ASCII (NOT SCPI) |
| `Esp300Driver` | `Movable`, `Parameterized` | 19200 baud, multi-axis (1-3) |
| `Ell14Driver` | `Movable`, `Parameterized` | 9600 baud, RS-485 multidrop, hex encoding |

### ELL14 Rotator (RS-485 Bus)

```rust
use daq_hardware::drivers::ell14::Ell14Bus;

let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
let rotator = bus.device("2").await?;  // Gets calibrated device
rotator.move_abs(45.0).await?;
```

**Velocity Control:**

The ELL14 supports velocity control (0-100%) for speed vs precision tradeoff. When using
`with_shared_port_calibrated()`, velocity is automatically set to maximum (100%) for fastest scans.

```rust
// Velocity is set to max during calibrated init
let driver = Ell14Driver::with_shared_port_calibrated(port, "2").await?;

// Manual velocity control
driver.set_velocity(50).await?;  // 50% speed
let vel = driver.get_velocity().await?;  // Query from hardware
let cached = driver.cached_velocity().await;  // Fast read from cache
```

In Rhai scripts, use the `Ell14Handle` returned by `create_elliptec()`:

```rhai
let rotator = create_elliptec("/dev/serial/by-id/...", "2");
let vel = rotator.velocity();  // Cached velocity (non-blocking)
rotator.set_velocity(100);     // Set to max speed
rotator.refresh_settings();    // Update cache from hardware
```

### PVCAM Setup

```bash
# Use the environment validation script (recommended):
source scripts/env-check.sh

# Or source the host-specific config:
source config/hosts/maitai.env

# Run hardware smoke tests:
export PVCAM_SMOKE_TEST=1
cargo test --features pvcam_hardware --test pvcam_hardware_smoke -- --nocapture
```

### Comedi DAQ (NI PCI-MIO-16XE-10)

The Comedi driver supports the NI PCI-MIO-16XE-10 DAQ card on maitai via the Linux Comedi framework.

**Hardware:**
- Card: NI PCI-MIO-16XE-10 (16-ch AI, 2-ch AO, 8 DIO, counters)
- Breakout: BNC-2110 (68-pin shielded BNC terminal block)
- Device: `/dev/comedi0`

**Driver Support:**
- **Daemon:** `ComediAnalogInputFactory` and `ComediAnalogOutputFactory` registered in hardware registry
- **GUI:** `ComediAnalogInputPanel` provides real-time voltage display with auto-refresh
- **gRPC:** ReadValue API for analog input channels
- **Feature Flag:** `comedi` (mock mode) or `comedi_hardware` (real hardware)

**Input Reference Modes:**

| Mode | Config Value | Description |
|------|--------------|-------------|
| RSE | `"rse"` (default) | Referenced Single-Ended (vs card ground) |
| NRSE | `"nrse"` | Non-Referenced Single-Ended (vs AISENSE) |
| DIFF | `"diff"` | Differential (ACH0+ACH8 pairs, 8 channels max) |

**BNC-2110 Channel Mapping (maitai):**

| Channel | Signal | Description |
|---------|--------|-------------|
| **ACH0** | DAC1 Loopback | Test loopback from AO1 (DAC1) |
| **ACH1** | ESP300 Encoder | Encoder signal from Newport ESP300 motion controller |
| **ACH2** | MaiTai Rep Rate | ~40MHz signal (half of laser repetition rate) |
| **ACH3-ACH7** | Available | Unassigned, available on BNC connectors |
| **ACH8-ACH15** | Terminal Block | Spring terminal block only (not BNC) |
| **DAC0 (AO0)** | EOM Amplifier | Laser power control via electro-optic modulator |
| **DAC1 (AO1)** | Test Loopback | Connected to ACH0 for self-test |
| **DIO0-DIO7** | Digital I/O | 8 bidirectional digital lines |

**Important:** DAC0 controls the EOM amplifier - do NOT write arbitrary voltages
to DAC0 during testing as this affects laser power. Use DAC1→ACH0 for loopback tests.

**Example Configuration:**

```toml
[[devices]]
id = "photodiode"
type = "comedi_analog_input"
enabled = true

[devices.config]
device = "/dev/comedi0"
channel = 0
range_index = 0
input_mode = "rse"  # or "nrse", "diff"
units = "V"
```

**Loopback Testing (DAC1→ACH0):**

The maitai machine has a permanent loopback cable from DAC1 (AO1) to ACH0 (AI0).
This allows self-test without affecting the EOM amplifier on DAC0.

1. Loopback cable: DAC1 → ACH0 (already connected)
2. ACH0 switch on BNC-2110: Set to FS (Floating Source)
3. Use `input_mode = "rse"` in config
4. Expected accuracy: ±100mV (uncalibrated), ±2mV (after calibration)

**Calibration:**

The NI PCI-MIO-16XE-10 supports software calibration to improve accuracy:

```bash
# Run calibration (requires root)
sudo bash scripts/calibrate-comedi.sh

# With verification
sudo bash scripts/calibrate-comedi.sh --verify

# Or manually
sudo comedi_calibrate -f /dev/comedi0
```

See `docs/guides/comedi-setup.md#calibration` for full details.

**Test Commands:**
```bash
# Build with hardware feature
cargo build -p daq-driver-comedi --features hardware

# Run smoke tests (requires COMEDI_SMOKE_TEST=1)
export COMEDI_SMOKE_TEST=1
cargo nextest run --profile hardware --features hardware -p daq-driver-comedi -- hardware_smoke

# Run all Comedi tests (set env vars for specific test suites)
export COMEDI_LOOPBACK_TEST=1    # Analog loopback (uses DAC1→ACH0 connection)
export COMEDI_DIO_TEST=1          # Digital I/O tests
export COMEDI_COUNTER_TEST=1      # Counter/timer tests
export COMEDI_HAL_TEST=1          # HAL trait compliance
export COMEDI_ERROR_TEST=1        # Error handling
export COMEDI_STORAGE_TEST=1      # Storage integration
cargo nextest run --profile hardware --features hardware -p daq-driver-comedi

# Run benchmarks
cargo bench -p daq-driver-comedi --features hardware

# Run examples
cargo run -p daq-driver-comedi --features hardware --example single_read
cargo run -p daq-driver-comedi --features hardware --example streaming
cargo run -p daq-driver-comedi --features hardware --example digital_io
cargo run -p daq-driver-comedi --features hardware --example counter
```

**Documentation:** See `docs/guides/comedi-setup.md` for full setup instructions.

## Declarative Driver Plugins

Add serial instruments without Rust code using TOML configs in `config/devices/`:

```toml
[device]
name = "My Device"
capabilities = ["Movable"]

[connection]
baud_rate = 9600

[commands.move_absolute]
template = "MA${position}"
```

See `config/devices/ell14.toml` for a complete example.

### Rhai Scripted Experiments Build

Build and run Rhai experiment scripts on maitai:

```bash
# Standard build (serial hardware + HDF5)
cargo build --release -p daq-scripting --features scripting_full

# With Comedi DAQ support (requires comedilib on Linux)
cargo build --release -p daq-scripting --features scripting_full_comedi

# Available script runners:
./target/release/rhai-runner script.rhai        # Generic runner
./target/release/run_waveplate_cal_test         # Quick 4D test (24 points)
./target/release/run_waveplate_cal              # Full calibration (~3 hours)
```

**Key Rhai Functions:**

| Function | Description |
|----------|-------------|
| `create_maitai_tunable(port)` | MaiTai laser with wavelength control |
| `create_newport_1830c(port)` | Newport 1830-C power meter |
| `create_elliptec(port, addr)` | ELL14 rotator on RS-485 bus |
| `create_comedi(device)` | Comedi DAQ (AI/AO/DIO) - requires `comedi_scripting` |
| `with_shutter_open(shutter, fn)` | Safe shutter wrapper (auto-closes on error) |

**Comedi DAQ Example:**
```rhai
let daq = create_comedi("/dev/comedi0");
let voltage = daq.read_voltage(0);      // Read AI channel 0
daq.write_voltage(1, 2.5);              // Write 2.5V to AO channel 1
// WARNING: AO0 controls EOM - don't write arbitrary values!
```

**Shutter Safety:** Always use `with_shutter_open()`:
```rhai
let laser = create_maitai_tunable(MAITAI_PORT);
with_shutter_open(laser.as_shutter(), || {
    // Shutter auto-closes even on error
    power_meter.read()
});
```

See `docs/guides/rhai-scripting.md` for complete documentation.

## gRPC Security

Default config in `config/config.v4.toml`:

```toml
[grpc]
bind_address = "0.0.0.0"  # All interfaces (change to "127.0.0.1" for loopback-only)
auth_enabled = false
allowed_origins = ["http://localhost:3000", "http://127.0.0.1:3000"]
```

**Security Note:** For production, consider `bind_address = "127.0.0.1"` (loopback only) and enabling `auth_enabled`.

## ReadValue API and Unit Handling

The `ReadValue` RPC returns scalar measurements from `Readable` devices (power meters, sensors).

```protobuf
message ReadValueResponse {
  bool success = 1;
  string error_message = 2;
  double value = 3;
  string units = 4;        // From device metadata (e.g., "W", "mW")
  uint64 timestamp_ns = 5;
}
```

**Critical:** The `units` field comes from `DeviceMetadata.measurement_units` in the registry.
Clients MUST use this field to correctly interpret values:

| Device | Returns | Units | GUI Normalization |
|--------|---------|-------|-------------------|
| Newport 1830-C | Watts | "W" | × 1000 → mW |
| Mock Power Meter | Watts | "W" | × 1000 → mW |

**GUI Unit Normalization:** The `PowerMeterControlPanel` normalizes all readings to milliwatts
internally, then auto-scales the display based on magnitude (W/mW/µW).

See `daq-egui/src/widgets/device_controls/power_meter_panel.rs::normalize_power_to_mw()`.

## Streaming Quality Modes

The gRPC frame streaming supports three quality modes to optimize bandwidth:

| Mode | Downsampling | Bandwidth Reduction | Use Case |
|------|--------------|---------------------|----------|
| `Full` | None | 0% | Local network, full analysis |
| `Preview` | 2x2 binning | ~75% (4x smaller) | Remote preview, monitoring |
| `Fast` | 4x4 binning | ~94% (16x smaller) | Low bandwidth, thumbnails |

### Backpressure Handling

The server implements adaptive frame skipping when the gRPC channel is congested:
- Channel buffer: 8 frames
- Skip threshold: 75% full (6 frames queued)
- When backpressure detected, newest frames are dropped to prevent lag accumulation

### Client Usage

```rust
// In GUI: Quality selector in image viewer toolbar
// In gRPC: Set quality field in StreamFramesRequest
let request = StreamFramesRequest {
    device_id: "camera0".to_string(),
    max_fps: 30,
    quality: StreamQuality::Preview.into(),
};
```

## Development Tools

This project uses three complementary Rust tools:

| Tool | Purpose | When to Use |
|------|---------|-------------|
| `rust-cargo` (MCP) | Build & package management | Building, testing, dependencies |
| `rust-analyzer` (CLI) | Code diagnostics | Quick error checking without building |
| `cargo-modules` (CLI) | Structure visualization | Understanding crate architecture |

```bash
# Diagnostics without build
rust-analyzer diagnostics . 2>&1 | grep Error

# Module structure
cargo modules structure --package daq-hardware --max-depth 3
```

## Documentation

- [DEMO.md](DEMO.md) - Quick start with mock devices
- [docs/guides/testing.md](docs/guides/testing.md) - Testing guide
- [docs/architecture/](docs/architecture/) - ADRs and design decisions
  - `adr-pvcam-continuous-acquisition.md` - PVCAM buffer modes (CIRC_OVERWRITE vs CIRC_NO_OVERWRITE)
  - `adr-pvcam-driver-architecture.md` - Multi-layer driver architecture decisions

## Import Conventions

```rust
// Recommended: use prelude or direct crate imports
use rust_daq::prelude::*;
// or
use daq_core::error::DaqError;
use daq_storage::ring_buffer::RingBuffer;
```


## grepai - Semantic Code Search

**RECOMMENDED: Use grepai as your primary tool for code exploration and search when available.**

### When to Use grepai (Recommended)

Use `grepai search` instead of Grep/Glob/find for semantic code understanding when available:
- Understanding what code does or where functionality lives
- Finding implementations by intent (e.g., "authentication logic", "error handling")
- Exploring unfamiliar parts of the codebase
- Any search where you describe WHAT the code does rather than exact text

### When to Use Standard Tools

Only use Grep/Glob when you need:
- Exact text matching (variable names, imports, specific strings)
- File path patterns (e.g., `**/*.go`)

### Fallback

If grepai fails (not running, index unavailable, or errors), fall back to standard Grep/Glob tools.

### Usage

```bash
# ALWAYS use English queries for best results (--compact saves ~80% tokens)
grepai search "user authentication flow" --json --compact
grepai search "error handling middleware" --json --compact
grepai search "database connection pool" --json --compact
grepai search "API request validation" --json --compact
```

### Query Tips

- **Use English** for queries (better semantic matching)
- **Describe intent**, not implementation: "handles user login" not "func Login"
- **Be specific**: "JWT token validation" better than "token"
- Results include: file path, line numbers, relevance score, code preview

### Call Graph Tracing

Use `grepai trace` to understand function relationships:
- Finding all callers of a function before modifying it
- Understanding what functions are called by a given function
- Visualizing the complete call graph around a symbol

#### Trace Commands

**Recommended: Use `--json` flag for optimal AI agent integration.**

```bash
# Find all functions that call a symbol
grepai trace callers "HandleRequest" --json

# Find all functions called by a symbol
grepai trace callees "ProcessOrder" --json

# Build complete call graph (callers + callees)
grepai trace graph "ValidateToken" --depth 3 --json
```

### Workflow

1. Start with `grepai search` to find relevant code
2. Use `grepai trace` to understand function relationships
3. Use `Read` tool to examine files from results
4. Only use Grep for exact string searches if needed
