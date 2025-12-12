//! Connection settings panel.

use eframe::egui;

/// Connection panel state
#[derive(Default)]
pub struct ConnectionPanel {
    /// Show advanced settings
    pub show_advanced: bool,
}

impl ConnectionPanel {
    /// Render the connection panel
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connection Settings");
        ui.separator();
        
        ui.checkbox(&mut self.show_advanced, "Show advanced settings");
        
        if self.show_advanced {
            ui.group(|ui| {
                ui.label("Advanced connection options:");
                ui.label("• Timeout settings");
                ui.label("• Retry configuration");
                ui.label("• TLS settings");
            });
        }
    }
}
