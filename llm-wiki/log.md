# Wiki Log

Append-only. One dated entry per ingest, significant query, or lint pass. See
[`schema.md`](./schema.md) for the format.

Entries are newest-on-bottom for append-friendliness; scroll to the end for
recent activity.

---

## 2026-04-19 — Initial seed (ingest)

**Source:** N/A (greenfield scaffold). Initialized the wiki in response to
"Did we ever implement an LLM Wiki?" — we had not.

**Sources consumed to seed pages:**

- `CLAUDE.md` — invariants, workflows, hardware machines, non-negotiable rules.
- `GEMINI.md` — parallel agent guide (note: lists 26 crates and Rust 1.75+ MSRV — **stale**, actual is 29 crates + Rust 1.92.0; cross-referenced in `invariants.md`).
- `docs/explanation/architecture.md` — data pipeline, Mullet Strategy, RunEngine composition, frame streaming, persistence tiers, legacy migration.
- `docs/explanation/newcomer-guide.md` — capability learning order, Plan/RunEngine, Parameter walkthrough, GUI GenericDevicePanel.
- `docs/explanation/plugin-schema.md` — driver-universal schema v3 details.
- `docs/reference/driver-capability-matrix.md` — per-factory capability declarations, feature gates, deployment targets.
- `docs/reference/inventory.md` — canonical binaries, config paths, crate layout, workspace support levels.
- `README.md`, `HISTORY.md` — project-level context.
- Workspace `Cargo.toml` + per-crate `Cargo.toml` headers — crate names, descriptions, versions.
- `rust-toolchain.toml` — 1.92.0 pin.
- `config/devices/*.toml` — 16 device manifests.

**Pages touched:** all of them (initial creation).

**Known gaps intentionally left for follow-up ingest:**

