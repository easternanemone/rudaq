# Build Profiles

**Status:** Active
**Last Updated:** April 2026
**Source of truth:** `Cargo.toml`, `.cargo/config.toml`, `.config/nextest.toml`, and `.github/workflows/*.yml`.

## Profile Summary

| Profile | Cargo Feature Set | Cargo Profile | When to Use |
|---------|-------------------|---------------|-------------|
| **dev** | `bin` defaults: `networking`, `server`, `db` | `dev` | Local daemon iteration and unit tests. |
| **release** | selected features | `release` | Deployment binaries and benchmarks. |
| **ci** | default features, with documented excludes | `dev` | PR validation. |
| **feature-matrix** | per-job | `dev` | Optional feature combinations. |
| **maitai** | `maitai` plus optional `db`/`metrics` as needed | `release` | Maitai lab machine. |
| **leabs** | `leabs_hardware` plus optional `db`/`metrics` as needed | `release` | LEABS lab machine. |
| **wasm** | `ui --no-default-features --features web` | `release` | Browser-based WASM GUI. |
| **hardware-test** | `hardware_tests` plus matching hardware features | `dev` | Tests requiring physical devices. |
| **demo** | `bin` defaults | `dev` | Quick-start mock hardware demo. |

## Development

Default daemon builds include networking, gRPC server wiring, and the SQLite control plane:

```bash
cargo build -p bin
cargo run -p bin -- daemon --port 50051
```

Native GUI:

```bash
cargo build -p ui
```

## CI

Primary PR validation mirrors:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets \
  --exclude ui --exclude comedi-sys --exclude driver-comedi -- -D warnings
cargo nextest run --workspace --profile ci \
  --exclude ui --exclude comedi-sys --exclude driver-comedi
```

`scripts/ci/pre-push-gate.sh` is the local pre-push parity script.

## Feature Matrix

Optional feature jobs currently cover:

| Matrix Job | Crate | Features |
|------------|-------|----------|
| storage / hdf5 | `storage` | `storage_hdf5` |
| storage / arrow | `storage` | `storage_arrow` |
| bin / all_hardware mock | `bin` | `all_hardware` |
| server / full stack | `server` | `modules,scripting,storage_hdf5,storage_arrow` |
| runtime / universal-smoke | `integration-tests` | `universal` |
| runtime / universal-db-smoke | `integration-tests` | `universal,db` |
| ui / wasm32 | `ui` | `web`, `--no-default-features`, wasm32 target |

See [feature-matrix.md](feature-matrix.md) for the full feature reference.

## Maitai Lab Hardware

Use the script; it carries the correct environment and build policy for the lab host.

```bash
bash scripts/ops/build-maitai.sh
bash scripts/deploy/deploy-maitai.sh
```

The `maitai` feature expands to real PVCAM, real Comedi, and serial support:

- `pvcam_hardware`
- `comedi_hardware`
- `hardware/serial`

SQLite control-plane support is the plain `db` feature. There are no RocksDB or SurrealDB build features.

## LEABS Lab Hardware

Use the deploy script for routine operation:

```bash
bash scripts/deploy/deploy-leabs.sh
bash scripts/deploy/deploy-leabs.sh --wasm-gui
```

Manual build:

```bash
source config/hosts/leabs-dev.env
cargo build --release -p bin --features "leabs_hardware,db"
```

`leabs_hardware` forwards to `driver-registry/andor_hardware`.

## WASM GUI

```bash
cargo check -p ui --lib --target wasm32-unknown-unknown \
  --no-default-features --features web

cd crates/ui && trunk build --release
```

## Hardware-In-The-Loop Tests

Hardware tests are ignored and feature-gated. Run them only on the matching lab machine.

```bash
source scripts/ops/env-check.sh
cargo nextest run --profile hardware --features hardware_tests
```

## Nextest Profiles

Profiles are defined in `.config/nextest.toml`.

| Profile | Use Case |
|---------|----------|
| `default` | Local development. |
| `ci` | GitHub Actions CI. |
| `hardware` | Physical hardware tests. |
| `libs-hardware` | LIBS-specific hardware tests. |
| `coverage` | Code coverage collection. |

## Related References

- [Feature matrix](feature-matrix.md)
- Runtime feature flags: `config/feature_flags.toml`
- Host environment configs: `config/hosts/maitai.env`, `config/hosts/leabs-dev.env`
- Cargo build config: `.cargo/config.toml`
- Nextest config: `.config/nextest.toml`
