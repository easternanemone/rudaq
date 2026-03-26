# Build Profiles

**Status:** Active
**Last Updated:** March 2026
**Purpose:** Canonical reference for all supported build profiles, linking each to its feature flags, CI workflow, and usage context.

---

## Profile Summary

| Profile | Cargo Feature Set | Cargo Profile | When to Use |
|---------|-------------------|---------------|-------------|
| **dev** | `default` (networking, server, db-surreal-mem) | `dev` | Local iteration, unit tests |
| **release** | `default` | `release` | Unsigned release builds, benchmarks |
| **ci** | `default` | `dev` | PR validation (fmt, clippy, nextest) |
| **feature-matrix** | per-job (see below) | `dev` | CI matrix of optional feature combos |
| **maitai** | `maitai` (pvcam_hardware, comedi_hardware, serial) + `db-surreal-rocksdb` | `release` | Maitai lab machine (Prime BSI, Comedi, serial devices) |
| **leabs** | `leabs_hardware` (andor_hardware) + `db-surreal-rocksdb` | `release` | LEABS lab machine (Andor iStar, serial devices) |
| **wasm** | `web` (no-default-features) | `release` | Browser-based WASM GUI |
| **hardware-test** | `hardware_tests` | `dev` | Tests requiring physical devices |
| **demo** | `default` | `dev` | Quick-start demo with mock hardware |

---

## Profile Details

### dev (Local Development)

The default `cargo build` / `cargo run` experience. Optimized for fast incremental rebuilds via `.cargo/config.toml`:

- `split-debuginfo = "packed"` -- faster macOS linking
- `opt-level = 2` for all dependencies (only your crates compile unoptimized)
- `opt-level = 2` for build scripts and proc-macros

**Default features** (from `bin` crate):
- `networking` -- gRPC transport layer
- `server` -- full gRPC service
- `db-surreal-mem` -- in-memory SurrealDB (no disk persistence)

**Commands:**
```bash
cargo build -p bin                     # daemon
cargo build -p ui                      # native GUI (standalone feature, default)
cargo run --bin rust-daq-daemon -- daemon --port 50051
```

**CI mapping:** None (local only).

---

### release

Standard `--release` build. Uses `incremental = false` and `split-debuginfo = "packed"` (see `.cargo/config.toml`). Used for deployment binaries, benchmarks, and the release workflow.

**Commands:**
```bash
cargo build --release -p bin           # daemon
cargo build --release -p ui            # native GUI
```

**CI mapping:** `release.yml` -- triggered on `v*` tags. Builds cross-platform release bundles (Linux x64, Windows x64, macOS x64/arm64) with optional code signing.

---

### ci (PR Validation)

The primary quality gate, run on every PR and push to `main`.

**Scope:**
- Workspace-wide, excluding `ui`, `comedi-sys`, and `driver-comedi`
- Default features only (no hardware SDKs)

**Steps and matching commands:**

| CI Step | Local Equivalent |
|---------|------------------|
| Format check | `cargo fmt --all -- --check` |
| Clippy | `cargo clippy --workspace --all-targets --exclude ui --exclude comedi-sys --exclude driver-comedi -- -D warnings` |
| Unit + integration tests | `cargo nextest run --workspace --profile ci --exclude ui --exclude comedi-sys --exclude driver-comedi` |
| Ring buffer benchmark | `cargo nextest run -p storage bench_ring_buffer_write_throughput --profile ci` |
| Performance regression gate | `python3 scripts/check-benchmark-regressions.py` |
| Dependency hygiene + SBOM | `bash scripts/hygiene/check-dependency-hygiene.sh` |

**Pre-push hook parity:** `scripts/ci/pre-push-gate.sh` mirrors the first three steps locally.

**Nextest profile:** `ci` (3 retries, 60s slow-timeout, no fail-fast). Defined in `.config/nextest.toml`.

**CI mapping:** `ci.yml` -- runs on PRs (non-docs paths) and pushes to `main`.

---

### feature-matrix (CI Feature Combinations)

Tests optional feature combinations that are not part of the default build. Each matrix job runs clippy + nextest for a specific crate/feature combo.

