# Script Inventory

Comprehensive inventory of all scripts in `scripts/`. Each entry lists
the canonical use case and whether it is invoked by CI, git hooks, or
only used by developers manually.

**Canonical quality gate:** `scripts/ci/pre-push-gate.sh` (mirrors CI `validate` job).

---

## Quality Gates and Hooks

| Script | Purpose | Invoked by |
|--------|---------|------------|
| `scripts/ci/pre-push-gate.sh` | **Canonical quality gate.** Runs fmt, clippy, tests; mirrors CI `validate` job. | `.beads/hooks/pre-push` (automatic), manual |
| `scripts/ops/install-hooks.sh [quick]` | One-time setup: wires pre-commit framework and verifies pre-push gate is linked. | Developer (one-time) |
| `scripts/ci/run-ast-grep.sh` | Wrapper to find `ast-grep` or `sg` binary; used by pre-commit hooks for structural lint. | `.pre-commit-config.yaml`, `.pre-commit-quick.yaml` |
| `scripts/ops/fast-check.sh` | Quick developer smoke test (cargo check + nextest + doctests). Not a gate. | Developer (convenience) |
| `scripts/ci/feature-check.sh` | cargo-hack feature powerset check on key crates. | Developer (local CI parity) |
| `scripts/hygiene/check-doc-drift.sh` | Detects stale documentation patterns (renamed files, old names). | `.pre-commit-config.yaml` (on markdown changes) |
| `scripts/hygiene/check-inventory-drift.sh` | Detects drift between cargo metadata and docs/reference/inventory.md. | Developer (manual) |
| `scripts/hygiene/check-dependency-hygiene.sh` | Runs cargo-audit, cargo-deny, cargo-machete. | CI (`ci.yml`, `ops.yml`), developer |
| `scripts/check-benchmark-regressions.py` | Performance regression gate with statistical analysis. | CI (`ci.yml` performance-regression job) |

## Deploy

| Script | Purpose | Invoked by |
|--------|---------|------------|
| `scripts/deploy/deploy-maitai.sh` | Full deploy to maitai machine (pull, clean, build, daemon, optional GUI). | Developer (manual) |
| `scripts/deploy/deploy-leabs.sh` | Full deploy to leabs-dev (remote build, daemon restart, optional `--wasm-gui`). | Developer (manual), CI (`nightly-hardware-smoke.yml`) |
| `scripts/ops/build-maitai.sh` | Build daemon with hardware features for maitai. | Developer (manual) |
| `scripts/ops/build-lab.sh [--release]` | Build daemon with pvcam_sdk for lab use. | Developer (manual) |
| `scripts/deploy/install-service.sh` | Install daemon as a systemd service. | Developer (one-time setup) |

## Operations and Hardware

| Script | Purpose | Invoked by |
|--------|---------|------------|
| `scripts/ops/demo.sh` | Launch mock-hardware demo (daemon + GUI or script). | Developer (demo) |
| `scripts/ops/env-check.sh` | Source before hardware tests to set up environment. | Developer (manual) |
| `scripts/ops/calibrate-comedi.sh` | Run Comedi DAQ card calibration. | Developer (manual, maitai) |
| `scripts/network-watchdog` | Layered WiFi + Tailscale connectivity watchdog. Runs as systemd oneshot. | systemd timer (`network-watchdog.timer`) |
| `scripts/ops/pvcam_sdk_examples.sh` | Run PVCAM SDK example binaries on remote maitai host. | Developer (manual) |
| `scripts/ops/stress-test-comedi-concurrent.sh` | Validate concurrent AI+AO access on Comedi hardware. | Developer (manual, maitai) |

## iSTAR Repro and Crash Analysis

| Script | Purpose | Invoked by |
|--------|---------|------------|
| `scripts/repro/repro-istar-stream-crash.sh` | iSTAR stream crash repro harness (grpcurl soak + artifact capture). | Developer (manual, leabs-dev) |
| `scripts/repro/istar-stream-overnight-matrix.sh` | Long-run iSTAR repro matrix over quality/FPS/exposure grids. | Developer (manual, leabs-dev) |
| `scripts/repro/leabs-daemon-crash-wrapper.sh` | Remote daemon crash-capture wrapper used by repro/watchdog flows. | Called by repro scripts |
| `scripts/repro/leabs-daemon-watchdog.sh` | Leabs daemon health monitor with auto-restart. | Developer (manual, long-running) |
| `scripts/ops/post-crash-forensics.sh` | Post-crash system forensics (dmesg, coredumps, journal, network). | Developer (after crash) |

## Benchmarks and Performance

| Script | Purpose | Invoked by |
|--------|---------|------------|
| `scripts/ops/bench-harness.sh` | Lightweight regression harness: storage, ring buffer, tap registry, startup. | Developer (manual) |
| `scripts/ops/measure-startup.sh` | Measure daemon startup latency for a given mode. | `bench-harness.sh`, developer |

## Hygiene and Maintenance

| Script | Purpose | Invoked by |
|--------|---------|------------|
| `scripts/hygiene/target-maintenance.sh` | Clean bloated `target/` directory. | Developer, cron (via install-target-maintenance) |
| `scripts/hygiene/install-target-maintenance.sh` | Install cron job for automatic target/ cleanup. | Developer (one-time) |
| `scripts/bd-safe.sh` | Worktree-safe beads commands (auto-discovers Dolt/SQLite backend). | Developer (manual, worktrees) |
| `scripts/hygiene/beads-worktree-hygiene.sh` | Detect/clean stale worktree-local beads runtime artifacts. | Developer (manual) |
| `scripts/beads-sync-ai-proxy.sh` | Sync beads data to ai-proxy BeadHub + Dolt instances. | launchd WatchPaths agent, developer |
| `scripts/ops/regenerate_blueprints.sh` | Regenerate Rerun blueprints using an isolated Python venv. | Developer (manual) |

## Echelle Spectroscopy

| Script | Purpose | Invoked by |
|--------|---------|------------|
| `scripts/echelle/overnight-soak.sh` | 12h echelle extraction stability soak (memory, frame drops, latency). | Developer (manual) |
| `scripts/echelle/analyze-soak-results.py` | Plot and analyze soak test CSV output (memory, latency, PASS/FAIL). | Developer (after soak) |
| `scripts/echelle/validate_vs_pypeit.py` | E2E validation: compare rust-daq extraction vs PypeIt reference. | Developer (manual) |
| `scripts/echelle/reference_extract_hg2.py` | PypeIt reference extraction for Hg2 calibration comparison. | Developer (manual) |
| `scripts/echelle/fixture_sidecar_hg2.py` | Generate test fixture sidecar data for Hg2 arc frames. | Developer (manual) |
| `scripts/echelle/ci_validation_step.yml` | Reusable CI step definition for echelle validation. | CI (reusable workflow) |
| `scripts/echelle/nightly-soak.yml` | Nightly soak test CI workflow definition. | CI (nightly schedule) |
| `scripts/echelle/mechelle_pypeit_template.pypeit` | PypeIt configuration template for Mechelle 5000 spectrograph. | `validate_vs_pypeit.py` |
