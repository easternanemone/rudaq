# Decision: Supported Role of `ui-slint` vs `egui`

**Status:** Accepted
**Date:** 2026-03-13
**Related work:** bd-pman.6.1 (`docs/reference/ui-workflow-costs.md`), bd-pman.6.2, `eval/slint-gui-comparison`

## 1) Decision context

The project currently has multiple UI surfaces. Contributors need a clear policy for where to invest implementation effort:

- What is production UI vs evaluation UI?
- Should new operator features go into `ui-slint`?
- Is a migration away from `egui` currently supported?

Without a decision, effort can fragment across parallel UI stacks and increase maintenance cost.

## 2) Evaluation evidence summary

From `docs/reference/ui-workflow-costs.md` (bd-pman.6.1):

- `crates/ui` (egui) is the primary UI with deep investment and operator coverage:
  - ~106 src files, ~60k LOC
  - 41 panels, 27 widgets
  - native + WASM from one codebase
- WASM egui path is operational and shared with native architecture (~7.7 MiB wasm artifact in current snapshot)
- `daq-rerun` is a specialized, feature-gated visualization workflow (not full UI parity)
- `crates/ui-slint` is much smaller and evaluation-oriented:
  - 6 src files, ~1.2k LOC
  - prototype panels only
- Historical benchmark context indicates strong Slint FPS potential (egui ~163 FPS vs Slint ~560 FPS in prior comparisons)

From `eval/slint-gui-comparison`:

- Branch includes explicit evaluation artifacts (gate assessment, feature-gap analysis, scoring matrix, benchmark reports)
- E2/E4/E5 outcomes are framed as **CONDITIONAL-GO**, not an unconditional migration recommendation
- Findings emphasize significant migration/boilerplate cost and docking/dynamic-panel risk despite promising performance

## 3) Decision

`ui-slint` is **supported as EXPERIMENTAL / EVALUATION ONLY**.

`crates/ui` (egui) remains the **primary and supported operator UI stack** for native and WASM workflows.

### Rationale

- The project has substantially more implementation and operational maturity in egui (roughly 50x code investment)
- egui already provides full operator-facing panel coverage and a proven shared native+WASM path
- Slint shows promising raw performance, but current evidence does not justify migration cost/risk for full parity
- The likely migration effort is large and uncertain relative to measured practical benefit in current workflows

## 4) Implications for contributors

### Invest time in

- New operator features, bug fixes, and UX improvements in `crates/ui` (egui)
- Keeping WASM parity and reliability in the existing egui architecture
- Targeted Rerun improvements only for specialized visualization use cases

### Do not invest (unless explicitly scoped)

- Porting production panels from egui to `ui-slint`
- Treating `ui-slint` as a production UI path
- Accepting feature-parity commitments in `ui-slint` without a new explicit decision

### When to revisit this decision

Re-open only when at least one of these is true:

1. A time-boxed Slint pilot demonstrates end-to-end parity for a representative high-value panel set (not only microbenchmarks)
2. Docking/dynamic panel architecture risk is resolved with stable APIs and validated UX
3. Measured production outcomes (operator latency, power/thermals, maintainability) show material benefit over egui

## 5) Follow-up actions

1. Keep `ui-slint` labeled experimental in relevant docs/README locations
2. Require explicit issue/epic scope before any non-trivial `ui-slint` feature work
3. Treat Slint work as evaluation deliverables (benchmarks, risk reduction, migration probes), not production commitments
4. Reassess in a future decision record if revisit criteria are met
