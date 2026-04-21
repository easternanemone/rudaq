# crate: `driver-mock`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-mock/Cargo.toml
  - crates/driver-mock/src/
  - docs/reference/driver-capability-matrix.md §Mock Drivers
see-also:
  - ../concepts/mock-registry.md
  - ../drivers/mock.md
-->

**Role:** Always-compiled mock drivers for testing, simulation, and demo
mode. **No feature flag.**

**Factories:** MockCamera, MockStage, MockRotator, MockLaser,
MockPowerMeter, MockDAQOutput. Full capability breakdown in
[`../concepts/mock-registry.md`](../concepts/mock-registry.md).

**Fidelity profiles:**

- `MockCameraProfile::{Fast,Realistic,Noisy,Faulty}`.
- `MockStageProfile::{Fast,Realistic,Noisy,Faulty}`.

**`ScenarioConfig`:** groups devices with a shared RNG seed for
deterministic multi-device tests. Same seed → same frames, same noise,
same fault injection.

**Consumers:**

- Unit tests across the workspace.
- `integration-tests` crate (multi-device orchestration).
- `bin` daemon in demo / dev mode (`config/demo.toml`, `config/demo_mock_all.toml`).
- GUI development.

**Rules:**

- Never feature-gate a mock behind `#[cfg(feature = "mock")]`. Mocks are default.
- Expand fidelity profiles before adding ad-hoc mock variants.
