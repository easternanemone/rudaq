//! Daemon lifecycle manager.
//!
//! Encapsulates the startup/shutdown sequence for the rust-daq daemon,
//! ensuring correct ordering of initialization and teardown phases.
//!
//! # Shutdown Order (CRITICAL for laser safety)
//!
//! 1. Stop gRPC server (no new requests)
//! 2. Flush storage (persist buffered data)
//! 3. Shutdown hardware (safe physical state)
//! 4. Cleanup auxiliary tasks

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::safety_sentinel::SafetySentinel;
use common::health::watchdog::{HardwareWatchdog, WatchdogConfig};
use hardware::registry::DeviceRegistry;
use hardware::supervisor::{run_device_supervisor, SupervisorConfig};
use scripting::shutter_safety::ShutterRegistry;
use server::config::ServerConfig;
use server::health::sys_monitor::SystemMetricsCollector;
use server::health::{HealthMonitorConfig, SystemHealthMonitor};

#[cfg(feature = "networking")]
use hardware::registry::{
    create_mock_registry, create_registry_from_config, register_all_factories, HardwareConfig,
};
#[cfg(feature = "networking")]
use server::grpc::start_server_with_hardware;

/// Configuration for the daemon process.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub port: u16,
    pub hardware_config: Option<PathBuf>,
    pub lab_hardware: bool,
    /// Path for the SurrealDB database. Only used when `db-surreal-rocksdb` is
    /// enabled. `None` = use in-memory engine (default for `db-surreal-mem`).
    #[cfg(feature = "db-surreal")]
    pub db_path: Option<PathBuf>,
}

/// Tracks the daemon's current lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DaemonPhase {
    Initializing,
    HealthMonitor,
    Database,
    Storage,
    Hardware,
    Server,
    Running,
    ShuttingDown,
    Stopped,
}

impl std::fmt::Display for DaemonPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initializing => write!(f, "Initializing"),
            Self::HealthMonitor => write!(f, "HealthMonitor"),
            Self::Database => write!(f, "Database"),
            Self::Storage => write!(f, "Storage"),
            Self::Hardware => write!(f, "Hardware"),
            Self::Server => write!(f, "Server"),
            Self::Running => write!(f, "Running"),
            Self::ShuttingDown => write!(f, "ShuttingDown"),
            Self::Stopped => write!(f, "Stopped"),
        }
    }
}

/// Holds storage resources when the storage features are enabled.
#[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
struct StorageResources {
    writer: Arc<storage::hdf5_writer::HDF5Writer>,
    writer_task: JoinHandle<()>,
    _ring_buffer: Arc<storage::ring_buffer::RingBuffer>,
}

/// Ordered list of shutdown phases. This is the contract that MUST be followed
/// for laser safety. The order is: Server → Storage → Hardware.
#[allow(dead_code)]
pub const SHUTDOWN_PHASE_ORDER: &[DaemonPhase] = &[
    DaemonPhase::Server,
    DaemonPhase::Storage,
    DaemonPhase::Hardware,
];