| Matrix Job | Crate | Features | Purpose |
|------------|-------|----------|---------|
| storage / hdf5 | `storage` | `storage_hdf5` | HDF5 storage backend |
| storage / arrow | `storage` | `storage_arrow` | Arrow IPC storage |
| db / rocksdb | `db` | `kv-rocksdb` | Persistent SurrealDB |
| bin / all_hardware mock | `bin` | `all_hardware` | All mock driver crates compile |
| server / full stack | `server` | `modules,scripting,storage_hdf5,storage_arrow` | Full server feature set |
| runtime / universal-smoke | `integration-tests` | `universal` | driver-universal runtime |
| runtime / hybrid-db-mem-smoke | `integration-tests` | `universal,db-surreal-mem` | In-memory DB runtime |
| runtime / hybrid-db-rocksdb-smoke | `integration-tests` | `(many, --no-default-features)` | RocksDB runtime integration |
| ui / wasm32 | `ui` | `web` (no-default-features, target wasm32) | Browser GUI compiles + clippy |

**Feature powerset** (cargo-hack): `common`, `storage`, `driver-registry`, `pool`, `experiment` -- each checked with `--feature-powerset --no-dev-deps`. FFI features and `storage_hdf5` are skipped (require native SDKs).

**Local parity:** `bash scripts/ci/feature-check.sh` runs the powerset checks. Use `bash scripts/ci/feature-check.sh common --quick` for a fast single-crate check.

**CI mapping:** `feature-matrix.yml` -- runs on PRs (non-docs paths) and pushes to `main`.

---

### maitai (Maitai Lab Hardware)

Full hardware build for the maitai-eos lab machine. Enables all native SDK drivers present on that host.

**Feature set:** `maitai` + `db-surreal-rocksdb`

The `maitai` feature expands to:
- `pvcam_hardware` -- real PVCAM SDK (Prime BSI camera)
- `comedi_hardware` -- real Comedi DAQ card
- `hardware/serial` -- serial port communication

Serial/SCPI devices (ELL14, ESP300, MaiTai laser, 1830-C) use `driver-universal` TOML manifests and require no feature flags.

**Environment:** Requires `config/hosts/maitai.env` to set `PVCAM_SDK_DIR`, `PVCAM_LIB_DIR`, etc. PVCAM SDK must be installed at `/opt/pvcam/sdk`.

**Commands:**
```bash
# Recommended: use the build script (handles env, clean, features)
bash scripts/ops/build-maitai.sh

# Full deploy (pull, build, daemon, GUI):
bash scripts/deploy/deploy-maitai.sh

# Deploy a feature branch:
bash scripts/deploy/deploy-maitai.sh --branch feat/my-feature --with-db
```

**CI mapping:** `nightly-hardware-smoke.yml` (scheduled daily + manual dispatch). Deploys to maitai-eos, runs gRPC hardware smoke validation. `hardware-tailscale.yml` provides SSH connectivity checks.

---

### leabs (LEABS Lab Hardware)

Hardware build for the leabs-dev lab machine. Enables Andor SDK3 drivers.

**Feature set:** `leabs_hardware` + `db-surreal-rocksdb`

The `leabs_hardware` feature expands to:
- `driver-registry/andor_hardware` -- real Andor SDK3 (iStar camera)

Serial/SCPI devices (IPG laser, Thorlabs PM400) use `driver-universal` TOML manifests.

**Environment:** Requires `config/hosts/leabs-dev.env` to set `ANDOR_SDK3_DIR`. Andor SDK3 library must be at `/usr/local/lib/libatcore.so`.

**Commands:**
```bash
# Full deploy (pull, build, daemon, GUI):
bash scripts/deploy/deploy-leabs.sh

# With WASM GUI:
bash scripts/deploy/deploy-leabs.sh --wasm-gui

# Deploy a feature branch:
bash scripts/deploy/deploy-leabs.sh --branch feat/my-feature
```

**CI mapping:** `nightly-hardware-smoke.yml` (scheduled daily + manual dispatch). Deploys to leabs-dev, runs gRPC hardware smoke validation.

