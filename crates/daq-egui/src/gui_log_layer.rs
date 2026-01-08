//! Custom tracing layer that forwards events to the GUI logging panel.
//!
//! This layer captures tracing events and sends them through a channel
//! to be displayed in the LoggingPanel.

use std::sync::mpsc;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::panels::LogLevel;

/// A log event that can be sent to the GUI logging panel.
#[derive(Debug, Clone)]
pub struct GuiLogEvent {
    /// Log level
    pub level: LogLevel,
    /// Source target (module path)
    pub target: String,
    /// Log message
    pub message: String,
}

/// Visitor to extract the message field from tracing events.
struct MessageVisitor {
    message: String,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
            // Remove surrounding quotes if present
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        } else if self.message.is_empty() {
            // Fallback: use any field as message
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            self.message
                .push_str(&format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if self.message.is_empty() {
            self.message = format!("{}={}", field.name(), value);
        }
    }
}

/// A tracing layer that sends events to the GUI logging panel.
pub struct GuiLogLayer {
    sender: mpsc::Sender<GuiLogEvent>,
}

impl GuiLogLayer {
    /// Create a new GuiLogLayer with the given sender.
    pub fn new(sender: mpsc::Sender<GuiLogEvent>) -> Self {
        Self { sender }
    }
}

impl<S> Layer<S> for GuiLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Convert tracing level to LogLevel
        let level = match *event.metadata().level() {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            Level::INFO => LogLevel::Info,
            Level::DEBUG => LogLevel::Debug,
            Level::TRACE => LogLevel::Trace,
        };

        // Extract target (module path)
        let target = event.metadata().target().to_string();

        // Extract message
        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            // If no message field, use the event name
            event.metadata().name().to_string()
        } else {
            visitor.message
        };

        // Send to the logging panel (ignore errors if receiver is dropped)
        let _ = self.sender.send(GuiLogEvent {
            level,
            target,
            message,
        });
    }
}

/// Create a channel for GUI log events.
///
/// Returns (sender, receiver) pair. The sender goes to GuiLogLayer,
/// the receiver goes to the logging panel.
pub fn create_log_channel() -> (mpsc::Sender<GuiLogEvent>, mpsc::Receiver<GuiLogEvent>) {
    // Use a bounded channel to prevent memory buildup if GUI is slow
    mpsc::channel()
}