- Crate pages are stubs — deeper API/type detail to be ingested from `cargo doc` output per crate.
- ADR ingest — `docs/adr/*.md` listed in `sources.md` but not yet individually ingested into concept pages.
- Hardware handoffs — `.claude/handoffs/2026-03-23-pvcam-epic.md` referenced but not ingested into `drivers/pvcam.md`.
- `HISTORY.md` is 21 KB — narrative project history, not yet distilled into architecture.md / concepts/*.

Relevant bead: file follow-up issue `seed llm-wiki with per-crate cargo-doc ingest` post-merge.

---

## 2026-04-19 — Doc reconciliation pass (lint → fix)

**Trigger:** user request: "I suspect a lot of the docs are currently out of date. While creating the Wiki, be sure to update any documentation such that it reflects the actual state of the repo, not legacy documentation."

**Findings (verified against code, not trusted from other docs):**

- `docs/explanation/architecture.md:20` — claimed 26 crates; actual is **29** (workspace `Cargo.toml` members). Fixed.
- `docs/explanation/architecture.md:62` — described `db` as "Embedded SurrealDB control-plane database". Initial audit confirmed SQLite was present but incorrectly assumed SurrealDB still existed as a feature-gated variant. The later deep audit below corrected this: SQLite is the only DB backend.
- `docs/explanation/architecture.md:163` — listed `PulseGenerator` capability. **No such trait exists** in `crates/common-traits/src/capabilities.rs` (grep returned 0 matches in the code tree). Removed from the capability list with an explicit note.
- `docs/explanation/architecture.md` capability list — **missing** 6 traits that exist in code: `StateRefreshable`, `CounterConfigurable`, `RangeIntrospectable`, `DeviceIntrospection`, `ReadableWithMetadata`, `SpectrumReadable`. Added.
- `GEMINI.md:7` — "edition 2021, MSRV 1.75+". Actual: **edition 2024**, pinned channel **1.92.0** (`rust-toolchain.toml`). Fixed.
- `GEMINI.md:17` — "26 crates". Actual: **29**. Fixed.
- `GEMINI.md:76` — described `AGENTS.md` as canonical agent policy without noting it's gitignored. Fixed.
- `.gitignore:153` contains `AGENTS.md` — file is **intentionally** not tracked in git; auto-injected at session start. This reframes the "dangling AGENTS.md reference" in `CLAUDE.md:3` / `CLAUDE.md:92` / `CLAUDE.md:116` / `GEMINI.md:76` as correct-by-design, not broken. Documented in `invariants.md` and `schema.md`. **No AGENTS.md file was created** — would conflict with the gitignore rule.
- `CLAUDE.md:1-3` — added explicit pointer to `llm-wiki/index.md` and `llm-wiki/schema.md` near the top.
- `GEMINI.md` §Key Files & Directories — added `llm-wiki/` pointer; clarified AGENTS.md gitignore status.

**Wiki self-corrections from this pass:**

- `concepts/capability-traits.md` — said "24 traits" and located them at `crates/common/src/capabilities.rs`. Real count is **30** at `crates/common-traits/src/capabilities.rs` (the `common` path no longer exists — `common/src/` does not contain `capabilities.rs`). Fixed both count and path; added the 6 missing traits to the table.
- `glossary.md` — "24 in total" → "30 in total" with authoritative path.

**Not fixed this pass (out of scope; bead-worthy):**

- `docs/reference/driver-capability-matrix.md` regeneration — dated 2026-03-13; needs a re-run now that 6 traits are newly enumerated and `PulseGenerator` is gone.
- `HISTORY.md` distillation into concept pages.
- Per-ADR ingest (31 ADRs under `docs/adr/`).
- Deep per-crate API ingestion from `cargo doc` output.

**Pages touched:** `docs/explanation/architecture.md`, `GEMINI.md`, `CLAUDE.md`, `llm-wiki/concepts/capability-traits.md`, `llm-wiki/glossary.md`, `llm-wiki/invariants.md`, `llm-wiki/schema.md`, `llm-wiki/log.md` (this entry).

---

## 2026-04-19 — Deep audit pass (user pushed back on staleness)

**Trigger:** user: "Are you certain that this implementation of LLM Wiki is comprehensive? Did you validate the original documentation is up to date with the current codebase? Perform a thorough review and fix all inconsistencies."

Validated 25 specific claims from the first ingest pass against the actual code (not against other docs). Findings below; all fixed inline.

**Correct as claimed (no change needed):**

- `PvcamFactory`, `AndorCameraFactory`, `AndorSpectrographFactory`, `ComediAnalogInputFactory`/`…Output`/`…DigitalIO`/`…Counter`, all 6 mock factories: verified by name + location.
- Feature-flag names `pvcam` / `pvcam_sdk` / `pvcam_hardware`, `andor` / `andor_hardware`, `comedi` / `comedi_hardware`: verified in `crates/driver-registry/Cargo.toml`.
- `driver-dover-motion` is **not** wired into `driver-registry` (grep confirms no reference). Still experimental.
- `DocumentSink` trait exists in `crates/storage/src/document_sink.rs:48` (not `DocumentConsumer` — older doc wording).
- `RunEngine` is a directory module at `crates/experiment/src/run_engine/` with `task_queue.rs`, `watchdog.rs`, plus `command_dispatch.rs`, `context.rs`, `documents.rs`, `executor.rs`, `mod.rs`, `readiness.rs`, `state_machine.rs`, `tests/` (more submodules than originally documented).
- `GrpcStreamObserver`, `ObserverFramePacket`, `StreamLimiter` exist at `crates/server/src/grpc/hardware_service/streaming.rs`.
- `compress_frame_into` / `decompress_frame_into` exist in `crates/protocol/src/compression.rs`.
- `HardwareWatchdog` exists at `crates/common/src/health/watchdog.rs:105`.
- Rhai op limit **is** 10 000 — `RhaiEngine::with_limit(10_000)` in `crates/scripting/src/rhai_engine.rs`.
- 30 capability traits in `crates/common-traits/src/capabilities.rs` (re-verified).
- driver-universal schema v3 (`EXPECTED_SCHEMA_VERSION: u32 = 3` in `driver-universal/src/config/parse.rs`).
- `bd-47p2` bead ID is real — appears in `capabilities.rs`, `driver-universal/src/*.rs`, `hardware/src/registry/*.rs`.
- `bd-2a2ne` is the bead ID for the SQLite-only migration (`crates/db/Cargo.toml:20`, `crates/db/src/sqlite_backend.rs:1`).
- `pvcam-sys` is nested at `crates/driver-pvcam/pvcam-sys/` and listed in root `Cargo.toml:14`.
- ADR-004, ADR-014, ADR-015 all exist (`docs/adr/004-panic-safety.md`, `014-frame-streaming-buffer-reuse.md`, `015-hybrid-persistence-architecture.md`).
- Config files exist: `config/demo.toml`, `config/demo_mock_all.toml`, `config/maitai_universal.toml`, `config/feature_flags.toml`, `config/config.v4.toml`.

**Wrong as originally claimed (fixed in this pass):**

- **SurrealDB is entirely removed from the codebase.** `db` is SQLite-only (`bd-2a2ne`, `rusqlite` + `tokio-rusqlite`). There is **no** `db-surreal` feature anywhere in the workspace. My previous architecture.md "fix" still described SurrealDB as feature-gated — that was still wrong. Rewrote:
  - `docs/explanation/architecture.md` line 62 (`db` crate description) — fully SQLite now.
  - `docs/explanation/architecture.md` §Persistence & Data Architecture (tier 2 → SQLite; removed SurrealDB tier-2 claim and the reconciliation-loop narrative that depended on LIVE SELECT).
  - `llm-wiki/crates/db.md` — removed "Optional SurrealDB variant" paragraph.
  - `llm-wiki/crates/bin.md` — removed `db-surreal` from feature-flag list.
  - `llm-wiki/architecture.md` — tier-2 + services row fixed.
- **`SafetyHeartbeat` is not a Rust type.** The safety-heartbeat mechanism is a `safety_heartbeat_task` module in `crates/bin/src/` (entry `spawn_heartbeat`), driven by `HeartbeatConfig` (`crates/hardware/src/registry/types.rs:280`) loaded from the `[safety_heartbeat]` TOML stanza. Fixed:
  - `docs/explanation/architecture.md` §Safety Architecture Layer 1.
  - `llm-wiki/architecture.md` §Safety.
  - `llm-wiki/glossary.md` — "SafetyHeartbeat" entry renamed to "Safety heartbeat" with correct file pointers.
  - `llm-wiki/hardware/maitai.md`, `llm-wiki/hardware/leabs-dev.md`, `llm-wiki/drivers/comedi.md`.
- **`create_canonical_mock_registry` takes a `workspace_root: &Path` argument.** Previously shown with no args. Fixed:
  - `llm-wiki/concepts/mock-registry.md`, `llm-wiki/concepts/driver-registry.md`.
- **`/dev/shm/ring.buf` is not a hardcoded path** in the code. The ring buffer accepts a caller-supplied path; the "/dev/shm/ring.buf" claim was from the human architecture doc and never reflected source. Fixed `llm-wiki/concepts/ring-buffer.md`.
- **3 sys crates are still `edition = "2021"`**: `andor-sdk3-sys`, `comedi-sys`, `pvcam-sys`. The rest of the workspace is 2024. There is no workspace-level `edition` field. Fixed `llm-wiki/invariants.md` and the three sys-crate pages.
- **Legacy Migration Status** in `docs/explanation/architecture.md` was factually wrong on several rows:
  - `GenericDriver::new` — no such method; only `new_serial` / `new_tcp` / `new_mock` exist.
  - `ScanServiceImpl` — already removed (0 grep matches).
  - `take_frame_receiver` / `subscribe_frames` — already removed (0 grep matches).
  - `PvcamDriver::new` / `PvcamDriver::from_config` — neither exists; the current constructor is `PvcamDriver::new_async(camera_name)` at `crates/driver-pvcam/src/lib.rs:353`.
  - `hardware::registry::create_mock_registry` **is** `#[deprecated]` (pointer to `create_canonical_mock_registry`) but was missing from the table.

  Rewrote the table into two sections: "Still in tree and `#[deprecated]`" (3 items) and "Already removed" (historical). Fixed downstream wiki: `crates/driver-pvcam.md`, `drivers/pvcam.md`, `crates/server.md`, `crates/experiment.md`, `crates/hardware.md`, `crates/storage.md`.

- **`leabs-dev` config path** — the canonical config is `config/leabs_hardware.toml`; my earlier "TBD" note was wrong. Fixed `llm-wiki/hardware/leabs-dev.md`.

- **`scripting` crate types** — the bridge is `ScriptEngine` trait + `RhaiEngine` default backend (`crates/scripting/src/rhai_engine.rs`). My earlier summary was too vague; added explicit type names and the op-limit citation.

**Left explicitly as open follow-ups (beads recommended):**

- `docs/how-to/surrealdb-integration.md` is retained on disk but describes a removed backend — should be moved to `docs/archive/` or deleted.
- `docs/reference/driver-capability-matrix.md` (dated 2026-03-13) needs regeneration given the 6 newly-documented capability traits and `PulseGenerator` removal.
- Per-crate `cargo doc` ingest to replace stubs with real API surfaces.
- `HISTORY.md` (21 KB narrative) hasn't been distilled.

**Pages touched this pass:** `docs/explanation/architecture.md`, `llm-wiki/architecture.md`, `llm-wiki/crates/db.md`, `llm-wiki/crates/bin.md`, `llm-wiki/crates/driver-pvcam.md`, `llm-wiki/crates/server.md`, `llm-wiki/crates/experiment.md`, `llm-wiki/crates/hardware.md`, `llm-wiki/crates/storage.md`, `llm-wiki/crates/scripting.md`, `llm-wiki/crates/pvcam-sys.md`, `llm-wiki/crates/comedi-sys.md`, `llm-wiki/crates/andor-sdk3-sys.md`, `llm-wiki/concepts/mock-registry.md`, `llm-wiki/concepts/driver-registry.md`, `llm-wiki/concepts/ring-buffer.md`, `llm-wiki/invariants.md`, `llm-wiki/glossary.md`, `llm-wiki/hardware/maitai.md`, `llm-wiki/hardware/leabs-dev.md`, `llm-wiki/drivers/comedi.md`, `llm-wiki/drivers/pvcam.md`, `llm-wiki/log.md` (this entry).

---

## 2026-04-21 — PR #622 remediation against current `main`

**Trigger:** user reported that PR #622 did not sufficiently compare docs to the actual codebase.

**Verification context:** rebased the PR branch onto current `origin/main` before editing, then checked claims against `Cargo.toml`, crate feature sections, `.github/workflows/feature-matrix.yml`, `crates/db`, `crates/server`, `crates/bin`, `crates/common-traits`, and `crates/driver-registry`.

**Additional fixes:**

- `README.md` still said 26 crates, SurrealDB control plane, and `driver-dover-motion` as built-in. Updated to 30 workspace members, SQLite control plane, and Dover as experimental/not registry-wired.
- `docs/reference/feature-matrix.md` and `docs/reference/build-profiles.md` still documented removed `db-surreal-*`, `kv-*`, and RocksDB profiles. Rewrote them from current crate features and CI matrix.
- `docs/how-to/surrealdb-integration.md` still read as a current operator guide. Replaced it with a SQLite control-plane note and historical SurrealDB context.
- `docs/how-to/migration-rollback-toolkit.md`, `operations.md`, and lab DB signoff guides still described SurrealDB/RocksDB procedures. Updated to SQLite and the plain `db` feature.
- `docs/explanation/newcomer-guide.md` still listed 23 capability traits, `PulseGenerator`, and `DocumentConsumer`. Updated to 30 traits, removed `PulseGenerator`, and corrected `DocumentSink`.
- `docs/reference/inventory.md`, `grpc-api.md`, ADR indexes, and LLM wiki pages still had stale DB/feature/capability details. Updated high-impact current references and marked historical ADRs as superseded where appropriate.

---

## 2026-04-21 — Agent entrypoint wiring for LLM wiki

**Trigger:** user asked whether the repo was set up to actually use the LLM wiki and requested updates to `AGENTS.md`, `CLAUDE.md`, and related files.

**Changes:**

- Added tracked `AGENTS.md` as the canonical repo-level agent policy and removed the `.gitignore` rule that suppressed it.
- Made `llm-wiki/index.md`, `llm-wiki/invariants.md`, and relevant linked pages mandatory startup context for non-trivial agent work.
- Clarified in `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and `llms.txt` that the wiki orients agents but source files, `Cargo.toml`, config, and workflow YAML remain the final authority.
- Updated `llm-wiki/schema.md` so wiki query workflow includes source verification and wiki edits still follow branch/worktree rules.
- Fixed stale `llm-wiki/index.md` counts for 30 capability traits and 30 crate pages.
- Added pointers from `README.md` and `docs/README.md` so agent-oriented contributors discover the wiki entrypoint.

---

## 2026-04-21 — bd-lj4g4 lamp-agnostic blaze API

**Trigger:** pointing the echelle merged-spectrum regression test at the post-slit-swap halogen-only flat (bd-cpph3 finding: halogen beats DH3P on every calibration metric) returned `None` from `fit_dh3p_continuum` because the API name and docstring hardcoded a DH3P/bimodal shape narrative, and the regression test extracted from the raw flat rather than the scatter-subtracted one.

**Changes:**

- Renamed `echelle::blaze::{Dh3pContinuum, Dh3pContinuumConfig, fit_dh3p_continuum, compute_blaze_from_dh3p_flat}` → `{LampContinuum, LampContinuumConfig, fit_lamp_continuum, compute_blaze_from_flat}`. Algorithm unchanged — uniform knots + median window + positive sigma-clip were already lamp-agnostic; the DH3P-ness lived in names and prose.
- Updated module-level doc-comment to describe three lamp cases (DH3P bimodal, D2-alone with Balmer emission, halogen-alone monotonic) and document that no per-lamp tuning is required.
- `echelle_merged_spectrum_regression` now extracts from the scatter-subtracted flat (calling `subtract_scattered_light` explicitly), points at the halogen fixture, skips if fixtures are missing (consistent with the Apr 21 `/debug/**/*.tiff` untrack), and stores the golden under `testdata/echelle/reference_merged_spectrum_halogen.hdf5`. Illumination-mask multiplier lowered from 10× to 1.5× because halogen's post-scatter dynamic range is ~2–3× vs DH3P's ~10×.

**Pages touched:** none directly — `crates/echelle.md` already describes echelle as a domain crate; the blaze rename is internal API detail that doesn't change the crate's role. No wiki text referenced the DH3P names.

---

## 2026-04-22 — Post-review: baseline_hgar_doe_matrix fixture skip

**Trigger:** CI on PR #640 and #641 failed on `echelle_hgar_doe_baseline::baseline_hgar_doe_matrix`, which hard-opens `debug/phase5/flats/dh3p_flat_5s_g2000_acc10.tiff` — gitignored since the Apr 21 `/debug/**/*.tiff` cleanup and thus absent on ephemeral CI runners. CodeRabbit reviews on both PRs surfaced no actionable inline comments (Free-plan summaries only).

**Fix:** same skip-if-missing pattern used by `echelle_merged_spectrum_regression_halogen_flat` applied to `baseline_hgar_doe_matrix`. Returns `Ok(())` with a skip notice when the DH3P flat is absent; hardware runners that retain the capture still exercise the DoE baseline. CI stays green without the fixture.

**Pages touched:** none. The test-skip pattern is internal to the integration test.

---

## 2026-04-22 — Docs staleness sweep (broader pass)

**Trigger:** user: "The docs staleness sweep is very important, proceed with that."

**Scope:** systematic grep of `docs/` for the stale patterns the previous audit pass enumerated. Much of the tree was already clean — `docs/reference/feature-matrix.md`, `docs/reference/build-profiles.md`, `docs/how-to/migration-rollback-toolkit.md`, `docs/how-to/surrealdb-integration.md`, `docs/adr/015-hybrid-persistence-architecture.md`, `docs/adr/runtime-driver-policy.md`, `docs/explanation/newcomer-guide.md`, and `docs/SUMMARY.md` / `docs/README.md` / `docs/adr/README.md` already carry correct SQLite language, 30-trait counts, or historical banners. The `scripts/hygiene/check-llm-wiki-drift.sh` guard also passes.

**Remaining stale items fixed:**

- `docs/how-to/pvcam-setup.md:134` — `PvcamDriver::new()` → `PvcamDriver::new_async()`. One-line fix.
- `docs/reference/driver-capability-matrix.md` — bumped generation date (2026-03-13 → 2026-04-22), added a changelog block naming the 6 capability traits that exist in `crates/common-traits/src/capabilities.rs` but were absent when the matrix was last generated (`StateRefreshable`, `CounterConfigurable`, `RangeIntrospectable`, `DeviceIntrospection`, `ReadableWithMetadata`, `SpectrumReadable`), explicitly flagged `PulseGenerator` as fictional, and noted the `mock_only` feature no longer exists / `full` is now an alias for `all_hardware`. Coverage-summary and per-factory tables left as-is (still accurate; new traits not yet mapped there).
- `docs/reference/deprecation-plan.md` — three sections (§2.3 `ScanServiceImpl`, §2.6 `GenericDriver::new()`, §2.7 `PvcamDriver::new()`) were citing line numbers in source files that no longer contain the named items (verified: `scan_service.rs` deleted; the cited line numbers now hold unrelated field declarations). Marked each section **Status: Removed**, retained the rationale for history, reshuffled the summary table from "Low, v1.0" to "**Done**: shipped" for those three, and corrected the `ScanService` row to clarify only the proto layer remains. Bumped "last audited" to 2026-04-22.
- `docs/how-to/hardware-drivers.md` — the "Reference Implementations" section ships code snippets that call `Ell14Driver::new_async()`, `Ell14Driver::new()`, `Ell14Bus::open()`, and `Newport1830CDriver::new_async()`. None of those types exist in the tree anymore; the devices moved to `driver-universal` TOML manifests. The pattern being taught (transport sharing, async-validating constructors) is still useful. Added "HISTORICAL" callouts inside each snippet so a newcomer does not try to compile the dead API, and rewrote the surrounding prose to make it clear the native-driver examples are reference patterns, not live code. Pointer to `config/devices/ell14.toml` / `newport_1830c.toml` retained.
- `docs/adr/004-panic-safety.md` — added a "Naming note" callout near the top explaining that `SafetyHeartbeat` is used as shorthand throughout but there is no `struct SafetyHeartbeat` in the code; the mechanism is `crates/bin/src/safety_heartbeat_task.rs::spawn_heartbeat` + `HeartbeatConfig`. Body of the ADR preserved.
- `docs/adr/016-capability-arc-dyn-pattern.md` — added a "Count note" banner that the "24 capability traits" figure is the snapshot at acceptance; the current count is 30, pattern unchanged.
- `docs/adr/017-error-type-pragmatization.md` — same treatment as ADR-016 for its "24 capability trait methods" figure.

**Deliberately left as-is:**

- `docs/how-to/surrealdb-integration.md` — retained on disk as a stale-link catcher with a "Current status" banner already at the top. Moving to `docs/archive/` would break inbound links from `docs/README.md`, `docs/SUMMARY.md`, `docs/adr/README.md`, and the architecture prose without benefit.
- `docs/adr/010-pvcam-pool-migration-results.md` / `docs/archive/test-suite-overhaul.md` — historical artifacts, no banner needed.
- `docs/explanation/refactoring-plan-2026-04.md` — references `subscribe_frames` deprecation; that trait method still exists in `common-traits/src/capabilities.rs:532` with `#[deprecated]`, so the plan is still accurate.