---

### wasm (Browser GUI)

WASM build of the `ui` crate for browser deployment.

**Feature set:** `--no-default-features --features web` on `ui` crate, targeting `wasm32-unknown-unknown`.

**Build tool:** `trunk` (external CLI, not a Cargo dependency). Deploy scripts auto-install it.

**Commands:**
```bash
# Compile check (CI parity):
cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web

# Full build:
cd crates/ui && trunk build --release

# Serve locally:
cd crates/ui/dist && python3 -m http.server 8080
```

**CI mapping:** `feature-matrix.yml` (the `wasm-check` job) -- checks compilation and runs clippy for the wasm32 target.

---

### hardware-test (Hardware-in-the-Loop)

Tests that require physical devices. Gated by `#[cfg(feature = "hardware_tests")]` + `#[ignore]`.

**Feature set:** `hardware_tests` (plus whatever hardware features match the connected devices).

**Nextest profile:** `hardware` (single-threaded, 120s slow-timeout, 3 retries). Test groups (`pvcam-hardware`, `andor-hardware`, `elliptec-hardware`, `serial-hardware`) ensure exclusive device access.

**Commands:**
```bash
# On maitai:
source scripts/ops/env-check.sh
cargo nextest run --profile hardware --features hardware_tests

# Specific driver:
cargo nextest run -p driver-pvcam --features pvcam_sdk --profile hardware
```

**CI mapping:** Not run in routine PR CI. Covered by `nightly-hardware-smoke.yml` (deploy + gRPC validation, not unit tests) and manual `hardware-tailscale.yml`.

---

### demo (Quick-Start Demo)

Runs the daemon with mock hardware for demonstration and local testing. No hardware SDKs needed.

**Feature set:** `default` (networking, server, db-surreal-mem).

**Commands:**
```bash
bash scripts/ops/demo.sh
```

The script builds the daemon, starts it with `config/demo.toml`, and offers options to run a scripted scan, launch the GUI, or keep the daemon running for manual interaction.

**CI mapping:** None (local only).

---

## Nextest Profiles Reference

All nextest profiles are defined in `.config/nextest.toml`:

| Profile | Retries | Slow Timeout | Fail-Fast | Use Case |
|---------|---------|--------------|-----------|----------|
| `default` | 2 | 30s (terminate after 4x) | yes | Local development |
| `ci` | 3 | 60s (terminate after 3x) | no | GitHub Actions CI |
| `hardware` | 3 | 120s (terminate after 3x) | no | Physical hardware tests |
| `libs-hardware` | inherits `hardware` | inherits | inherits | LIBS-specific hardware tests |
| `coverage` | 0 | 90s (terminate after 4x) | no | Code coverage collection |

---

## CI Workflow Map

| Workflow | Trigger | Profile Used | Runner |
|----------|---------|--------------|--------|
| `ci.yml` | PRs, push to `main` | ci | self-hosted (leabs) |
| `feature-matrix.yml` | PRs, push to `main` | ci (feature-matrix + powerset + wasm) | self-hosted (leabs) |
| `docs.yml` | Docs-path PRs, push to `main` | (markdown link check) | self-hosted (leabs) |
| `release.yml` | `v*` tags, manual | release | multi-platform (ubuntu, windows, macos-intel, macos-arm64) |
| `nightly-hardware-smoke.yml` | Daily 07:15 UTC, manual | maitai / leabs | self-hosted (leabs) |
| `hardware-tailscale.yml` | Manual | (SSH connectivity) | self-hosted (leabs) |
| `ops.yml` | Weekly Monday 06:35 UTC, manual | (audit, benchmarks, Windows check) | self-hosted (leabs) / windows-latest |

---

## Related References

- Feature matrix details: [feature-matrix.md](feature-matrix.md)
- Runtime feature flags: `config/feature_flags.toml`
- Host environment configs: `config/hosts/maitai.env`, `config/hosts/leabs-dev.env`
- Cargo build config: `.cargo/config.toml`
- Nextest config: `.config/nextest.toml`
