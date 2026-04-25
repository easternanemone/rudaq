# ADR-018: v4 manifest-first universal driver UI

## Status

Accepted; implementation in progress. Phase 1 manifest-UX work (F1, F2, F6) is
merged to main; Phase 1 tooling (F3c wizard, F3d migrate-v3), the manifest
migration (F4), and the emulator regex removal (F5) are PR'd. Phase 2 schema
(G1) is PR'd; synthesis (G2a/G2b) is PR'd; G2c wiring, G3 layout slots, and G4
panel retirement remain. Several "Related artifacts" below are introduced by
sibling PRs in the same program — they will resolve when those PRs merge.

## Context

Through v3 the manifest-first universal driver architecture solved _protocol_
authoring — an undergrad could describe a serial or TCP device in TOML and
`driver-universal` would expose Movable/Readable/SetParameter capabilities over
gRPC without a single line of Rust. But the UI layer still required a
hand-written egui panel per device, which defeated the undergrad-accessible
promise for the most visible part of the system. As of the 2026-04-17 deep
code review, hand-written universal-driver panels had become the long pole in
onboarding new devices.

The v3 response-parsing story was also a problem in its own right. It layered
four parsing tiers (SCPI auto / format / transform / regex) and defaulted to
regex for anything non-trivial, forcing authors through PCRE to express even
simple "PREFIX: value" responses. The review called this the "regex cliff."

This ADR records the v4 architectural direction that lifts both problems.

## Decision

Three coordinated changes make the whole manifest-first stack undergrad-accessible:

1. **Response-parsing v4** (Phase 1, F1–F2, F5). The primary parsing concept
   becomes format templates — literals interleaved with `{field:type}`
   placeholders — and named `variants = [...]` lists for devices that reply
   with multiple fixed shapes (ELL14 33-byte vs 36-byte `device_info`). Regex
   is demoted to a `[responses.X.advanced]` escape hatch that emits a
   deprecation warning at load time. Transform ops add `map`, `match_one_of`,
   and `format` so common prefix-strip / enum-validate / field-extract tasks
   no longer require PCRE. The emulator synthesizes responses from the same
   variants, eliminating the regex back-path in `emulator/response_gen.rs`.

2. **Manifest-authoring tooling** (Phase 1, F3, F4, F6). Four CLI binaries
   ship in `driver-universal`:
   - `manifest-check` validates a TOML with "did you mean?" hints
   - `export-manifest-schema` emits JSON Schema so VS Code and other
     editors get schema-aware autocomplete and inline errors
   - `migrate-v3` converts v3 manifests to v4 format with conservative
     auto-rewrites and explicit TODOs for ambiguous cases
   - `manifest-wizard` walks an author through a new device interactively
   
   The curated evalexpr palette (F6) exposes named conversion ops
   (`to_pulses`, `clamp`, `scale`, `offset`, unit conversions) so formula
   fields read like arithmetic, not like a DSL.

3. **Config-driven UI** (Phase 2, G1–G4). `[commands.X.ui]` and
   `[parameters.X.ui]` tables let authors declare widget shape, layout slot,
   label, and unit next to the command they control. A synthesizer converts
   the validated manifest into a `ControlPanelConfig` that the existing
   `ConfigDrivenPanel` renderer consumes. Universal-driver-eligible panels
   (stage, rotator, power_meter, generic, MaiTai) are then deleted. Native
   SDK panels (Andor, Shamrock, Dover, PVCAM, Comedi) explicitly stay
   bespoke because they expose capabilities that don't map to a generic
   widget set.

## Consequences

### Positive

- **Undergrad acceptance criterion.** Once Phase 2 lands, a new SCPI device
  goes from manifest to working panel on the daemon in under 15 minutes —
  the program's headline target. The H1 walkthrough already includes a DC
  power supply tutorial that exercises every major v4 feature in under 150
  lines of TOML, so the manifest half is verifiable today; the synthesized
  panel half ships with G2c.
- **Fewer code paths for the same feature.** Each universal-driver panel
  deleted removes ~200–500 lines of near-duplicate egui glue.
- **Schema is load-bearing.** `export-manifest-schema` means the IDE
  surfaces schema violations as you type rather than at `manifest-check` or
  device-start time.
- **Emulator stays honest.** Round-trip tests assert that every format
  variant is invertible, so the emulator can't drift out of sync with the
  parser.

### Negative

- **Bespoke panels remain for native SDK devices.** The config-driven path
  deliberately does not try to cover acquisition-heavy or vendor-shaped
  workflows. This keeps two UI code paths alive indefinitely.
- **`driver-universal` ⇄ `hardware::config::schema` boundary.** The
  synthesizer needs access to `ControlPanelConfig`, but `driver-universal`
  historically didn't depend on `hardware`. G2 introduces either a new
  dependency edge or an intermediate IR in `driver-universal` that the UI
  crate translates. See G2 PR for the final decision.
- **Migration debt.** The regex path is demoted but not removed; a handful
  of manifests (maitai.toml, siglent_sdg1025.toml `advanced` responses)
  still rely on it. H2 removes the v2 loader only after F4's migration PR
  lands; full regex removal is deferred.

### Neutral

- **One-file-per-device becomes the norm.** Protocol + UI live together in
  the same TOML. This trades discoverability ("where is this widget
  defined?") for colocation ("everything about this device is here").

## Scope boundaries

- **In scope**: driver-universal manifest schema, its tooling, emulator
  parity with the parser, config-driven UI synthesis, retirement of
  universal-driver-eligible panels.
- **Out of scope**: native SDK driver panel redesign (PVCAM, Andor, Dover,
  Comedi), acquisition data pipeline, gRPC protocol.

## Related artifacts

Already on main:

- `docs/explanation/architecture.md` — updated by this change
- `crates/driver-universal/src/config/{raw,validated,parse}.rs` — v4 schema (F1, F2)
- `crates/driver-universal/src/bin/{manifest_check,export_manifest_schema}.rs` — authoring tools (F3a, F3b)

Pending merge in sibling PRs (links resolve once merged):

- `docs/how-to/write-a-device-manifest.md` — undergrad walkthrough (H1, PR #656)
- `docs/explanation/v4-config-driven-ui-plan.md` — implementation plan (PR #661)
- `llm-wiki/drivers.md` refresh — agent-facing reference (Task #18)
- `crates/driver-universal/src/bin/migrate_v3.rs` — F3d migrate tool (PR #649)
- `crates/driver-universal/src/bin/manifest_wizard.rs` — F3c interactive wizard (PR #658)
- `config/devices/tutorial_device_example.toml` — reference manifest (H1, PR #656)

## Beads program

- Epic: `bd-jcb4x`
- Sub-epic F (Phase 1 manifest UX): `bd-jcb4x.1`
- Sub-epic G (Phase 2 config-driven UI): `bd-jcb4x.2`, depends on
  `bd-1xi2p.8` (sub-epic D, UI shell simplification)
- Sub-epic H (Phase 3 polish): `bd-jcb4x.3`
