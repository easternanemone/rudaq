# crate: `scripting`

<!--
last-ingested: 2026-04-19
sources:
  - crates/scripting/
  - docs/reference/script-inventory.md
  - examples/*.rhai
see-also:
  - ./experiment.md
  - ./daq-modules.md
-->

**Role:** Embed the Rhai scripting language and bridge it to
`RunEngine` / `DeviceRegistry`. Optional PyO3 bindings.

**Key types:**

- `ScriptEngine` — backend-agnostic trait (so a future Python / Lua backend can slot in).
- `RhaiEngine` — the default embedded backend (`crates/scripting/src/rhai_engine.rs`). Constructed via `RhaiEngine::with_hardware()` or `RhaiEngine::with_limit(10_000)` for the sandbox op cap.

**Key features:**

- Rhai sandbox: **10 000 operation limit** (verified — `RhaiEngine::with_limit(10_000)` in `rhai_engine.rs`), timeout protection.
- Synchronous-looking Rhai wrappers over async Rust (`stage.move_abs(10.0)` blocks the Rhai fiber until resolved).
- Hot-swap: scripts are uploaded via gRPC and executed immediately — no daemon restart.

**Replaced:** `ScriptHost` → `ScriptEngine` (old alias removed).
`execute_script` free function (was in `hardware`) has been removed.

**Examples:** `examples/*.rhai` (e.g. `demo_scan.rhai`).

**Python bindings:** optional via PyO3 feature; off by default.

**Security:** scripts are untrusted user input. Keep operation limits
and timeouts; do not expand the Rhai surface without threat review.
