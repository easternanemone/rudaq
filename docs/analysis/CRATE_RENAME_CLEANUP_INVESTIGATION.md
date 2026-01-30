# Crate Rename Merge – Cleanup Investigation

**Date:** 2026-01-29  
**Method:** Octocode `localSearchCode`, grep, PR #253 / #254 review, config/docs audit.  
**Scope:** Post-merge gaps from refactor(crates) rename epic (bd-q7l1). **Both PR #253 and PR #254.**

---

## 1. What the Crate Rename PRs Did

### 1.1 PR #253 (bd-q7l1.1)

**Merged:** 2026-01-28 ([#253](https://github.com/TheFermiSea/rust-daq/pull/253)).

| Old | New | Dir | Package |
|-----|-----|-----|---------|
| `protocol` | `protocol` | `crates/protocol` | `protocol` |
| `pool` | `pool` | `crates/pool` | `pool` |
| `daq-core` | `common` | `crates/common` | `common` |

- Renamed directories, package names, **Cargo.toml** deps, **Rust `use`**, and **some** docs.

### 1.2 PR #254 (Phases 2–4)

**Merged:** 2026-01-28 ([#254](https://github.com/TheFermiSea/rust-daq/pull/254)). Branch: `crate-rename-phases-2-4`.

**Phase 2 (Core):** `storage` → `storage`, `server` → `server`, `daq-client` → `client`.  
**Phase 3 (Hardware):** `hardware` → `hardware`, `drivers` → `drivers`.  
**Phase 4 (Apps):** `ui` → `ui`, `scripting` → `scripting`, `experiment` → `experiment`, `bin` → `bin`, `examples` → `examples`.

- Updated **Cargo.toml** names/deps, **Rust imports**, **docs**, and **`include_str!`** paths.  
- **Did not touch:** `release-please-config.json`, `deny.toml`, CI/workflows, `regenerate_blueprints.sh`, `build-maitai.sh` / `build-lab.sh`, or server/rerun blueprint paths in **server.rs** / **rerun_sink.rs**.

### 1.3 Combined Layout

**Current:** `crates/{bin,client,common,drivers,driver-*,experiment,hardware,pool,protocol,plugin-*,scripting,server,storage,ui}`. **No** `crates/daq-*` remain.

### 1.4 What Neither PR Updated

**Neither #253 nor #254** changed: `release-please-config.json`, `deny.toml`, CI/workflows (`ci.yml`, `code-quality.yml`, `coverage.yml`), `.pre-commit-config.yaml`, `scripts/regenerate_blueprints.sh`, `scripts/build-maitai.sh`, `scripts/build-lab.sh`, or the hardcoded `crates/server/blueprints/...` paths in **server.rs** / **rerun_sink.rs** / **blueprints/README.md**. Those still use old names or paths → **cleanup gaps** below.

---

## 2. Gaps and Broken References

### 2.1 Config / Tooling (High – Breaks Commands)

| Location | Issue | Fix |
|----------|-------|-----|
| **release-please-config.json** `extra-files` | Lists `crates/protocol/`, `crates/common/`, `crates/hardware/`, `crates/server/`, `crates/experiment/`, `crates/scripting/`, `crates/ui/`, `crates/bin/` | Use actual paths: `protocol`, `common`, `hardware`, `server`, `experiment`, `scripting`, `ui`, `bin` |
| **deny.toml** `[[licenses.clarify]]` | `crate = "bin"`, `"ui"`, `"experiment"` | Package names are `bin`, `ui`, `experiment` |
| **CI / workflows** | `--exclude ui`, `--exclude server` | Exclude `ui`, `server` (actual package names) |
| **scripts/build-maitai.sh** | `-p bin` | `-p bin` |
| **scripts/build-lab.sh** | `-p bin` | `-p bin` |
| **scripts/regenerate_blueprints.sh** | `BLUEPRINT_DIR="crates/server/blueprints"` | `crates/server/blueprints` (dir does not exist otherwise) |
| **.pre-commit-config.yaml** | `--exclude ui` | `--exclude ui` |
| **devbox.json** | `tests:core`: `cargo test -p daq-core -p hardware -p server` | `-p common -p hardware -p server` |

### 2.2 Runtime / Code (High – Wrong Paths)

| Location | Issue | Fix |
|----------|-------|-----|
| **crates/server/src/grpc/server.rs** | Default `RERUN_BLUEPRINT` = `crates/server/blueprints/daq_default.rbl`; error message references `crates/server/blueprints/generate_blueprints.py` | Use `crates/server/blueprints/...` |
| **crates/server/src/rerun_sink.rs** | Doc examples `cd crates/server/blueprints`, `load_blueprint("crates/server/blueprints/...")` | `crates/server/blueprints` |
| **crates/server/blueprints/README.md** | `crates/server/blueprints/...`, `crates/server/src/rerun_sink.rs` | `crates/server/blueprints`, `crates/server/src/rerun_sink.rs` |

### 2.3 Docs – Old Crate / Path Names

| Location | Issue |
|----------|--------|
| **README.md** | Links to `crates/ui/`, `crates/server/`, `crates/experiment/`; `-p bin`, `ui` |
| **CLAUDE.md** | Supervisor table: `protocol`, `pool`; build steps `-p bin`, `-p ui` |
| **AGENTS.md** | Workspace crates list: `protocol`, `ui`, `bin`, etc. |
| **CONTRIBUTING.md** | `server`, `experiment`, `examples` paths |
| **JULES.md** | `crates/protocol/`, `crates/server/`, etc. |
| **docs/README.md** | `pool`, `protocol`, `ui` README links |
| **docs/architecture/** | ARCHITECTURE, NEWCOMER_GUIDE, adr-*, etc.: `server`, `experiment`, `protocol`, `pool`, `ui`, `bin` paths |
| **.planning/codebase/** | STRUCTURE, CONVENTIONS, CONCERNS, ARCHITECTURE, TESTING, INTEGRATIONS: use `crates/*` paths (driver-*, plugin-*, etc.) |
| **.planning/phases/** | Multiple PLAN/RESEARCH/VERIFICATION files reference `ui`, `experiment`, `server`, etc. |
| **.prompts/** | Driver plugins prompts: `hardware`, `server`, etc. |
| **CLAUDE.md.suggested** | `protocol`, `ui` |
| **crates/rust-daq/README.md** | `crates/protocol/`, `crates/bin/` |
| **crates/bin/README.md** | `crates/bin` |
| **crates/server/README.md** | `server`, `protocol` |
| **crates/storage/README.md** | `../pool`, `../experiment` |
| **crates/common/README.md** | `../server` |
| **knowledge.md**, **COMPARISON_***, **DEMO.md**, **GEMINI.md** | Various `daq-*` refs |

### 2.4 Rust / ADR – Old Crate Names in Text

| Location | Issue | Fix |
|----------|-------|-----|
| **crates/pool/README.md** | Examples `use daq_pool::Pool`, `use daq_pool::{BufferPool, PooledBuffer}` | `use pool::...` |
| **crates/driver-pvcam/.../frame_pool.rs** | Comment "Re-export FrameData from **pool**" | (already fixed) |
| **docs/architecture/adr-pvcam-pool-migration-results.md** | `daq_pool::Loaned`, `daq_pool::FrameData`; `crates/pool/` | `pool::`, `crates/pool/` |
| **.planning/codebase/CONVENTIONS.md** | `daq_proto::` | `protocol::` |
| **.planning/phases/06-*.md, 05-05, 03-01, 01-* ** | `daq_proto::daq::...` in code blocks | `protocol::daq::...` (or note “historical”) |
| **DEMO.md** | Python `from daq_proto import ...` | Python stub module name may differ; verify and update if still used |

### 2.5 Lockfiles / Generated

| Location | Issue |
|----------|--------|
| **crates/examples/Cargo.lock** | Contains `protocol`, `daq-core`; examples `Cargo.toml` uses `protocol`, `common`. Lockfile stale or from pre-rename resolution. |
| **crates/bin/Cargo.lock** | Contains `daq-core`; bin uses `common`. Stale. |

### 2.6 Ast-grep / Rule Paths

| Location | Issue |
|----------|--------|
| **crates/rust-daq/ast-grep-rules/rules/best-practices.yml** | `crates/ui/src/` | `crates/ui/src/` |
| **crates/rust-daq/ast-grep-rules/rules/suggest-tracing.yml** | `crates/server/src/services/` | Server layout may differ; confirm and fix path |
| **crates/rust-daq/ast-grep-rules/rules/legacy-migration.yml** | `crates/hardware/src/` | `crates/hardware/src/` |
| **crates/rust-daq/ast-grep-rules/rules/enforce-daq-error.yml** | `crates/hardware/` | `crates/hardware/` |

### 2.7 CI Artifacts / Blueprint Paths

| Location | Issue |
|----------|--------|
| **.github/workflows/ci.yml** | `crates/server/blueprints/*.rbl` in upload artifacts | Should use `crates/server/blueprints/*.rbl` |

### 2.8 Driver/Plugin Crates (Phase 5 – Done)

**Phase 5** renamed `daq-driver-*` → `driver-*` and `daq-plugin-*` → `plugin-*`. These crates now use the short names.

| Kind | Crates |
|------|--------|
| **Drivers** | `driver-pvcam`, `driver-comedi`, `driver-mock`, `driver-thorlabs`, `driver-newport`, `driver-spectra-physics`, `driver-red-pitaya`, `driver-generic` |
| **Plugins** | `plugin-api`, `plugin-example` |

- **Workspace**: `Cargo.toml` members list `crates/driver-*`, `crates/plugin-*`.
- **Deps**: `hardware`, `drivers`, `scripting`, `ui`, `rust-daq`, `examples` depend on `driver-*`; `rust-daq` and examples use `plugin-api`.

**Obsolete:** Renaming `daq-driver-*` → e.g. `driver-pvcam`, `driver-comedi`, … and `daq-plugin-*` → `plugin-api`, `plugin-example` would be **new work** (e.g. a “Phase 5”): it touches many `Cargo.toml` deps, `use` statements, and CI/config references. Not covered by the current cleanup.

---

## 3. Verdict

- **Rust code**: `use` and deps were updated in **both #253 and #254**; **runtime paths** in `server.rs` / `rerun_sink.rs` and **scripts** still use `server` and wrong `-p` names (unchanged by either PR).
- **Config**: **release-please**, **deny.toml**, **CI**, **pre-commit**, **build scripts** reference old names/paths → **build, release, and tooling can fail or behave incorrectly**.
- **Docs**: Widespread `daq-*` paths and names; **not blockers** but confuse navigation and onboarding.

---

## 4. Recommended Cleanup (Priority Order)

1. **Config / scripts (immediate)**  
   - Fix **release-please-config** `extra-files`, **deny.toml** crate names, **devbox.json** `tests:core` (`-p common -p hardware -p server`), **regenerate_blueprints.sh** `BLUEPRINT_DIR`, **build-maitai.sh** / **build-lab.sh** `-p` to use `bin`.  
   - Fix **server** default blueprint path and error strings, **rerun_sink** docs, **server/blueprints/README**.

2. **CI / pre-commit**  
   - Replace `--exclude ui` / `--exclude server` with `--exclude ui` / `--exclude server`.  
   - Fix blueprint artifact path in **ci.yml** to `crates/server/blueprints/*.rbl`.

3. **Docs**  
   - Sweep README, CONTRIBUTING, CLAUDE, AGENTS, JULES, **docs/** and **.planning/** for `daq-{proto,pool,core,server,experiment,egui,bin}`; update to **protocol**, **pool**, **common**, **server**, **experiment**, **ui**, **bin** and current paths.

4. **ADR / planning**  
   - Update **adr-pvcam-pool-migration-results**, **CONVENTIONS**, and phase plans that use `daq_proto` / `daq_pool` in examples.

5. **Lockfiles**  
   - **crates/examples**: `cargo update -p protocol -p common` (or `cargo fetch`) and commit **Cargo.lock** so it no longer references `protocol` / `daq-core`.  
   - **crates/bin**: same for **bin**’s **Cargo.lock** re `daq-core`.

6. **Ast-grep rules**  
   - Update **best-practices**, **suggest-tracing**, **legacy-migration**, **enforce-daq-error** to use **ui** / **server** / **hardware** paths.

7. **Phase 5 – driver/plugin renames (done)**  
   - `daq-driver-*` → `driver-*`, `daq-plugin-*` → `plugin-*` completed. Ensure all docs reference `driver-*` / `plugin-*`.

---

## 5. Validation (Reproduced)

- `test -d crates/server/blueprints` → **false**; `crates/server/blueprints` exists. `regenerate_blueprints.sh` would fail.
- `cargo build -p bin` → **error: package ID specification \`bin\` did not match any packages**. Use `-p bin`.

---

## 6. References (file:line)

| Item | Location |
|------|----------|
| PR #253 | https://github.com/TheFermiSea/rust-daq/pull/253 |
| PR #254 | https://github.com/TheFermiSea/rust-daq/pull/254 |
| release-please extra-files | `release-please-config.json` L24–34 |
| deny.toml crate names | `deny.toml` L86, 106, 111–112 |
| CI exclude | `.github/workflows/ci.yml` L54, 59, 166, 243, 452, 481; `code-quality.yml` L82, 281, 298, 310; `coverage.yml` L44 |
| regenerate_blueprints | `scripts/regenerate_blueprints.sh` L7–8 |
| build scripts | `scripts/build-maitai.sh` L59; `scripts/build-lab.sh` L29 |
| devbox tests:core | `devbox.json` L15 |
| Server blueprint path | `crates/server/src/grpc/server.rs` L1334, 1349 |
| Rerun sink docs | `crates/server/src/rerun_sink.rs` L62, 70, 343, 348 |
| Blueprints README | `crates/server/blueprints/README.md` L7, 10, 29 |
| pool README | `crates/pool/README.md` L17, 39 |
| frame_pool comment | `crates/driver-pvcam/.../frame_pool.rs` L46 |
| examples lockfile | `crates/examples/Cargo.lock` (`protocol`, `daq-core`) |
| bin lockfile | `crates/bin/Cargo.lock` (`daq-core`) |

---

## 7. Next Steps

- Create follow-up **bd** tasks (or sub-tasks under an epic) for: (1) config/scripts, (2) CI/pre-commit, (3) docs sweep, (4) ADR/planning, (5) examples lockfile, (6) ast-grep rules.  
- Run **regenerate_blueprints.sh** and **build-maitai.sh** after edits to confirm no regressions.  
- Optionally add a **“Crate rename cleanup”** note to the bd-q7l1 epic or PR #253 / #254 descriptions and link this document.
