# crate: `integration-tests`

<!--
last-ingested: 2026-04-19
sources:
  - crates/integration-tests/
  - docs/how-to/testing.md
see-also:
  - ../workflows/build-test-lint.md
  - ../workflows/hardware-testing.md
-->

**Role:** Workspace-level integration test suite. Exercises cross-crate
flows: driver registry → RunEngine → storage sinks → server → client.

**Examples:** `tests/multi_device_orchestration.rs`.

**CI slice:**

```
cargo nextest run -p integration-tests --features universal --profile ci
```

**Hardware tests** live here behind `#[cfg(feature = "hardware_tests")]`
+ `#[ignore]`. Run only on maitai / leabs-dev via
`cargo nextest run --profile hardware --features hardware_tests`.
