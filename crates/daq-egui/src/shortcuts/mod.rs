//! Keyboard shortcuts system for efficient GUI operation
//!
//! This module provides:
//! - Centralized shortcut management with conflict detection
//! - Customizable key bindings with persistence
//! - Context-aware shortcuts (global vs panel-specific)
//! - Cheat sheet UI for discoverability
//! - Settings UI for customization

mod action;
mod cheat_sheet;
mod manager;
mod settings;

pub use action::{ShortcutAction, ShortcutContext};
pub use cheat_sheet::CheatSheetPanel;
pub use manager::{KeyBinding, ShortcutManager};
pub use settings::ShortcutSettingsPanel;
