//! Shortcut actions and contexts

/// Context in which a shortcut is active
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ShortcutContext {
    /// Global shortcuts (work anywhere in the app)
    Global,
    /// Image viewer specific shortcuts
    ImageViewer,
    /// Signal plotter specific shortcuts
    SignalPlotter,
}

impl ShortcutContext {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::ImageViewer => "Image Viewer",
            Self::SignalPlotter => "Signal Plotter",
        }
    }
}

/// Actions that can be triggered by keyboard shortcuts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ShortcutAction {
    // === Global Actions ===
    /// Open settings dialog (Ctrl+,)
    OpenSettings,
    /// Show/hide keyboard cheat sheet (?)
    ToggleCheatSheet,
    /// Save current frame/data (Ctrl+S)
    SaveCurrent,

    // === Image Viewer Actions ===
    /// Start/stop acquisition (Space)
    ToggleAcquisition,
    /// Start/stop recording (R)
    ToggleRecording,
    /// Fit image to view (F)
    FitToView,
    /// Set zoom to 100% (1)
    Zoom100,
    /// Set zoom to 200% (2)
    Zoom200,
    /// Set zoom to 300% (3)
    Zoom300,
    /// Set zoom to 400% (4)
    Zoom400,
    /// Set zoom to 500% (5)
    Zoom500,
    /// Set zoom to 600% (6)
    Zoom600,
    /// Set zoom to 700% (7)
    Zoom700,
    /// Set zoom to 800% (8)
    Zoom800,
    /// Set zoom to 900% (9)
    Zoom900,
    /// Zoom in (+/=)
    ZoomIn,
    /// Zoom out (-)
    ZoomOut,
    /// Pan image up (↑)
    PanUp,
    /// Pan image down (↓)
    PanDown,
    /// Pan image left (←)
    PanLeft,
    /// Pan image right (→)
    PanRight,
    /// Toggle crosshair (C)
    ToggleCrosshair,
    /// Toggle histogram overlay (H)
    ToggleHistogram,
    /// Cycle through colormaps (M)
    CycleColormap,
}

impl ShortcutAction {
    /// Get the context where this action is valid
    pub fn context(&self) -> ShortcutContext {
        match self {
            Self::OpenSettings | Self::ToggleCheatSheet | Self::SaveCurrent => {
                ShortcutContext::Global
            }
            Self::ToggleAcquisition
            | Self::ToggleRecording
            | Self::FitToView
            | Self::Zoom100
            | Self::Zoom200
            | Self::Zoom300
            | Self::Zoom400
            | Self::Zoom500
            | Self::Zoom600
            | Self::Zoom700
            | Self::Zoom800
            | Self::Zoom900
            | Self::ZoomIn
            | Self::ZoomOut
            | Self::PanUp
            | Self::PanDown
            | Self::PanLeft
            | Self::PanRight
            | Self::ToggleCrosshair
            | Self::ToggleHistogram
            | Self::CycleColormap => ShortcutContext::ImageViewer,
        }
    }

    /// Get human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            Self::OpenSettings => "Open settings",
            Self::ToggleCheatSheet => "Show/hide keyboard shortcuts",
            Self::SaveCurrent => "Save current frame",
            Self::ToggleAcquisition => "Start/stop acquisition",
            Self::ToggleRecording => "Start/stop recording",
            Self::FitToView => "Fit image to view",
            Self::Zoom100 => "Zoom to 100%",
            Self::Zoom200 => "Zoom to 200%",
            Self::Zoom300 => "Zoom to 300%",
            Self::Zoom400 => "Zoom to 400%",
            Self::Zoom500 => "Zoom to 500%",
            Self::Zoom600 => "Zoom to 600%",
            Self::Zoom700 => "Zoom to 700%",
            Self::Zoom800 => "Zoom to 800%",
            Self::Zoom900 => "Zoom to 900%",
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::PanUp => "Pan image up",
            Self::PanDown => "Pan image down",
            Self::PanLeft => "Pan image left",
            Self::PanRight => "Pan image right",
            Self::ToggleCrosshair => "Toggle crosshair",
            Self::ToggleHistogram => "Toggle histogram overlay",
            Self::CycleColormap => "Cycle through colormaps",
        }
    }
}
