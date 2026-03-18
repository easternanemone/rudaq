//! Liveness heartbeat log for overnight run forensics (bd-7xqd).
//!
//! Writes a structured JSONL file (one JSON object per line per minute)
//! containing system vitals: CPU%, RSS, disk free, active device count,
//! device health summary, RunEngine state, and frames acquired. Enables
//! post-mortems on failed overnight runs without parsing full daemon logs.
//!
//! # Output Format
//!
//! Each line is a self-contained JSON object:
//! ```json
//! {"ts":"2026-03-17T03:14:00Z","cpu_pct":12.3,"rss_mb":847,"disk_free_gb":42.1,
//!  "devices":12,"healthy":10,"faulted":1,"degraded":1,
//!  "engine":"running","run_uid":"abc123","seq_num":47,"queued":0}
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, System, get_current_pid,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use common::health::device_health::DeviceHealth;
use experiment::RunEngine;
use hardware::registry::DeviceRegistry;

const BYTES_PER_MB: u64 = 1024 * 1024;
const BYTES_PER_GB: u64 = 1024 * 1024 * 1024;

/// One heartbeat entry written as a single JSONL line.
#[derive(Debug, Serialize)]
struct HeartbeatEntry {
    /// ISO 8601 timestamp.
    ts: String,
    /// Global CPU usage percentage.
    cpu_pct: f32,
    /// Daemon RSS in megabytes.
    rss_mb: u64,
    /// Free disk space on the storage volume in gigabytes (rounded to 1 decimal).
    disk_free_gb: f64,
    /// Total registered devices.
    devices: usize,
    /// Devices in Healthy state.
    healthy: usize,
    /// Devices in Faulted state.
    faulted: usize,
    /// Devices in Degraded state.
    degraded: usize,
    /// RunEngine state (idle, running, paused, aborting).
    engine: String,
    /// Current run UID, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    run_uid: Option<String>,
    /// Current event sequence number within the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    seq_num: Option<u32>,
    /// Number of plans queued.
    queued: usize,
}

/// Configuration for the heartbeat logger.
pub struct HeartbeatLogConfig {
    /// Path to the JSONL output file.
    pub path: PathBuf,
    /// How often to write a heartbeat entry.
    pub interval: Duration,
    /// Storage volume path for disk free checks.
    pub storage_path: PathBuf,
}

impl Default for HeartbeatLogConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("/tmp/rust_daq_heartbeat.jsonl"),
            interval: Duration::from_secs(60),
            storage_path: PathBuf::from("/tmp"),
        }
    }
}

/// Run the heartbeat logger as a background task.
///
/// Collects system vitals every `config.interval` and appends a JSONL
/// line to `config.path`. Exits when `cancel` is triggered.
pub async fn run_heartbeat_log(
    registry: Arc<DeviceRegistry>,
    run_engine: Arc<RunEngine>,
    config: HeartbeatLogConfig,
    cancel: CancellationToken,
) {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    let mut disks = Disks::new_with_refreshed_list();
    let current_pid = get_current_pid().ok();

    info!(
        path = %config.path.display(),
        interval_secs = config.interval.as_secs(),
        "Heartbeat logger started"
    );

    let mut interval = tokio::time::interval(config.interval);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("Heartbeat logger shutting down");
                return;
            }
            _ = interval.tick() => {
                let entry = collect_entry(
                    &mut sys,
                    &mut disks,
                    current_pid,
                    &config.storage_path,
                    &registry,
                    &run_engine,
                ).await;

                if let Err(e) = append_entry(&config.path, &entry) {
                    warn!(error = %e, "Failed to write heartbeat entry");
                }
            }
        }
    }
}

#[allow(clippy::cast_precision_loss)]
async fn collect_entry(
    sys: &mut System,
    disks: &mut Disks,
    current_pid: Option<sysinfo::Pid>,
    storage_path: &Path,
    registry: &DeviceRegistry,
    run_engine: &RunEngine,
) -> HeartbeatEntry {
    // Refresh CPU and memory
    sys.refresh_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );

    let cpu_pct = sys.global_cpu_usage();

    // Get daemon RSS
    let rss_mb = current_pid
        .and_then(|pid| {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_memory(),
            );
            sys.process(pid).map(|p| p.memory() / BYTES_PER_MB)
        })
        .unwrap_or(0);

    // Get disk free space
    disks.refresh_specifics(true, DiskRefreshKind::nothing().with_storage());
    let disk_free_gb = disks
        .iter()
        .filter(|d| storage_path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space() as f64 / BYTES_PER_GB as f64)
        .unwrap_or(0.0);
    // Round to 1 decimal
    let disk_free_gb = (disk_free_gb * 10.0).round() / 10.0;

    // Device health summary
    let device_count = registry.len();
    let health_states = registry.list_device_health();
    let mut healthy = 0usize;
    let mut faulted = 0usize;
    let mut degraded = 0usize;
    for (_, state) in &health_states {
        match state.health {
            DeviceHealth::Healthy => healthy += 1,
            DeviceHealth::Faulted => faulted += 1,
            DeviceHealth::Degraded => degraded += 1,
            DeviceHealth::Recovering => {} // counted separately if needed
        }
    }

    // RunEngine state
    let engine_state = run_engine.state().await;
    let run_uid = run_engine.current_run_uid().await;
    let seq_num = run_engine.current_progress().await;
    let queued = run_engine.queue_len().await;

    HeartbeatEntry {
        ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        cpu_pct,
        rss_mb,
        disk_free_gb,
        devices: device_count,
        healthy,
        faulted,
        degraded,
        engine: engine_state.to_string(),
        run_uid,
        seq_num,
        queued,
    }
}

