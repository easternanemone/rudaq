//! Device reconciliation and panel management.

use super::*;

/// Additional DaqApp methods in a separate impl block (split for helper functions)
impl DaqApp {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn set_control_panel_layout_mode(&mut self, mode: ControlPanelLayoutMode) {
        if self.control_panel_layout_mode == mode {
            return;
        }
        self.control_panel_layout_mode = mode;
        self.invalidate_all_panel_widgets();
        self.logging_panel.info(
            "UI",
            &format!("Control panel layout set to {}", mode.label()),
        );
    }

    pub(super) fn invalidate_all_panel_widgets(&mut self) {
        // Unified panel store (bd-1xi2p.8 D2): clearing drops every panel's
        // widget + grpc_config cache + command_widgets in one call.
        self.panel_controllers.clear();
    }

    /// Remove all state associated with a device control panel.
    ///
    /// Returns the removed DevicePanelInfo if the panel existed, None otherwise.
    /// Used for cleanup when panels are closed or during app shutdown.
    pub(crate) fn remove_panel_data(&mut self, id: usize) -> Option<DevicePanelInfo> {
        self.panel_controllers.remove(&id);
        self.device_panel_info.remove(&id)
    }

    pub(super) fn start_device_reconcile(&mut self) {
        let Some(ref client) = self.client else {
            return;
        };

        // Increment epoch to invalidate stale results
        self.device_reconcile_epoch = self.device_reconcile_epoch.wrapping_add(1);
        let epoch = self.device_reconcile_epoch;
        let daemon_url = self.current_daemon_url();

        // Clone what we need for async task
        let mut client = client.clone();
        let tx = self.device_reconcile_tx.clone();

        self.runtime.spawn(async move {
            match client.list_devices().await {
                Ok(devices) => {
                    let _ = tx
                        .send(DeviceReconcileMsg::Ok {
                            epoch,
                            daemon_url,
                            devices,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(DeviceReconcileMsg::Err {
                            epoch,
                            daemon_url,
                            error: e.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    pub(super) fn poll_device_reconcile(&mut self) {
        while let Ok(msg) = self.device_reconcile_rx.try_recv() {
            match msg {
                DeviceReconcileMsg::Ok {
                    epoch,
                    daemon_url,
                    devices,
                } => {
                    // Ignore stale results
                    let stale = epoch != self.device_reconcile_epoch;
                    #[cfg(not(target_arch = "wasm32"))]
                    let stale = stale || daemon_url != self.daemon_address.to_string();
                    let _ = &daemon_url; // suppress unused warning on WASM
                    if stale {
                        tracing::debug!(
                            epoch,
                            current_epoch = self.device_reconcile_epoch,
                            "Ignoring stale device reconciliation result"
                        );
                        continue;
                    }

                    self.apply_device_reconcile(devices);
                }
                DeviceReconcileMsg::Err {
                    epoch,
                    daemon_url,
                    error,
                } => {
                    // Ignore stale errors
                    let stale = epoch != self.device_reconcile_epoch;
                    #[cfg(not(target_arch = "wasm32"))]
                    let stale = stale || daemon_url != self.daemon_address.to_string();
                    let _ = &daemon_url;
                    if stale {
                        continue;
                    }

                    tracing::warn!("Device reconciliation failed: {}", error);
                    // Mark all panels as Pending (will retry on next connection)
                    for info in self.device_panel_info.values_mut() {
                        info.availability = DeviceAvailability::Pending;
                    }
                }
            }
        }
    }

    pub(super) fn apply_device_reconcile(&mut self, devices: Vec<DeviceInfo>) {
        let device_map: HashMap<String, DeviceInfo> =
            devices.into_iter().map(|d| (d.id.clone(), d)).collect();

        // Collect panel migrations to avoid borrowing conflicts
        let mut migrations: Vec<(usize, DevicePanelKind)> = Vec::new();

        for (panel_id, panel_info) in &mut self.device_panel_info {
            let device_id = &panel_info.device_info.id;

            if let Some(daemon_device) = device_map.get(device_id) {
                // Device found on daemon
                panel_info.availability = DeviceAvailability::Available;

                // Check if capabilities changed (requires panel migration)
                let new_kind = panel_kind_for_device(daemon_device);
                if new_kind != panel_info.kind {
                    tracing::info!(
                        panel_id,
                        device_id,
                        old_kind = ?panel_info.kind,
                        new_kind = ?new_kind,
                        "Device capabilities changed - migrating panel"
                    );

                    // Update kind and device info
                    panel_info.kind = new_kind;
                    panel_info.device_info = daemon_device.clone();

                    // Defer migration to avoid borrow conflicts
                    migrations.push((*panel_id, new_kind));
                } else {
                    // Just update device info (metadata may have changed)
                    panel_info.device_info = daemon_device.clone();
                }
            } else {
                // Device not found on daemon
                panel_info.availability = DeviceAvailability::Missing;
                tracing::warn!(
                    panel_id,
                    device_id,
                    "Device panel references missing device"
                );
            }
        }

        // Apply panel migrations
        for (panel_id, _new_kind) in migrations {
            self.invalidate_panel_widget(panel_id);
        }

        // Auto-close panels for devices no longer on the daemon.
        let stale_panels: Vec<(usize, String)> = self
            .device_panel_info
            .iter()
            .filter(|(_, info)| info.availability == DeviceAvailability::Missing)
            .map(|(id, info)| (*id, info.device_info.id.clone()))
            .collect();
        for (panel_id, device_id) in stale_panels {
            tracing::info!(panel_id, device_id, "Auto-closing panel for missing device");
            self.ui_actions
                .push(UiAction::CloseDevicePanel { id: panel_id });
        }
    }

    pub(super) fn invalidate_panel_widget(&mut self, panel_id: usize) {
        // Unified panel store (bd-1xi2p.8 D2): dropping the controller entry
        // clears the widget, the gRPC config decision cache, and any command
        // widgets in a single call. `device_panel_info` is deliberately
        // preserved — this helper handles capability-change migrations, not
        // panel removal.
        self.panel_controllers.remove(&panel_id);
    }
}