**Verification:** `bash scripts/hygiene/check-llm-wiki-drift.sh` passes. Final grep for remaining `PvcamDriver::new(` / `PvcamDriver::from_config` / `DocumentConsumer` across `docs/` returns only deliberate historical mentions (deprecation plan status row, architecture.md's own audit callout).

**Pages touched:** `docs/how-to/pvcam-setup.md`, `docs/how-to/hardware-drivers.md`, `docs/reference/driver-capability-matrix.md`, `docs/reference/deprecation-plan.md`, `docs/adr/004-panic-safety.md`, `docs/adr/016-capability-arc-dyn-pattern.md`, `docs/adr/017-error-type-pragmatization.md`, `llm-wiki/log.md` (this entry).

**Still open (not blocking):**

- Regenerate `docs/reference/driver-capability-matrix.md` *properly* by introspecting `DriverFactory::capabilities()` for every registered factory and expanding the Coverage Summary to cover all 30 traits. The date bump + changelog in this pass is a stop-gap.
- Potential sweep of the 17+ ADRs in `docs/adr/` for other out-of-date specifics; not triggered by the known stale-pattern list.
- `HISTORY.md` distillation into concept pages (still open from earlier entries).

---

## 2026-04-23 — H1 tutorial manifest + write-a-device-manifest.md (ingest)

**Source:** jcb4x Phase 3 H1 (task #13). PR `feat/jcb4x-h1-tutorial-docs`.

**What landed:**

- `config/devices/tutorial_device_example.toml` — fictitious ACME DPS-3010 bench DC power supply. Exercises every major v4 manifest feature (format strings, variants, transforms, parameter metadata, `evalexpr` conversions, capabilities, inline `[ui.control_panel]`) in one file and is referenced throughout the new how-to.
- `docs/how-to/write-a-device-manifest.md` — v4-focused walkthrough: transport, commands, response parsing (format / variants / transforms and when to reach for each), parameters, conversions, capabilities, UI, plus `manifest-check` / `migrate-v3` / planned `manifest-wizard` usage. Cross-links to `device-config.md` (v3 background), `CONFIG_README.md` (UI section catalogue), and the working manifests that best illustrate each pattern.
- `docs/SUMMARY.md` — mdBook TOC entry added under Hardware Drivers beside the existing `device-config.md` (schema-v3 deep-dive).

**Deliberately NOT touched:**

- `docs/how-to/device-config.md` left intact. It documents v3 mechanics and the v2 → v3 migration in detail; the new doc layers v4 patterns on top and cross-links instead of duplicating.
- `llm-wiki/crates/driver-universal.md` — no update needed for this PR; the crate's surface area has not shifted, only its documentation.

**Pages touched:** `config/devices/tutorial_device_example.toml` (new), `docs/how-to/write-a-device-manifest.md` (new), `docs/SUMMARY.md`, `llm-wiki/log.md` (this entry).

**Verification:** `manifest-check config/devices/tutorial_device_example.toml` → `OK  11 commands, 5 responses, 2 parameters`. `cargo nextest run -p driver-universal` → 292/292.
