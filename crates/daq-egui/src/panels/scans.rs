//! Scans panel - configure and monitor multi-axis scans.

use eframe::egui;
use tokio::runtime::Runtime;

use crate::client::DaqClient;

/// Scans panel state
#[derive(Default)]
pub struct ScansPanel {
    /// Cached scan list
    scans: Vec<daq_proto::daq::ScanStatus>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
}

impl ScansPanel {
    /// Render the scans panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        ui.heading("Scans");
        
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh").clicked() {
                self.refresh(client, runtime);
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
        
        ui.add_space(8.0);
        
        if self.scans.is_empty() {
            ui.label("No scans found. Create scans via gRPC or CLI.");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("scans_list")
                .show(ui, |ui| {
                    for scan in &self.scans {
                        self.render_scan_card(ui, scan);
                    }
                });
        }
    }
    
    /// Render a single scan as a card
    fn render_scan_card(&self, ui: &mut egui::Ui, scan: &daq_proto::daq::ScanStatus) {
        let state_color = match scan.state {
            1 => egui::Color32::GRAY,    // CREATED
            2 => egui::Color32::YELLOW,  // RUNNING
            3 => egui::Color32::BLUE,    // PAUSED
            4 => egui::Color32::GREEN,   // COMPLETED
            5 => egui::Color32::GRAY,    // STOPPED
            6 => egui::Color32::RED,     // ERROR
            _ => egui::Color32::WHITE,
        };
        
        let state_name = match scan.state {
            1 => "Created",
            2 => "Running",
            3 => "Paused",
            4 => "Completed",
            5 => "Stopped",
            6 => "Error",
            _ => "Unknown",
        };
        
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.colored_label(state_color, "●");
                ui.label(format!("{} - {}", scan.scan_id, state_name));
            });
            
            // Progress bar
            if scan.total_points > 0 {
                let progress = scan.current_point as f32 / scan.total_points as f32;
                let progress_bar = egui::ProgressBar::new(progress)
                    .text(format!("{}/{} points ({:.1}%)", 
                        scan.current_point, 
                        scan.total_points,
                        scan.progress_percent
                    ));
                ui.add(progress_bar);
            }
            
            // Error message
            if !scan.error_message.is_empty() {
                ui.colored_label(egui::Color32::RED, &scan.error_message);
            }
        });
    }
    
    /// Refresh the scan list
    fn refresh(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.error = None;
        self.status = None;
        
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };
        
        let mut client = client.clone();
        match runtime.block_on(client.list_scans()) {
            Ok(scans) => {
                self.scans = scans;
                self.last_refresh = Some(std::time::Instant::now());
                self.status = Some(format!("Loaded {} scans", self.scans.len()));
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
    }
}
