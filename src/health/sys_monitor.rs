//! System metrics collection using sysinfo (bd-3ti1)
//!
//! Gathers OS-level metrics (CPU, RAM, Disk) and reports them to the
//! SystemHealthMonitor.

use crate::health::{ErrorSeverity, SystemHealthMonitor};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Collects system metrics and reports to health monitor
pub struct SystemMetricsCollector {
    monitor: Arc<SystemHealthMonitor>,
    system: System,
    update_interval: Duration,
}

impl SystemMetricsCollector {
    /// Create a new collector
    pub fn new(monitor: Arc<SystemHealthMonitor>) -> Self {
        Self {
            monitor,
            system: System::new_all(),
            update_interval: Duration::from_secs(5),
        }
    }

    /// Start the collection loop in a background task
    pub async fn run(mut self) {
        let mut interval = tokio::time::interval(self.update_interval);

        loop {
            interval.tick().await;

            // Refresh metrics
            self.system.refresh_specifics(
                RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            );

            // Calculate metrics
            let cpu_usage = self.system.global_cpu_usage();
            let total_memory = self.system.total_memory();
            let used_memory = self.system.used_memory();
            let memory_percent = if total_memory > 0 {
                (used_memory as f64 / total_memory as f64) * 100.0
            } else {
                0.0
            };

            // Format status message
            let status = format!(
                "CPU: {:.1}%, RAM: {:.1}% ({}/{} MB)",
                cpu_usage,
                memory_percent,
                used_memory / 1024 / 1024,
                total_memory / 1024 / 1024
            );

            // Report heartbeat
            self.monitor
                .heartbeat_with_message("system_metrics", Some(status))
                .await;

            // Check thresholds and report warnings
            if cpu_usage > 90.0 {
                self.monitor
                    .report_error(
                        "system_metrics",
                        ErrorSeverity::Warning,
                        format!("High CPU usage: {:.1}%", cpu_usage),
                        vec![("metric", "cpu"), ("value", &cpu_usage.to_string())],
                    )
                    .await;
            }

            if memory_percent > 90.0 {
                self.monitor
                    .report_error(
                        "system_metrics",
                        ErrorSeverity::Warning,
                        format!("High memory usage: {:.1}%", memory_percent),
                        vec![("metric", "memory"), ("value", &memory_percent.to_string())],
                    )
                    .await;
            }
        }
    }
}
