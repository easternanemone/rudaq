//! Demo application showing DoubleSlider widget usage.
//!
//! Run with: cargo run -p daq-egui --example double_slider_demo --features standalone

#![cfg(feature = "standalone")]

use daq_egui::widgets::DoubleSlider;
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 400.0])
            .with_title("Double Slider Demo"),
        ..Default::default()
    };

    eframe::run_native(
        "Double Slider Demo",
        options,
        Box::new(|_cc| Ok(Box::new(DemoApp::default()))),
    )
}

#[derive(Default)]
struct DemoApp {
    // Basic range
    basic_range: (f64, f64),
    basic_bounds: (f64, f64),

    // Stepped range
    stepped_range: (f64, f64),
    stepped_bounds: (f64, f64),

    // Custom formatted range
    custom_range: (f64, f64),
    custom_bounds: (f64, f64),

    // Narrow range (for precision)
    narrow_range: (f64, f64),
    narrow_bounds: (f64, f64),
}

impl DemoApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            basic_range: (25.0, 75.0),
            basic_bounds: (0.0, 100.0),
            stepped_range: (20.0, 80.0),
            stepped_bounds: (0.0, 100.0),
            custom_range: (400.0, 800.0),
            custom_bounds: (350.0, 1000.0),
            narrow_range: (0.25, 0.75),
            narrow_bounds: (0.0, 1.0),
        }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Double Slider Widget Examples");
            ui.add_space(20.0);

            // Basic example
            ui.group(|ui| {
                ui.label("Basic Range Selection:");
                ui.add(DoubleSlider::new(
                    &mut self.basic_range,
                    &mut self.basic_bounds,
                ));
                ui.label(format!(
                    "Range: [{:.2}, {:.2}]",
                    self.basic_range.0, self.basic_range.1
                ));
            });
            ui.add_space(10.0);

            // Stepped example
            ui.group(|ui| {
                ui.label("Stepped Range (step = 5.0):");
                ui.add(
                    DoubleSlider::new(&mut self.stepped_range, &mut self.stepped_bounds).step(5.0),
                );
                ui.label(format!(
                    "Range: [{:.0}, {:.0}]",
                    self.stepped_range.0, self.stepped_range.1
                ));
            });
            ui.add_space(10.0);

            // Custom format example
            ui.group(|ui| {
                ui.label("Wavelength Selection (nm):");
                ui.add(
                    DoubleSlider::new(&mut self.custom_range, &mut self.custom_bounds)
                        .label_format(|v| format!("{:.0} nm", v))
                        .width(500.0),
                );
                ui.label(format!(
                    "Range: [{:.0} nm, {:.0} nm]",
                    self.custom_range.0, self.custom_range.1
                ));
            });
            ui.add_space(10.0);

            // No labels example
            ui.group(|ui| {
                ui.label("Narrow Range (no labels):");
                ui.add(
                    DoubleSlider::new(&mut self.narrow_range, &mut self.narrow_bounds)
                        .show_labels(false)
                        .height(16.0),
                );
                ui.label(format!(
                    "Range: [{:.3}, {:.3}]",
                    self.narrow_range.0, self.narrow_range.1
                ));
            });
            ui.add_space(20.0);

            // Use case examples
            ui.separator();
            ui.heading("Use Cases");
            ui.add_space(10.0);

            ui.label("• Histogram clipping bounds");
            ui.label("• ROI coordinate ranges (x_min..x_max, y_min..y_max)");
            ui.label("• Time/frequency window selection");
            ui.label("• Wavelength range filtering");
            ui.label("• Parameter range constraints");
        });
    }
}