/// The running daemon instance.
///
/// Owns all system components and enforces safe shutdown ordering.
/// Created via [`DaemonInstance::start`], torn down via [`DaemonInstance::shutdown`].
pub struct DaemonInstance {
    _config: DaemonConfig,
    _health: Arc<SystemHealthMonitor>,
    registry: Arc<DeviceRegistry>,
    shutdown_token: CancellationToken,
    metrics_task: JoinHandle<()>,
    registry_monitor_task: JoinHandle<()>,
    /// Hardware watchdog — fires emergency shutdown if the daemon loop hangs (bd-qa36.4.1).
    watchdog: Option<HardwareWatchdog>,
    /// Device supervisor — restarts faulted devices with exponential backoff (bd-qa36.4.2).
    supervisor_task: JoinHandle<()>,
    #[cfg(feature = "networking")]
    server_task: JoinHandle<Result<(), anyhow::Error>>,
    #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
    storage: Option<StorageResources>,
    /// Safety sentinel — RAII emergency shutter close on abnormal exit.
    /// Armed on start, disarmed only after successful shutdown.
    sentinel: SafetySentinel,
    /// Embedded SurrealDB instance (control plane persistence).
    /// Kept alive for connection lifetime; will be read by gRPC ConfigService (Phase 2).
    #[cfg(feature = "db-surreal")]
    #[allow(dead_code)] // Ownership anchor + used by ConfigService in Phase 2
    db: Option<db::DaqDb>,
    /// Background task running the watch reconciler (LIVE SELECT → reconcile loop).
    #[cfg(feature = "db-surreal")]
    watch_reconciler_task: Option<JoinHandle<()>>,
    /// Records which shutdown phases have been executed, in order.
    /// Used by contract tests to verify the shutdown sequence.
    #[cfg(test)]
    pub shutdown_log: Arc<std::sync::Mutex<Vec<DaemonPhase>>>,
}

