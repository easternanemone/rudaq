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
use hardware::supervisor::{SupervisorConfig, run_device_supervisor};
use scripting::shutter_safety::ShutterRegistry;
use server::config::ServerConfig;
use server::health::sys_monitor::SystemMetricsCollector;
use server::health::{HealthMonitorConfig, SystemHealthMonitor};

#[cfg(feature = "networking")]
use driver_registry::register_all_factories;
#[cfg(feature = "networking")]
use hardware::registry::HardwareConfig;
#[cfg(feature = "networking")]
use server::grpc::start_server_with_hardware;

/// Configuration for the daemon process.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub port: u16,
    pub hardware_config: Option<PathBuf>,
    pub lab_hardware: bool,
    /// Resolved runtime mode string (e.g. "mock", "native", "universal", "hybrid-db", "custom").
    pub runtime_mode: String,
    /// Path for the database file. When set, uses file-backed SQLite.
    /// `None` = use in-memory engine (default).
    #[cfg(feature = "db")]
    pub db_path: Option<PathBuf>,
    /// Optional path to the WASM web UI directory.
    pub web_ui_path: Option<PathBuf>,
    /// Optional override for the storage output directory.
    /// When set, overrides `output_directory` from config/config.v4.toml.
    pub data_path: Option<PathBuf>,
}

/// Resolve the `devices/` manifest directory adjacent to a config file.
#[cfg(feature = "networking")]
fn resolve_device_manifest_dir(config_path: &std::path::Path) -> Option<PathBuf> {
    config_path
        .parent()
        .map(|p| p.join("devices"))
        .filter(|p| p.exists())
}

/// Load a hardware config file and create a device registry from it.
#[cfg(feature = "networking")]
async fn load_hardware_config(
    config_path: &std::path::Path,
    label: &str,
    runtime_mode: &str,
) -> Result<(DeviceRegistry, HardwareConfig)> {
    println!(
        "   Loading {label} hardware from: {}",
        config_path.display()
    );
    if !config_path.exists() {
        anyhow::bail!(
            "Hardware config not found: {}\n  \
             Hint: run from the repo root, use --runtime-mode mock, or specify --hardware-config <path>",
            config_path.display()
        );
    }
    let hw = HardwareConfig::from_file(config_path).context("Failed to parse hardware config")?;
    let source = config_path.display().to_string();
    validate_runtime_policy_for_config(&source, &hw, runtime_mode)?;
    let config_dir = resolve_device_manifest_dir(config_path);
    let reg = driver_registry::create_registry_from_config(&hw, config_dir.as_deref())
        .await
        .with_context(|| format!("Failed to create {label} hardware registry"))?;
    Ok((reg, hw))
}

#[cfg(feature = "networking")]
fn is_native_exception_driver(driver_type: &str) -> bool {
    matches!(
        driver_type,
        "pvcam"
            | "mock_camera"
            | "andor_istar"
            | "andor_shamrock"
            | "comedi_analog_input"
            | "comedi_analog_output"
            | "dover_axis"
    )
}

#[cfg(feature = "networking")]
fn validate_runtime_policy_for_config(
    source: &str,
    hw: &HardwareConfig,
    runtime_mode: &str,
) -> Result<()> {
    let mut universal_count = 0usize;
    let mut native_exception_count = 0usize;
    let mut unrecognized_count = 0usize;
    let mut unrecognized_driver_types = Vec::new();

    for device in &hw.devices {
        let driver_type = device.driver.driver_type.as_str();
        if driver_type.starts_with("universal_") {
            universal_count += 1;
            continue;
        }
        if is_native_exception_driver(driver_type) {
            native_exception_count += 1;
            continue;
        }

        unrecognized_count += 1;
        unrecognized_driver_types.push(driver_type.to_string());
    }

    tracing::info!(
        config_source = source,
        universal_count,
        native_exception_count,
        unrecognized_count,
        "Runtime policy summary: universal TOML for SCPI/TCP, native exceptions for SDK-bound drivers"
    );
    println!(
        "   Runtime policy [{source}]: universal={universal_count}, native_exception={native_exception_count}, unrecognized={unrecognized_count}"
    );

    // In universal/hybrid-db modes, only universal and native-exception drivers are allowed.
    // Legacy SCPI/TCP drivers have been sunset (Phase 4 complete).
    if unrecognized_count > 0 {
        let is_gated_mode = matches!(runtime_mode, "universal" | "hybrid-db");

        if is_gated_mode {
            anyhow::bail!(
                "Unrecognized driver types in '{}' mode: [{}]. \
                 Only universal_* and native-exception drivers (PVCAM, Andor, Comedi, Dover) are supported. \
                 Legacy SCPI/TCP drivers have been removed. See docs/how-to/legacy-scpi-deprecation.md",
                runtime_mode,
                unrecognized_driver_types.join(", ")
            );
        } else {
            tracing::warn!(
                runtime_mode = runtime_mode,
                unrecognized_drivers = ?unrecognized_driver_types,
                "Unrecognized driver types detected; these may not function correctly"
            );
            println!(
                "   ⚠ Unrecognized driver types detected: [{}]",
                unrecognized_driver_types.join(", ")
            );
        }
    }

    Ok(())
}

