//! Central shortcut manager with conflict detection

use eframe::egui;
use std::collections::HashMap;

use super::action::{ShortcutAction, ShortcutContext};

/// A key binding (key + modifiers)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct KeyBinding {
    pub key: egui::Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyBinding {
    /// Create a simple key binding without modifiers
    pub fn new(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    /// Create a key binding with Ctrl modifier
    pub fn ctrl(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
        }
    }

    /// Create a key binding with Shift modifier
    pub fn shift(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: true,
            alt: false,
        }
    }

    /// Check if this binding matches the current input state
    pub fn matches(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            i.key_pressed(self.key)
                && i.modifiers.ctrl == self.ctrl
                && i.modifiers.shift == self.shift
                && i.modifiers.alt == self.alt
        })
    }

    /// Get human-readable label for this binding
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        parts.push(&format!("{:?}", self.key));
        parts.join("+")
    }
}

/// Central shortcut manager
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ShortcutManager {
    /// Map from action to key binding
    bindings: HashMap<ShortcutAction, KeyBinding>,
    /// Reverse map from key binding to action (for conflict detection)
    #[serde(skip)]
    reverse_map: HashMap<(ShortcutContext, KeyBinding), ShortcutAction>,
}

impl Default for ShortcutManager {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl ShortcutManager {
    /// Create a new manager with default key bindings
    pub fn with_defaults() -> Self {
        use egui::Key;
        use ShortcutAction::*;

        let mut manager = Self {
            bindings: HashMap::new(),
            reverse_map: HashMap::new(),
        };

        // === Global shortcuts ===
        manager.set_binding(OpenSettings, KeyBinding::ctrl(Key::Comma));
        manager.set_binding(ToggleCheatSheet, KeyBinding::shift(Key::Slash)); // ? key
        manager.set_binding(SaveCurrent, KeyBinding::ctrl(Key::S));

        // === Image Viewer shortcuts ===
        manager.set_binding(ToggleAcquisition, KeyBinding::new(Key::Space));
        manager.set_binding(ToggleRecording, KeyBinding::new(Key::R));
        manager.set_binding(FitToView, KeyBinding::new(Key::F));

        // Zoom levels (1-9)
        manager.set_binding(Zoom100, KeyBinding::new(Key::Num1));
        manager.set_binding(Zoom200, KeyBinding::new(Key::Num2));
        manager.set_binding(Zoom300, KeyBinding::new(Key::Num3));
        manager.set_binding(Zoom400, KeyBinding::new(Key::Num4));
        manager.set_binding(Zoom500, KeyBinding::new(Key::Num5));
        manager.set_binding(Zoom600, KeyBinding::new(Key::Num6));
        manager.set_binding(Zoom700, KeyBinding::new(Key::Num7));
        manager.set_binding(Zoom800, KeyBinding::new(Key::Num8));
        manager.set_binding(Zoom900, KeyBinding::new(Key::Num9));

        // Zoom in/out
        manager.set_binding(ZoomIn, KeyBinding::new(Key::Plus));
        manager.set_binding(ZoomOut, KeyBinding::new(Key::Minus));

        // Pan with arrow keys
        manager.set_binding(PanUp, KeyBinding::new(Key::ArrowUp));
        manager.set_binding(PanDown, KeyBinding::new(Key::ArrowDown));
        manager.set_binding(PanLeft, KeyBinding::new(Key::ArrowLeft));
        manager.set_binding(PanRight, KeyBinding::new(Key::ArrowRight));

        // Overlays
        manager.set_binding(ToggleCrosshair, KeyBinding::new(Key::C));
        manager.set_binding(ToggleHistogram, KeyBinding::new(Key::H));
        manager.set_binding(CycleColormap, KeyBinding::new(Key::M));

        manager
    }

    /// Set a key binding for an action
    pub fn set_binding(&mut self, action: ShortcutAction, binding: KeyBinding) {
        self.bindings.insert(action, binding);
        self.reverse_map.insert((action.context(), binding), action);
    }

    /// Get the key binding for an action
    pub fn get_binding(&self, action: ShortcutAction) -> Option<KeyBinding> {
        self.bindings.get(&action).copied()
    }

    /// Check if an action should be triggered in the current context
    pub fn check_action(
        &self,
        ctx: &egui::Context,
        context: ShortcutContext,
        action: ShortcutAction,
    ) -> bool {
        // Only check actions valid in this context
        if action.context() != context && action.context() != ShortcutContext::Global {
            return false;
        }

        if let Some(binding) = self.bindings.get(&action) {
            binding.matches(ctx)
        } else {
            false
        }
    }

    /// Get all bindings for a specific context
    pub fn bindings_for_context(
        &self,
        context: ShortcutContext,
    ) -> Vec<(ShortcutAction, KeyBinding)> {
        let mut bindings: Vec<_> = self
            .bindings
            .iter()
            .filter(|(action, _)| {
                action.context() == context || action.context() == ShortcutContext::Global
            })
            .map(|(action, binding)| (*action, *binding))
            .collect();
        bindings.sort_by_key(|(action, _)| format!("{:?}", action));
        bindings
    }

    /// Check for conflicts when setting a new binding
    pub fn find_conflict(
        &self,
        action: ShortcutAction,
        binding: KeyBinding,
    ) -> Option<ShortcutAction> {
        let context = action.context();

        // Check if this binding is already used in the same context
        for (existing_action, existing_binding) in &self.bindings {
            if *existing_binding == binding && *existing_action != action {
                // Check if contexts overlap (same context or one is global)
                let existing_context = existing_action.context();
                if existing_context == context
                    || existing_context == ShortcutContext::Global
                    || context == ShortcutContext::Global
                {
                    return Some(*existing_action);
                }
            }
        }
        None
    }

    /// Reset all bindings to defaults
    pub fn reset_to_defaults(&mut self) {
        *self = Self::with_defaults();
    }

    /// Get all actions grouped by context
    pub fn all_actions_by_context(&self) -> HashMap<ShortcutContext, Vec<ShortcutAction>> {
        let mut result: HashMap<ShortcutContext, Vec<ShortcutAction>> = HashMap::new();

        for action in self.bindings.keys() {
            result
                .entry(action.context())
                .or_insert_with(Vec::new)
                .push(*action);
        }

        // Sort actions within each context
        for actions in result.values_mut() {
            actions.sort_by_key(|a| format!("{:?}", a));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_bindings() {
        let manager = ShortcutManager::with_defaults();

        // Check that some key bindings exist
        assert!(manager
            .get_binding(ShortcutAction::ToggleAcquisition)
            .is_some());
        assert!(manager.get_binding(ShortcutAction::FitToView).is_some());
        assert!(manager.get_binding(ShortcutAction::OpenSettings).is_some());
    }

    #[test]
    fn test_conflict_detection() {
        let mut manager = ShortcutManager::with_defaults();

        let binding = KeyBinding::new(egui::Key::Space);

        // Space is already bound to ToggleAcquisition
        let conflict = manager.find_conflict(ShortcutAction::ToggleRecording, binding);
        assert_eq!(conflict, Some(ShortcutAction::ToggleAcquisition));
    }

    #[test]
    fn test_context_filtering() {
        let manager = ShortcutManager::with_defaults();

        let image_bindings = manager.bindings_for_context(ShortcutContext::ImageViewer);

        // Should include both ImageViewer and Global actions
        assert!(image_bindings
            .iter()
            .any(|(a, _)| *a == ShortcutAction::FitToView));
        assert!(image_bindings
            .iter()
            .any(|(a, _)| *a == ShortcutAction::OpenSettings));
    }
}
