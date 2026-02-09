# rust-daq

## Environment Setup (PVCAM machines only)

```bash
source scripts/env-check.sh              # Validate & configure environment
source config/hosts/maitai.env           # Or use host-specific config

# Build & Test (local development)
cargo build                              # Default features (mock hardware)
cargo nextest run                        # Run all tests (recommended)
cargo nextest run test_name              # Single test
cargo nextest run -p common            # Specific crate
cargo test --doc                         # Doctests (not in nextest)

# Quality Checks
cargo fmt --all                          # Format
cargo clippy --all-targets               # Lint

# Build Daemon for Maitai (REAL PVCAM HARDWARE + ALL SERIAL DEVICES)
# ⚠️  CRITICAL: Use build-maitai.sh - it includes ALL hardware drivers!
# The 'maitai' feature enables: PVCAM (real SDK), thorlabs, newport, spectra_physics, serial
bash scripts/build-maitai.sh             # Clean build with ALL real hardware

# Or manually (must clean to avoid cached mock build):
cargo clean -p bin -p rust_daq -p driver-pvcam
cargo build --release -p bin --features maitai

# Run Daemon
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml

# Build GUI (separate build - does NOT require hardware features)
cargo build --release -p ui --bin rust-daq-gui

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

**Verification:** Check that daemon log shows `pvcam_sdk feature enabled: true` and does NOT show `using mock mode`.

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
- Each device should have its control panel via `GenericDevicePanel`,
  which auto-composes compact widgets from `DeviceInfo.capabilities`:
  - **Readable** devices (power meters, Comedi AI): gauge + value + auto-refresh
  - **Movable** devices (rotators, stages): position + jog + go-to + home
  - **Emission/Shutter** (lasers): toggle buttons on a single row
  - **WavelengthTunable** (lasers, power meters): slider + text input
  - **Settable** (Comedi AO): voltage slider + quick-set presets
  - **Camera:** ImageViewerPanel with streaming controls (separate panel)
- Camera should stream real images (not synthetic gradients)
- Comedi channels should show real voltage readings

## Beads Commands

### Human-Readable Commands (Interactive Use)

```bash
bd create "Title" -d "Description"                    # Create task
bd create "Title" -d "..." --type epic                # Create epic
bd create "Title" -d "..." --parent {EPIC_ID}         # Create child task
bd create "Title" -d "..." --parent {ID} --deps {ID}  # Child with dependency
bd list                                               # List all beads
bd show ID                                            # Show details
bd ready                                              # Find unblocked tasks
bd update ID --status done                            # Mark child done
bd update ID --status inreview                        # Mark standalone done
bd update ID --design ".designs/{ID}.md"              # Set design doc path
bd close ID                                           # Close task
bd epic status ID                                     # Epic completion status
```

### JSON Output Commands (Automation & Scripting)

Add `--json` flag for machine parsing in scripts:

```bash
bd list --json                                        # Parse all beads as JSON
bd show ID --json                                     # Get single bead as JSON
bd ready --json                                       # Find unblocked tasks (parseable)
bd epic status ID --json                              # Epic status as JSON
```

**When to use `--json`:**
- Piping to `jq` for filtering/transforming (e.g., `bd ready --json | jq '.[] | select(.type=="epic")'`)
- Parsing in shell scripts or tools (reduces ~80% parsing complexity)
- Continuous integration or automation workflows
- Exporting data to external systems

**When to use human-readable:**
- One-off interactive commands
- Checking quick status in terminal
- Reading investigation notes or comments

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
use common::limits::{validate_frame_size, MAX_SCRIPT_SIZE, MAX_FRAME_BYTES};

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

Driver crates implement `common::driver::DriverFactory` for registry integration:

```rust
use common::driver::{DriverFactory, DeviceComponents, Capability};
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

Register factories at startup in bin:
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
cargo build -p driver-comedi --features hardware

# Run smoke tests (requires COMEDI_SMOKE_TEST=1)
export COMEDI_SMOKE_TEST=1
cargo nextest run --profile hardware --features hardware -p driver-comedi -- hardware_smoke

# Run all Comedi tests (set env vars for specific test suites)
export COMEDI_LOOPBACK_TEST=1    # Analog loopback (uses DAC1→ACH0 connection)
export COMEDI_DIO_TEST=1          # Digital I/O tests
export COMEDI_COUNTER_TEST=1      # Counter/timer tests
export COMEDI_HAL_TEST=1          # HAL trait compliance
export COMEDI_ERROR_TEST=1        # Error handling
export COMEDI_STORAGE_TEST=1      # Storage integration
cargo nextest run --profile hardware --features hardware -p driver-comedi

# Run benchmarks
cargo bench -p driver-comedi --features hardware

