# Repository Guidelines

## Current Project Structure

**Status**: Single-crate application (flat structure)

```
rust-daq/
├── Cargo.toml          # Single crate configuration
├── src/                # Application source code
│   ├── main.rs         # Entry point
│   ├── lib.rs          # Library exports
│   ├── app.rs          # Core application state
│   ├── core.rs         # Core traits (Instrument, DataProcessor)
│   ├── data/           # Data processors (FFT, IIR, Trigger, Storage)
│   ├── instrument/     # Instrument implementations (ESP300, MaiTai, etc.)
│   └── gui/            # GUI components (egui)
├── config/             # Configuration files
├── tests/              # Integration tests
└── target/             # Build artifacts (do not commit)
```

**Future Architecture** (planned with Python integration):
- Workspace structure with separate `rust_daq/` GUI crate
- Plugin system in `plugins/` directory for modular instrument drivers
- PyO3 bindings in `python/` for high-level scripting

## Build, Test, and Development Commands
- `cargo check` — Fast compile-time validation before opening a PR
- `cargo run` — Launches the desktop application with default features
- `cargo run --features full` — Run with all optional features (HDF5, Arrow, VISA)
- `cargo fmt` — Format code using `rustfmt`; required prior to commits
- `cargo clippy --all-targets --all-features` — Static analysis for common Rust pitfalls
- `cargo test --all-features` — Run all tests with optional feature backends

## Coding Style & Naming Conventions
We rely on standard Rust 4-space indentation and `rustfmt` defaults. Use `snake_case` for functions and files, `CamelCase` for types, and SCREAMING_SNAKE_CASE for constants. Keep modules cohesive—instrument drivers live in `src/instrument/`, data processors in `src/data/`, and GUI components in `src/gui/`. Document intent with concise comments when logic spans multiple async tasks or channels.

## Testing Guidelines
Unit tests live beside their modules; integration coverage belongs in `tests/`, e.g., `tests/integration_test.rs`. Run `cargo test --all-features` before pushing to ensure optional backends compile. When adding hardware integrations, provide mocked pathways or feature flags so tests run in CI without devices attached.

## Commit & Pull Request Guidelines
Follow the Conventional Commits pattern already in history (`feat:`, `fix:`, etc.), referencing issue IDs where relevant. Each PR should summarise the change scope, note impacted subsystems (UI, pipeline, plugin), and include screenshots or logs when UI or acquisition behavior changes. Link configuration updates to the matching sample in `config/` so reviewers can reproduce the scenario.

## Multi-Agent Workflow & Tooling
- **Jules Working Directory**: Jules AI agents work in a separate `rust_daq/` directory to avoid conflicts. This is NOT the main project structure—it's a Jules-specific workspace.
- **Main Project**: All production code lives in `src/` at the repository root (single-crate structure).
- Always request a dedicated `git worktree` before starting. The default repo hosts active automation; parallel agents working in the same directory risk clobbering each other's changes.
- Run `BEADS_DB=.beads/daq.db bd …` whenever you interact with the beads tracker; creating `$HOME/.beads` is disallowed in the sandbox and will fail.
- Before handing the repo back, run `cargo check` and `git status -sb` to verify the workspace is clean. Our recent error-handling updates live in `src/error.rs` and `src/app.rs`; confirm they remain intact if you touch those files.