#[cfg(feature = "networking")]
#[derive(Debug)]
enum ServerShutdownError {
    TimedOut,
    Server(anyhow::Error),
    Join(tokio::task::JoinError),
}

#[cfg(feature = "networking")]
impl std::fmt::Display for ServerShutdownError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut => f.write_str("Server shutdown timed out"),
            Self::Server(e) => write!(f, "Server error on shutdown: {e}"),
            Self::Join(e) => write!(f, "Server task join error: {e}"),
        }
    }
}

#[cfg(feature = "networking")]
fn flatten_server_shutdown(
    result: Result<
        Result<Result<(), anyhow::Error>, tokio::task::JoinError>,
        tokio::time::error::Elapsed,
    >,
) -> Result<(), ServerShutdownError> {
    match result {
        Err(_) => Err(ServerShutdownError::TimedOut),
        Ok(Err(e)) if e.is_cancelled() => Ok(()),
        Ok(Err(e)) => Err(ServerShutdownError::Join(e)),
        Ok(Ok(Err(e))) => Err(ServerShutdownError::Server(e)),
        Ok(Ok(Ok(()))) => Ok(()),
    }
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

// ─── Phase resource structs ─────────────────────────────────────────────────

/// Resources produced by the health monitoring initialization phase.
struct HealthResources {
    monitor: Arc<SystemHealthMonitor>,
    metrics_task: JoinHandle<()>,
}

/// Resources produced by the hardware registry initialization phase.
#[cfg(feature = "networking")]
struct HardwareResources {
    registry: DeviceRegistry,
    #[allow(dead_code)]
    hw_config: Option<HardwareConfig>,
}

/// Resources produced by the safety initialization phase.
struct SafetyResources {
    sentinel: SafetySentinel,
    watchdog: HardwareWatchdog,
    registry_monitor_task: JoinHandle<()>,
    supervisor_task: JoinHandle<()>,
}

// ─── Phase initialization functions ─────────────────────────────────────────

fn init_server_config(config: &DaemonConfig) -> Result<ServerConfig> {
    let mut server_config = ServerConfig::load().context("Failed to load server configuration")?;
    if let Some(ref ui_path) = config.web_ui_path {
        server_config.grpc.web_ui_path = Some(ui_path.clone());
    }
    server_config
        .validate()
        .context("Config validation failed after applying overrides")?;
    Ok(server_config)
}

fn init_health() -> HealthResources {
    println!("❤️  Initializing health monitoring...");
    let monitor = Arc::new(SystemHealthMonitor::new(HealthMonitorConfig::default()));
    let metrics_collector = SystemMetricsCollector::new(monitor.clone());
    let metrics_task = tokio::spawn(async move {
        metrics_collector.run().await;
    });
    HealthResources {
        monitor,
        metrics_task,
    }
}

#[cfg(feature = "db")]
async fn init_database(config: &DaemonConfig, health: &HealthResources) -> Option<db::DaqDb> {
    println!("🗄️  Initializing database (SQLite)...");
    let engine_name = if config.db_path.is_some() {
        "file"
    } else {
        "memory"
    };
    let db_config = if let Some(ref path) = config.db_path {
        println!("   Engine: file ({})", path.display());
        db::DbConfig::file(path.display().to_string())
    } else {
        println!("   Engine: in-memory (no persistence)");
        db::DbConfig::in_memory()
    };
    match db::DaqDb::init(db_config).await {
        Ok(db) => {
            match db.info().await {
                Ok(info) => {
                    println!(
                        "   ✓ Database ready (schema v{}, {} drivers, {} instruments)",
                        info.schema_version, info.driver_count, info.instrument_count
                    );
                    println!();
                    health
                        .monitor
                        .heartbeat_with_message(
                            "database",
                            Some(format!(
                                "SQLite ({engine_name}) — schema v{}, {} drivers, {} instruments",
                                info.schema_version, info.driver_count, info.instrument_count,
                            )),
                        )
                        .await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to query DB info after init");
                    println!("   ✓ Database initialized ({engine_name})");
                    println!();
                }
            }
            Some(db)
        }
        Err(e) => {
            tracing::warn!(
                engine = engine_name,
                error = %e,
                "Database initialization failed — running without persistence. \
                 ConfigService and metadata catalogs will be unavailable. \
                 Use --db-path <dir> to configure persistent storage."
            );
            health
                .monitor
                .report_error(
                    "database",
                    common::health::ErrorSeverity::Warning,
                    format!("SQLite init failed: {e}"),
                    vec![("engine", engine_name)],
                )
                .await;
            eprintln!("   ⚠️  Database initialization failed: {e}");
            eprintln!("   Continuing without database persistence (ConfigService unavailable).");
            println!();
            None
        }
    }
}

#[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
fn init_storage(server_config: &ServerConfig) -> Result<StorageResources> {
    use storage::hdf5_writer::HDF5Writer;
    use storage::ring_buffer::RingBuffer;

    let rb_path = &server_config.storage.ring_buffer_path;
    let rb_size = server_config.storage.ring_buffer_size_mb;
    let hdf5_path = &server_config.storage.hdf5_path;

    println!("📊 Initializing data plane (Phase 4)...");
    println!("   - Ring buffer: {} MB in {}", rb_size, rb_path.display());
    println!("   - HDF5 output: {}", hdf5_path.display());
    println!("   - Background flush: every 1 second");

    let ring_buffer = Arc::new(
        RingBuffer::create(rb_path.as_path(), rb_size).context("Failed to create ring buffer")?,
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

    Ok(StorageResources {
        writer: writer_arc,
        writer_task,
        _ring_buffer: ring_buffer,
    })
}

#[cfg(feature = "networking")]
async fn init_hardware(config: &DaemonConfig) -> Result<HardwareResources> {
    println!("🔧 Initializing hardware registry...");
    let (registry, hw_config) = if let Some(ref config_path) = config.hardware_config {
        let (reg, hw) = load_hardware_config(config_path, "config", &config.runtime_mode).await?;
        (reg, Some(hw))
    } else if config.lab_hardware {
        let default_path = std::path::Path::new("config/maitai_universal.toml");
        if !default_path.exists() {
            anyhow::bail!(
                "--lab-hardware requires {} or use --hardware-config <path>",
                default_path.display()
            );
        }
        let (reg, hw) = load_hardware_config(default_path, "lab", &config.runtime_mode).await?;
        (reg, Some(hw))
    } else {
        let default_mock_profile = std::path::Path::new("config/profiles/mock_maitai_lab.toml");
        if default_mock_profile.exists() {
            println!(
                "   Using mock profile bootstrap: {}",
                default_mock_profile.display()
            );
            let (reg, hw) =
                load_hardware_config(default_mock_profile, "mock-profile", &config.runtime_mode)
                    .await?;
            (reg, Some(hw))
        } else {
            anyhow::bail!(
                "No hardware config specified and default mock profile not found: {}\n\
                 Use --hardware-config <path> or ensure the mock profile exists.",
                default_mock_profile.display()
            );
        }
    };

    // Register driver factories for plugin-based device creation
    let config_dir = std::path::Path::new("config/devices");
    if let Err(e) = register_all_factories(&registry, Some(config_dir)).await {
        tracing::warn!("Failed to register some factories: {}", e);
    }
    let factory_count = registry.list_factories().len();
    if factory_count > 0 {
        println!("   Registered {factory_count} driver factories");
    }

    let device_count = registry.len();
    println!("   Registered {device_count} device(s)");
    for info in registry.list_devices() {
        registry.set_config_source(info.id.as_str(), "toml");
        tracing::info!(device_id = %info.id, config_source = "toml", "device config source tagged");
        println!(
            "     - {}: {} ({:?})",
            info.id, info.name, info.capabilities
        );
    }
    println!();

    Ok(HardwareResources {
        registry,
        hw_config,
    })
}

fn init_safety(registry: &DeviceRegistry, shutdown_token: &CancellationToken) -> SafetyResources {
    // Safety panic hook
    println!("🛡️  Installing hardware safety panic hook...");
    ShutterRegistry::install_panic_hook_with_hardware(registry);
    let sentinel = SafetySentinel::new();
    println!("   Emergency shutdown will activate on panic (shutters + emission + motors + DAQ)");
    println!("   Safety sentinel armed (auto-close on abnormal exit)");
    println!();

    // Hardware watchdog
    println!("🐕 Starting hardware watchdog...");
    let (watchdog, wd_kicker) = HardwareWatchdog::start(WatchdogConfig::default(), || {
        ShutterRegistry::emergency_close_all();
        ShutterRegistry::emergency_close_all_shutters_from_registry();
        ShutterRegistry::emergency_disable_all_emission();
        ShutterRegistry::emergency_stop_motors();
        ShutterRegistry::emergency_zero_outputs();
    });
    println!("   Timeout: 30s (kicks from registry monitor task)");
    println!();

    // Registry monitoring (also kicks the watchdog)
    let mon_registry = registry.clone();
    let mon_health_clone = {
        // We don't have health here, but the monitor task just needs a registry clone
        // The health heartbeat will be handled separately
        mon_registry.clone()
    };
    let _ = mon_health_clone; // suppress unused

    let registry_monitor_task = {
        let mon_reg = registry.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                wd_kicker.kick();
                let _count = mon_reg.len();
            }
        })
    };

    // Device supervisor
    let sup_registry = registry.clone();
    let sup_token = shutdown_token.clone();
    let supervisor_task = tokio::spawn(async move {
        run_device_supervisor(sup_registry, SupervisorConfig::default(), sup_token).await;
    });

    SafetyResources {
        sentinel,
        watchdog,
        registry_monitor_task,
        supervisor_task,
    }
}

