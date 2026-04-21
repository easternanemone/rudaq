# Invariants

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md
  - rust-toolchain.toml
  - Cargo.toml (workspace)
  - docs/explanation/architecture.md
see-also:
  - ./schema.md
  - ../AGENTS.md
  - ../CLAUDE.md
-->

Hard rules that must hold across the codebase. If code or docs contradict
these, the code/docs are wrong — fix them, don't bend the rule.

## Toolchain & language

- **Rust 1.92.0** pinned in `rust-toolchain.toml`. Do **not** bump without a bead and a compatibility review.
- **Edition 2024** for all crates **except** `andor-sdk3-sys`, `comedi-sys`, and `pvcam-sys` which are still on `edition = "2021"` (verified 2026-04-19). Bump those in a dedicated PR — they are FFI binding crates and editions should not block workspace-wide upgrades.
- There is **no `edition = "..."` field at workspace level** in root `Cargo.toml`; per-crate `edition` is authoritative. Root `Cargo.toml` carries `rust-version = "1.85.0"` (advisory MSRV), which is separate from the pinned channel.
- **Tokio** async throughout. No `std::thread::sleep` in async code; no `block_on` on the main runtime.
- `async_trait` is retained for capability traits because `Arc<dyn Trait>` requires boxed futures — do not migrate capability traits to native async-fn-in-trait.

## State management

- **Hardware state is always `Parameter<T>`** with `BoxFuture<'static, Result<()>>` callbacks. **Never `Arc<Mutex<T>>`** for hardware state.
- `Observable<T>` (the primitive under `Parameter<T>`) is backed by `tokio::sync::watch`. Do not reimplement observation.
- `DeviceId` is `Arc<str>`-backed. Cheap to clone; never `String::clone`.

## Errors

- Propagate with `?`. Add context via `anyhow::Context`.
- **No `.unwrap()` in library code.** Use `.expect("<reason invariant holds>")` only for true invariants.
- Library errors: `thiserror`. Top-level / application errors: `anyhow`.
- `unsafe_code = "warn"` workspace-wide. Unsafe blocks live in `*-sys` crates and a handful of FFI boundaries — nowhere else without review.

## Drivers

- **`driver-universal` is the forward path** for new serial / TCP / SCPI devices. Do **not** create a new `driver-<name>` crate unless the device requires a vendor C SDK.
- Each SDK driver has a paired `*-sys` crate for raw FFI bindings.
- Each driver implements `DriverFactory` and is registered in `driver-registry` behind the correct feature gate (`pvcam`, `andor`, `comedi`, or none for universal/mock).
- Mock drivers are **always compiled** and registered via `create_canonical_mock_registry()`. No `#[cfg(feature = "mock")]`.

## Testing

- **Mock devices** come from `driver_registry::create_canonical_mock_registry()`. No feature flags needed.
- **`cargo nextest run`** is the test runner. `cargo test --doc` for doctests only (nextest can't run them).
- Nextest profiles: `default` (2 retries), `ci` (3 retries, no fail-fast), `hardware` (6 min timeout).
- Hardware tests: `#[cfg(feature = "hardware_tests")]` + `#[ignore]`. Run only via `--profile hardware --features hardware_tests`.
- Timing-sensitive tests: `#[tokio::test(start_paused = true)]` for deterministic virtual time.

## Git & PRs

- **Never push directly to `main`.** Feature branch + PR always. (Exception: single-file fix ≤20 lines.)
- Feature branch format: `feat/<bead-id>-description` or `claude/<slug>`.
- Branch → commit → `git push -u origin HEAD` → open a **draft** PR.
- Commits reference a bead where applicable: `TODO(bd-xxxx)` in code and `(bd-xxxx)` in commit messages.

## Task tracking

- **`bd` (beads) for all tracking.** Do **not** use `TodoWrite`, `TaskCreate`, or markdown TODO lists.
- `bd prime` on session start if unclear; `bd ready` to find available work; `bd update <id> --claim` before working; `bd close <id>` when done; `bd dolt push` before `git push`.
- Persistent per-repo knowledge: `bd remember`. Do not create `MEMORY.md` files.

## Session close

Work is **not complete** until `git push` succeeds.

```
git pull --rebase
bd dolt push
git push
git status   # must show "up to date with origin"
```

## Search

- Primary: `colgrep`. AST patterns: `sg` (ast-grep). Do not reach for shell `grep` / `find` first.

## Hygiene

- **Fix pre-existing warnings in any file you touch.** Leave every file cleaner than you found it.
- **Fix pre-existing test failures.** "Already broken" is not acceptable. If non-trivial, file a bead and link it from the PR.
- TODO / FIXME **must** reference a bead: `TODO(bd-xxxx)`.

## Known conventions & staleness watchlist

- **`AGENTS.md` is gitignored** (see `.gitignore:153`) — the file is *expected* to be absent from the git tree. It is auto-injected into agent sessions by tooling. Do not commit a tracked `AGENTS.md` without coordinating with the maintainers.
- `docs/reference/driver-capability-matrix.md` was generated 2026-03-13 — verify before trusting for feature-gate decisions.
- Workspace `Cargo.toml` carries `rust-version = "1.85.0"` (advisory MSRV). The pinned channel is **`1.92.0`** in `rust-toolchain.toml`. If the two diverge in a confusing way, the pin wins.
