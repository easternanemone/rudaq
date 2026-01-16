//! Status bar widget for the DAQ GUI.
//!
//! Displays connection state, breadcrumb navigation, transient status messages,
//! and version information in a fixed-height bottom panel.

use eframe::egui;

use crate::icons;
use crate::layout::{self, colors};
use crate::reconnect::ConnectionState;

/// Status bar widget displaying connection state and contextual information.
///
/// The status bar has three sections:
/// - **Left**: Breadcrumb/context path
/// - **Center**: Transient status message (with automatic timeout)
/// - **Right**: Connection indicator and version number
pub struct StatusBar {
    /// Current breadcrumb/context path (e.g., "Devices > Motor Stage")
    breadcrumb: Option<String>,
    /// Transient status message
    status_message: Option<StatusMessage>,
}

/// A transient status message with automatic timeout.
#[derive(Clone)]
pub struct StatusMessage {
    /// The message text
    pub text: String,
    /// The message level (determines styling)
    pub level: StatusLevel,
    /// When this message was created
    pub created_at: std::time::Instant,
    /// How long to show this message (None = until manually cleared)
    pub duration: Option<std::time::Duration>,
}

/// Level/severity of a status message.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    /// Informational message
    Info,
    /// Success message
    Success,
    /// Warning message
    Warning,
    /// Error message
    Error,
}

impl StatusLevel {
    /// Get the icon for this status level.
    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Info => icons::status::INFO,
            Self::Success => icons::status::SUCCESS,
            Self::Warning => icons::status::WARNING,
            Self::Error => icons::status::ERROR,
        }
    }

    /// Get the color for this status level.
    #[must_use]
    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::Info => colors::INFO,
            Self::Success => colors::SUCCESS,
            Self::Warning => colors::WARNING,
            Self::Error => colors::ERROR,
        }
    }
}

impl StatusBar {
    /// Create a new status bar.
    #[must_use]
    pub fn new() -> Self {
        Self {
            breadcrumb: None,
            status_message: None,
        }
    }

    /// Set the breadcrumb/context path.
    pub fn set_breadcrumb(&mut self, breadcrumb: impl Into<String>) {
        self.breadcrumb = Some(breadcrumb.into());
    }

    /// Clear the breadcrumb.
    pub fn clear_breadcrumb(&mut self) {
        self.breadcrumb = None;
    }

    /// Set a transient status message.
    pub fn set_status(&mut self, text: impl Into<String>, level: StatusLevel) {
        self.status_message = Some(StatusMessage {
            text: text.into(),
            level,
            created_at: std::time::Instant::now(),
            duration: Some(std::time::Duration::from_secs(5)),
        });
    }

    /// Set a persistent status message (no timeout).
    #[allow(dead_code)]
    pub fn set_persistent_status(&mut self, text: impl Into<String>, level: StatusLevel) {
        self.status_message = Some(StatusMessage {
            text: text.into(),
            level,
            created_at: std::time::Instant::now(),
            duration: None,
        });
    }

    /// Clear the status message.
    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// Check and clear expired status messages.
    fn check_status_expiry(&mut self) {
        if let Some(ref msg) = self.status_message {
            if let Some(duration) = msg.duration {
                if msg.created_at.elapsed() >= duration {
                    self.status_message = None;
                }
            }
        }
    }

