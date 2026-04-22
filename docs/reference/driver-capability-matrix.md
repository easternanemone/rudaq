# Driver Capability Matrix

Per-factory capability declarations and deployment target support for all driver crates in the workspace.

> **Generated**: 2026-04-22 (prior: 2026-03-13) | **Source of truth**: `DriverFactory::capabilities()` return values and `driver-registry` feature gates.
>
> **Changes since 2026-03-13:**
>
> - Six capability traits have been added to `crates/common-traits/src/capabilities.rs` since the prior generation: `StateRefreshable` (already represented here — post-reconnection parameter refresh, bd-47p2), plus `CounterConfigurable`, `RangeIntrospectable`, `DeviceIntrospection`, `ReadableWithMetadata`, `SpectrumReadable`. These are not yet mapped into the Coverage Summary table below; capability-trait total is now **30** (`crates/common-traits/src/capabilities.rs`).
> - The `PulseGenerator` trait mentioned in older architecture prose does not exist in the code and has been removed from `docs/explanation/architecture.md`.
> - Driver-registry `mock_only` feature no longer exists; `full` is now simply an alias for `all_hardware` (see `crates/driver-registry/Cargo.toml`). The pvcam feature-gate tiers are unchanged.

## Native SDK Drivers

These require vendor SDKs and are feature-gated in `driver-registry/Cargo.toml`.

| Factory | Crate | `driver_type` | Feature Gate | Capabilities | Deployment Targets |
|---------|-------|---------------|-------------|--------------|-------------------|
| `PvcamFactory` | `driver-pvcam` | `pvcam` | `pvcam` / `pvcam_sdk` / `pvcam_hardware` | FrameProducer, Triggerable, ExposureControl, Commandable, Parameterized | **maitai** (Linux x86_64, PVCAM SDK) |
| `AndorCameraFactory` | `driver-andor-sdk3` | `andor_istar` | `andor` / `andor_hardware` | FrameProducer, Triggerable, ExposureControl, Parameterized | **leabs-dev** (Linux x86_64, Andor SDK3) |
| `AndorSpectrographFactory` | `driver-andor-sdk3` | `andor_shamrock` | `andor` / `andor_hardware` | WavelengthTunable, ShutterControl, Parameterized | **leabs-dev** (Linux x86_64, Andor SDK3) |
| `ComediAnalogInputFactory` | `driver-comedi` | `comedi_analog_input` | `comedi` / `comedi_hardware` | Readable, Parameterized | **maitai** (Linux, Comedi drivers) |
| `ComediAnalogOutputFactory` | `driver-comedi` | `comedi_analog_output` | `comedi` / `comedi_hardware` | Settable, Parameterized | **maitai** (Linux, Comedi drivers) |
| `ComediDigitalIOFactory` | `driver-comedi` | `comedi_digital_io` | `comedi` / `comedi_hardware` | Settable | **maitai** (Linux, Comedi drivers) |
| `ComediCounterFactory` | `driver-comedi` | `comedi_counter` | `comedi` / `comedi_hardware` | Readable, Settable | **maitai** (Linux, Comedi drivers) |
| `DoverAxisFactory` | `driver-dover-motion` | `dover_axis` | *(not wired into driver-registry)* | Movable, Parameterized, TriggerOnPosition | **Windows** (Dover Motion SDK, USB/Ethernet) |

### Feature Flag Tiers

Each native SDK driver has three feature tiers:

| Tier | Example | Meaning |
|------|---------|---------|
| Base | `pvcam` | Compile the crate with **mock** SDK paths |
| SDK | `pvcam_sdk` | Link against the real vendor SDK |
| Hardware | `pvcam_hardware` | Enable hardware-gated integration tests |

Convenience: `all_hardware = ["pvcam", "comedi", "andor"]` compiles all with mock paths.

## driver-universal (TOML Manifest Devices)

Always compiled. No feature flag needed. Capabilities are declared per-manifest in `config/devices/*.toml`.
All driver-universal devices now support **StateRefreshable** (post-reconnection parameter refresh, bd-47p2).

| Manifest | `name` | Capabilities | Protocol | Deployment Targets |
|----------|--------|-------------|----------|-------------------|
| `ell14.toml` | Thorlabs ELL14 | Movable, Parameterized, StateRefreshable | Serial (binary) | **maitai** (USB serial) |
| `esp300.toml` | Newport ESP300 | Movable, Parameterized, StateRefreshable | Serial (ASCII) | **maitai** (RS-232) |
| `maitai.toml` | Spectra-Physics MaiTai | Readable, WavelengthTunable, ShutterControl, EmissionControl, Parameterized, Commandable, StateRefreshable | Serial (ASCII) | **maitai** (RS-232) |
| `newport_1830c.toml` | Newport 1830-C | Readable, WavelengthTunable, Parameterized, StateRefreshable | Serial (ASCII) | **maitai** (RS-232/GPIB) |
| `ipg_laser.toml` | IPG YLPP-200-1-50-R | Readable, EmissionControl, Commandable, StateRefreshable | Serial (ASCII) | **leabs-dev** (RS-232) |
| `thorlabs_pm400.toml` | Thorlabs PM400 | Readable, WavelengthTunable, Commandable, StateRefreshable | Serial (SCPI) | **leabs-dev** (USB serial) |