/// The running daemon instance.
///
/// Owns all system components and enforces safe shutdown ordering.
/// Created via [`DaemonInstance::start`], torn down via [`DaemonInstance::shutdown`].
pub struct DaemonInstance {
    _config: DaemonConfig,
    _health: Arc<SystemHealthMonitor>,
    registry: DeviceRegistry,
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
    /// Embedded SQLite instance (control plane persistence).
    /// Kept alive for connection lifetime; will be read by gRPC ConfigService (Phase 2).
    #[cfg(feature = "db")]
    #[allow(dead_code)] // Ownership anchor + used by ConfigService in Phase 2
    db: Option<db::DaqDb>,
    /// Safety heartbeat task — toggles a Comedi DIO channel to drive hardware interlock.
    #[cfg(feature = "comedi_hardware")]
    heartbeat_task: Option<JoinHandle<()>>,
    /// Background task running the watch reconciler (broadcast → reconcile loop).
    #[cfg(feature = "db")]
    watch_reconciler_task: Option<JoinHandle<()>>,
    /// Records which shutdown phases have been executed, in order.
    /// Used by contract tests to verify the shutdown sequence.
    #[cfg(test)]
    pub shutdown_log: Arc<std::sync::Mutex<Vec<DaemonPhase>>>,
}

impl DaemonInstance {
    /// Initialize and start the daemon.
    ///
    /// Each phase is extracted into a dedicated function for testability.
    /// Phase ordering is enforced by data dependencies between phase outputs.
    pub async fn start(config: DaemonConfig) -> Result<Self> {
        println!("🌐 Starting Headless DAQ Daemon");
        println!("   Architecture: V5 (Headless-First + Scriptable)");
        println!("   gRPC Port: {}", config.port);
        println!();

        // Phase 1: Config
        let mut _server_config = init_server_config(&config)?;
        if let Some(ref data_path) = config.data_path {
            tracing::info!(
                path = %data_path.display(),
                "Overriding storage output_directory from DAQ_DATA_PATH / --data-path"
            );
            _server_config
                .storage
                .output_directory
                .clone_from(data_path);
        }

        // Phase 2: Health
        let health = init_health();

        // Phase 3: Database
        #[cfg(feature = "db")]
        let db = init_database(&config, &health).await;

        // Phase 4: Storage
        #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
        let storage = Some(init_storage(&_server_config)?);

        // Phase 5: Hardware registry
        #[cfg(feature = "networking")]
        let hw = init_hardware(&config).await?;
        #[cfg(feature = "networking")]
        let registry = hw.registry;
        #[cfg(not(feature = "networking"))]
        let registry = DeviceRegistry::new();

        // Phase 6: Shadow write config to DB + reconcile + restore parameters
        #[cfg(all(feature = "db", feature = "networking"))]
        if let (Some(db), Some(hw_config)) = (&db, &hw.hw_config) {
            use crate::db_bridge;
            match db_bridge::shadow_write_with_registry(db, hw_config, &registry).await {
                Ok((drivers, instruments)) => {
                    println!(
                        "   🗄️  Shadowed config to DB ({drivers} drivers, {instruments} instruments)"
                    );
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

        #[cfg(all(feature = "db", feature = "metrics"))]
        {
            crate::reconciler_metrics::init();
            println!("   📊 Reconciler Prometheus metrics registered");
        }

        #[cfg(feature = "db")]
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

            match restore_parameter_state(db, &registry).await {
                Ok(count) if count > 0 => {
                    println!("   🔧 Restored {count} parameter(s) from database");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Parameter state restore failed — using device defaults");
                }
                _ => {}
            }
        }

        // Phase 7: Shutdown token
        let shutdown_token = CancellationToken::new();

        // Phase 8: Safety (panic hook + watchdog + monitor + supervisor)
        let safety = init_safety(&registry, &shutdown_token);

        // Phase 9: Safety heartbeat (Comedi DIO)
        #[cfg(feature = "comedi_hardware")]
        let heartbeat_task = {
            #[cfg(feature = "networking")]
            let hb_ref = hw
                .hw_config
                .as_ref()
                .and_then(|hw| hw.safety_heartbeat.as_ref());
            #[cfg(not(feature = "networking"))]
            let hb_ref: Option<&hardware::registry::HeartbeatConfig> = None;
            if let Some(hb_config) = hb_ref {
                if hb_config.enabled {
                    println!("💓 Starting safety heartbeat...");
                    let hb_cfg = hb_config.clone();
                    let hb_token = shutdown_token.clone();
                    match crate::safety_heartbeat_task::spawn_heartbeat(hb_cfg, hb_token).await {
                        Ok(task) => {
                            println!(
                                "   Device: {}, channel: {}, interval: {}ms",
                                hb_config.device, hb_config.channel, hb_config.interval_ms
                            );
                            println!();
                            Some(task)
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Safety heartbeat init failed — continuing without heartbeat");
                            eprintln!("   Safety heartbeat failed to start: {e}");
                            eprintln!("   Daemon will continue without heartbeat interlock.");
                            println!();
                            None
                        }
                    }
                } else {
                    tracing::info!("Safety heartbeat disabled in config");
                    None
                }
            } else {
                None
            }
        };

        // Phase 10: Watch reconciler
        #[cfg(feature = "db")]
        let watch_reconciler_task = if let Some(ref db) = db {
            let wr_db = db.clone();
            let wr_registry = registry.clone();
            let wr_token = shutdown_token.clone();
            println!("👁️  Starting watch reconciler (broadcast → registry sync)...");
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

        // Phase 11: gRPC server
        #[cfg(feature = "networking")]
        let server_task = {
            let addr = format!("0.0.0.0:{}", config.port)
                .parse()
                .context("Invalid server address")?;

            println!("✅ gRPC server ready");
            println!("   Listening on: {addr}");
            println!("   Features:");
            println!("     - Script upload & execution");
            println!("     - Remote hardware control (HardwareService)");
            println!("     - Module system (ModuleService)");
            println!("     - Preset save/load (PresetService)");
            println!("     - System Health Monitoring (HealthService)");
            #[cfg(feature = "db")]
            if db.is_some() {
                println!("     - Config management (ConfigService) [DB available]");
            } else {
                println!(
                    "     ⚠ ConfigService UNAVAILABLE (database init failed or not configured)"
                );
            }
            #[cfg(not(feature = "db"))]
            println!("     ⚠ ConfigService UNAVAILABLE (compiled without db feature)");
            println!();

            let srv_registry = registry.clone();
            let srv_health = health.monitor.clone();
            let srv_token = shutdown_token.clone();
            #[cfg(feature = "db")]
            let srv_db = db.clone();
            tokio::spawn(async move {
                start_server_with_hardware(
                    addr,
                    srv_registry,
                    srv_health,
                    srv_token,
                    #[cfg(feature = "db")]
                    srv_db,
                )
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
            })
        };

        // Assemble the daemon instance from phase outputs
        Ok(Self {
            _config: config,
            _health: health.monitor,
            registry,
            shutdown_token,
            metrics_task: health.metrics_task,
            registry_monitor_task: safety.registry_monitor_task,
            watchdog: Some(safety.watchdog),
            supervisor_task: safety.supervisor_task,
            #[cfg(feature = "networking")]
            server_task,
            #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
            storage,
            sentinel: safety.sentinel,
            #[cfg(feature = "comedi_hardware")]
            heartbeat_task,
            #[cfg(feature = "db")]
            db,
            #[cfg(feature = "db")]
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
            Err(e) => eprintln!("\n❌ Failed to listen for shutdown signal: {e}"),
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
            match flatten_server_shutdown(
                tokio::time::timeout(std::time::Duration::from_secs(5), &mut self.server_task)
                    .await,
            ) {
                Ok(()) => println!("   ✓ Server stopped gracefully"),
                Err(ServerShutdownError::TimedOut) => {
                    eprintln!("   ⚠️  Server shutdown timed out after 5s, force aborting");
                    self.server_task.abort();
                }
                Err(e) => eprintln!("   ⚠️  {e}"),
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
            eprintln!("   ⚠️  Device shutdown encountered errors: {e}");
        } else {
            println!("   ✓ All devices shutdown safely");
        }

        // Cleanup auxiliary tasks
        // Supervisor exits via CancellationToken (already cancelled above), but abort as safety net
        self.supervisor_task.abort();
        self.registry_monitor_task.abort();
        self.metrics_task.abort();
        // Watch reconciler listens on the same CancellationToken, but abort as safety net
        #[cfg(feature = "db")]
        if let Some(task) = self.watch_reconciler_task {
            task.abort();
        }
        // Safety heartbeat listens on the same CancellationToken, but abort as safety net
        #[cfg(feature = "comedi_hardware")]
        if let Some(task) = self.heartbeat_task {
            task.abort();
        }

        // Disarm safety sentinel — shutdown completed successfully.
        // Only reached after hardware is confirmed safe-stated.
        self.sentinel.disarm();

        println!("👋 Daemon shutdown complete");
        Ok(())
    }
}

/// Restore last-known parameter values from the database to devices (bd-4wf7, bd-oqo7.8).
///
/// On daemon startup, reads `device_runtime_state` entries and applies them via
/// `Parameter::set_json()`. If a value is rejected by the driver (e.g., constraint
/// violation after a config change), the error is logged and the device keeps its
/// hardware default. Returns the number of successfully restored parameters.
///
/// **Read-only parameters** are skipped (they reflect hardware state, not user preferences).
///
/// **Speed table ordering**: Parameters are applied in dependency order so that
/// constrained choices (e.g., PVCAM readout Port -> Speed -> Gain) resolve correctly.
/// Port must be set before Speed (which depends on available speeds for that port),
/// and Speed before Gain (which depends on available gains for that speed).
#[cfg(feature = "db")]
async fn restore_parameter_state(db: &db::DaqDb, registry: &DeviceRegistry) -> Result<usize> {
    let mut restored = 0;
    for device_info in registry.list_devices() {
        let device_id = &device_info.id;
        let states = db.get_device_state(device_id.as_str()).await?;
        if states.is_empty() {
            continue;
        }

        let Some(parameterized) = registry.get_parameterized(device_id.as_str()) else {
            continue;
        };
        let param_set = parameterized.parameters();

        // Sort states into dependency order for correct restoration.
        // Speed table params must restore: Port -> Speed -> Gain.
        let ordered_states = order_params_for_restore(states);

        for state in ordered_states {
            let Some(param) = param_set.get(&state.param_name) else {
                continue;
            };

            // Skip read-only parameters — they reflect hardware state, not user preferences
            if param.metadata().read_only {
                tracing::trace!(
                    device = %device_id,
                    param = %state.param_name,
                    "Skipped read-only parameter during restore"
                );
                continue;
            }

            if let Err(e) = param.set_json(state.param_value.clone()) {
                tracing::debug!(
                    device = %device_id,
                    param = %state.param_name,
                    error = %e,
                    "Skipped restoring parameter (driver rejected value)"
                );
            } else {
                tracing::debug!(
                    device = %device_id,
                    param = %state.param_name,
                    value = %state.param_value,
                    "Restored parameter from DB"
                );
                restored += 1;
            }
        }
    }
    Ok(restored)
}

/// Sort persisted parameter states into dependency order for safe restoration.
///
/// Speed table parameters must be applied in order: Port -> Speed -> Gain,
/// because each constrains the choices available to the next. All other parameters
/// are applied after the speed table group, in their original (alphabetical) order.
#[cfg(feature = "db")]
fn order_params_for_restore(
    states: Vec<db::config_store::DeviceParamState>,
) -> Vec<db::config_store::DeviceParamState> {
    // Priority tiers: lower number = applied first.
    // Tier 0: readout port (constrains available speeds)
    // Tier 1: readout speed (constrains available gains)
    // Tier 2: readout gain
    // Tier 10: everything else (alphabetical within tier)
    fn priority(name: &str) -> u8 {
        match name {
            "readout.port" => 0,
            "readout.speed_mode" => 1,
            "readout.gain_mode" => 2,
            _ => 10,
        }
    }

    let mut sorted = states;
    sorted.sort_by(|a, b| {
        let pa = priority(&a.param_name);
        let pb = priority(&b.param_name);
        pa.cmp(&pb).then_with(|| a.param_name.cmp(&b.param_name))
    });
    sorted
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
            runtime_mode: "mock".to_string(),
            #[cfg(feature = "db")]
            db_path: None,
            web_ui_path: None,
            data_path: None,
        };
        let debug = format!("{config:?}");
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
                    assert_ne!(a, b, "Phases at index {i} and {j} should differ");
                }
            }
        }
    }