    /// Render the status bar.
    ///
    /// # Arguments
    /// * `ctx` - The egui context
    /// * `connection_state` - Current connection state
    /// * `error_count` - Optional number of errors to display
    ///
    /// # Example
    /// ```ignore
    /// status_bar.show(ctx, self.connection.state(), Some(5));
    /// ```
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        connection_state: &ConnectionState,
        error_count: Option<u32>,
    ) {
        // Check for expired status messages
        self.check_status_expiry();

        egui::TopBottomPanel::bottom("app_status_bar")
            .exact_height(layout::STATUS_BAR_HEIGHT)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // === Left section: Breadcrumb ===
                    self.render_breadcrumb(ui);

                    // Flexible space to push center and right sections
                    ui.add_space(ui.available_width() * 0.1);

                    // === Center section: Status message ===
                    self.render_status_message(ui);

                    // Expand to push right section to the edge
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // === Right section: Version and connection indicator ===
                        self.render_right_section(ui, connection_state, error_count);
                    });
                });
            });
    }

    /// Render the breadcrumb section (left).
    fn render_breadcrumb(&self, ui: &mut egui::Ui) {
        if let Some(ref breadcrumb) = self.breadcrumb {
            ui.label(egui::RichText::new(breadcrumb).small().color(colors::MUTED));
        } else {
            // Placeholder to maintain layout
            ui.label(egui::RichText::new("rust-daq").small().color(colors::MUTED));
        }
    }

    /// Render the status message section (center).
    fn render_status_message(&mut self, ui: &mut egui::Ui) {
        if let Some(ref msg) = self.status_message {
            let color = msg.level.color();
            let icon = msg.level.icon();

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icon).color(color).size(14.0));
                ui.label(egui::RichText::new(&msg.text).small().color(color));
            });

            // Request repaint if message has a duration (for auto-clear)
            if msg.duration.is_some() {
                ui.ctx().request_repaint();
            }
        }
    }

    /// Render the right section (connection indicator and version).
    fn render_right_section(
        &self,
        ui: &mut egui::Ui,
        connection_state: &ConnectionState,
        error_count: Option<u32>,
    ) {
        // Version number (rightmost)
        let version = env!("CARGO_PKG_VERSION");
        ui.label(
            egui::RichText::new(format!("v{}", version))
                .small()
                .color(colors::MUTED),
        );

        ui.add_space(8.0);

        // Error count (if any)
        if let Some(count) = error_count {
            if count > 0 {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(icons::status::ERROR)
                            .color(colors::ERROR)
                            .size(14.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("{}", count))
                            .small()
                            .color(colors::ERROR),
                    );
                });
                ui.add_space(8.0);
            }
        }

        // Connection indicator
        let (icon, color, tooltip) = match connection_state {
            ConnectionState::Connected { .. } => (
                icons::status::CONNECTED,
                colors::CONNECTED,
                "Connected to daemon",
            ),
            ConnectionState::Disconnected => (
                icons::status::DISCONNECTED,
                colors::DISCONNECTED,
                "Disconnected",
            ),
            ConnectionState::Connecting => {
                (icons::status::LOADING, colors::CONNECTING, "Connecting...")
            }
            ConnectionState::Reconnecting { .. } => (
                icons::status::LOADING,
                colors::RECONNECTING,
                "Reconnecting...",
            ),
            ConnectionState::Error { .. } => {
                (icons::status::ERROR, colors::ERROR, "Connection error")
            }
        };

        let response = ui.label(egui::RichText::new(icon).color(color).size(16.0));

        // Build tooltip with appropriate detail
        let tooltip_text = match connection_state {
            ConnectionState::Reconnecting { attempt, .. } => {
                format!("Reconnecting (attempt {})", attempt)
            }
            ConnectionState::Error { message, .. } => {
                format!("Error: {}", message)
            }
            _ => tooltip.to_string(),
        };
        response.on_hover_text(tooltip_text);
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_level_icons() {
        // Just verify that icons are non-empty strings
        assert!(!StatusLevel::Info.icon().is_empty());
        assert!(!StatusLevel::Success.icon().is_empty());
        assert!(!StatusLevel::Warning.icon().is_empty());
        assert!(!StatusLevel::Error.icon().is_empty());
    }

    #[test]
    fn test_status_bar_breadcrumb() {
        let mut bar = StatusBar::new();
        assert!(bar.breadcrumb.is_none());

        bar.set_breadcrumb("Devices > Motor");
        assert_eq!(bar.breadcrumb, Some("Devices > Motor".to_string()));

        bar.clear_breadcrumb();
        assert!(bar.breadcrumb.is_none());
    }

    #[test]
    fn test_status_message_expiry() {
        let mut bar = StatusBar::new();
        bar.status_message = Some(StatusMessage {
            text: "Test".to_string(),
            level: StatusLevel::Info,
            created_at: std::time::Instant::now() - std::time::Duration::from_secs(10),
            duration: Some(std::time::Duration::from_secs(5)),
        });

        bar.check_status_expiry();
        assert!(
            bar.status_message.is_none(),
            "Expired message should be cleared"
        );
    }

    #[test]
    fn test_persistent_status_no_expiry() {
        let mut bar = StatusBar::new();
        bar.status_message = Some(StatusMessage {
            text: "Persistent".to_string(),
            level: StatusLevel::Warning,
            created_at: std::time::Instant::now() - std::time::Duration::from_secs(100),
            duration: None, // Persistent
        });

        bar.check_status_expiry();
        assert!(
            bar.status_message.is_some(),
            "Persistent message should not expire"
        );
    }
}