### Additional Manifests (Examples / Templates)

| Manifest | Purpose |
|----------|---------|
| `esp301_example.toml` | Example for Newport ESP301 (similar to ESP300) |
| `minimal_device_template.toml` | Skeleton for new device manifests |
| `modbus_example.toml` | Example Modbus RTU device |
| `red_pitaya_pid.toml` | Red Pitaya PID controller (TCP/SCPI) |
| `sample_temperature_controller.toml` | Example temperature controller |

## Mock Drivers

Always compiled. Used for testing, demos, and development without hardware.

| Factory | `driver_type` | Capabilities |
|---------|---------------|-------------|
| `MockCameraFactory` | `mock_camera` | FrameProducer, Triggerable, ExposureControl, Stageable, Parameterized |
| `MockStageFactory` | `mock_stage` | Movable, Parameterized |
| `MockRotatorFactory` | `mock_rotator` | Movable, Parameterized |
| `MockLaserFactory` | `mock_laser` | Readable, WavelengthTunable, ShutterControl, EmissionControl, Parameterized |
| `MockPowerMeterFactory` | `mock_power_meter` | Readable, Parameterized |
| `MockDAQOutputFactory` | `mock_daq_output` | Settable, Parameterized |

## Capability Coverage Summary

Which capabilities are exercised by which driver categories:

| Capability | Native SDK | driver-universal | Mock |
|-----------|-----------|-----------------|------|
| **FrameProducer** | PVCAM, Andor iStar | — | MockCamera |
| **Triggerable** | PVCAM, Andor iStar | — | MockCamera |
| **ExposureControl** | PVCAM, Andor iStar | — | MockCamera |
| **Readable** | Comedi AI, Comedi Counter | MaiTai, Newport 1830-C, IPG, Thorlabs PM400 | MockLaser, MockPowerMeter |
| **Settable** | Comedi AO, Comedi DIO, Comedi Counter | — | MockDAQOutput |
| **Movable** | Dover Motion | ELL14, ESP300 | MockStage, MockRotator |
| **WavelengthTunable** | Andor Shamrock | MaiTai, Newport 1830-C, Thorlabs PM400 | MockLaser |
| **ShutterControl** | Andor Shamrock | MaiTai | MockLaser |
| **EmissionControl** | — | MaiTai, IPG | MockLaser |
| **Commandable** | PVCAM | MaiTai, IPG, Thorlabs PM400 | — |
| **Parameterized** | All native except Comedi DIO | ELL14, ESP300, MaiTai, Newport 1830-C | All mocks |
| **StateRefreshable** | — | All driver-universal devices | — |
| **Stageable** | — | — | MockCamera |
| **TriggerOnPosition** | Dover Motion | — | — |
| **GatedCamera** | — | — | *(test-only in experiment crate)* |
| **SpectrometerControl** | — | — | *(test-only in experiment crate)* |

### Gaps

- **EmissionControl**: No native SDK driver implements this. Covered only by driver-universal manifests and MockLaser.
- **Commandable**: Only PVCAM among native drivers. Three driver-universal manifests support it, but no mock.
- **TriggerOnPosition**: Only Dover Motion. No mock or universal equivalent.
- **GatedCamera / SpectrometerControl**: Test-only factories in `experiment/src/run_engine.rs`. No production driver.
- **Dover Motion**: Crate exists but is **not registered** in `driver-registry`. Requires manual wiring or a future feature gate.

## Deployment Target Summary

| Target | OS | Drivers Available |
|--------|-----|-------------------|
| **maitai** | Ubuntu 22.04 x86_64 | PVCAM, Comedi (AI/AO/DIO/Counter), ELL14, ESP300, MaiTai, Newport 1830-C, all mocks |
| **leabs-dev** | Ubuntu 22.04 x86_64 | Andor iStar, Andor Shamrock, IPG laser, Thorlabs PM400, all mocks |
| **Windows LIBS** | Windows 10/11 | Dover Motion *(not yet wired)*, all mocks |
| **Dev (macOS/Linux)** | Any | All mocks, driver-universal (with simulated serial) |
| **CI** | Ubuntu (GitHub Actions) | All mocks, `all_hardware` feature (mock SDK paths) |
| **WASM** | Browser | UI only (no drivers — connects to daemon via gRPC-web) |