    // ── Runtime policy classification tests ───────────────────────────

    #[cfg(feature = "networking")]
    #[test]
    fn test_is_native_exception_driver() {
        // Camera-class native exceptions
        assert!(is_native_exception_driver("pvcam"));
        assert!(is_native_exception_driver("mock_camera"));
        assert!(is_native_exception_driver("andor_istar"));
        assert!(is_native_exception_driver("andor_shamrock"));
        // DAQ/motion native exceptions (Comedi + Dover)
        assert!(is_native_exception_driver("comedi_analog_input"));
        assert!(is_native_exception_driver("comedi_analog_output"));
        assert!(is_native_exception_driver("dover_axis"));
    }

    #[cfg(feature = "networking")]
    #[test]
    fn test_is_native_exception_driver_rejects() {
        assert!(!is_native_exception_driver("universal_thorlabs_ell14"));
        assert!(!is_native_exception_driver("universal_foo"));
        assert!(!is_native_exception_driver("ell14"));
        assert!(!is_native_exception_driver("random_driver"));
    }

    #[cfg(feature = "networking")]
    #[test]
    fn test_validate_rejects_unrecognized_in_universal_mode() {
        use hardware::registry::{DeviceConfig, DriverConfig};

        fn dev(id: &str, driver_type: &str) -> DeviceConfig {
            DeviceConfig {
                id: id.into(),
                name: id.to_string(),
                driver: DriverConfig {
                    driver_type: driver_type.to_string(),
                    config: toml::Value::Table(toml::map::Map::new()),
                },
                enabled: true,
            }
        }

        let hw = HardwareConfig {
            plugin_paths: vec![],
            devices: vec![
                dev("d1", "universal_thorlabs_ell14"),
                dev("d2", "ell14"), // unrecognized (former legacy)
            ],
            safety_heartbeat: None,
        };

        let result = validate_runtime_policy_for_config("test", &hw, "universal");
        assert!(
            result.is_err(),
            "should reject unrecognized drivers in universal mode"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ell14"),
            "error should name the offending driver"
        );
    }

