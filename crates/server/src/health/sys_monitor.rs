//! System metrics collection and resource pressure guards (bd-3ti1, bd-102j)
//!
//! Gathers OS-level metrics (CPU, RAM, Disk) and reports them to the
//! SystemHealthMonitor. Also pauses the RunEngine when low-disk or high-RSS
//! conditions would risk a long-running acquisition.

use common::health::{ErrorSeverity, SystemHealthMonitor};
use common::limits::HEALTH_CHECK_INTERVAL;
use experiment::{EngineState, RunEngine};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, System, get_current_pid,
};

const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
const DEFAULT_MIN_FREE_DISK_BYTES: u64 = 10 * BYTES_PER_GIB;
const DEFAULT_MAX_RSS_BYTES: u64 = 4 * BYTES_PER_GIB;

/// Collects system metrics and reports to health monitor.
pub struct SystemMetricsCollector {
    monitor: Arc<SystemHealthMonitor>,
    system: System,
    update_interval: Duration,
}

impl SystemMetricsCollector {
    /// Create a new collector.
    pub fn new(monitor: Arc<SystemHealthMonitor>) -> Self {
        Self {
            monitor,
            system: System::new_all(),
            update_interval: HEALTH_CHECK_INTERVAL,
        }
    }

    /// Start the collection loop in a background task.
    pub async fn run(mut self) {
        let mut interval = tokio::time::interval(self.update_interval);

        loop {
            interval.tick().await;

            self.system.refresh_specifics(
                RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            );

            let cpu_usage = self.system.global_cpu_usage();
            let total_memory = self.system.total_memory();
            let used_memory = self.system.used_memory();
            #[expect(
                clippy::cast_precision_loss,
                reason = "Resource-pressure percentages are display-only telemetry."
            )]
            let memory_percent = if total_memory > 0 {
                (used_memory as f64 / total_memory as f64) * 100.0
            } else {
                0.0
            };

            let status = format!(
                "CPU: {:.1}%, RAM: {:.1}% ({}/{} MB)",
                cpu_usage,
                memory_percent,
                used_memory / 1024 / 1024,
                total_memory / 1024 / 1024
            );

            self.monitor
                .heartbeat_with_message("system_metrics", Some(status))
                .await;

            if cpu_usage > 90.0 {
                self.monitor
                    .report_error(
                        "system_metrics",
                        ErrorSeverity::Warning,
                        format!("High CPU usage: {cpu_usage:.1}%"),
                        vec![("metric", "cpu"), ("value", &cpu_usage.to_string())],
                    )
                    .await;
            }

