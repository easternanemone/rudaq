//! Offline notice widget for graceful degradation when disconnected.
//!
//! Provides a user-friendly message when the daemon is not connected,
//! with instructions for starting the daemon and what features are available.

use eframe::egui;

/// Context for rendering offline notice - what panel/feature this is for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineContext {
    /// Device management panels (Devices, Instruments)
    Devices,
    /// Experiment/scan execution
    Experiments,
    /// Script execution
    Scripts,
    /// Data storage operations
    Storage,
    /// Module management
    Modules,
    /// Generic panel
    #[allow(dead_code)]
    Generic,
}

impl OfflineContext {
    /// Human-readable label for the context
    pub fn label(&self) -> &'static str {
        match self {
            Self::Devices => "device control",
            Self::Experiments => "experiment execution",
            Self::Scripts => "script execution",
            Self::Storage => "storage operations",
            Self::Modules => "module management",
            Self::Generic => "this feature",
        }
    }

    /// Icon for the context
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Devices => "🔧",
            Self::Experiments => "🔬",
            Self::Scripts => "📜",
            Self::Storage => "💾",
            Self::Modules => "📦",
            Self::Generic => "ℹ️",
        }
    }
}

/// Render an offline notice with helpful instructions.
///
/// Returns true if the notice was shown (i.e., client is None).
/// Panels should use this to skip their normal content when disconnected.
///
/// # Example
/// ```ignore
/// pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
///     if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
///         return; // Skip rendering device controls when offline
///     }
///     // Normal panel content...
/// }
/// ```
pub fn offline_notice(ui: &mut egui::Ui, is_offline: bool, context: OfflineContext) -> bool {
    if !is_offline {
        return false;
    }

    ui.vertical_centered(|ui| {
        ui.add_space(20.0);

        // Icon and title
        ui.heading(format!("{} Not Connected", context.icon()));
        ui.add_space(8.0);

        // Context-specific message
        ui.label(format!(
            "Connect to the daemon to enable {}.",
            context.label()
        ));
        ui.add_space(16.0);

        // Instructions box
        ui.group(|ui| {
            ui.heading("Quick Start");
            ui.add_space(4.0);

            ui.label("1. Start the daemon:");
            ui.add_space(2.0);
            ui.code("cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml");
            ui.add_space(8.0);

            ui.label("2. Use the connection bar below to connect to:");
            ui.add_space(2.0);
            ui.code("http://127.0.0.1:50051");
        });

        ui.add_space(16.0);
        ui.label("If retries are paused due to a circuit breaker, check the status bar countdown.");

        // What's available offline
        ui.collapsing("What works offline?", |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::GREEN, "✓");
                ui.label("Getting Started guide");
            });
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::GREEN, "✓");
                ui.label("Local log viewing");
            });
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::GREEN, "✓");
                ui.label("Connection settings");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::RED, "✗");
                ui.label("Device control (requires daemon)");
            });
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::RED, "✗");
                ui.label("Script execution (requires daemon)");
            });
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::RED, "✗");
                ui.label("Data acquisition (requires daemon)");
            });
        });
    });

    true
}

/// Render a compact offline banner (for use in headers/toolbars).
///
/// Shows a small warning indicator that the panel requires connection.
#[allow(dead_code)]
pub fn offline_banner(ui: &mut egui::Ui, is_offline: bool) {
    if !is_offline {
        return;
    }

    ui.horizontal(|ui| {
        ui.colored_label(egui::Color32::YELLOW, "⚠");
        ui.colored_label(
            egui::Color32::from_gray(180),
            "Offline - connect to daemon for full functionality",
        );
    });
}

/// Render a minimal offline indicator (just an icon with tooltip).
#[allow(dead_code)]
pub fn offline_indicator(ui: &mut egui::Ui, is_offline: bool) {
    if !is_offline {
        return;
    }

    let response = ui.colored_label(egui::Color32::YELLOW, "⚠");
    response.on_hover_text("Not connected to daemon. Some features are unavailable.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offline_context_labels() {
        assert_eq!(OfflineContext::Devices.label(), "device control");
        assert_eq!(OfflineContext::Scripts.label(), "script execution");
    }

    #[test]
    fn test_offline_context_icons() {
        assert!(!OfflineContext::Devices.icon().is_empty());
        assert!(!OfflineContext::Generic.icon().is_empty());
    }
}
