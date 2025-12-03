//! Native egui/eframe GUI for rust-daq.
//!
//! This provides a lightweight, cross‑platform control panel that talks to the
//! headless daemon over gRPC. It is intentionally minimal but gives you:
//! - Connection to a daemon (localhost:50051 by default)
//! - Device discovery via `HardwareService`
//! - Live scalar reads on demand
//!
//! Build (with networking enabled):
//! ```bash
//! cargo run --features networking --bin rust_daq_gui_egui
//! ```

#![cfg(feature = "networking")]

use eframe::{egui, epi};
use rust_daq::grpc::{
    HardwareServiceClient, ListDevicesRequest, ReadValueRequest, DeviceInfo as ProtoDeviceInfo,
};
use std::time::Instant;
use tonic::transport::Channel;

#[derive(Clone)]
struct DeviceRow {
    id: String,
    name: String,
    driver_type: String,
    last_value: Option<f64>,
    last_units: String,
    last_updated: Option<Instant>,
    error: Option<String>,
}

struct DaqGuiApp {
    daemon_addr: String,
    status_line: String,
    devices: Vec<DeviceRow>,
}

impl DaqGuiApp {
    fn new() -> Self {
        Self {
            daemon_addr: "http://127.0.0.1:50051".to_string(),
            status_line: String::from("Not connected. Set daemon address and click Refresh."),
            devices: Vec::new(),
        }
    }

    async fn make_client(&self) -> anyhow::Result<HardwareServiceClient<Channel>> {
        let addr = if self.daemon_addr.starts_with("http") {
            self.daemon_addr.clone()
        } else {
            format!("http://{}", self.daemon_addr)
        };
        let channel = Channel::from_shared(addr)?.connect().await?;
        Ok(HardwareServiceClient::new(channel))
    }

    async fn refresh_devices_async(&mut self) -> anyhow::Result<()> {
        let mut client = self.make_client().await?;
        let response = client
            .list_devices(ListDevicesRequest {
                capability_filter: None,
            })
            .await?;
        let body = response.into_inner();

        self.devices = body
            .devices
            .into_iter()
            .map(|d: ProtoDeviceInfo| DeviceRow {
                id: d.id,
                name: d.name,
                driver_type: d.driver_type,
                last_value: None,
                last_units: String::new(),
                last_updated: None,
                error: None,
            })
            .collect();
        Ok(())
    }

    async fn read_once_async(
        &mut self,
        device_index: usize,
    ) -> anyhow::Result<()> {
        if device_index >= self.devices.len() {
            return Ok(());
        }
        let device_id = self.devices[device_index].id.clone();
        let mut client = self.make_client().await?;
        let response = client
            .read_value(ReadValueRequest { device_id })
            .await?;
        let body = response.into_inner();

        if device_index < self.devices.len() {
            let row = &mut self.devices[device_index];
            if body.success {
                row.last_value = Some(body.value);
                row.last_units = body.units;
                row.last_updated = Some(Instant::now());
                row.error = None;
            } else {
                row.error = Some(body.error_message);
            }
        }

        Ok(())
    }
}

impl epi::App for DaqGuiApp {
    fn name(&self) -> &str {
        "rust-daq egui GUI"
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut epi::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("rust-daq egui GUI");
            ui.horizontal(|ui| {
                ui.label("Daemon:");
                ui.text_edit_singleline(&mut self.daemon_addr);
                if ui.button("Refresh devices").clicked() {
                    self.status_line = "Refreshing devices…".to_string();
                    let mut clone = self.clone_for_task();
                    egui::Context::spawn(ctx.clone(), async move {
                        if let Err(e) = clone.refresh_devices_async().await {
                            clone.status_line = format!("Refresh failed: {e}");
                        } else {
                            clone.status_line =
                                format!("Loaded {} devices", clone.devices.len());
                        }
                        ctx.request_repaint();
                    });
                }
            });
            ui.label(&self.status_line);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.devices.is_empty() {
                ui.label("No devices loaded. Click \"Refresh devices\".");
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("device_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.heading("ID");
                        ui.heading("Name");
                        ui.heading("Driver");
                        ui.heading("Last value");
                        ui.heading("");
                        ui.end_row();

                        for (idx, row) in self.devices.iter_mut().enumerate() {
                            ui.label(&row.id);
                            ui.label(&row.name);
                            ui.label(&row.driver_type);

                            if let Some(v) = row.last_value {
                                ui.label(format!("{v:.4} {}", row.last_units));
                            } else if let Some(err) = &row.error {
                                ui.colored_label(egui::Color32::RED, err);
                            } else {
                                ui.label("-");
                            }

                            if ui.button("Read").clicked() {
                                let mut clone = self.clone_for_task();
                                egui::Context::spawn(ctx.clone(), async move {
                                    let _ = clone.read_once_async(idx).await;
                                    ctx.request_repaint();
                                });
                            }

                            ui.end_row();
                        }
                    });
            });
        });
    }
}

impl DaqGuiApp {
    fn clone_for_task(&self) -> Self {
        Self {
            daemon_addr: self.daemon_addr.clone(),
            status_line: self.status_line.clone(),
            devices: self.devices.clone(),
        }
    }
}

pub fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        initial_window_size: Some(egui::vec2(900.0, 600.0)),
        ..Default::default()
    };

    eframe::run_native(
        "rust-daq egui GUI",
        native_options,
        Box::new(|_cc| Box::new(DaqGuiApp::new())),
    )
}


