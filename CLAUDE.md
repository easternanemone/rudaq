# CLAUDE.md

Guidance for Claude Code in this repo. Full agent policy: [`AGENTS.md`](AGENTS.md). Beads workflow: `bd prime`.

**LLM Wiki is mandatory context.** Start at [`llm-wiki/index.md`](llm-wiki/index.md), then read [`llm-wiki/invariants.md`](llm-wiki/invariants.md) and the relevant crate/concept/driver/workflow pages before broad code search. The wiki orients; source wins on conflicts. If source and wiki disagree, fix the wiki or file a bead.

## LLM Wiki Context Protocol

For every non-trivial task:

1. Read the wiki index and relevant linked pages.
2. Use wiki pages to target source inspection; do not treat wiki prose as sufficient proof for behavior.
3. After changing durable architecture, features, workflows, hardware facts, or crate ownership, update affected `llm-wiki/` pages and append [`llm-wiki/log.md`](llm-wiki/log.md).
4. For wiki edits, follow [`llm-wiki/schema.md`](llm-wiki/schema.md).

## Non-Negotiable Rules

- **NEVER push directly to main** — feature branch + PR always (exception: single-file fix under 20 lines)
- **Use `bd` for ALL task tracking** — never TodoWrite, TaskCreate, or markdown TODOs
- **Use `colgrep`** as primary search tool; `sg` (ast-grep) for AST patterns
- **Fix pre-existing warnings** in any file you touch — leave every file cleaner than you found it
- **Fix pre-existing test failures** — "already broken" is not acceptable; create a bead if non-trivial

## Git Workflow

```bash
git checkout -b feat/<issue-id>-description   # Always branch first
# ... commit to branch ...
git push -u origin HEAD && gh pr create       # PR required for review
```

Exception: single-file fixes under 20 lines may go direct to main.

## Build / Test / Lint

```bash
# Fast smoke
cargo check --workspace --exclude ui

# Tests
cargo nextest run                        # Local
cargo nextest run --profile ci           # CI profile (3 retries, no fail-fast)
cargo nextest run -p <crate>             # Single crate
cargo nextest run <test_name>            # Single test
cargo test --doc                         # Doctests (nextest doesn't support)

# Format + Lint
cargo fmt --all -- --check               # Format check (CI parity)
cargo clippy --workspace --all-targets --exclude ui --exclude comedi-sys --exclude driver-comedi -- -D warnings  # CI clippy gate

# Full local smoke (check + nextest + doctests)
bash scripts/ops/fast-check.sh

# CI parity slices
cargo nextest run --workspace --exclude ui --exclude comedi-sys --exclude driver-comedi --profile ci
cargo nextest run -p integration-tests --features universal --profile ci
cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web

# Hardware tests (maitai only)
source scripts/ops/env-check.sh && cargo nextest run --profile hardware --features hardware_tests
# Maitai build: bash scripts/ops/build-maitai.sh  (NEVER build without this for real hardware)
```

## Code Style

- Rust 1.92.0 pinned (`rust-toolchain.toml`), edition 2024, Tokio async throughout
- **Hardware state**: always `Parameter<T>` with `BoxFuture<'static, Result<()>>` callbacks — never `Arc<Mutex<T>>`
- **Errors**: propagate with `?`, context via `anyhow::Context`; no `.unwrap()` in library code; `.expect("reason")` for invariants
- `async_trait` retained for capability traits — native async fn in trait requires static dispatch, but `Arc<dyn Trait>` needs boxed futures
- TODO/FIXME must reference a bead: `TODO(bd-xxxx)`

## Architecture (30-crate workspace)

```
Foundation:  common-traits, common, pool, protocol
Hardware:    hardware, driver-registry, driver-mock, driver-universal + SDK drivers (pvcam, andor, comedi, dover)
Engine:      experiment (RunEngine), scripting (Rhai), daq-modules
Services:    server (gRPC), client, db (SQLite), storage (HDF5/Arrow/Zarr)
Data:        echelle, atomic-reference
Apps:        bin (daemon), ui, ui-graph, ui-slint
Testing:     integration-tests
```

**`driver-universal` is the forward path.** New serial/TCP/SCPI devices → TOML manifests in `config/devices/`, not new crates.

Key abstractions: `DeviceId` (Arc<str>-backed), `Parameter<T>` (reactive state), capability traits (`Movable`, `FrameProducer`, …), `Plan` + `RunEngine` (Bluesky-style), `RingBuffer` (mmap/seqlock).

Start with [`llm-wiki/architecture.md`](llm-wiki/architecture.md) for dense agent context, then use [`docs/explanation/architecture.md`](docs/explanation/architecture.md) for long-form human docs.

## Testing Patterns

- **Mock devices**: `driver_registry::create_canonical_mock_registry()` — always available, no feature flags
- **Nextest profiles**: `default` (2 retries), `ci` (3 retries, no fail-fast), `hardware` (6min timeout)
- **Timing tests**: `#[tokio::test(start_paused = true)]` for deterministic timing
- **Hardware gating**: `#[cfg(feature = "hardware_tests")]` + `#[ignore]`

## Hardware Machines

| Machine | SSH | Daemon URL | Devices |
|---------|-----|-----------|---------|
| maitai | `maitai@maitai-eos` | `http://100.117.5.12:50051` | 15 (PVCAM, Comedi, ELL14×3, MaiTai, ESP300×3, Newport PM) |
| leabs-dev | `ssh leabs-dev` | `http://100.109.21.118:50051` | 3 (Andor iStar, IPG YLPP-200, Thorlabs PM400) |

Hardware-in-the-loop access: Native GUI + AccessKit (preferred) or WASM GUI + Chrome — see AGENTS.md.

## Quick Commands (Slash)

- `/test` — cargo nextest with smart defaults
- `/clippy` — clippy with CI-parity flags
- `/check` — fast cargo check
- `/grind` — autonomous beads issue loop
- `/worktree-init` — create parallel worktrees
- `/worktree-deliver` — commit, PR, close bead from worktree

## Session Close (MANDATORY)

Work is NOT complete until `git push` succeeds:

```bash
bd close <id>          # Close finished issues
bd dolt push           # Sync beads remote
git push               # Push code
git status             # Verify "up to date with origin"
```

## References

- Agent policy: `AGENTS.md`
- LLM wiki index: `llm-wiki/index.md`
- LLM wiki schema: `llm-wiki/schema.md`
- Architecture: `docs/explanation/architecture.md`
- Testing: `docs/how-to/testing.md`
- Hardware setup: `docs/how-to/hardware-setup.md`
- Driver guide: `docs/how-to/hardware-drivers.md`
- WASM DOM interop: `docs/how-to/wasm-dom-interop.md`
- Echelle calibration CLI: `docs/how-to/echelle-calibration-development.md`
- Feature flags: `config/feature_flags.toml`
- Build config: `.cargo/config.toml`

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:b9766037 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd dolt push   # if remote 'origin' missing: bash scripts/ops/setup-beads-dolt-remote.sh
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
