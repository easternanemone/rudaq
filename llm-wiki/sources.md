# Raw Sources

<!--
last-ingested: 2026-04-19
sources: (self-referential; this page is the map)
see-also:
  - ./schema.md
-->

Pointers to every class of authoritative source the wiki ingests from.
When `llm-wiki/` is silent on a topic, check here first, then grep the code.

## In-repo — canonical human docs

- `docs/explanation/` — prose architecture (`architecture.md`, `newcomer-guide.md`, `plugin-schema.md`, `pvcam-integration-map.md`, `rerun-visualization.md`, `refactoring-plan-2026-04.md`).
- `docs/reference/` — tables and contracts (`inventory.md`, `driver-capability-matrix.md`, `device-metadata-contract.md`, `plugin-schema.md`, `grpc-api.md`, `feature-matrix.md`, `deprecation-plan.md`, `build-profiles.md`, `streaming-policy.md`, `ui-workflow-costs.md`, `pvcam-sdk.md`, `dover-motion-api.md`, `echelle-*.md`, `hardware-qualification-runner-plan.md`, `script-inventory.md`).
- `docs/how-to/` — procedures (`testing.md`, `hardware-setup.md`, `hardware-drivers.md`, `wasm-dom-interop.md`, `echelle-calibration-development.md`, storage format guides).
- `docs/tutorials/` — learning paths (demo mode, etc.).
- `docs/adr/` — Architecture Decision Records. Ingest one at a time.
- `docs/archive/` — historical; do not ingest as current truth.
- `docs/SUMMARY.md` — mdBook table of contents.

## In-repo — agent / LLM docs

- `CLAUDE.md` — Claude Code quickstart + non-negotiable rules.
- `AGENTS.md` — canonical agent policy (cross-agent).
- `GEMINI.md` — Gemini-specific guide (has known staleness, see `invariants.md`).
- `.claude/skills/` — skills (rust-skills, driver-plugin-builder, openspec tools, ms-rust, etc.).
- `.claude/handoffs/` — dated handoff notes (e.g. `2026-03-23-pvcam-epic.md`).
- `.claude/hooks/` — session-start / auto-commit / auto-push hook scripts.
- `.prompts/` — staged driver-plugins prompts (001–006).

## In-repo — code & config

- Workspace root `Cargo.toml` — authoritative list of 30 workspace members.
- `rust-toolchain.toml` — Rust channel pin (1.92.0).
- `crates/*/Cargo.toml` — per-crate metadata.
- `crates/*/src/lib.rs` and module docs — source of truth for APIs.
- `config/devices/*.toml` — driver-universal device manifests.
- `config/*.toml` — runtime daemon configs (`demo.toml`, `demo_mock_all.toml`, `maitai_universal.toml`, `config.v4.toml`, `feature_flags.toml`).
- `scripts/ops/` — build / env / CI helpers (`build-maitai.sh`, `env-check.sh`, `fast-check.sh`).
- `examples/*.rhai` — Rhai script examples.
- `.github/workflows/` — CI pipelines.

## Issue tracker

- `.beads/` — Dolt-backed beads database. Access via `bd` CLI. Not meant for direct reads.
- Each bead is a potential ingest source when closed with substantive notes.

## Git history & PRs

- `git log` on `main` — merged PRs are canonical ingest triggers.
- `HISTORY.md` — narrative high-level project history (21 KB; still to be distilled into concept pages).

## External — vendor SDKs

- PVCAM SDK (Photometrics) — `docs/reference/pvcam-sdk.md` + vendor manuals.
- Andor SDK3 — vendor docs + `andor-sdk3-sys`.
- Linux Comedi — kernel module + `comedi-sys`.
- Dover Motion MotionSynergyAPI — vendor docs + `dover-motion-sys`.
- Bluesky conceptual docs — <https://nsls-ii.github.io/bluesky/>.
- Rhai language — <https://rhai.rs>.

## Ingest priority order

When ingesting a new change, walk sources in this order:

1. The PR diff itself (title, description, touched files).
2. Any `docs/adr/NNN-*.md` introduced or updated.
3. `CLAUDE.md` / `AGENTS.md` / `invariants.md` — if the change alters a rule, update these first.
4. Relevant `concepts/`, `crates/`, `drivers/`, or `hardware/` pages.
5. `index.md` — only if new pages were added.
6. `log.md` — always, last.
