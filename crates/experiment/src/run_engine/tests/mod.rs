//! RunEngine test suite.
//!
//! Tests are organized by component to match the composed architecture:
//!
//! - `queue` — plan queue operations (TaskQueue)
//! - `watchdog` — orphan-plan detection (WatchdogManager)
//! - `executor` — plan execution, document emission, state machine
//! - `readiness` — calibration freshness and config-match gating
//! - `adaptive` — adaptive scan feedback and condition evaluation
//!
//! Shared mock types live in `helpers`.

mod helpers;

mod adaptive;
mod command_dispatch;
mod executor;
mod queue;
mod readiness;
mod state_broadcast;
mod watchdog;