# Run examples
cargo run -p driver-comedi --features hardware --example single_read
cargo run -p driver-comedi --features hardware --example streaming
cargo run -p driver-comedi --features hardware --example digital_io
cargo run -p driver-comedi --features hardware --example counter
```

**Documentation:** See `docs/guides/comedi-setup.md` for full setup instructions.

## Declarative Driver Plugins (Schema v3)

Add serial/TCP instruments without Rust code using TOML configs in `config/devices/`.
The `driver-universal` crate provides a parse-don't-validate pipeline: TOML → RawManifest → DeviceManifest → DeviceComponents.

```toml
schema_version = 3

[device]
name = "My Device"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600
timeout_ms = 1000

[commands.read]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read" }
```

Features: MiniJinja templates, tiered response parsing (SCPI auto-parse, format strings, transform pipelines, regex), evalexpr formulas, real Serial/TCP transports.

See `config/devices/ell14.toml` for a complete example.

### Rhai Scripted Experiments Build

Build and run Rhai experiment scripts on maitai:

```bash
# Standard build (serial hardware + HDF5)
cargo build --release -p scripting --features scripting_full

# With Comedi DAQ support (requires comedilib on Linux)
cargo build --release -p scripting --features scripting_full_comedi

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

See `ui/src/widgets/device_controls/power_meter_panel.rs::normalize_power_to_mw()`.

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
cargo modules structure --package hardware --max-depth 3
```

## ast-grep Quick Reference (Rust)

**When to use ast-grep vs Grep:**
- **ast-grep**: structural patterns (unwrap calls, unsafe blocks, async patterns, trait impls)
- **Grep**: exact text matches (imports, string literals, variable names)

**Common Rust patterns:**

```bash
sg -p '$EXPR.unwrap()' --lang rust .                    # Find all .unwrap() calls
sg -p 'unsafe { $$$ }' --lang rust .                    # Find unsafe blocks
sg -p '$EXPR.expect($MSG)' --lang rust .                # Find .expect() calls
sg -p 'panic!($$$)' --lang rust .                       # Find panic! macros
sg -p 'todo!($$$)' --lang rust .                        # Find todo! macros
sg -p 'std::thread::sleep($EXPR)' --lang rust .         # Find blocking sleep in async code
sg -p 'impl $TRAIT for $TYPE { $$$ }' --lang rust .     # Find trait implementations
sg -p '#[test] fn $NAME() { $$$ }' --lang rust .        # Find test functions
```

**For complex structural rules (inside/has/not):** Load the full skill with `/ast-grep` first.

## Documentation

- [DEMO.md](DEMO.md) - Quick start with mock devices
- [docs/guides/testing.md](docs/guides/testing.md) - Testing guide
- [docs/architecture/](docs/architecture/) - ADRs and design decisions
  - `adr-pvcam-continuous-acquisition.md` - PVCAM buffer modes (CIRC_OVERWRITE vs CIRC_NO_OVERWRITE)
  - `adr-pvcam-driver-architecture.md` - Multi-layer driver architecture decisions

## Import Conventions

```rust
// Import directly from individual crates
use common::error::DaqError;
use common::capabilities::Movable;
use storage::ring_buffer::RingBuffer;
```


## CocoIndex - Semantic Code Search

Local pgvector-backed semantic index of the entire rust-daq codebase (~12,300 chunks with `nomic-embed-code` embeddings). Kept current by a launchd live updater.

**When to use CocoIndex vs Grep/Glob:**
- **CocoIndex**: you know *what* you want but not the exact keywords, finding similar implementations, exploring by concept
- **Grep/Glob**: exact symbol names, string literals, file path patterns

### Semantic Search (requires embedding server on vasp-03)

```bash
COCOINDEX_DATABASE_URL="postgresql://briansquires@localhost/cocoindex" \
  ~/beefcake2/.venv/bin/python -c "
import sys; sys.path.insert(0, '$HOME/beefcake2')
import index_flow_v2
result = index_flow_v2.semantic_search('YOUR QUERY HERE', top_k=10)
for r in result.results:
    print(f\"[{r['score']:.3f}] {r['filename']}\")
    print(f\"  {r['chunk_content'][:120]}\")
"
```

### Text Search (no embedding server needed)

```bash
psql -d cocoindex -c "SELECT filename, chunk_content FROM code_chunks WHERE chunk_content ILIKE '%DriverFactory%' LIMIT 10;"
```

### Crate-Scoped Search

```bash
psql -d cocoindex -c "SELECT filename, chunk_content FROM code_chunks WHERE filename LIKE 'crates/common/%' AND chunk_content ILIKE '%Parameter%' LIMIT 20;"
```

### Workflow

1. Use CocoIndex semantic search to discover relevant files and chunks
2. Use `Read` tool with actual file paths for full context and line numbers
3. Use Grep only for precise exact-text lookups

**For full reference (embedding server status, index coverage, live updater):** Load `/cocoindex-search`
