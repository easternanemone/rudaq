# Mock Registry — `create_canonical_mock_registry()`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-mock/
  - crates/driver-registry/
  - docs/reference/driver-capability-matrix.md §Mock Drivers
  - CLAUDE.md §Testing Patterns
see-also:
  - ./driver-registry.md
  - ../drivers/mock.md
  - ../workflows/hardware-testing.md
-->

Deterministic, feature-flag-free mocks. The default way to test
anything that touches a device.

## The canonical call

```rust
// Signature (crates/driver-registry/src/lib.rs:291):
// pub async fn create_canonical_mock_registry(
//     workspace_root: &std::path::Path,
// ) -> Result<DeviceRegistry, DaqError>

let registry =
    driver_registry::create_canonical_mock_registry(workspace_root).await?;
```

Returns a fully populated `DeviceRegistry` with one of each mock driver,
all connected and ready. The `workspace_root` is needed to resolve
universal-driver manifest paths referenced in the embedded
`CANONICAL_MOCK_CONFIG`. The older `hardware::registry::create_mock_registry`
is deprecated (`hardware/src/registry/loading.rs:153`) — prefer this one.

## Mock factories

| Factory | `driver_type` | Capabilities |
|---------|---------------|--------------|
| `MockCameraFactory` | `mock_camera` | FrameProducer, Triggerable, ExposureControl, Stageable, Parameterized |
| `MockStageFactory` | `mock_stage` | Movable, Parameterized |
| `MockRotatorFactory` | `mock_rotator` | Movable, Parameterized |
| `MockLaserFactory` | `mock_laser` | Readable, WavelengthTunable, ShutterControl, EmissionControl, Parameterized |
| `MockPowerMeterFactory` | `mock_power_meter` | Readable, Parameterized |
| `MockDAQOutputFactory` | `mock_daq_output` | Settable, Parameterized |

## Fidelity levels

`MockCameraProfile` and `MockStageProfile` select behavior:

- `Fast` — zero jitter, instant responses. Default for unit tests.
- `Realistic` — plausible timing, no noise. Good for integration.
- `Noisy` — realistic timing + injected measurement noise.
- `Faulty` — occasional transient errors for error-path coverage.

## Scenario seeds

`ScenarioConfig` groups devices with a shared RNG seed:

```rust
let scenario = ScenarioConfig::new(42)
    .with_camera(MockCameraProfile::Realistic)
    .with_stage(MockStageProfile::Noisy);
let registry = build_mock_registry(scenario);
```

Same seed → same frame pattern, same noise, same fault injection. Use for
flake-free integration tests that exercise cross-device timing.

## No feature flags

Mocks are always compiled. Do **not** gate any mock behind
`#[cfg(feature = "mock")]`. They are part of the default build.

## When to use which

- Unit test a capability trait impl → `Fast` mocks, no scenario.
- Integration test a multi-device plan → `Realistic`, scenario with seed.
- Fault-injection test (e.g. watchdog, retry, panic hook) → `Faulty` or explicit error injection.
- Demo / dev daemon without hardware → `Realistic` with a UI-friendly seed.

## Where hardware tests go instead

Anything that needs a real SDK or wire connection → see
[`../workflows/hardware-testing.md`](../workflows/hardware-testing.md).
Hardware tests live behind `#[cfg(feature = "hardware_tests")]` and
`#[ignore]`, and run only under nextest `hardware` profile on maitai /
leabs-dev.
