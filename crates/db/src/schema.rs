//! Schema definitions for the DAQ database.
//!
//! The database models these record types:
//!
//! - **`driver`** — Static driver definitions (imported from TOML plugin files).
//! - **`instrument`** — Physical device instances, each linked to a driver.
//! - **`experiment_plan`** — Named experiment definitions with pre-translated commands.
//! - **`run_record`** — Execution history linked to plans.
//! - **`device_feature`** — Discovered parameter metadata (types, ranges, enum values).
//! - **`device_runtime_state`** — Last-known device parameter values for restart recovery.
//! - **`device_lifecycle_event`** — State transition log for post-mortem analysis.

/// Current schema version. Bump this when adding migrations.
pub const SCHEMA_VERSION: u32 = 9;
