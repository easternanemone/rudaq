# crate: `daq-modules`

<!--
last-ingested: 2026-04-19
sources:
  - crates/daq-modules/Cargo.toml
  - crates/daq-modules/src/
see-also:
  - ./experiment.md
-->

**Role:** Experiment-module plugin system. Modular framework for
composing experiment workflows with runtime module assignment.

**Use case:** when a single Rhai script becomes unwieldy, factor
workflow steps into modules that can be combined at runtime.

**Dependents:** `server`, `experiment`, `bin`.

(Thin stub — expand on next ingest with concrete module examples and
the plugin-registration API.)
