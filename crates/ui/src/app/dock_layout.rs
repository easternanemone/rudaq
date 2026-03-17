//! Dock layout management - default layout, layout version constants.

use super::*;

impl DaqApp {
    pub(super) fn default_dock_state() -> DockState<Panel> {
        // Start with Instruments + ImageViewer as tabbed panels in the main content area
        let mut dock_state = DockState::new(vec![Panel::Instruments, Panel::ImageViewer]);
        let surface = dock_state.main_surface_mut();

        // Split left for Nav
        let [_nav, content] = surface.split_left(NodeIndex::root(), 0.15, vec![Panel::Nav]);

        // Split bottom of content for Logs
        let [_content, _logs] = surface.split_below(content, 0.75, vec![Panel::Logs]);

        dock_state
    }
}