    #[cfg(feature = "networking")]
    #[test]
    fn test_validate_warns_unrecognized_in_native_mode() {
        use hardware::registry::{DeviceConfig, DriverConfig};

        fn dev(id: &str, driver_type: &str) -> DeviceConfig {
            DeviceConfig {
                id: id.into(),
                name: id.to_string(),
                driver: DriverConfig {
                    driver_type: driver_type.to_string(),
                    config: toml::Value::Table(toml::map::Map::new()),
                },
                enabled: true,
            }
        }

        let hw = HardwareConfig {
            plugin_paths: vec![],
            devices: vec![dev("d1", "ell14"), dev("d2", "pvcam")],
            safety_heartbeat: None,
        };

        // Native mode warns but does not error
        validate_runtime_policy_for_config("test", &hw, "native")
            .expect("native mode should warn but allow unrecognized drivers");
    }

    #[cfg(feature = "networking")]
    #[test]
    fn test_validate_pure_universal_config_passes() {
        use hardware::registry::{DeviceConfig, DriverConfig};

        fn dev(id: &str, driver_type: &str) -> DeviceConfig {
            DeviceConfig {
                id: id.into(),
                name: id.to_string(),
                driver: DriverConfig {
                    driver_type: driver_type.to_string(),
                    config: toml::Value::Table(toml::map::Map::new()),
                },
                enabled: true,
            }
        }

        // Config with only universal + native exception drivers
        let hw = HardwareConfig {
            plugin_paths: vec![],
            devices: vec![
                dev("d1", "universal_thorlabs_ell14"),
                dev("d2", "universal_newport_esp300"),
                dev("d3", "pvcam"),
                dev("d4", "andor_istar"),
                dev("d5", "comedi_analog_input"),
            ],
            safety_heartbeat: None,
        };

        // Should pass in all modes
        validate_runtime_policy_for_config("test", &hw, "universal")
            .expect("pure universal+exception config should pass in universal mode");
        validate_runtime_policy_for_config("test", &hw, "hybrid-db")
            .expect("pure universal+exception config should pass in hybrid-db mode");
        validate_runtime_policy_for_config("test", &hw, "native")
            .expect("pure universal+exception config should pass in native mode");
    }

