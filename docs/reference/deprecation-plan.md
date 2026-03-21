# Deprecation & Legacy Code Removal Plan

Inventory of deprecated APIs, legacy compatibility shims, and their removal conditions.
Last audited: 2026-03-21 (Phase 5 repo cleanup).

---

## 1. Deprecated Protobuf Fields

### 1.1 DeviceInfo boolean capability flags (`hardware.proto`)

| Field | Field Number | Deprecated Since |
|-------|-------------|------------------|
| `is_movable` | 10 | v0.6.0 |
| `is_readable` | 11 | v0.6.0 |
| `is_triggerable` | 12 | v0.6.0 |
| `is_frame_producer` | 13 | v0.6.0 |
| `is_exposure_controllable` | 14 | v0.6.0 |
| `is_shutter_controllable` | 15 | v0.6.0 |
| `is_wavelength_tunable` | 16 | v0.6.0 |
| `is_emission_controllable` | 17 | v0.6.0 |
| `is_parameterized` | 18 | v0.6.0 |

**Location:** `crates/protocol/proto/hardware.proto:149-158`

**Why it exists:** Original API used individual booleans to describe device capabilities.
The `repeated string capabilities = 100` field (bd-4myc.2) replaced them with an
extensible list. The booleans remain because:

1. The server still populates them for wire-compat (`server/src/grpc/hardware_service/helpers.rs:312-322`).
2. The UI `PersistedPanelInfo` struct reads them during deserialization of old persisted
   state (`ui/src/app/types.rs:698-748`).

**Replacement:** `DeviceInfo.capabilities` repeated string field (field 100).

**Consumers still reading booleans:**
- `ui/src/app/types.rs` -- deserialization migration path for old saved panel state.

**Consumers already migrated to `capabilities` strings:**
- `ui/src/device_ext.rs` -- `DeviceInfoExt` trait reads `capabilities` field.
- All server-side code uses the `Capability` enum in Rust, booleans set only for proto.

**Removal condition:** Remove when no deployed UI client older than the migration
release is expected to reconnect. Practically, safe to remove after v1.0 since all
labs are updated together.

**Removal steps:**
1. Delete the 9 boolean fields from `hardware.proto` (keep field numbers reserved).
2. Remove population in `server/src/grpc/hardware_service/helpers.rs`.
3. Remove legacy deserialization fields from `ui/src/app/types.rs` `PersistedPanelInfo`.
4. Remove the `migrate_from_legacy_booleans` logic in `PersistedPanelInfo::from`.

---

### 1.2 ScanService (entire service) (`experiment.proto`)

**Location:** `crates/protocol/proto/experiment.proto:8-47`

**Why it exists:** Original coordinated scan API. Deprecated in v0.6.0 when
`RunEngineService` introduced the declarative Plan-based experiment system.
Kept through v0.7.0 for the two-release deprecation period.

**Replacement:** `RunEngineService` (see migration table in `experiment.proto:14-19`).

| ScanService Method | RunEngineService Equivalent |
|--------------------|---------------------------|
| `CreateScan + StartScan` | `QueuePlan + StartEngine` |
| `PauseScan` | `PauseEngine` |
| `ResumeScan` | `ResumeEngine` |
| `StopScan` | `AbortPlan` |
| `GetScanStatus` | `GetEngineStatus` |
| `ListScans` | `GetEngineStatus` (check `queued_plans`) |
| `StreamScanProgress` | `StreamDocuments` |

**Consumers:**
- `server/src/grpc/scan_service.rs` -- full implementation (~1000 lines).
- `server/src/grpc/mod.rs:86-87` -- re-export with `#[allow(deprecated)]`.
- `server/src/grpc/server.rs:1688-2308` -- wired into gRPC server.
- `server/src/grpc/audit_log.rs:76-80` -- audit log route list.
- `client/` -- `ScanServiceClient` re-exported.

**Removal condition:** No known external consumers use `ScanService` directly. All
lab scripts use Rhai -> RunEngine. Safe to remove at v1.0.

**Removal steps:**
1. Remove `ScanService` definition and all messages from `experiment.proto`.
2. Delete `server/src/grpc/scan_service.rs`.
3. Remove wiring from `server/src/grpc/server.rs` (both `start_server` paths).
4. Remove re-exports from `server/src/grpc/mod.rs`.
5. Remove audit log entries from `server/src/grpc/audit_log.rs`.
6. Remove `ScanServiceClient` from `client/`.

