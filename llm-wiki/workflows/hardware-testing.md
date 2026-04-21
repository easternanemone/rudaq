# Hardware Testing

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md §Testing Patterns
  - CLAUDE.md §Hardware Machines
  - docs/how-to/hardware-setup.md
  - docs/how-to/testing.md
see-also:
  - ../hardware/maitai.md
  - ../hardware/leabs-dev.md
  - ./build-test-lint.md
-->

Most tests run against mocks. Real-hardware tests gate behind features and
a dedicated nextest profile.

## Mocks first

- `driver_registry::create_canonical_mock_registry()` — always available, no feature flags. Deterministic RNG seed via `ScenarioConfig`.
- Mock fidelity levels: `Fast`, `Realistic`, `Noisy`, `Faulty` (see `driver-mock`).
- Use mocks for all default CI; real hardware only on labeled hosts.

## Gating hardware tests

```rust
#[cfg(feature = "hardware_tests")]
#[tokio::test]
#[ignore]                              // `cargo test` skips; hardware profile runs
async fn pvcam_live_capture() { ... }
```

## Running

```
# Must be on maitai (PVCAM / Comedi / universal) or leabs-dev (Andor / IPG / Thorlabs)
source scripts/ops/env-check.sh
cargo nextest run --profile hardware --features hardware_tests
```

Nextest `hardware` profile: 6-minute timeout per test.

## Building for real hardware

**Always** use the wrapper on maitai:

```
bash scripts/ops/build-maitai.sh
```

Sets feature flags and environment correctly (PVCAM SDK paths, Comedi
kernel-module interop). Never `cargo build` directly for maitai production.

## Machine matrix

| Machine | SSH | Daemon URL | Drivers |
|---------|-----|------------|---------|
| **maitai** | `maitai@maitai-eos` | `http://100.117.5.12:50051` | PVCAM, Comedi (AI/AO/DIO/Counter), universal (ELL14×3, ESP300×3, MaiTai, Newport PM), all mocks |
| **leabs-dev** | `ssh leabs-dev` | `http://100.109.21.118:50051` | Andor iStar, Andor Shamrock, universal (IPG YLPP-200, Thorlabs PM400), all mocks |

Full details: [`../hardware/maitai.md`](../hardware/maitai.md),
[`../hardware/leabs-dev.md`](../hardware/leabs-dev.md).

## UI interactive testing

- Native GUI + AccessKit (preferred) for headed tests over SSH X forwarding.
- WASM GUI + Chrome for browser-only paths.
- Interaction details in `AGENTS.md` (once added).

## Timing-sensitive tests

Use virtual time to keep tests deterministic:

```rust
#[tokio::test(start_paused = true)]
async fn watchdog_fires_after_timeout() {
    tokio::time::advance(Duration::from_secs(31)).await;
    ...
}
```

## Post-failure triage

1. Re-run with `--no-capture` for stdout.
2. Check `/tmp/rust_daq_heartbeat.jsonl` on the hardware host for pre-failure vitals.
3. Check daemon logs.
4. If the failure recurs under mocks, file a bead and reproduce locally.
