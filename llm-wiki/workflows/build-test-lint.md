# Build / Test / Lint

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md §Build / Test / Lint
  - scripts/ops/fast-check.sh
  - .github/workflows/
see-also:
  - ./hardware-testing.md
  - ../invariants.md
-->

CI parity is the bar. Every command here is intended to match a slice of CI.

## Fast smoke

```
cargo check --workspace --exclude ui
```

## Tests

| Command | Purpose |
|---------|---------|
| `cargo nextest run` | Local default, 2 retries. |
| `cargo nextest run --profile ci` | CI profile: 3 retries, no fail-fast. |
| `cargo nextest run -p <crate>` | Single crate. |
| `cargo nextest run <test_name>` | Single test. |
| `cargo test --doc` | Doctests (nextest does not run doctests). |

## Format + lint

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets \
  --exclude ui --exclude comedi-sys --exclude driver-comedi \
  -- -D warnings
```

The exclusions match the CI clippy gate (`ui` excluded because it
cross-compiles to wasm separately; `comedi-sys` / `driver-comedi` excluded
because they are Linux-Comedi-only).

## Full local smoke

```
bash scripts/ops/fast-check.sh   # check + nextest + doctests
```

## CI parity slices

```
cargo nextest run --workspace --exclude ui --exclude comedi-sys --exclude driver-comedi --profile ci
cargo nextest run -p integration-tests --features universal --profile ci
cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web
```

## Common failure modes

- **Clippy drift on a crate you touched** → per [`invariants.md`](../invariants.md) §Hygiene, fix the warnings you introduced *and* any pre-existing ones in files you touched.
- **Nextest hangs** → check for missing `#[tokio::test(start_paused = true)]` on timing-sensitive tests.
- **Doctest failure** → doctests do not run under nextest; always run `cargo test --doc` separately.
- **WASM check fails** → only `ui` crate targets wasm; isolate with `-p ui`.