---

## 2. Deprecated Rust APIs

### 2.1 `FrameProducer::take_frame_receiver()` (common)

**Location:** `crates/common/src/capabilities.rs:485-492`

**Deprecated since:** v0.2.0

**Why it exists:** Original frame delivery used `mpsc::Receiver<Frame>` with heap
allocation per frame. Replaced by `register_primary_output()` which delivers pooled
`LoanedFrame` objects for zero-allocation streaming.

**Replacement:** `FrameProducer::register_primary_output()`

**Removal condition:** After v1.0. The default implementation already returns `None`.
Must verify no driver overrides it.

---

### 2.2 `FrameProducer::subscribe_frames()` (common)

**Location:** `crates/common/src/capabilities.rs:543-552`

**Deprecated since:** v0.3.0

**Why it exists:** Used `broadcast::Receiver<Arc<Frame>>` for multi-subscriber frame
access. Replaced by `register_primary_output()` for primary consumers and
`register_observer()` for secondary (non-blocking tap) access.

**Replacement:** `register_primary_output()` or `register_observer()`.

**Still used by:**
- `driver-mock/src/mock_camera.rs:848` -- mock camera still sends on legacy broadcast
  and reliable channels alongside the pooled path. This is intentional for test
  coverage of the migration period.

**Removal condition:** After v1.0. Remove legacy broadcast paths from mock camera.

---

### 2.3 `ScanServiceImpl` (server)

**Location:** `crates/server/src/grpc/scan_service.rs:165-175`

**Deprecated since:** v0.7.0

**Why it exists:** See Section 1.2 above (proto-level deprecation).

**Replacement:** `RunEngineServiceImpl`

**Removal condition:** Same as proto `ScanService` -- v1.0.

---

### 2.4 `TiffWriter::write_frame()` (storage)

**Location:** `crates/storage/src/tiff_writer.rs:90-93`

**Deprecated since:** v0.3.0

**Why it exists:** Original TIFF writer took `&Frame` (owned data). Replaced by
`write_frame_data()` which accepts raw byte slices for zero-copy compatibility
with pooled frames.

**Replacement:** `TiffWriter::write_frame_data()`

**Removal condition:** After v1.0. Verify no callers remain.

---

### 2.5 `hardware::config::schema::DeviceConfig` (hardware)

**Location:** `crates/hardware/src/config/schema.rs:43-45`

**Deprecated since:** pre-v0.2.0

**Why it exists:** Original device configuration schema for the hardware crate's
built-in config parser. Superseded by `driver_universal::config` which supports
schema v3 TOML manifests with richer semantics.

**Replacement:** `driver_universal::config`

**Still used by:** `scripting` and `ui` crates for TOML-based control panel
definitions (`ControlSection`, `ParameterConfig`, `CommandConfig`).

**Removal condition:** After the UI control panel system is migrated to read
from `driver_universal::config` directly. This is a larger refactor -- target v1.0+.

---

### 2.6 `GenericDriver::new()` (hardware/manifest_driver)

**Location:** `crates/hardware/src/manifest_driver/driver.rs:175`

**Deprecated since:** v0.2.0

**Why it exists:** Original constructor that only supported serial connections.
Replaced by `new_serial()` (same behavior, clearer name) as TCP support was added.

**Replacement:** `GenericDriver::new_serial()`

**Removal condition:** After v1.0. Trivial removal.

---

### 2.7 `PvcamDriver::new()` (driver-pvcam)

**Location:** `crates/driver-pvcam/src/lib.rs:285`

**Deprecated since:** pre-v0.2.0

**Why it exists:** Synchronous constructor that calls `block_on()`. Replaced by
`new_async()` to avoid blocking the Tokio runtime.

**Replacement:** `PvcamDriver::new_async()`

**Removal condition:** After v1.0. Verify no synchronous call sites remain.

---

### 2.8 `PvcamFeatures::io_script_control()` (driver-pvcam)

**Location:** `crates/driver-pvcam/src/components/features/mod.rs:2906`

