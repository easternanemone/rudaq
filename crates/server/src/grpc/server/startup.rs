//! Top-level gRPC server bootstrap.
//!
//! Hosts the `build_grpc_server!` macro and the two public entry points
//! (`start_server`, `start_server_with_hardware`) that wire CORS, auth,
//! TLS, the integrated WASM UI layer, and every gRPC service onto a
//! tonic `Server`. Extracted from `server/mod.rs` (C1 step 3).
//!
//! Both functions are declared `pub` (required for `pub use` re-export from
//! a private submodule); the effective external visibility is gated by the
//! private `mod startup;` declaration in the parent and the `pub use
//! startup::{start_server, start_server_with_hardware};` re-export that
//! keeps the crate-public path at `crate::grpc::server::start_server*`.

use super::*;
use common::pipeline::{MeasurementSink, Tee};

/// Build a gRPC `Server` with the standard middleware stack (CORS, auth, tracing, audit).
///
/// Extracted as a macro because Tower's builder returns deeply nested generic types
/// that cannot be named in a function signature without `type_alias_impl_trait`.
macro_rules! build_grpc_server {
    ($grpc_settings:expr, $cors:expr, $web_ui_path:expr) => {{
        use crate::grpc::audit_log::AuditLogLayer;
        use crate::grpc::request_tracing::RequestTracingLayer;
        let auth_settings = $grpc_settings.clone();
        Server::builder()
            .accept_http1(true)
            // HTTP/2 flow control optimization (bd-rgnx.11): larger window sizes reduce
            // flow control overhead for high-bandwidth camera frame streaming.
            // 2 MB stream window, 4 MB connection window (defaults are 64 KB / 64 KB).
            .initial_stream_window_size(2 * 1024 * 1024)
            .initial_connection_window_size(4 * 1024 * 1024)
            // Static file serving layer (bd-j8k9): must be outermost to intercept
            // non-gRPC requests before CORS/auth processing
            .layer(WebUiLayer::new($web_ui_path))
            .layer($cors)
            .layer(interceptor(move |request: Request<()>| {
                validate_auth(&auth_settings, &request)?;
                Ok(request)
            }))
            .layer(RequestTracingLayer::new())
            .layer(AuditLogLayer::new())
    }};
}