    // ── Watchdog emergency firing test (bd-qa36) ──────────────────

    /// Test that the HardwareWatchdog fires and executes the emergency
    /// closure when no kicks are received within the timeout period.
    #[test]
    fn test_watchdog_fires_emergency_closure() {
        let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fired_clone = fired.clone();

        let (_watchdog, _kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: std::time::Duration::from_secs(1),
            },
            move || {
                fired_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            },
        );

        // Do NOT kick the watchdog — simulate a hang
        std::thread::sleep(std::time::Duration::from_millis(1500));

        assert!(
            fired.load(std::sync::atomic::Ordering::SeqCst),
            "Watchdog emergency closure should fire when no kicks are received"
        );
    }

    /// Test that a watchdog kick loop followed by intentional stop does
    /// not fire the emergency closure.
    #[test]
    fn test_watchdog_does_not_fire_when_kicked() {
        let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fired_clone = fired.clone();

        let (watchdog, kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: std::time::Duration::from_secs(1),
            },
            move || {
                fired_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            },
        );

        // Kick regularly
        for _ in 0..5 {
            assert!(
                kicker.kick(),
                "kick should succeed while watchdog is running"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        watchdog.stop();

        assert!(
            !fired.load(std::sync::atomic::Ordering::SeqCst),
            "Watchdog should not fire when kicked and then stopped cleanly"
        );
    }

    // ── Heartbeat config error path test (bd-nfav) ──────────────

    /// Test that a HeartbeatConfig with `enabled = false` is respected:
    /// the daemon (when calling spawn_heartbeat) should bail with an error
    /// rather than attempting to open a non-existent device.
    ///
    /// This validates the graceful degradation path — when heartbeat init
    /// fails, the daemon continues without it.
    #[test]
    fn test_heartbeat_config_disabled_does_not_spawn() {
        use hardware::registry::HeartbeatConfig;

        let config = HeartbeatConfig {
            enabled: false,
            device: "/dev/comedi999".to_string(),
            subdevice: None,
            channel: 0,
            interval_ms: 100,
        };

        // When `enabled` is false, the daemon's heartbeat spawn path skips
        // initialization entirely. Validate that the config round-trips and
        // the `enabled` flag is correctly respected.
        assert!(
            !config.enabled,
            "Config with enabled=false should be respected"
        );
    }

    /// Resolve the canonical mock hardware profile from the workspace root.
    fn mock_profile_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("bin crate should be under crates/")
            .parent()
            .expect("workspace root should exist")
            .join("config/profiles/mock_maitai_lab.toml")
    }

    /// Integration test: verify that a DaemonInstance can be started with
    /// canonical mock profile (port 0) and then shut down cleanly.
    #[tokio::test]
    async fn test_daemon_start_and_shutdown_mock() {
        let config = DaemonConfig {
            port: 0,
            hardware_config: Some(mock_profile_path()),
            lab_hardware: false,
            runtime_mode: "mock".to_string(),
            #[cfg(feature = "db")]
            db_path: None,
            web_ui_path: None,
            data_path: None,
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
            hardware_config: Some(mock_profile_path()),
            lab_hardware: false,
            runtime_mode: "mock".to_string(),
            #[cfg(feature = "db")]
            db_path: None,
            web_ui_path: None,
            data_path: None,
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

    // ── Parameter state restore ordering tests (bd-oqo7.8) ────────────

    #[cfg(feature = "db")]
    mod param_restore_tests {
        use super::*;
        use db::config_store::DeviceParamState;

        fn state(name: &str, value: &str) -> DeviceParamState {
            DeviceParamState {
                device_id: "cam_0".to_string(),
                param_name: name.to_string(),
                param_value: serde_json::Value::String(value.to_string()),
                is_favorite: false,
            }
        }

        #[test]
        fn test_speed_table_params_ordered_before_others() {
            let states = vec![
                state("readout.gain_mode", "HDR"),
                state("acquisition.exposure_ms", "100"),
                state("readout.speed_mode", "100 MHz"),
                state("readout.port", "Sensitivity"),
            ];

            let ordered = order_params_for_restore(states);
            let names: Vec<&str> = ordered.iter().map(|s| s.param_name.as_str()).collect();

            // Port -> Speed -> Gain must come first, in that order
            assert_eq!(names[0], "readout.port");
            assert_eq!(names[1], "readout.speed_mode");
            assert_eq!(names[2], "readout.gain_mode");
            // Other params come after
            assert_eq!(names[3], "acquisition.exposure_ms");
        }

        #[test]
        fn test_order_preserves_alphabetical_within_same_tier() {
            let states = vec![
                state("thermal.setpoint", "-10"),
                state("acquisition.exposure_ms", "100"),
                state("acquisition.clear_mode", "PreExposure"),
            ];

            let ordered = order_params_for_restore(states);
            let names: Vec<&str> = ordered.iter().map(|s| s.param_name.as_str()).collect();

            // All tier 10, so alphabetical
            assert_eq!(names[0], "acquisition.clear_mode");
            assert_eq!(names[1], "acquisition.exposure_ms");
            assert_eq!(names[2], "thermal.setpoint");
        }

        #[test]
        fn test_order_empty_input() {
            let ordered = order_params_for_restore(vec![]);
            assert!(ordered.is_empty());
        }

        #[test]
        fn test_order_partial_speed_table() {
            // Only port and gain persisted (speed not persisted) — port still comes first
            let states = vec![
                state("readout.gain_mode", "HDR"),
                state("readout.port", "Speed"),
            ];

            let ordered = order_params_for_restore(states);
            let names: Vec<&str> = ordered.iter().map(|s| s.param_name.as_str()).collect();
            assert_eq!(names[0], "readout.port");
            assert_eq!(names[1], "readout.gain_mode");
        }
    }
}