**Deprecated since:** v0.1.0

**Why it exists:** Legacy PVCAM 2.x scripting API. PVCAM 3.x uses the parameter-based
I/O control API (`io_control()`, `set_io_address()`, `set_io_direction()`, `set_io_state()`).

**Replacement:** `io_control()` and related parameter-based methods.

**Removal condition:** After v1.0. All supported cameras run PVCAM 3.x+.

---

## 3. Legacy Compatibility Code (Must Keep)

These items provide backward compatibility that is still actively needed. They are
marked with `// LEGACY:` comments in the source and must NOT be removed until their
stated conditions are met.

### 3.1 Legacy strfmt template syntax (`{var}` -> `{{ var }}`)

**Location:** `crates/hardware/src/manifest_driver/templating.rs`

**What:** `convert_legacy_template()` converts old `{var}` placeholder syntax
(from the strfmt library) to minijinja `{{ var }}` syntax.

**Why it must stay:** Existing TOML device manifests in `config/devices/` and user
configurations may use the `{var}` syntax. Breaking these would brick device
communication.

**Removal condition:** After a full audit confirms all manifests use `{{ var }}`
syntax, and a deprecation warning has been shown for at least one release.

---

### 3.2 Legacy v1 config fields in driver-universal

**Location:** `crates/driver-universal/src/config/raw.rs`

**Fields:**
- `scripts` (line 65) -- v1 Rhai script definitions.
- `trait_mapping` (line 77) -- v1 trait mappings (accepted but ignored in v3).
- `query` field on `RawCommandConfig` (line 217) -- v2 query flag.
- `pattern` field on `RawResponseConfig` (line 241) -- v1 regex alias.
- `fields` (line 251) -- v1 regex capture type declarations.

**Why they must stay:** Schema v3 parsing accepts these fields to avoid breaking
existing v1/v2 manifests. The `deny_unknown_fields` attribute is intentionally
absent to allow forward-compatible parsing.

**Removal condition:** After all manifests in `config/devices/` are verified to use
schema v3 syntax only. Run audit: check for `pattern =`, `scripts =`, `trait_mapping =`,
`query =` keys in TOML manifests.

---

### 3.3 Settable trait fallback in command dispatch

**Location:** `crates/experiment/src/run_engine/command_dispatch.rs:79-90`

**What:** `execute_set_parameter()` tries the legacy `Settable` trait before the new
`Parameterized` trait + `Parameter<T>` system.

**Why it must stay:** Some drivers (particularly `driver-universal` devices) may still
implement `Settable` without full `Parameterized` support. The fallback ensures these
devices remain controllable.

**Removal condition:** After all drivers implement `Parameterized`. Audit: check which
`DeviceComponents` set `settable` but not `parameterized`.

---

### 3.4 `RUSTDAQ_RUNTIME_MODE` environment variable

**Location:** `crates/bin/src/main.rs:690-697`

**What:** Falls back to `RUSTDAQ_RUNTIME_MODE` if `DAQ_RUNTIME_MODE` is not set.

**Why it must stay:** Deployed systemd service files and CI scripts may reference the
old variable name. The cost of keeping the fallback is trivial (one env var check).

**Removal condition:** After all service files, CI configs, and deployment scripts have
been updated to use `DAQ_RUNTIME_MODE`. Low priority.

---

### 3.5 `plugin` module alias in hardware crate

**Location:** `crates/hardware/src/lib.rs:57-59`

**What:** `pub use manifest_driver as plugin;` provides a backward-compatible import
path for code that referenced `hardware::plugin`.

**Why it must stay:** External scripts or downstream consumers may use the old path.
Cost of keeping is zero (one `pub use` alias).

**Removal condition:** After v1.0. Search for `hardware::plugin` references.

---

### 3.6 UI legacy daemon address migration

**Location:** `crates/ui/src/connection.rs:10-20`, `crates/ui/src/app/mod.rs:314-325`

**What:** On first load, migrates the orphaned `daemon_address` eframe storage key
to the new `AppSettings` structure.

**Why it must stay:** Users with existing browser localStorage from pre-`AppSettings`
releases would lose their saved daemon address.

**Removal condition:** After v1.0. All labs will have loaded the UI at least once by
then, completing the migration.

---

