# driver: mock

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-mock/
  - docs/reference/driver-capability-matrix.md §Mock Drivers
see-also:
  - ../concepts/mock-registry.md
  - ../crates/driver-mock.md
-->

**Crate:** `driver-mock` (always compiled, no feature flag).

## Factories

| Factory | `driver_type` | Capabilities |
|---------|---------------|--------------|
| `MockCameraFactory` | `mock_camera` | FrameProducer, Triggerable, ExposureControl, Stageable, Parameterized |
| `MockStageFactory` | `mock_stage` | Movable, Parameterized |
| `MockRotatorFactory` | `mock_rotator` | Movable, Parameterized |
| `MockLaserFactory` | `mock_laser` | Readable, WavelengthTunable, ShutterControl, EmissionControl, Parameterized |
| `MockPowerMeterFactory` | `mock_power_meter` | Readable, Parameterized |
| `MockDAQOutputFactory` | `mock_daq_output` | Settable, Parameterized |

## Fidelity profiles

`MockCameraProfile` and `MockStageProfile` pick behavior:

- `Fast` — zero jitter, instant responses. Default for unit tests.
- `Realistic` — plausible timing, no noise.
- `Noisy` — realistic timing + injected measurement noise.
- `Faulty` — occasional transient errors for error-path coverage.

## Scenario seeds

`ScenarioConfig` groups devices with a shared RNG seed for deterministic
multi-device tests.

## Canonical entry point

```rust
let registry = driver_registry::create_canonical_mock_registry();
```

See [`../concepts/mock-registry.md`](../concepts/mock-registry.md).

## What mocks **do not** cover

- `Commandable` — no mock implements this capability.
- `TriggerOnPosition` — only `driver-dover-motion` (experimental / unwired).
- `GatedCamera`, `SpectrometerControl` — test-only factories inside the
  `experiment` crate; no general-purpose mocks.

If your test needs a capability that no mock provides, prefer extending
`driver-mock` over creating a one-off test fake.