/// Start the DAQ gRPC server
///
/// Provides RunEngineService and optionally ControlService (when `scripting` feature is enabled).
/// ControlService includes script execution, stream_measurements, and stream_status methods.
pub async fn start_server(addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    use crate::grpc::health_service::HealthServiceImpl;
    use crate::grpc::proto::health::health_check_response::ServingStatus;
    use crate::grpc::proto::health::health_server::HealthServer;

    let config = ServerConfig::load()?;
    let grpc_settings = config.grpc;

    if grpc_settings.auth_enabled && grpc_settings.auth_token().is_none() {
        return Err("grpc.auth_enabled is true but grpc.auth_token is not configured".into());
    }

    let bind_addr = grpc_settings.bind_socket(addr.port());
    if bind_addr.ip() != addr.ip() {
        eprintln!(
            "⚠️  Overriding gRPC bind address {} -> {} (set grpc.bind_address to change)",
            addr.ip(),
            bind_addr.ip()
        );
    }

    // Create shared RunEngine FIRST (bd-si2c)
    let registry = hardware::registry::DeviceRegistry::new();
    let run_engine_instance = std::sync::Arc::new(experiment::RunEngine::new(registry));

    // Create DaqServer with shared RunEngine when scripting enabled (bd-si2c)
    // start_server() has no external shutdown coordination, so create a standalone token
    #[cfg(feature = "scripting")]
    let standalone_token = CancellationToken::new();
    #[cfg(all(feature = "scripting", feature = "storage_hdf5"))]
    let server = DaqServer::new(None, run_engine_instance.clone(), standalone_token)?;
    #[cfg(all(feature = "scripting", not(feature = "storage_hdf5")))]
    let server = DaqServer::new(run_engine_instance.clone(), standalone_token)?;
    #[cfg(all(not(feature = "scripting"), feature = "storage_hdf5"))]
    let server = DaqServer::new(None)?;
    #[cfg(all(not(feature = "scripting"), not(feature = "storage_hdf5")))]
    let server = DaqServer::new()?;

    let run_engine = RunEngineServiceImpl::new(
        run_engine_instance,
        #[cfg(feature = "metrics")]
        None, // No DaqMetrics in legacy start_server path
    );

    let health_service = HealthServiceImpl::new();

    health_service.set_serving_status("", ServingStatus::Serving);
    health_service.set_serving_status("daq.ControlService", ServingStatus::Serving);
    health_service.set_serving_status("daq.RunEngineService", ServingStatus::Serving);

    println!("DAQ gRPC server listening on {bind_addr}");

    if !grpc_settings.auth_enabled {
        // SECURITY (bd-qa36.8.2): Script upload/execution endpoints accept arbitrary
        // Rhai code. Without auth, any network client can execute scripts with daemon
        // privileges. This warning is intentionally loud.
        eprintln!("⚠️  gRPC auth is disabled (set grpc.auth_enabled=true to require auth)");
        eprintln!("🚨 SECURITY: Script upload/execution endpoints are unauthenticated!");
        eprintln!("   Any client can upload and execute Rhai scripts with daemon privileges.");
        eprintln!("   Set grpc.auth_enabled=true and grpc.auth_token in production.");
    }

    let cors = build_cors_layer(&grpc_settings)?;
    let tls_config = build_tls_config(&grpc_settings)?;
    if tls_config.is_none() {
        eprintln!(
            "⚠️  gRPC TLS is disabled (set grpc.tls_cert_path + grpc.tls_key_path to enable)"
        );
    }

    let mut builder = build_grpc_server!(grpc_settings, cors, grpc_settings.web_ui_path.as_ref());

    if let Some(tls_config) = tls_config {
        builder = builder.tls_config(tls_config)?;
    }

    #[cfg(feature = "scripting")]
    let builder = builder.add_service(tonic_web::enable(
        ControlServiceServer::new(server).max_encoding_message_size(64 * 1024 * 1024),
    ));

    builder
        .add_service(tonic_web::enable(HealthServer::new(health_service)))
        .add_service(tonic_web::enable(RunEngineServiceServer::new(run_engine)))
        .serve(bind_addr)
        .await?;

    Ok(())
}

