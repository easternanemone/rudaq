//! Smart Streaming sequence editor for PVCAM cameras (bd-cdh5.4).

use eframe::egui;

pub struct SmartStreamEditor {
    /// Exposure sequence in milliseconds
    pub exposures: Vec<u32>,
    /// Buffer for adding a new exposure
    pub new_exposure: u32,
}

impl SmartStreamEditor {
    pub fn new() -> Self {
        Self {
            exposures: vec![10, 20, 50, 100], // Example sequence
            new_exposure: 10,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, _device_id: &str) -> bool {
        let mut changed = false;

        ui.heading("Smart Streaming Sequence");
        ui.label("Define a sequence of exposure times to be uploaded to the camera FPGA.");

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.new_exposure).suffix(" ms"));
            if ui.button("➕ Add").clicked() {
                self.exposures.push(self.new_exposure);
                changed = true;
            }
            if ui.button("🗑 Clear All").clicked() {
                self.exposures.clear();
                changed = true;
            }
        });

        ui.separator();

        let mut to_remove = None;
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for (i, exposure) in self.exposures.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}: ", i + 1));
                        if ui.add(egui::DragValue::new(exposure).suffix(" ms")).changed() {
                            changed = true;
                        }
                        if ui.button("❌").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
            });

        if let Some(i) = to_remove {
            self.exposures.remove(i);
            changed = true;
        }

        ui.separator();

        let upload_clicked = ui.button("🚀 Upload to Hardware").clicked();

        // bd-ota0: Return true if any change occurred OR upload was requested
        // Previously the 'changed' variable was computed but never returned
        changed || upload_clicked
    }
}
