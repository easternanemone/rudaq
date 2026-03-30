//! Connection management - connect, disconnect, health checks, auto-connect.

use super::*;

impl DaqApp {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn connect(&mut self) {
        if self.connection.is_busy() {
            return;
        }

        // Validate and normalize the address input
        match DaemonAddress::parse(&self.address_input, AddressSource::UserInput) {
            Ok(addr) => {
                self.daemon_address = addr;
                self.address_error = None;
            }
            Err(e) => {
                self.address_error = Some(e.to_string());
                self.logging_panel
                    .error("Connection", &format!("Invalid address: {}", e));
                return;
            }
        }

        self.logging_panel.connection_status = LogConnectionStatus::Connecting;
        self.logging_panel.info(
            "Connection",
            &format!(
                "Connecting to {} ({})",
                self.daemon_address,
                self.daemon_address.source().label()
            ),
        );

        self.connection
            .connect(self.daemon_address.clone(), &self.runtime);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn disconnect(&mut self) {
        self.client = None;
        self.daemon_version = None;
        self.connection.disconnect();
        self.logging_panel.connection_status = LogConnectionStatus::Disconnected;
        self.logging_panel
            .info("Connection", "Disconnected from daemon");
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn switch_daemon_mode(&mut self, mode: DaemonMode) {
        tracing::info!("Switching daemon mode to: {}", mode.label());

        // Stop existing daemon before switching modes.
        if let Some(ref mut launcher) = self.daemon_launcher {
            launcher.stop();
        }
        self.daemon_launcher = None;

        // Disconnect current connection
        self.disconnect();

        // Update daemon mode
        self.daemon_mode = mode.clone();

        // Update address
        if let Ok(addr) = DaemonAddress::parse(&mode.daemon_url(), AddressSource::Default) {
            self.daemon_address = addr;
            self.address_input = self.daemon_address.original().to_string();
        }

        // Start new daemon if needed
        if mode.should_auto_start() {
            let port = mode.port().unwrap_or(50051);
            let mut launcher = DaemonLauncher::new(port);
            if let Err(e) = launcher.start_with_mode(&mode) {
                self.logging_panel
                    .error("Daemon", &format!("Failed to start: {}", e));
            }
            self.daemon_launcher = Some(launcher);
            self.auto_connect_state = AutoConnectState::WaitingForDaemon {
                since: Instant::now(),
            };
        } else {
            // For remote mode, try to connect immediately
            self.auto_connect_state = AutoConnectState::ReadyToConnect;
        }

        self.logging_panel
            .info("Daemon", &format!("Switched to {} mode", mode.label()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn poll_connect_results(&mut self, ctx: &egui::Context) {
        // Poll connection manager for results
        if let Some((client, daemon_version)) =
            self.connection.poll(&self.runtime, &self.daemon_address)
        {
            self.client = Some(client);
            self.daemon_version.clone_from(&daemon_version);
            self.logging_panel.connection_status = LogConnectionStatus::Connected;
            self.logging_panel.info(
                "Connection",
                &format!(
                    "Connected to {} ({})",
                    self.daemon_address.as_str(),
                    self.daemon_address.source().label()
                ),
            );

            // Log version info
            match daemon_version {
                Some(ref daemon_ver) => {
                    tracing::info!(
                        "Daemon version: {}, GUI version: {}",
                        daemon_ver,
                        self.gui_version
                    );
                    if daemon_ver != &self.gui_version {
                        tracing::warn!(
                            "Version mismatch detected! Daemon: {}, GUI: {}. Some features may not work correctly.",
                            daemon_ver,
                            self.gui_version
                        );
                    }
                }
                None => {
                    tracing::warn!("Connected but failed to get daemon version");
                }
            }
        }

        // Update logging panel status based on connection state
        match self.connection.state() {
            ConnectionState::Error { .. } => {
                if self.logging_panel.connection_status != LogConnectionStatus::Error {
                    self.logging_panel.connection_status = LogConnectionStatus::Error;
                    if let Some(err) = self.connection.state().error_message() {
                        self.logging_panel
                            .error("Connection", &format!("Connection failed: {}", err));
                    }
                }
            }
            ConnectionState::Reconnecting { attempt, .. } => {
                self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                if let Some(err) = self.connection.state().error_message() {
                    self.logging_panel.warn(
                        "Connection",
                        &format!("Reconnecting (attempt {}): {}", attempt, err),
                    );
                }
            }
            ConnectionState::CircuitBreaker { last_error, .. } => {
                if self.logging_panel.connection_status != LogConnectionStatus::CircuitBreaker {
                    self.logging_panel.connection_status = LogConnectionStatus::CircuitBreaker;
                    self.logging_panel.warn(
                        "Connection",
                        &format!("Circuit breaker open: {}", last_error),
                    );
                }
            }
            ConnectionState::HalfOpen { .. } => {
                self.logging_panel.connection_status = LogConnectionStatus::Connecting;
            }
            _ => {}
        }

        // Request repaint if connection attempt is in progress
        if self.connection.is_busy() || self.connection.seconds_until_retry().is_some() {
            ctx.request_repaint();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn maybe_spawn_health_check(&mut self) {
        if !self.connection.should_health_check() {
            return;
        }
        let Some(ref client) = self.client else {
            return;
        };

        // Mark health check as started
        self.connection.mark_health_check_started();

        // Clone what we need for the async task
        let mut client = client.clone();
        let tx = self.health_tx.clone();

        self.runtime.spawn(async move {
            // Measure RTT for the health check (bd-j3xz.3.3)
            let start = crate::time::Instant::now();
            match client.health_check().await {
                Ok(()) => {
                    let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let _ = tx.send(HealthCheckResult::Success { rtt_ms }).await;
                }
                Err(e) => {
                    let _ = tx.send(HealthCheckResult::Failed(e.to_string())).await;
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn poll_health_checks(&mut self) {
        while let Ok(result) = self.health_rx.try_recv() {
            match result {
                HealthCheckResult::Success { rtt_ms } => {
                    self.connection.record_health_success(rtt_ms);
                }
                HealthCheckResult::Failed(error) => {
                    let should_reconnect = self.connection.record_health_failure(&error);

                    if should_reconnect {
                        // Clear client - connection is stale
                        self.client = None;
                        self.daemon_version = None;
                        self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                        self.logging_panel.warn(
                            "Connection",
                            &format!("Connection lost ({}), reconnecting...", error),
                        );

                        // Trigger reconnect
                        self.connection
                            .trigger_health_reconnect(self.daemon_address.clone(), &self.runtime);
                    }
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn update_connection_diagnostics(&mut self) {
        let health_status = self.connection.health_status();

        // Calculate relative times
        let secs_since_last_success = health_status
            .last_success
            .map(|t| t.elapsed().as_secs_f64());
        let secs_since_last_error = health_status
            .last_error_at
            .map(|t| t.elapsed().as_secs_f64());

        self.logging_panel.connection_diagnostics = ConnectionDiagnostics {
            last_rtt_ms: health_status.last_rtt_ms,
            total_errors: health_status.total_errors,
            secs_since_last_error,
            last_error_message: health_status.last_error_message.clone(),
            secs_since_last_success,
            consecutive_failures: health_status.consecutive_failures,
        };
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn process_auto_connect(&mut self, ctx: &egui::Context) {
        use std::time::Duration;

        match &self.auto_connect_state {
            AutoConnectState::WaitingForDaemon { since } => {
                let elapsed = since.elapsed();

                // Check if daemon process has started
                if let Some(ref mut launcher) = self.daemon_launcher {
                    if launcher.is_running() && elapsed > Duration::from_millis(500) {
                        // Give daemon time to start listening
                        tracing::info!("Daemon is running, initiating auto-connect");
                        self.auto_connect_state = AutoConnectState::ReadyToConnect;
                    } else if elapsed > Duration::from_secs(10) {
                        // Timeout - daemon didn't start
                        tracing::error!("Timeout waiting for daemon to start");
                        self.auto_connect_state = AutoConnectState::Skipped;
                        self.logging_panel
                            .error("Daemon", "Timeout waiting for daemon to start");
                    }
                } else {
                    // No launcher but in WaitingForDaemon - shouldn't happen, skip
                    self.auto_connect_state = AutoConnectState::Skipped;
                }
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            AutoConnectState::ReadyToConnect => {
                if !self.connection.is_busy() {
                    tracing::info!("Auto-connecting to daemon at {}", self.daemon_address);
                    self.connect();
                    self.auto_connect_state = AutoConnectState::Complete;
                }
            }
            AutoConnectState::Complete | AutoConnectState::Skipped => {
                // No action needed
            }
        }
    }

    pub(super) fn on_connection_established(&mut self) {
        tracing::info!("Connection established - triggering panel refreshes");

        // Reset panels to force them to refresh their data
        // This clears cached data and triggers new loads on next render
        self.scripts_panel = ScriptsPanel::default();
        self.modules_panel = ModulesPanel::default();
        self.storage_panel = StoragePanel::default();
        self.run_history_panel = RunHistoryPanel::default();

        // Reset InstrumentManagerPanel to trigger auto-refresh on reconnect
        // (keeps panel state like selected device, but clears device list and refresh flag)
        self.instrument_manager_panel.reset_refresh_state();

        self.logging_panel
            .info("Connection", "Connected - panels will refresh data");

        // Update browser tab title to show connected daemon
        #[cfg(target_arch = "wasm32")]
        set_page_title(&format!(
            "DAQ Panel — Connected ({})",
            self.current_daemon_url()
        ));

        // Start device reconciliation to validate persisted panels
        self.start_device_reconcile();
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn detect_connection_transitions(&mut self) {
        let is_connected = self.connection.state().is_connected();

        if is_connected && !self.was_connected {
            // Just connected - trigger panel refreshes
            self.on_connection_established();
        }

        self.was_connected = is_connected;
    }

    pub(super) fn current_daemon_url(&self) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.daemon_address.to_string()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.wasm_connection.url_input.trim().to_string()
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn wasm_connect(&mut self, ctx: &egui::Context) {
        let url = self.wasm_connection.url_input.trim().to_string();
        if url.is_empty() {
            self.wasm_connection.status = "Enter a server URL".to_string();
            return;
        }

        // Clear stale state from any previous connection (fixes beefcake-48ad / bd-0zu5)
        self.wasm_disconnect();

        self.wasm_connection.connecting = true;
        self.wasm_connection.status = "Connecting...".to_string();

        // connect_web is synchronous — creates gRPC-web channel immediately
        let client = client::DaqClient::connect_web(&url);
        self.client = Some(client);
        self.wasm_connection.connecting = false;
        self.wasm_connection.status = "Connected".to_string();
        self.was_connected = true;

        self.logging_panel
            .info("Connection", &format!("Connected to {}", url));

        // Reset all panels and trigger device reconciliation so stale data from a
        // previous daemon is discarded (bd-0zu5: reconnect without page reload).
        self.on_connection_established();

        ctx.request_repaint();
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn wasm_disconnect(&mut self) {
        self.client = None;
        self.daemon_version = None;
        self.device_panel_info.clear();
        self.invalidate_all_panel_widgets();
        self.was_connected = false;
        self.device_reconcile_epoch += 1;
        // Cancel any in-flight connection attempt
        self.wasm_connection.connecting = false;
        self.wasm_connection.connect_rx = None;
        // Remove device control tabs from the dock
        if let Some(ref mut dock) = self.dock_state {
            dock.retain_tabs(|tab| !matches!(tab, Panel::DeviceControl { .. }));
        }
        // Update browser tab title to show disconnected state
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            doc.set_title("DAQ Panel — Disconnected");
        }
    }
}