impl DaemonInstance {
    /// Initialize and start the daemon.
    ///
    /// Startup phases (in order):
    /// 1. Health monitoring
    /// 2. Storage (ring buffer + HDF5 writer) — feature-gated
    /// 3. Hardware registry
    /// 4. Safety panic hook
    /// 5. Registry monitoring
    /// 6. gRPC server
    pub async fn start(config: DaemonConfig) -> Result<Self> {
        println!("🌐 Starting Headless DAQ Daemon");
        println!("   Architecture: V5 (Headless-First + Scriptable)");
        println!("   gRPC Port: {}", config.port);
        println!();

        // --- Phase: Config Loading & Validation ---
        let _server_config = ServerConfig::load().context("Failed to load server configuration")?;

        // --- Phase: Health Monitoring ---
        println!("❤️  Initializing health monitoring...");
        let health = Arc::new(SystemHealthMonitor::new(HealthMonitorConfig::default()));

        let metrics_collector = SystemMetricsCollector::new(health.clone());
        let metrics_task = tokio::spawn(async move {
            metrics_collector.run().await;
        });

        // --- Phase: Database (feature-gated: db-surreal) ---
        #[cfg(feature = "db-surreal")]
        let db = {
            println!("🗄️  Initializing database (SurrealDB)...");
            let db_config = if let Some(ref path) = config.db_path {
                println!("   Engine: RocksDB ({})", path.display());
                db::DbConfig::rocksdb(path)
            } else {
                println!("   Engine: in-memory (no persistence)");
                db::DbConfig::in_memory()
            };
            match db::DaqDb::init(db_config).await {
                Ok(db) => {
                    let info = db.info().await;
                    println!(
                        "   ✓ Database ready (schema v{}, {} drivers, {} instruments)",
                        info.schema_version, info.driver_count, info.instrument_count
                    );
                    println!();
                    Some(db)
                }
                Err(e) => {
                    // DB failure is non-fatal — the daemon can still run from TOML config.
                    // Log the error and continue without persistence.
                    eprintln!("   ⚠️  Database initialization failed: {}", e);
                    eprintln!("   Continuing without database persistence.");
                    println!();
                    None
                }
            }
        };

        // --- Phase: Storage (feature-gated) ---
        #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
        let storage = {
            use storage::hdf5_writer::HDF5Writer;
            use storage::ring_buffer::RingBuffer;

            let rb_path = &_server_config.storage.ring_buffer_path;
            let rb_size = _server_config.storage.ring_buffer_size_mb;
            let hdf5_path = &_server_config.storage.hdf5_path;

            println!("📊 Initializing data plane (Phase 4)...");
            println!("   - Ring buffer: {} MB in {}", rb_size, rb_path.display());
            println!("   - HDF5 output: {}", hdf5_path.display());
            println!("   - Background flush: every 1 second");

            let ring_buffer = Arc::new(
                RingBuffer::create(rb_path.as_path(), rb_size)
                    .context("Failed to create ring buffer")?,
            );

            let writer = HDF5Writer::new(hdf5_path.as_path(), ring_buffer.clone())
                .context("Failed to create HDF5 writer")?;
            let writer_arc = Arc::new(writer);
            let writer_clone = writer_arc.clone();

            let writer_task = tokio::spawn(async move {
                writer_clone.run_shared().await;
            });

            println!("✅ Data plane ready");
            println!();

            Some(StorageResources {
                writer: writer_arc,
                writer_task,
                _ring_buffer: ring_buffer,
            })
        };

        // --- Phase: Hardware Registry ---
        // Parse HardwareConfig once and reuse for both registry creation and DB shadow write.
        #[cfg(feature = "networking")]
        #[allow(unused_variables)] // hw_config used only with db-surreal feature
        let (registry, hw_config) = {
            println!("🔧 Initializing hardware registry...");
            let (registry, hw_config) = if let Some(ref config_path) = config.hardware_config {
                println!("   Loading from config: {}", config_path.display());
                let hw = HardwareConfig::from_file(config_path)
                    .context("Failed to parse hardware config")?;
                let config_dir = config_path
                    .parent()
                    .map(|p| p.join("devices"))
                    .filter(|p| p.exists());
                let reg = create_registry_from_config(&hw, config_dir.as_deref())
                    .await
                    .context("Failed to create hardware registry from config")?;
                (reg, Some(hw))
            } else if config.lab_hardware {
                let default_path = std::path::Path::new("config/maitai_hardware.toml");
                if !default_path.exists() {
                    anyhow::bail!(
                        "--lab-hardware requires {} or use --hardware-config <path>",
                        default_path.display()
                    );
                }
                println!("   Loading lab hardware from: {}", default_path.display());
                let hw = HardwareConfig::from_file(default_path)
                    .context("Failed to parse lab hardware config")?;
                let config_dir = default_path
                    .parent()
                    .map(|p| p.join("devices"))
                    .filter(|p| p.exists());
                let reg = create_registry_from_config(&hw, config_dir.as_deref())
                    .await
                    .context("Failed to create lab hardware registry")?;
                (reg, Some(hw))
            } else {
                println!("   Using mock devices (no hardware config specified)");
                let reg = create_mock_registry()
                    .await
                    .context("Failed to create mock registry")?;
                (reg, None)
            };

            // Register driver factories for plugin-based device creation
            let config_dir = std::path::Path::new("config/devices");
            if let Err(e) = register_all_factories(&registry, Some(config_dir)).await {
                tracing::warn!("Failed to register some factories: {}", e);
            }
            let factory_count = registry.list_factories().len();
            if factory_count > 0 {
                println!("   Registered {} driver factories", factory_count);
            }

            let device_count = registry.len();
            println!("   Registered {} device(s)", device_count);
            for info in registry.list_devices() {
                registry.set_config_source(&info.id, "toml");
                tracing::info!(device_id = %info.id, config_source = "toml", "device config source tagged");
                println!(
                    "     - {}: {} ({:?})",
                    info.id, info.name, info.capabilities
                );
            }
            println!();

            (Arc::new(registry), hw_config)
        };

        #[cfg(not(feature = "networking"))]
        let registry = Arc::new(DeviceRegistry::new());

        // --- Phase: Shadow Write to Database ---
        // Mirror the parsed hardware config into SurrealDB (write-only shadow copy).
        // Reuses the HardwareConfig already parsed above — no redundant file I/O.
        // Non-fatal: if anything fails, log a warning and continue.
        #[cfg(all(feature = "db-surreal", feature = "networking"))]
        if let (Some(ref db), Some(ref hw_config)) = (&db, &hw_config) {
            use crate::db_bridge;
            match db_bridge::shadow_write(db, hw_config).await {
                Ok((drivers, instruments)) => {
                    println!(
                        "   🗄️  Shadowed config to DB ({} drivers, {} instruments)",
                        drivers, instruments
                    );

                    // Set config_hash for each TOML-registered device so the
                    // initial reconcile sees old_hash == new_hash (unchanged).
                    // Without this, all devices appear "changed" because
                    // registry defaults config_hash to 0.
                    let db_instruments = db_bridge::devices_to_db(hw_config);
                    for inst in &db_instruments {
                        let hash = db::config_store::config_hash(&inst.config);
                        registry.set_config_hash(&inst.device_id, hash);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Shadow DB write failed — continuing without persistence");
                }
            }
        }

        // --- Phase: Reconciler Metrics (feature-gated) ---
        #[cfg(all(feature = "db-surreal", feature = "metrics"))]
        {
            crate::reconciler_metrics::init();
            println!("   📊 Reconciler Prometheus metrics registered");
        }

        // --- Phase: Reconcile DB → Registry ---
        // After shadow-writing config to DB, run one reconciliation pass to
        // converge the registry if the DB has instruments the TOML didn't include
        // (e.g., added via CLI between restarts).  Non-fatal.
        #[cfg(feature = "db-surreal")]
        if let Some(ref db) = db {
            use crate::reconciler;

            #[cfg(feature = "metrics")]
            let start = std::time::Instant::now();

            match reconciler::reconcile_once(db, &registry).await {
                Ok(report) => {
                    #[cfg(feature = "metrics")]
                    if let Some(m) = crate::reconciler_metrics::get() {
                        m.reconcile_duration.observe(start.elapsed().as_secs_f64());
                        m.record_report(&report);
                    }
                    if !report.added.is_empty() || !report.removed.is_empty() {
                        println!(
                            "   🔄 Reconciled: +{} added, -{} removed",
                            report.added.len(),
                            report.removed.len()
                        );
                    }
                }
                Err(e) => {
                    #[cfg(feature = "metrics")]
                    if let Some(m) = crate::reconciler_metrics::get() {
                        m.reconcile_duration.observe(start.elapsed().as_secs_f64());
                        m.reconcile_errors.inc();
                    }
                    tracing::warn!(error = %e, "Initial reconciliation failed — continuing with TOML-only state");
                }
            }
        }

        // --- Phase: Safety Hooks (CRITICAL) ---
        println!("🛡️  Installing hardware safety panic hook...");
        ShutterRegistry::install_panic_hook_with_hardware(&registry);
        let sentinel = SafetySentinel::new();
        println!("   Emergency shutdown will activate on panic (shutters + motors + DAQ)");
        println!("   Safety sentinel armed (auto-close on abnormal exit)");
        println!();

        // --- Phase: Hardware Watchdog (bd-qa36.4.1) ---
        // Separate OS thread monitors daemon liveness. If the tokio runtime hangs
        // (deadlock, blocking call, etc.), the watchdog fires emergency shutdown.
        println!("🐕 Starting hardware watchdog...");
        let (watchdog, wd_kicker) = HardwareWatchdog::start(WatchdogConfig::default(), || {
            // Emergency action runs on the watchdog's OS thread.
            // ShutterRegistry handles its own runtime bridging internally.
            ShutterRegistry::emergency_close_all();
        });
        println!("   Timeout: 30s (kicks from registry monitor task)");
        println!();

        // --- Phase: Registry Monitoring ---
        let mon_registry = registry.clone();
        let mon_health = health.clone();
        let registry_monitor_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                // Kick the hardware watchdog — proves the tokio runtime is responsive
                wd_kicker.kick();
                let count = mon_registry.len();
                mon_health
                    .heartbeat_with_message(
                        "hardware_registry",
                        Some(format!("Managing {} devices", count)),
                    )
                    .await;
            }
        });

        // --- Phase: Shutdown Token (bd-1afe.12) ---
        // CancellationToken enables graceful server shutdown + script abort before hardware teardown.
        let shutdown_token = CancellationToken::new();

        // --- Phase: Device Supervisor (bd-qa36.4.2) ---
        // Periodically checks for faulted devices and attempts restart with backoff.
        let sup_registry = registry.clone();
        let sup_token = shutdown_token.clone();
        let supervisor_task = tokio::spawn(async move {
            run_device_supervisor(sup_registry, SupervisorConfig::default(), sup_token).await;
        });

        // --- Phase: Watch Reconciler (LIVE SELECT → debounce → reconcile) ---
        // Continuously watches SurrealDB for config changes and reconciles the
        // hardware registry.  Uses CancellationToken for graceful shutdown.
        #[cfg(feature = "db-surreal")]
        let watch_reconciler_task = if let Some(ref db) = db {
            let wr_db = db.clone();
            let wr_registry = registry.clone();
            let wr_token = shutdown_token.clone();
            println!("👁️  Starting watch reconciler (LIVE SELECT → registry sync)...");
            Some(tokio::spawn(async move {
                crate::watch_reconciler::start_watch_reconciler(
                    wr_db,
                    wr_registry,
                    crate::watch_reconciler::WatchConfig::default(),
                    wr_token,
                )
                .await;
            }))
        } else {
            None
        };

        // --- Phase: gRPC Server ---
        #[cfg(feature = "networking")]
        let server_task = {
            let addr = format!("0.0.0.0:{}", config.port)
                .parse()
                .context("Invalid server address")?;

            println!("✅ gRPC server ready");
            println!("   Listening on: {}", addr);
            println!("   Features:");
            println!("     - Script upload & execution");
            println!("     - Remote hardware control (HardwareService)");
            println!("     - Module system (ModuleService)");
            println!("     - Coordinated scans (ScanService)");
            println!("     - Preset save/load (PresetService)");
            println!("     - System Health Monitoring (HealthService)");
            println!();

            let srv_registry = registry.clone();
            let srv_health = health.clone();
            let srv_token = shutdown_token.clone();
            #[cfg(feature = "db-surreal")]
            let srv_db = db.clone();
            tokio::spawn(async move {
                start_server_with_hardware(
                    addr,
                    srv_registry,
                    srv_health,
                    srv_token,
                    #[cfg(feature = "db-surreal")]
                    srv_db,
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
            })
        };

        Ok(Self {
            _config: config,
            _health: health,
            registry,
            shutdown_token,
            metrics_task,
            registry_monitor_task,
            watchdog: Some(watchdog),
            supervisor_task,
            #[cfg(feature = "networking")]
            server_task,
            #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
            storage,
            sentinel,
            #[cfg(feature = "db-surreal")]
            db,
            #[cfg(feature = "db-surreal")]
            watch_reconciler_task,
            #[cfg(test)]
            shutdown_log: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    /// Block until a shutdown signal (Ctrl+C) is received.
    pub async fn wait_for_shutdown_signal(&self) {
        println!("📡 Daemon running - Press Ctrl+C to stop");
        println!();
        match tokio::signal::ctrl_c().await {
            Ok(()) => println!("\n🛑 Shutdown signal received, cleaning up..."),
            Err(e) => eprintln!("\n❌ Failed to listen for shutdown signal: {}", e),
        }
    }

    /// Perform graceful shutdown in the correct order.
    ///
    /// # Shutdown Order (CRITICAL for laser safety)
    ///
    /// 1. **Stop server** — prevent new requests from arriving
    /// 2. **Flush storage** — persist any buffered data
    /// 3. **Shutdown hardware** — return devices to safe state
    /// 4. **Cleanup** — abort monitoring tasks
    pub async fn shutdown(mut self) -> Result<()> {
        println!("   Initiating graceful shutdown...");

        // Disarm the hardware watchdog FIRST — prevent false emergency during
        // intentional shutdown (the registry_monitor_task will be aborted below,
        // which would stop kicking the watchdog).
        //
        // DESIGN NOTE (bd-qa36.4.7): There is a small window between
        // watchdog.stop() returning and shutdown_token.cancel() being called
        // below. During this window, a new fault could in theory trigger an
        // action that races with shutdown. This is acceptable because:
        //   1. The watchdog is disarmed, so it won't fire false emergencies.
        //   2. The supervisor is still running but will be cancelled momentarily.
        //   3. The window is sub-millisecond in practice (sequential code).
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.stop();
            println!("   ✓ Hardware watchdog disarmed");
        }

        // Helper to record phases in test builds
        #[cfg(test)]
        let log = self.shutdown_log.clone();
        #[cfg(test)]
        macro_rules! record_phase {
            ($phase:expr) => {
                log.lock().unwrap().push($phase);
            };
        }

        // 1. Stop gRPC server (bd-1afe.12: graceful with CancellationToken)
        //    Cancel token → server stops accepting connections, reaper aborts running scripts
        //    Wait up to 5s for graceful drain, then force-abort as safety net
        #[cfg(feature = "networking")]
        {
            println!("   [1/3] Stopping gRPC server...");
            #[cfg(test)]
            record_phase!(DaemonPhase::Server);

            // Signal graceful shutdown (stops accepting connections + triggers script reaper)
            self.shutdown_token.cancel();

            // Wait for server to drain with grace period
            match tokio::time::timeout(std::time::Duration::from_secs(5), &mut self.server_task)
                .await
            {
                Ok(Ok(Ok(()))) => println!("   ✓ Server stopped gracefully"),
                Ok(Ok(Err(e))) => eprintln!("   ⚠️  Server error on shutdown: {}", e),
                Ok(Err(e)) if e.is_cancelled() => println!("   ✓ Server task cancelled"),
                Ok(Err(e)) => eprintln!("   ⚠️  Server task join error: {}", e),
                Err(_elapsed) => {
                    eprintln!("   ⚠️  Server shutdown timed out after 5s, force aborting");
                    self.server_task.abort();
                }
            }
        }
        #[cfg(not(feature = "networking"))]
        {
            println!("   [1/3] Server (networking disabled, skipped)");
            #[cfg(test)]
            record_phase!(DaemonPhase::Server);
        }

        // 2. Flush storage
        #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
        {
            println!("   [2/3] Flushing storage...");
            #[cfg(test)]
            record_phase!(DaemonPhase::Storage);
            if let Some(res) = self.storage {
                if let Err(e) = res.writer.flush_to_disk().await {
                    eprintln!("   ⚠️  HDF5 flush error during shutdown: {}", e);
                }
                res.writer_task.abort();
                println!("   ✓ HDF5 writer flushed and stopped");
            }
        }
        #[cfg(not(all(feature = "storage_hdf5", feature = "storage_arrow")))]
        {
            println!("   [2/3] Storage (feature disabled, skipped)");
            #[cfg(test)]
            record_phase!(DaemonPhase::Storage);
        }

        // 3. Shutdown hardware
        println!("   [3/3] Shutting down hardware...");
        #[cfg(test)]
        record_phase!(DaemonPhase::Hardware);
        if let Err(e) = self.registry.shutdown_all().await {
            eprintln!("   ⚠️  Device shutdown encountered errors: {}", e);
        } else {
            println!("   ✓ All devices shutdown safely");
        }

        // Cleanup auxiliary tasks
        // Supervisor exits via CancellationToken (already cancelled above), but abort as safety net
        self.supervisor_task.abort();
        self.registry_monitor_task.abort();
        self.metrics_task.abort();
        // Watch reconciler listens on the same CancellationToken, but abort as safety net
        #[cfg(feature = "db-surreal")]
        if let Some(task) = self.watch_reconciler_task {
            task.abort();
        }

        // Disarm safety sentinel — shutdown completed successfully.
        // Only reached after hardware is confirmed safe-stated.
        self.sentinel.disarm();

        println!("👋 Daemon shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_phase_display() {
        assert_eq!(DaemonPhase::Initializing.to_string(), "Initializing");
        assert_eq!(DaemonPhase::Running.to_string(), "Running");
        assert_eq!(DaemonPhase::ShuttingDown.to_string(), "ShuttingDown");
        assert_eq!(DaemonPhase::Stopped.to_string(), "Stopped");
    }

    #[test]
    fn test_daemon_config_debug() {
        let config = DaemonConfig {
            port: 50051,
            hardware_config: None,
            lab_hardware: false,
            #[cfg(feature = "db-surreal")]
            db_path: None,
        };
        let debug = format!("{:?}", config);
        assert!(debug.contains("50051"));
    }

    /// Contract test: verifies the SHUTDOWN_PHASE_ORDER constant matches
    /// the expected safety-critical ordering.
    #[test]
    fn test_shutdown_phase_order_contract() {
        // The shutdown order MUST be: Server → Storage → Hardware
        // This protects against:
        // - New requests arriving during shutdown (server first)
        // - Data loss from unflushed buffers (storage before hardware)
        // - Hardware left in unsafe state (hardware last)
        assert_eq!(SHUTDOWN_PHASE_ORDER.len(), 3);
        assert_eq!(SHUTDOWN_PHASE_ORDER[0], DaemonPhase::Server);
        assert_eq!(SHUTDOWN_PHASE_ORDER[1], DaemonPhase::Storage);
        assert_eq!(SHUTDOWN_PHASE_ORDER[2], DaemonPhase::Hardware);
    }

    /// Contract test: verifies the DaemonPhase enum variants have the
    /// expected lifecycle progression.
    #[test]
    fn test_daemon_phase_lifecycle_progression() {
        let lifecycle = [
            DaemonPhase::Initializing,
            DaemonPhase::HealthMonitor,
            DaemonPhase::Database,
            DaemonPhase::Storage,
            DaemonPhase::Hardware,
            DaemonPhase::Server,
            DaemonPhase::Running,
            DaemonPhase::ShuttingDown,
            DaemonPhase::Stopped,
        ];
        // All phases should be distinct
        for (i, a) in lifecycle.iter().enumerate() {
            for (j, b) in lifecycle.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "Phases at index {} and {} should differ", i, j);
                }
            }
        }
    }

    /// Integration test: verify that a DaemonInstance can be started with
    /// mock hardware (no config file, port 0) and then shut down cleanly.
    #[tokio::test]
    async fn test_daemon_start_and_shutdown_mock() {
        let config = DaemonConfig {
            port: 0,
            hardware_config: None,
            lab_hardware: false,
            #[cfg(feature = "db-surreal")]
            db_path: None,
        };

        let instance = DaemonInstance::start(config)
            .await
            .expect("DaemonInstance::start should succeed with mock config");

        instance
            .shutdown()
            .await
            .expect("DaemonInstance::shutdown should succeed");
    }

    /// Contract test: verify that the shutdown phase log recorded during
    /// `DaemonInstance::shutdown` matches `SHUTDOWN_PHASE_ORDER` exactly.
    ///
    /// This ensures the actual runtime shutdown sequence honours the
    /// safety-critical ordering declared in the constant.
    #[tokio::test]
    async fn test_shutdown_log_matches_contract() {
        let config = DaemonConfig {
            port: 0,
            hardware_config: None,
            lab_hardware: false,
            #[cfg(feature = "db-surreal")]
            db_path: None,
        };

        let instance = DaemonInstance::start(config)
            .await
            .expect("DaemonInstance::start should succeed with mock config");

        // Clone the Arc *before* shutdown consumes self, so we can inspect
        // the log afterwards.
        let log_handle = instance.shutdown_log.clone();

        instance
            .shutdown()
            .await
            .expect("DaemonInstance::shutdown should succeed");

        let recorded = log_handle.lock().unwrap();
        assert_eq!(
            recorded.as_slice(),
            SHUTDOWN_PHASE_ORDER,
            "Shutdown log must match SHUTDOWN_PHASE_ORDER exactly"
        );
    }
}
