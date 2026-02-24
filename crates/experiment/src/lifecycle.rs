//! Run lifecycle hooks for cross-crate integration.
//!
//! The `RunLifecycleHook` trait allows external crates (e.g., `bin`) to observe
//! and react to run lifecycle events without introducing a dependency from
//! `experiment` to `db`. The bin crate implements this trait with `DaqDb` to
//! persist heartbeat timestamps.

use async_trait::async_trait;

/// Hook for run lifecycle events.
///
/// Implementors receive periodic callbacks during plan execution, enabling
/// heartbeat monitoring, metrics emission, or other cross-cutting concerns.
///
/// # Architecture
///
/// This trait exists so that `experiment` (which owns `RunEngine`) can notify
/// the persistence layer without depending on `db`. The `bin` crate wires
/// the two together at startup.
#[async_trait]
pub trait RunLifecycleHook: Send + Sync {
    /// Called periodically (~every 10s) during plan execution.
    ///
    /// Implementations should update the `heartbeat_at` timestamp in the
    /// run record so the reconciler can detect stale (crashed) runs.
    async fn on_heartbeat(&self, run_uid: &str) -> anyhow::Result<()>;
}