fn append_entry(path: &Path, entry: &HeartbeatEntry) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    let line = serde_json::to_string(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    writeln!(file, "{line}")?;
    debug!(path = %path.display(), "Heartbeat entry written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = HeartbeatLogConfig::default();
        assert_eq!(config.interval, Duration::from_secs(60));
        assert_eq!(config.path, PathBuf::from("/tmp/rust_daq_heartbeat.jsonl"));
    }

    #[tokio::test]
    async fn test_heartbeat_writes_valid_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("heartbeat.jsonl");

        let registry = Arc::new(DeviceRegistry::new());
        let run_engine = Arc::new(RunEngine::new(registry.clone()));

        let config = HeartbeatLogConfig {
            path: log_path.clone(),
            interval: Duration::from_millis(50),
            storage_path: dir.path().to_path_buf(),
        };

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let task = tokio::spawn(async move {
            run_heartbeat_log(registry, run_engine, config, cancel_clone).await;
        });

        // Let a few entries be written
        tokio::time::sleep(Duration::from_millis(180)).await;
        cancel.cancel();
        task.await.expect("heartbeat task should not panic");

        // Verify the file exists and contains valid JSONL
        let contents = std::fs::read_to_string(&log_path).expect("should read log file");
        let lines: Vec<&str> = contents.lines().collect();
        assert!(
            !lines.is_empty(),
            "should have at least one heartbeat entry"
        );

        for line in &lines {
            let entry: serde_json::Value =
                serde_json::from_str(line).expect("each line should be valid JSON");
            assert!(entry.get("ts").is_some(), "should have timestamp");
            assert!(entry.get("cpu_pct").is_some(), "should have cpu_pct");
            assert!(entry.get("rss_mb").is_some(), "should have rss_mb");
            assert!(entry.get("devices").is_some(), "should have devices");
            assert!(entry.get("engine").is_some(), "should have engine state");
            assert_eq!(
                entry["engine"].as_str().unwrap(),
                "idle",
                "engine should be idle"
            );
        }
    }

    #[tokio::test]
    async fn test_heartbeat_exits_on_cancel() {
        let registry = Arc::new(DeviceRegistry::new());
        let run_engine = Arc::new(RunEngine::new(registry.clone()));
        let config = HeartbeatLogConfig {
            interval: Duration::from_secs(60),
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let task = tokio::spawn(async move {
            run_heartbeat_log(registry, run_engine, config, cancel_clone).await;
        });

        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("should exit within 2s")
            .expect("should not panic");
    }

    #[test]
    fn test_append_entry_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");

        let entry = HeartbeatEntry {
            ts: "2026-03-17T03:14:00Z".to_string(),
            cpu_pct: 12.3,
            rss_mb: 847,
            disk_free_gb: 42.1,
            devices: 12,
            healthy: 10,
            faulted: 1,
            degraded: 1,
            engine: "running".to_string(),
            run_uid: Some("abc123".to_string()),
            seq_num: Some(47),
            queued: 0,
        };

        append_entry(&path, &entry).expect("should write entry");
        append_entry(&path, &entry).expect("should append second entry");

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 2, "should have 2 lines");

        let parsed: serde_json::Value =
            serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(parsed["devices"], 12);
        assert_eq!(parsed["engine"], "running");
        assert_eq!(parsed["run_uid"], "abc123");
    }

    #[test]
    fn test_optional_fields_skipped_when_idle() {
        let entry = HeartbeatEntry {
            ts: "2026-03-17T03:14:00Z".to_string(),
            cpu_pct: 5.0,
            rss_mb: 200,
            disk_free_gb: 100.0,
            devices: 3,
            healthy: 3,
            faulted: 0,
            degraded: 0,
            engine: "idle".to_string(),
            run_uid: None,
            seq_num: None,
            queued: 0,
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            !json.contains("run_uid"),
            "run_uid should be omitted when None"
        );
        assert!(
            !json.contains("seq_num"),
            "seq_num should be omitted when None"
        );
    }
}