### 3.7 Mock driver backward-compatible constructors

**Locations:**
- `crates/driver-mock/src/mock_stage.rs:390-393` -- `MockStage::new()`
- `crates/driver-mock/src/mock_camera.rs:506-512` -- `MockCamera::new()`
- `crates/driver-mock/src/mock_power_meter.rs:319-358` -- `MockPowerMeter` defaults

**What:** Simple constructors (`new()`) that delegate to the builder/config APIs.

**Why they must stay:** Used extensively in tests across all crates. These are convenience
APIs, not truly deprecated -- they wrap the builder pattern for simple use cases.

**Removal condition:** Do NOT remove. These are stable convenience APIs for testing.

---

### 3.8 Mock camera legacy broadcast/reliable channels

**Location:** `crates/driver-mock/src/mock_camera.rs:848`

**What:** Mock camera sends frames on both the new pooled path AND the legacy
`broadcast` + `reliable` channels.

**Why it must stay:** Tests for the deprecated `subscribe_frames()` and
`take_frame_receiver()` APIs still exercise these paths. Removing would break
test coverage of the migration period.

**Removal condition:** Remove when `subscribe_frames()` and `take_frame_receiver()`
are removed (after v1.0).

---

### 3.9 ImperativePlan wrapper

**Location:** `crates/experiment/src/plans_imperative.rs`

**What:** Wraps legacy direct hardware commands as Plan objects so they emit Documents
through the RunEngine.

**Why it must stay:** Bridges Rhai scripts that use `stage.move_abs()` style calls
with the RunEngine document system. Removing would break all existing user scripts.

**Removal condition:** Never -- this is the intentional integration layer between
imperative scripting and the declarative RunEngine.

---

### 3.10 DeviceRegistry `components_to_registered` bridge

**Location:** `crates/hardware/src/registry.rs:855-870`

**What:** Converts `DeviceComponents` from the new `DriverFactory` pattern into the
internal `RegisteredDevice` structure.

**Why it must stay:** Core plumbing. The comment says "legacy" but this is the active
bridge between factory output and registry storage. Not actually removable.

**Removal condition:** Never -- this is active infrastructure, not legacy code.

---

## 4. Already Removed (for reference)

### 4.1 Legacy SCPI/TCP serial drivers

**Removed in:** Phase 4 (v0.7.x)

**What was removed:** `driver-thorlabs`, `driver-newport`, `driver-spectra-physics`,
`driver-red-pitaya` crates.

**Replaced by:** `driver-universal` TOML manifests in `config/devices/`.

**Evidence:**
- `crates/driver-registry/src/lib.rs:39-40` -- removal note.
- `crates/scripting/src/bindings.rs:720-722` -- removal note.
- `crates/bin/src/daemon_manager.rs:141-149` -- hard error in gated modes.
- `docs/how-to/legacy-scpi-deprecation.md` -- full migration guide.

---

## 5. Removal Priority Summary

| Priority | Item | Target Version | Effort |
|----------|------|---------------|--------|
| **High** | ScanService (proto + impl) | v1.0 | Medium (delete ~1200 lines) |
| **High** | Proto boolean capability flags | v1.0 | Low (delete fields + population code) |
| **Medium** | `take_frame_receiver()` | v1.0 | Low (trait method + mock paths) |
| **Medium** | `subscribe_frames()` | v1.0 | Low (trait method + mock paths) |
| **Medium** | `TiffWriter::write_frame()` | v1.0 | Low (single method) |
| **Low** | `DeviceConfig` in hardware schema | v1.0+ | Medium (scripting/UI refactor) |
| **Low** | `GenericDriver::new()` | v1.0 | Trivial |
| **Low** | `PvcamDriver::new()` | v1.0 | Trivial |
| **Low** | `io_script_control()` | v1.0 | Low |
| **Keep** | Template syntax compat | audit first | - |
| **Keep** | v1 config fields | audit first | - |
| **Keep** | Settable fallback | audit first | - |
| **Keep** | `RUSTDAQ_RUNTIME_MODE` | low priority | - |
| **Keep** | `plugin` alias | v1.0 | Trivial |
| **Keep** | UI daemon migration | v1.0 | Trivial |
| **Keep** | ImperativePlan | never | - |
