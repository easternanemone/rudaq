//! Devices panel - list and control hardware devices.

use eframe::egui;
use tokio::runtime::Runtime;

use crate::client::DaqClient;

/// Cached device information
#[derive(Clone)]
struct DeviceCache {
    info: daq_proto::daq::DeviceInfo,
    state: Option<daq_proto::daq::DeviceStateResponse>,
}

/// Pending action to execute after UI rendering
enum PendingAction {
    Refresh,
    MoveAbsolute { device_id: String, value: f64 },
    MoveRelative { device_id: String, value: f64 },
    ReadValue { device_id: String },
}

/// Devices panel state
#[derive(Default)]
pub struct DevicesPanel {
    /// Cached device list
    devices: Vec<DeviceCache>,
    /// Selected device ID
    selected_device: Option<String>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
    /// Move target position
    move_target: f64,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Pending action to execute
    pending_action: Option<PendingAction>,
}

impl DevicesPanel {
    /// Render the devices panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Clear pending action
        self.pending_action = None;
        
        ui.heading("Devices");
        
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh").clicked() {
                self.pending_action = Some(PendingAction::Refresh);
            }
            
            if let Some(last) = self.last_refresh {
                let elapsed = last.elapsed();
                ui.label(format!("Updated {}s ago", elapsed.as_secs()));
            }
        });
        
        ui.separator();
        
        // Show error/status messages
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }
        if let Some(status) = &self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }
        
        // Clone selected device for rendering (avoids borrow issues)
        let selected_device = self.selected_device.as_ref()
            .and_then(|id| self.devices.iter().find(|d| &d.info.id == id).cloned());
        
        // Device list and details in two columns
        ui.columns(2, |columns| {
            // Left column: device list
            columns[0].heading("Device List");
            columns[0].separator();
            
            if self.devices.is_empty() {
                columns[0].label("No devices found. Click Refresh to load.");
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("device_list")
                    .show(&mut columns[0], |ui| {
                        for device in &self.devices {
                            let selected = self.selected_device.as_ref() == Some(&device.info.id);
                            let label = format!(
                                "{} ({})",
                                device.info.name,
                                device.info.driver_type
                            );
                            
                            if ui.selectable_label(selected, &label).clicked() {
                                self.selected_device = Some(device.info.id.clone());
                            }
                        }
                    });
            }
            
            // Right column: device details
            columns[1].heading("Device Details");
            columns[1].separator();
            
            if let Some(device) = &selected_device {
                self.render_device_details(&mut columns[1], device);
            } else {
                columns[1].label("Select a device to view details");
            }
        });
        
        // Execute pending action after UI is done borrowing self
        if let Some(action) = self.pending_action.take() {
            self.execute_action(action, client, runtime);
        }
    }
    
    /// Render details for a selected device
    fn render_device_details(&mut self, ui: &mut egui::Ui, device: &DeviceCache) {
        let info = &device.info;
        
        ui.group(|ui| {
            ui.heading(&info.name);
            ui.label(format!("ID: {}", info.id));
            ui.label(format!("Driver: {}", info.driver_type));
            
            ui.separator();
            ui.label("Capabilities:");
            ui.horizontal(|ui| {
                if info.is_movable { ui.label("📐 Movable"); }
                if info.is_readable { ui.label("📖 Readable"); }
                if info.is_triggerable { ui.label("⚡ Triggerable"); }
                if info.is_frame_producer { ui.label("📷 Camera"); }
                if info.is_exposure_controllable { ui.label("⏱ Exposure"); }
                if info.is_shutter_controllable { ui.label("🚪 Shutter"); }
                if info.is_wavelength_tunable { ui.label("🌈 Wavelength"); }
                if info.is_emission_controllable { ui.label("💡 Emission"); }
            });
        });
        
        // State display
        if let Some(state) = &device.state {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Current State");
                ui.label(format!("Online: {}", if state.online { "✅" } else { "❌" }));
                
                if let Some(pos) = state.position {
                    ui.label(format!("Position: {:.4}", pos));
                }
                if let Some(reading) = state.last_reading {
                    ui.label(format!("Last reading: {:.4}", reading));
                }
                if let Some(armed) = state.armed {
                    ui.label(format!("Armed: {}", armed));
                }
                if let Some(exposure) = state.exposure_ms {
                    ui.label(format!("Exposure: {:.2} ms", exposure));
                }
            });
        }
        
        // Control section for movable devices
        if info.is_movable {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Motion Control");
                
                ui.horizontal(|ui| {
                    ui.label("Target:");
                    ui.add(egui::DragValue::new(&mut self.move_target)
                        .speed(0.1)
                        .suffix(" units"));
                });
                
                ui.horizontal(|ui| {
                    if ui.button("Move Absolute").clicked() {
                        self.pending_action = Some(PendingAction::MoveAbsolute {
                            device_id: info.id.clone(),
                            value: self.move_target,
                        });
                    }
                    if ui.button("Move Relative").clicked() {
                        self.pending_action = Some(PendingAction::MoveRelative {
                            device_id: info.id.clone(),
                            value: self.move_target,
                        });
                    }
                });
                
                // Quick move buttons
                ui.horizontal(|ui| {
                    for delta in [-10.0, -1.0, -0.1, 0.1, 1.0, 10.0] {
                        let label = if delta > 0.0 { format!("+{}", delta) } else { format!("{}", delta) };
                        if ui.button(label).clicked() {
                            self.pending_action = Some(PendingAction::MoveRelative {
                                device_id: info.id.clone(),
                                value: delta,
                            });
                        }
                    }
                });
            });
        }
        
        // Read button for readable devices
        if info.is_readable {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Read Value");
                if ui.button("📖 Read Now").clicked() {
                    self.pending_action = Some(PendingAction::ReadValue {
                        device_id: info.id.clone(),
                    });
                }
            });
        }
    }
    
    /// Execute a pending action
    fn execute_action(
        &mut self,
        action: PendingAction,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        match action {
            PendingAction::Refresh => self.refresh_devices(client, runtime),
            PendingAction::MoveAbsolute { device_id, value } => {
                self.move_device(client, runtime, &device_id, value, false);
            }
            PendingAction::MoveRelative { device_id, value } => {
                self.move_device(client, runtime, &device_id, value, true);
            }
            PendingAction::ReadValue { device_id } => {
                self.read_device(client, runtime, &device_id);
            }
        }
    }
    
    /// Refresh the device list from the daemon
    fn refresh_devices(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.error = None;
        self.status = None;
        
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };
        
        let mut client = client.clone();
        match runtime.block_on(async {
            let devices = client.list_devices().await?;
            let mut cached = Vec::new();
            
            for info in devices {
                let state = client.get_device_state(&info.id).await.ok();
                cached.push(DeviceCache { info, state });
            }
            
            Ok::<_, anyhow::Error>(cached)
        }) {
            Ok(devices) => {
                self.devices = devices;
                self.last_refresh = Some(std::time::Instant::now());
                self.status = Some(format!("Loaded {} devices", self.devices.len()));
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
    }
    
    /// Move a device
    fn move_device(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        value: f64,
        relative: bool,
    ) {
        self.error = None;
        self.status = None;
        
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };
        
        let mut client = client.clone();
        let device_id = device_id.to_string();
        
        let result = runtime.block_on(async {
            if relative {
                client.move_relative(&device_id, value).await
            } else {
                client.move_absolute(&device_id, value).await
            }
        });
        
        match result {
            Ok(response) => {
                if response.success {
                    self.status = Some(format!("Moved to {:.4}", response.final_position));
                } else {
                    self.error = Some(response.error_message);
                }
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
    }
    
    /// Read value from a device
    fn read_device(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        self.error = None;
        self.status = None;
        
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };
        
        let mut client = client.clone();
        let device_id = device_id.to_string();
        
        match runtime.block_on(client.read_value(&device_id)) {
            Ok(response) => {
                if response.success {
                    self.status = Some(format!("Value: {:.4} {}", response.value, response.units));
                } else {
                    self.error = Some(response.error_message);
                }
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
    }
}