/// Start the DAQ gRPC server with hardware control (bd-4x6q)
///
/// Provides HardwareService for direct device control and optionally ControlService
/// (when `scripting` feature is enabled) for script management and data streaming.
///
/// # Arguments
/// * `addr` - Socket address to listen on
/// * `registry` - Device registry for hardware access
///
/// # Example
/// ```ignore
/// use server::grpc::start_server_with_hardware;
/// use hardware::registry::create_mock_registry;
///
/// let registry = create_mock_registry().await?;
/// let addr = "127.0.0.1:50051".parse()?;
/// start_server_with_hardware(addr, registry).await?;
/// ```
pub async fn start_server_with_hardware(
    addr: std::net::SocketAddr,
    registry: hardware::registry::DeviceRegistry,
    health_monitor: std::sync::Arc<common::health::SystemHealthMonitor>,
    shutdown_token: CancellationToken,
    #[cfg(feature = "db")] _db: Option<db::DaqDb>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::grpc::hardware_service::HardwareServiceImpl;
    use crate::grpc::module_service::ModuleServiceImpl;
    use crate::grpc::ni_daq_service::NiDaqServiceImpl;
    use storage::ring_buffer::RingBuffer;
    // use crate::grpc::plugin_service::PluginServiceImpl; // Unused
    use crate::grpc::preset_service::{PresetServiceImpl, default_preset_storage_path};
    use crate::grpc::proto::hardware_service_server::HardwareServiceServer;
    use crate::grpc::proto::health::health_check_response::ServingStatus;
    use crate::grpc::proto::health::health_server::HealthServer;
    use crate::grpc::proto::health_service_server::HealthServiceServer; // Custom HealthService
    use crate::grpc::proto::module_service_server::ModuleServiceServer;
    use protocol::ni_daq::ni_daq_service_server::NiDaqServiceServer;
    use tonic::codec::CompressionEncoding;
    // use crate::grpc::proto::plugin_service_server::PluginServiceServer; // Unused
    use crate::grpc::proto::preset_service_server::PresetServiceServer;
    use crate::grpc::proto::storage_service_server::StorageServiceServer;
    use crate::grpc::storage_service::StorageServiceImpl;

    let config = ServerConfig::load()?;
    let grpc_settings = config.grpc;
    let storage_settings = config.storage;

    if grpc_settings.auth_enabled && grpc_settings.auth_token().is_none() {
        return Err("grpc.auth_enabled is true but grpc.auth_token is not configured".into());
    }

    let bind_addr = grpc_settings.bind_socket(addr.port());
    if bind_addr.ip() != addr.ip() {
        eprintln!(
            "⚠️  Overriding gRPC bind address {} -> {} (set grpc.bind_address to change)",
            addr.ip(),
            bind_addr.ip()
        );
    }

    // Create ring buffer for scan data persistence (The Mullet Strategy)
    // Use /dev/shm on Linux, /tmp on macOS for memory-mapped storage
    let ring_buffer_path = storage_settings.ring_buffer_path.clone();
    let ring_buffer_size = storage_settings.ring_buffer_size_mb as u64;

    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    let ring_buffer = match tokio::task::spawn_blocking(move || {
        RingBuffer::create(&ring_buffer_path, ring_buffer_size as usize)
    })
    .await
    {
        Ok(Ok(rb)) => {
            println!(
                "  - RingBuffer: {} ({} MB)",
                storage_settings.ring_buffer_path.display(),
                ring_buffer_size
            );
            Some(std::sync::Arc::<storage::ring_buffer::RingBuffer>::new(rb))
        }
        Ok(Err(e)) => {
            eprintln!(
                "Warning: Failed to create ring buffer: {e}. Scan data will not be persisted."
            );
            None
        }
        Err(e) => {
            eprintln!("Warning: Ring buffer creation task panicked or was cancelled: {e}");
            None
        }
    };

    if let Some(ref rb) = ring_buffer {
        rb.set_tap_channel_capacity(storage_settings.tap_channel_size);
    }

    // NOTE: HDF5Writer is NOT spawned here at startup. Frames are only written
    // to disk when explicitly requested via StorageService::start_recording().
    // The ring buffer acts as a circular in-memory buffer that StorageService
    // taps into on demand.

    // Create shared RunEngine FIRST - used by both RunEngineService and ControlService/scripts (bd-si2c)
    let run_engine = std::sync::Arc::new(experiment::RunEngine::new(registry.clone()));

    // Initialize control server WITHOUT internal RingBuffer logic (we wire it manually)
    // Pass shared run_engine for script execution (bd-si2c)
    #[cfg(all(feature = "storage_hdf5", feature = "scripting"))]
    let control_server = DaqServer::new(None, run_engine.clone(), shutdown_token.clone())?;
    #[cfg(all(feature = "storage_hdf5", not(feature = "scripting")))]
    let control_server = DaqServer::new(None)?;
    #[cfg(all(not(feature = "storage_hdf5"), feature = "scripting"))]
    let control_server = DaqServer::new(run_engine.clone(), shutdown_token.clone())?;
    #[cfg(all(not(feature = "storage_hdf5"), not(feature = "scripting")))]
    let control_server = DaqServer::new()?;

    // Setup Reliable Sink (RingBuffer Writer)
    let reliable_sink_tx = if let Some(ref rb) = ring_buffer {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Measurement>(512);
        let rb_clone = rb.clone();

        // Spawn writer task
        tokio::spawn(async move {
            let mut last_frame_numbers = HashMap::<String, u64>::new();
            while let Some(mut measurement) = rx.recv().await {
                if let Some((source, fault)) =
                    annotate_measurement_data_integrity(&mut measurement, &mut last_frame_numbers)
                {
                    log_data_integrity_fault(&source, fault);
                }

                if let Ok(frame) = encode_measurement_frame(&measurement) {
                    let rb = rb_clone.clone();
                    // Offload blocking mmap write to avoid stalling Tokio
                    // workers under contention (bd-tvp6).
                    if let Err(e) = tokio::task::spawn_blocking(move || rb.write(&frame))
                        .await
                        .expect("ring buffer write task panicked")
                    {
                        tracing::error!(error = %e, "Failed to write measurement to ring buffer");
                    }
                }
            }
        });
        Some(tx)
    } else {
        None
    };

    // Wire Pipelines (bd-37tw.7 - Tee-based)
    // Connect measurement sources to Tee -> (RingBuffer, Server Broadcast)
    //
    // SAFETY (bd-jnfu.6): Collect device info and sources while holding lock,
    // then DROP lock before performing async operations to prevent deadlock/contention.
    {
        // Phase 1: Collect devices and sources (lock-free with DashMap)
        let devices_to_wire: Vec<_> = registry
            .list_devices()
            .into_iter()
            .filter_map(|info| {
                registry
                    .get_measurement_source_frame(&info.id)
                    .map(|source| (info.id.clone(), source))
            })
            .collect();

        // Phase 2: Perform async registration (no lock held)
        for (device_id, source) in devices_to_wire {
            println!("  - Wiring pipeline for device: {device_id}");

            // 1. Create channel for Source output (Arc<Frame>)
            let frame_chan = std::env::var("DAQ_PIPELINE_FRAME_CH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(16);
            let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(frame_chan);

            // 2. Register source output (ASYNC - safe now, no lock held)
            if let Err(e) = source.register_output(frame_tx).await {
                eprintln!("Failed to register output for {device_id}: {e}");
                continue;
            }

            // 3. Create channel for Measurement (Tee Input)
            let meas_chan = std::env::var("DAQ_PIPELINE_MEAS_CH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(16);
            let (meas_tx, meas_rx) = tokio::sync::mpsc::channel(meas_chan);
            let image_meas_tx = meas_tx.clone();
            let device_id_clone = device_id.clone();
            let extraction_feed =
                load_echelle_profile_for_device(&registry, &device_id).map(|profile| {
                    let (extract_tx, mut extract_rx) =
                        tokio::sync::watch::channel(None::<Arc<common::data::Frame>>);
                    let spectrum_meas_tx = meas_tx.clone();
                    let extraction_device_id = device_id.clone();
                    let extraction_profile = profile.clone();
                    tokio::spawn(async move {
                        let mut u16_scratch = Vec::new();
                        while extract_rx.changed().await.is_ok() {
                            let Some(frame) = extract_rx.borrow_and_update().clone() else {
                                continue;
                            };

                            match echelle::extract_preview_with_u16_scratch(
                                extraction_profile.as_ref(),
                                &frame.data,
                                frame.width,
                                frame.height,
                                frame.bit_depth,
                                frame.frame_number,
                                &mut u16_scratch,
                            ) {
                                Ok(preview) => {
                                    for measurement in preview.to_measurements_with_context(
                                        false,
                                        Some(&extraction_device_id),
                                    ) {
                                        if spectrum_meas_tx.send(measurement).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        device_id = %extraction_device_id,
                                        frame_number = frame.frame_number,
                                        error = %error,
                                        "Echelle extraction failed; raw frame streaming continues"
                                    );
                                }
                            }

                            tokio::task::yield_now().await;
                        }
                    });

                    tracing::info!(
                        device_id = %device_id,
                        "Enabled server-side echelle spectrum extraction for frame source"
                    );

                    extract_tx
                });

            // 4. Spawn Converter Task (Frame -> Measurement)
            tokio::spawn(async move {
                while let Some(frame) = frame_rx.recv().await {
                    let measurement = build_image_measurement(&device_id_clone, &frame);

                    if image_meas_tx.send(measurement).await.is_err() {
                        break; // Downstream closed
                    }
                    if let Some(extract_tx) = &extraction_feed {
                        extract_tx.send_replace(Some(frame.clone()));
                    }
                }
            });

            // 5. Create Tee
            let mut tee = Tee::new((*control_server.data_sender()).clone()); // Lossy output (Server Bus)

            // 6. Connect Reliable Output (if RingBuffer is present)
            if let Some(ref rb_tx) = reliable_sink_tx {
                tee.connect_reliable(rb_tx.clone());
            }

            // 7. Start Tee (Consume Measurement Stream)
            if let Err(e) = tee.register_input(meas_rx) {
                eprintln!("Failed to register Tee input for {device_id}: {e}");
            }
        }
    }

    // Game loop broadcast channel — created early so Rerun can subscribe (Phase 6).
    // Drivers → mpsc → game loop → broadcast → { gRPC StreamSystemState, Rerun }
    let (state_broadcast_tx, _) =
        tokio::sync::broadcast::channel::<common::state_cache::SystemStateSnapshot>(64);

    // Setup Rerun Visualization (gRPC server mode for remote GUI clients)
    #[cfg(feature = "rerun_sink")]
    {
        // Port 9876 is the default Rerun gRPC port
        // same_machine=false enables higher memory limits for remote clients
        let rerun_bind = std::env::var("RERUN_BIND").unwrap_or_else(|_| "0.0.0.0".to_string());
        let rerun_port: u16 = std::env::var("RERUN_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9876);
        match crate::rerun_sink::RerunSink::new_server(&rerun_bind, rerun_port, false) {
            Ok(rerun) => {
                println!(
                    "  - Rerun Visualization: Active (gRPC server on {}:{})",
                    rerun_bind, rerun_port
                );

                // Optional blueprint: default path or override via RERUN_BLUEPRINT
                let blueprint_choice = std::env::var("RERUN_BLUEPRINT")
                    .unwrap_or_else(|_| "crates/server/blueprints/daq_default.rbl".to_string());
                let skip_blueprint = matches!(
                    blueprint_choice.to_ascii_lowercase().as_str(),
                    "none" | "off" | "skip"
                );

                if skip_blueprint {
                    println!(
                        "    - Blueprint: skipped (RERUN_BLUEPRINT={})",
                        blueprint_choice
                    );
                } else {
                    match rerun.load_blueprint_if_exists(&blueprint_choice) {
                        Ok(true) => println!("    - Blueprint: {}", blueprint_choice),
                        Ok(false) => println!(
                            "    - Blueprint: not found at {} (generate with `python crates/server/blueprints/generate_blueprints.py`)",
                            blueprint_choice
                        ),
                        Err(e) => eprintln!(
                            "Warning: Failed to load blueprint {}: {}",
                            blueprint_choice, e
                        ),
                    }
                }

                rerun.monitor_broadcast(control_server.data_sender().subscribe());
                rerun.monitor_system_state(state_broadcast_tx.subscribe());
            }
            Err(e) => {
                eprintln!("Warning: Failed to start Rerun visualization: {}", e);
            }
        }
    }

    // RunEngine was already created above (bd-si2c) - shared between RunEngineService and scripts
    // Wire DaqMetrics into RunEngineService for Prometheus observability (bd-2r60, bd-4de1)
    #[cfg(feature = "metrics")]
    let daq_metrics = {
        let m = Arc::new(crate::grpc::metrics_service::DaqMetrics::new());
        tracing::info!("DaqMetrics wired to RunEngineService for Prometheus export");
        Some(m)
    };
    let run_engine_server = {
        let svc = RunEngineServiceImpl::new(
            run_engine.clone(),
            #[cfg(feature = "metrics")]
            daq_metrics,
        );
        #[cfg(feature = "db")]
        let svc = svc.with_db(_db.clone());
        svc
    };

    // Spawn orphan-plan watchdog (bd-c9z1)
    // Detects plans stuck in Running/Paused with no client activity and aborts them.
    let _watchdog_handle = run_engine.spawn_watchdog();

    // Spawn webhook alerting task if configured (bd-kctc)
    #[cfg(feature = "alerting")]
    if let Some(notifier) = crate::alerting::WebhookNotifier::new(config.alerting.clone()) {
        let notifier = std::sync::Arc::new(notifier);
        let health_rx = registry.subscribe_health_changes();
        let doc_rx = run_engine.subscribe();
        let alert_cancel = shutdown_token.clone();
        tokio::spawn(crate::alerting::run_alerting_task(
            notifier,
            health_rx,
            doc_rx,
            alert_cancel,
        ));
        tracing::info!("Webhook alerting enabled");
    }

    // Spawn heartbeat logger for overnight run forensics (bd-7xqd)
    {
        let hb_config = crate::health::heartbeat_log::HeartbeatLogConfig {
            storage_path: storage_settings.output_directory.clone(),
            ..Default::default()
        };
        let hb_registry = registry.clone();
        let hb_engine = run_engine.clone();
        let hb_cancel = shutdown_token.clone();
        tokio::spawn(crate::health::heartbeat_log::run_heartbeat_log(
            hb_registry,
            hb_engine,
            hb_config,
            hb_cancel,
        ));
    }

    // Pause the engine if disk or process RSS crosses danger thresholds (bd-102j).
    tokio::spawn(
        crate::health::sys_monitor::ResourceGuard::new(
            health_monitor.clone(),
            run_engine.clone(),
            storage_settings.output_directory.clone(),
        )
        .run(),
    );

    // Game loop for system state broadcasting (Phase 4)
    //
    // Drivers → mpsc → game loop → broadcast → { gRPC StreamSystemState, Rerun }
    // The state poller reads all Readable devices at 10 Hz and feeds the game loop.
    let (state_update_tx, state_update_rx) = tokio::sync::mpsc::channel(256);

    let shutdown_gl = shutdown_token.clone();
    tokio::spawn(common::state_cache::run_game_loop(
        state_update_rx,
        state_broadcast_tx.clone(),
        common::state_cache::GameLoopConfig::default(),
        shutdown_gl,
    ));

    // State poller: reads all Readable devices and pushes updates to the game loop.
    let poller_registry = registry.clone();
    let poller_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        use common::state_cache::{NodeStateUpdate, NodeValue, now_ns};

        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                () = poller_shutdown.cancelled() => break,
                _ = tick.tick() => {
                    for device_info in poller_registry.list_devices() {
                        if let Some(readable) = poller_registry.get_readable(&device_info.id)
                            && let Ok(value) = readable.read().await
                        {
                            let _ = state_update_tx.try_send(NodeStateUpdate {
                                device_id: device_info.id.to_string(),
                                timestamp_ns: now_ns(),
                                value: NodeValue::Analog(value),
                                metadata: std::collections::HashMap::new(),
                            });
                        }
                    }
                }
            }
        }
    });

    let hardware_server = {
        let svc =
            HardwareServiceImpl::new(registry.clone()).with_state_broadcast(state_broadcast_tx);
        #[cfg(feature = "db")]
        let svc = svc.with_db(_db.clone());
        svc
    };
    let module_server = ModuleServiceImpl::new(registry.clone());
    let ni_daq_server = NiDaqServiceImpl::new(registry.clone());

    // Create PluginService with shared factory and registry (bd-0451)
    #[cfg(feature = "serial")]
    let plugin_server = {
        // No lock needed for registry anymore
        let factory = registry.plugin_factory();
        PluginServiceImpl::new(factory, registry.clone())
    };

    let preset_server = PresetServiceImpl::new(registry.clone(), default_preset_storage_path());
    let storage_server = StorageServiceImpl::new(ring_buffer.clone(), storage_settings);

    // Standard gRPC Health Check (grpc.health.v1)
    let standard_health_service = crate::grpc::health_service::HealthServiceImpl::new();

    // Custom System Health Monitoring (with per-device health via bd-qa36.4.3)
    let custom_health_service =
        crate::grpc::custom_health_service::HealthServiceImpl::new(health_monitor)
            .with_registry(registry.clone());

    // Wire DB state into health responses (bd-9n9k.3)
    #[cfg(feature = "db")]
    let custom_health_service = {
        let (db_available, engine, message) = if let Some(ref db) = _db {
            match db.info().await {
                Ok(info) => {
                    let engine_kind = info
                        .engine
                        .split(':')
                        .next()
                        .unwrap_or("unknown")
                        .to_string();
                    let healthy = info.healthy;
                    (
                        healthy,
                        Some(engine_kind),
                        Some(if healthy {
                            format!(
                                "schema v{}, {} drivers, {} instruments",
                                info.schema_version, info.driver_count, info.instrument_count
                            )
                        } else {
                            "Database initialized but health check failing".to_string()
                        }),
                    )
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to query DB info for health check");
                    (false, None, Some(format!("DB info query failed: {e}")))
                }
            }
        } else {
            (
                false,
                None,
                Some("Database initialization failed or not configured".to_string()),
            )
        };
        custom_health_service.with_db_state(db_available, engine, message)
    };

    // Register serving status for all services
    standard_health_service.set_serving_status("", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.ControlService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.HardwareService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.ModuleService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.PresetService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.StorageService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.RunEngineService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.HealthService", ServingStatus::Serving); // Register custom service too
    #[cfg(feature = "serial")]
    standard_health_service.set_serving_status("daq.PluginService", ServingStatus::Serving);
    // Register ConfigService status based on DB availability (bd-9n9k.3)
    #[cfg(feature = "db")]
    if _db.is_some() {
        standard_health_service.set_serving_status("daq.ConfigService", ServingStatus::Serving);
    } else {
        standard_health_service.set_serving_status("daq.ConfigService", ServingStatus::NotServing);
    }

    println!("DAQ gRPC server (with hardware) listening on {bind_addr}");
    println!("  - ControlService: script management");
    println!("  - HardwareService: direct device control");
    println!("  - HealthService: system health monitoring (bd-ergo)");
    println!("  - ModuleService: experiment modules (bd-c0ai)");
    #[cfg(feature = "serial")]
    println!("  - PluginService: YAML-defined instrument plugins (bd-0451)");
    println!("  - PresetService: configuration save/load (bd-akcm)");
    println!("  - StorageService: HDF5 data storage (bd-p6im)");

    println!("  - AuditLog: hardware-mutating calls → tracing target \"audit\" (bd-1afe.10)");

    if !grpc_settings.auth_enabled {
        eprintln!("⚠️  gRPC auth is disabled (set grpc.auth_enabled=true to require auth)");
    }

    let cors = build_cors_layer(&grpc_settings)?;
    let tls_config = build_tls_config(&grpc_settings)?;
    if tls_config.is_none() {
        eprintln!(
            "⚠️  gRPC TLS is disabled (set grpc.tls_cert_path + grpc.tls_key_path to enable)"
        );
    }

    #[cfg(feature = "serial")]
    let mut server_builder = build_grpc_server!(
        grpc_settings,
        cors.clone(),
        grpc_settings.web_ui_path.as_ref()
    );

    #[cfg(feature = "serial")]
    if let Some(tls_config) = tls_config {
        server_builder = server_builder.tls_config(tls_config)?;
    }

    #[cfg(all(feature = "serial", feature = "scripting"))]
    let server_builder = server_builder.add_service(tonic_web::enable(
        ControlServiceServer::new(control_server).max_encoding_message_size(64 * 1024 * 1024),
    ));

    #[cfg(feature = "serial")]
    let server_builder = server_builder
        .add_service(tonic_web::enable(HealthServer::new(
            standard_health_service,
        )))
        .add_service(tonic_web::enable(HealthServiceServer::new(
            custom_health_service,
        )))
        .add_service(tonic_web::enable(RunEngineServiceServer::new(
            run_engine_server.clone(),
        )))
        // HardwareService needs larger message size for camera frame streaming (16 MB)
        // gRPC-level compression (bd-rgnx.11): enable gzip for frame streaming responses
        .add_service(tonic_web::enable(
            HardwareServiceServer::new(hardware_server)
                .max_encoding_message_size(64 * 1024 * 1024)
                .accept_compressed(CompressionEncoding::Gzip)
                .send_compressed(CompressionEncoding::Gzip),
        ))
        .add_service(tonic_web::enable(NiDaqServiceServer::new(
            ni_daq_server.clone(),
        )))
        .add_service(tonic_web::enable(ModuleServiceServer::new(module_server)))
        .add_service(tonic_web::enable(PluginServiceServer::new(plugin_server)))
        .add_service(tonic_web::enable(PresetServiceServer::new(preset_server)))
        .add_service(tonic_web::enable(StorageServiceServer::new(storage_server)));

    // ConfigService — SQLite config management (bd-itsc)
    #[cfg(all(feature = "serial", feature = "db"))]
    let server_builder = if let Some(ref db) = _db {
        server_builder.add_service(tonic_web::enable(
            crate::grpc::proto::config_service_server::ConfigServiceServer::new(
                crate::grpc::config_service::ConfigServiceImpl::new(
                    db.clone(),
                    Some(registry.clone()),
                ),
            ),
        ))
    } else {
        server_builder
    };

    #[cfg(not(feature = "serial"))]
    let mut server_builder = build_grpc_server!(
        grpc_settings,
        cors.clone(),
        grpc_settings.web_ui_path.as_ref()
    );

    #[cfg(not(feature = "serial"))]
    if let Some(tls_config) = tls_config {
        server_builder = server_builder.tls_config(tls_config)?;
    }

    #[cfg(all(not(feature = "serial"), feature = "scripting"))]
    let server_builder = server_builder.add_service(tonic_web::enable(
        ControlServiceServer::new(control_server).max_encoding_message_size(64 * 1024 * 1024),
    ));

    #[cfg(not(feature = "serial"))]
    let server_builder = server_builder
        .add_service(tonic_web::enable(HealthServer::new(
            standard_health_service,
        )))
        .add_service(tonic_web::enable(HealthServiceServer::new(
            custom_health_service,
        )))
        .add_service(tonic_web::enable(RunEngineServiceServer::new(
            run_engine_server,
        )))
        // HardwareService needs larger message size for camera frame streaming (16 MB)
        // gRPC-level compression (bd-rgnx.11): enable gzip for frame streaming responses
        .add_service(tonic_web::enable(
            HardwareServiceServer::new(hardware_server)
                .max_encoding_message_size(64 * 1024 * 1024)
                .accept_compressed(CompressionEncoding::Gzip)
                .send_compressed(CompressionEncoding::Gzip),
        ))
        .add_service(tonic_web::enable(NiDaqServiceServer::new(ni_daq_server)))
        .add_service(tonic_web::enable(ModuleServiceServer::new(module_server)))
        .add_service(tonic_web::enable(PresetServiceServer::new(preset_server)))
        .add_service(tonic_web::enable(StorageServiceServer::new(storage_server)));

    // ConfigService — SQLite config management (bd-itsc)
    #[cfg(all(not(feature = "serial"), feature = "db"))]
    let server_builder = if let Some(ref db) = _db {
        server_builder.add_service(tonic_web::enable(
            crate::grpc::proto::config_service_server::ConfigServiceServer::new(
                crate::grpc::config_service::ConfigServiceImpl::new(
                    db.clone(),
                    Some(registry.clone()),
                ),
            ),
        ))
    } else {
        server_builder
    };

    // Start Prometheus metrics server if enabled (bd-v299)
    #[cfg(feature = "metrics")]
    let _metrics_handle = {
        let metrics_port: u16 = std::env::var("METRICS_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9091);
        match crate::grpc::metrics_service::start_metrics_server(metrics_port).await {
            Ok(handle) => {
                println!(
                    "  - Prometheus Metrics: http://0.0.0.0:{}/metrics (bd-v299)",
                    metrics_port
                );
                Some(handle)
            }
            Err(e) => {
                eprintln!("⚠️  Failed to start metrics server: {}", e);
                None
            }
        }
    };

    // SAFETY (bd-1afe.12): Use serve_with_shutdown so the daemon can trigger graceful
    // server termination. When shutdown_token is cancelled, the server stops accepting
    // new connections and drains existing RPCs. The reaper task in DaqServer concurrently
    // aborts running scripts to prevent commands on disconnected hardware.
    if grpc_settings.web_ui_path.is_some() {
        println!(
            "🚀 Serving integrated WASM UI from {} on port {}",
            grpc_settings.web_ui_path.as_ref().unwrap().display(),
            bind_addr.port()
        );
    }

    server_builder
        .serve_with_shutdown(bind_addr, shutdown_token.cancelled())
        .await?;

    Ok(())
}