            if memory_percent > 90.0 {
                self.monitor
                    .report_error(
                        "system_metrics",
                        ErrorSeverity::Warning,
                        format!("High memory usage: {memory_percent:.1}%"),
                        vec![("metric", "memory"), ("value", &memory_percent.to_string())],
                    )
                    .await;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourcePressureKind {
    Disk,
    Memory,
}

impl ResourcePressureKind {
    fn metric(self) -> &'static str {
        match self {
            Self::Disk => "disk",
            Self::Memory => "memory",
        }
    }

    fn cleared_message(self) -> &'static str {
        match self {
            Self::Disk => "ResourcePressureEvent cleared: disk space recovered above threshold",
            Self::Memory => "ResourcePressureEvent cleared: daemon RSS recovered below threshold",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResourcePressureEvent {
    kind: ResourcePressureKind,
    observed_bytes: u64,
    threshold_bytes: u64,
    target: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ResourcePressureFlags {
    disk_active: bool,
    memory_active: bool,
}

/// Background guard that pauses the RunEngine during low-disk or high-memory pressure.
pub struct ResourceGuard {
    monitor: Arc<SystemHealthMonitor>,
    run_engine: Arc<RunEngine>,
    storage_path: PathBuf,
    system: System,
    disks: Disks,
    current_pid: Option<Pid>,
    update_interval: Duration,
    min_free_disk_bytes: u64,
    max_rss_bytes: u64,
    pressure_flags: ResourcePressureFlags,
}

impl ResourceGuard {
    /// Create a resource guard with default thresholds.
    pub fn new(
        monitor: Arc<SystemHealthMonitor>,
        run_engine: Arc<RunEngine>,
        storage_path: PathBuf,
    ) -> Self {
        Self {
            monitor,
            run_engine,
            storage_path,
            system: System::new_with_specifics(
                RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
            ),
            disks: Disks::new_with_refreshed_list(),
            current_pid: get_current_pid().ok(),
            update_interval: HEALTH_CHECK_INTERVAL,
            min_free_disk_bytes: DEFAULT_MIN_FREE_DISK_BYTES,
            max_rss_bytes: DEFAULT_MAX_RSS_BYTES,
            pressure_flags: ResourcePressureFlags::default(),
        }
    }

    /// Start the resource guard in a background task.
    pub async fn run(mut self) {
        let mut interval = tokio::time::interval(self.update_interval);

        loop {
            interval.tick().await;

            let free_disk_bytes = self.refresh_free_disk_bytes();
            let rss_bytes = self.refresh_rss_bytes();

            self.monitor
                .heartbeat_with_message(
                    "resource_guard",
                    Some(format_resource_guard_status(
                        &self.storage_path,
                        free_disk_bytes,
                        rss_bytes,
                    )),
                )
                .await;

            let disk_event = free_disk_bytes
                .filter(|available| *available < self.min_free_disk_bytes)
                .map(|available| ResourcePressureEvent {
                    kind: ResourcePressureKind::Disk,
                    observed_bytes: available,
                    threshold_bytes: self.min_free_disk_bytes,
                    target: self.storage_path.display().to_string(),
                });

            let memory_target = self
                .current_pid
                .map(|pid| format!("pid={}", pid.as_u32()))
                .unwrap_or_else(|| "pid=unknown".to_string());
            let memory_event = rss_bytes
                .filter(|rss| *rss > self.max_rss_bytes)
                .map(|rss| ResourcePressureEvent {
                    kind: ResourcePressureKind::Memory,
                    observed_bytes: rss,
                    threshold_bytes: self.max_rss_bytes,
                    target: memory_target.clone(),
                });

            self.handle_transition(
                ResourcePressureKind::Disk,
                self.pressure_flags.disk_active,
                disk_event.clone(),
            )
            .await;
            self.handle_transition(
                ResourcePressureKind::Memory,
                self.pressure_flags.memory_active,
                memory_event.clone(),
            )
            .await;

            self.pressure_flags.disk_active = disk_event.is_some();
            self.pressure_flags.memory_active = memory_event.is_some();
        }
    }

    fn refresh_free_disk_bytes(&mut self) -> Option<u64> {
        self.disks
            .refresh_specifics(false, DiskRefreshKind::everything());
        select_available_space_for_path(
            &self.storage_path,
            self.disks
                .list()
                .iter()
                .map(|disk| (disk.mount_point(), disk.available_space())),
        )
    }

    fn refresh_rss_bytes(&mut self) -> Option<u64> {
        let current_pid = self.current_pid?;
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[current_pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        self.system
            .process(current_pid)
            .map(|process| process.memory())
    }

    async fn handle_transition(
        &self,
        kind: ResourcePressureKind,
        was_active: bool,
        event: Option<ResourcePressureEvent>,
    ) {
        match (was_active, event) {
            (false, Some(event)) => self.emit_pressure_event(event).await,
            (true, None) => {
                tracing::info!(target: "resource_pressure", "{}", kind.cleared_message());
            }
            _ => {}
        }
    }

    async fn emit_pressure_event(&self, event: ResourcePressureEvent) {
        let message = format_resource_pressure_event(&event);
        let engine_state = self.run_engine.state().await;
        let pause_requested = if engine_state == EngineState::Running {
            self.run_engine.pause().await.is_ok()
        } else {
            false
        };

        self.monitor
            .report_error(
                "resource_guard",
                ErrorSeverity::Error,
                &message,
                vec![
                    ("metric".to_string(), event.kind.metric().to_string()),
                    ("target".to_string(), event.target.clone()),
                    (
                        "observed_bytes".to_string(),
                        event.observed_bytes.to_string(),
                    ),
                    (
                        "threshold_bytes".to_string(),
                        event.threshold_bytes.to_string(),
                    ),
                    ("engine_state".to_string(), engine_state.to_string()),
                    ("pause_requested".to_string(), pause_requested.to_string()),
                ],
            )
            .await;

        tracing::warn!(
            target: "resource_pressure",
            metric = event.kind.metric(),
            target = event.target,
            observed_bytes = event.observed_bytes,
            threshold_bytes = event.threshold_bytes,
            engine_state = %engine_state,
            pause_requested,
            "{}",
            message
        );
    }
}

fn format_resource_guard_status(
    storage_path: &Path,
    free_disk_bytes: Option<u64>,
    rss_bytes: Option<u64>,
) -> String {
    let disk_status = free_disk_bytes
        .map(format_bytes_gib)
        .unwrap_or_else(|| "unknown".to_string());
    let memory_status = rss_bytes
        .map(format_bytes_gib)
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "resource_guard: free_disk={} on {}, rss={}",
        disk_status,
        storage_path.display(),
        memory_status
    )
}

fn format_resource_pressure_event(event: &ResourcePressureEvent) -> String {
    match event.kind {
        ResourcePressureKind::Disk => format!(
            "ResourcePressureEvent: free disk on {} is {} (threshold {})",
            event.target,
            format_bytes_gib(event.observed_bytes),
            format_bytes_gib(event.threshold_bytes)
        ),
        ResourcePressureKind::Memory => format!(
            "ResourcePressureEvent: daemon RSS on {} is {} (threshold {})",
            event.target,
            format_bytes_gib(event.observed_bytes),
            format_bytes_gib(event.threshold_bytes)
        ),
    }
}

fn format_bytes_gib(bytes: u64) -> String {
    #[expect(
        clippy::cast_precision_loss,
        reason = "Human-readable GiB formatting is display-only telemetry."
    )]
    let gib = bytes as f64 / BYTES_PER_GIB as f64;
    format!("{gib:.1} GiB")
}

fn select_available_space_for_path<'a>(
    path: &Path,
    disks: impl IntoIterator<Item = (&'a Path, u64)>,
) -> Option<u64> {
    disks
        .into_iter()
        .filter(|(mount_point, _)| path.starts_with(mount_point))
        .max_by_key(|(mount_point, _)| mount_point.as_os_str().len())
        .map(|(_, available_space)| available_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_available_space_for_path_prefers_deepest_mount() {
        let available = select_available_space_for_path(
            Path::new("/data/daq/runs/today"),
            [
                (Path::new("/"), 100),
                (Path::new("/data"), 75),
                (Path::new("/data/daq"), 50),
            ],
        );

        assert_eq!(available, Some(50));
    }

    #[test]
    fn test_format_resource_pressure_event_for_disk() {
        let event = ResourcePressureEvent {
            kind: ResourcePressureKind::Disk,
            observed_bytes: 9 * BYTES_PER_GIB,
            threshold_bytes: 10 * BYTES_PER_GIB,
            target: "/data/daq".to_string(),
        };

        let message = format_resource_pressure_event(&event);
        assert!(message.contains("ResourcePressureEvent"));
        assert!(message.contains("/data/daq"));
        assert!(message.contains("9.0 GiB"));
        assert!(message.contains("10.0 GiB"));
    }

    #[test]
    fn test_format_resource_pressure_event_for_memory() {
        let event = ResourcePressureEvent {
            kind: ResourcePressureKind::Memory,
            observed_bytes: 5 * BYTES_PER_GIB,
            threshold_bytes: 4 * BYTES_PER_GIB,
            target: "pid=123".to_string(),
        };

        let message = format_resource_pressure_event(&event);
        assert!(message.contains("ResourcePressureEvent"));
        assert!(message.contains("pid=123"));
        assert!(message.contains("5.0 GiB"));
        assert!(message.contains("4.0 GiB"));
    }
}
