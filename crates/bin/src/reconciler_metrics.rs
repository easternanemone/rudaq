//! Prometheus metrics for the config reconciler (bd-a36r).
//!
//! Provides counters and histograms to observe reconciler health:
//! - How often reconciliation runs and how long it takes
//! - Config change counts by operation (add/remove/update)
//! - Error rates
//! - Watch reconnection frequency
//!
//! Metrics are registered with the global `REGISTRY` from `server::grpc::metrics_service`,
//! so they automatically appear at the `/metrics` Prometheus endpoint.
//!
//! Gated behind the `metrics` feature flag.

use prometheus::{
    register_histogram_with_registry, register_int_counter_vec_with_registry,
    register_int_counter_with_registry, Histogram, HistogramOpts, IntCounter, IntCounterVec, Opts,
    Registry,
};
use std::sync::OnceLock;

/// Global reconciler metrics instance.
///
/// Initialised once during daemon startup via [`init`]. Accessed from
/// `do_reconcile` and `start_watch_reconciler` via [`get`].
static INSTANCE: OnceLock<ReconcilerMetrics> = OnceLock::new();

/// Reconciler Prometheus metrics.
#[derive(Clone)]
pub struct ReconcilerMetrics {
    /// Config changes applied, labelled by operation (`add`, `remove`, `update`).
    pub config_changes: IntCounterVec,

    /// Duration of each `reconcile_once()` call in seconds.
    pub reconcile_duration: Histogram,

    /// Total reconcile errors (DB failures, driver instantiation failures, etc.).
    pub reconcile_errors: IntCounter,

    /// Total LIVE SELECT reconnections (indicates watch instability).
    pub watch_reconnections: IntCounter,

    /// Total driver metadata drift detections.
    pub metadata_drifts: IntCounter,

    /// Total driver metadata repairs written back to DB.
    pub metadata_repairs: IntCounter,
}

impl ReconcilerMetrics {
    /// Create and register metrics with the given Prometheus registry.
    pub fn with_registry(registry: &Registry) -> Self {
        let config_changes = register_int_counter_vec_with_registry!(
            Opts::new(
                "daq_config_changes_total",
                "Total config changes applied by the reconciler"
            ),
            &["operation"],
            registry
        )
        .expect("Failed to create daq_config_changes_total counter");

        let reconcile_duration = register_histogram_with_registry!(
            HistogramOpts::new(
                "daq_reconcile_duration_seconds",
                "Duration of reconcile_once() calls"
            )
            // Buckets tuned for typical reconcile times: sub-ms to multi-second.
            .buckets(vec![
                0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0
            ]),
            registry
        )
        .expect("Failed to create daq_reconcile_duration_seconds histogram");

        let reconcile_errors = register_int_counter_with_registry!(
            "daq_reconcile_errors_total",
            "Total reconciliation errors",
            registry
        )
        .expect("Failed to create daq_reconcile_errors_total counter");

        let watch_reconnections = register_int_counter_with_registry!(
            "daq_watch_reconnections_total",
            "Total LIVE SELECT reconnection attempts",
            registry
        )
        .expect("Failed to create daq_watch_reconnections_total counter");

        let metadata_drifts = register_int_counter_with_registry!(
            "daq_reconcile_metadata_drifts_total",
            "Total driver metadata drift detections",
            registry
        )
        .expect("Failed to create daq_reconcile_metadata_drifts_total counter");

        let metadata_repairs = register_int_counter_with_registry!(
            "daq_reconcile_metadata_repairs_total",
            "Total driver metadata repairs",
            registry
        )
        .expect("Failed to create daq_reconcile_metadata_repairs_total counter");

        Self {
            config_changes,
            reconcile_duration,
            reconcile_errors,
            watch_reconnections,
            metadata_drifts,
            metadata_repairs,
        }
    }

    /// Record results from a [`ReconcileReport`](crate::reconciler::ReconcileReport).
    pub fn record_report(&self, report: &crate::reconciler::ReconcileReport) {
        let added = report.added.len() as u64;
        let removed = report.removed.len() as u64;
        let updated = report.updated.len() as u64;
        if added > 0 {
            self.config_changes
                .with_label_values(&["add"])
                .inc_by(added);
        }
        if removed > 0 {
            self.config_changes
                .with_label_values(&["remove"])
                .inc_by(removed);
        }
        if updated > 0 {
            self.config_changes
                .with_label_values(&["update"])
                .inc_by(updated);
        }
        let errors = report.errors.len() as u64;
        if errors > 0 {
            self.reconcile_errors.inc_by(errors);
        }
        let drifts = report.metadata_drifts.len() as u64;
        if drifts > 0 {
            self.metadata_drifts.inc_by(drifts);
        }
        let repairs = report.metadata_repairs.len() as u64;
        if repairs > 0 {
            self.metadata_repairs.inc_by(repairs);
        }
    }
}

/// Initialise the global reconciler metrics.
///
/// Call once during daemon startup. Uses the global `REGISTRY` from the
/// server metrics module so all metrics appear at `/metrics`.
pub fn init() {
    INSTANCE
        .get_or_init(|| ReconcilerMetrics::with_registry(&server::grpc::metrics_service::REGISTRY));
}

/// Get the global reconciler metrics (returns `None` if not initialised).
pub fn get() -> Option<&'static ReconcilerMetrics> {
    INSTANCE.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation_and_recording() {
        let registry = Registry::new();
        let m = ReconcilerMetrics::with_registry(&registry);

        // Initial state: all zeros.
        assert_eq!(m.config_changes.with_label_values(&["add"]).get(), 0);
        assert_eq!(m.reconcile_errors.get(), 0);
        assert_eq!(m.watch_reconnections.get(), 0);
        assert_eq!(m.metadata_drifts.get(), 0);
        assert_eq!(m.metadata_repairs.get(), 0);

        // Simulate a reconcile report.
        let report = crate::reconciler::ReconcileReport {
            added: vec!["a".into(), "b".into()],
            removed: vec!["c".into()],
            updated: vec![],
            errors: vec!["oops".into()],
            metadata_drifts: vec!["mock_driver".into()],
            metadata_repairs: vec!["mock_driver".into()],
            unchanged: 5,
        };
        m.record_report(&report);

        assert_eq!(m.config_changes.with_label_values(&["add"]).get(), 2);
        assert_eq!(m.config_changes.with_label_values(&["remove"]).get(), 1);
        assert_eq!(m.config_changes.with_label_values(&["update"]).get(), 0);
        assert_eq!(m.reconcile_errors.get(), 1);
        assert_eq!(m.metadata_drifts.get(), 1);
        assert_eq!(m.metadata_repairs.get(), 1);

        // Watch reconnection counter.
        m.watch_reconnections.inc();
        assert_eq!(m.watch_reconnections.get(), 1);

        // Duration histogram.
        m.reconcile_duration.observe(0.042);
        assert_eq!(m.reconcile_duration.get_sample_count(), 1);
    }
}
