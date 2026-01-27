//! Extension trait for ConnectionState UI methods.

use daq_client::ConnectionState;
use eframe::egui;

/// Extension trait providing UI-specific methods for ConnectionState.
pub trait ConnectionStateExt {
    /// Returns the UI indicator color for the connection state.
    fn color(&self) -> egui::Color32;
}

impl ConnectionStateExt for ConnectionState {
    fn color(&self) -> egui::Color32 {
        match self {
            Self::Disconnected => egui::Color32::GRAY,
            Self::Connecting => egui::Color32::YELLOW,
            Self::Connected { .. } => egui::Color32::GREEN,
            Self::Reconnecting { .. } => egui::Color32::YELLOW,
            Self::Error { .. } => egui::Color32::RED,
        }
    }
}
