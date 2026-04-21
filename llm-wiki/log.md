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
