pub use egui_phosphor::Variant;
pub use egui_phosphor::regular::*;

pub fn add_to_fonts(fonts: &mut egui::FontDefinitions) {
    egui_phosphor::add_to_fonts(fonts, Variant::Regular);
}

pub mod nav {
    use super::*;
    pub const DEVICES: &str = MONITOR_PLAY;
    pub const SCRIPTS: &str = CODE;
    pub const SCANS: &str = CHART_LINE;
    pub const STORAGE: &str = DATABASE;
    pub const IMAGE_VIEWER: &str = CAMERA;
    pub const SIGNAL_PLOTTER: &str = CHART_LINE_UP;
    pub const LOGGING: &str = LIST_BULLETS;
    pub const MODULES: &str = CUBE;
    pub const GETTING_STARTED: &str = ROCKET_LAUNCH;
    pub const PLAN_RUNNER: &str = PLAY_CIRCLE;
    pub const DOCUMENT_VIEWER: &str = FILE_TEXT;
    pub const INSTRUMENT_MANAGER: &str = SLIDERS_HORIZONTAL;
}

pub mod action {
    use super::*;
    pub const CONNECT: &str = PLUG;
    pub const DISCONNECT: &str = PLUGS;
    pub const REFRESH: &str = ARROW_CLOCKWISE;
    pub const SETTINGS: &str = GEAR;
    pub const START: &str = PLAY;
    pub const STOP: &str = super::STOP;
    pub const PAUSE: &str = super::PAUSE;
    pub const RECORD: &str = super::RECORD;
    pub const SAVE: &str = FLOPPY_DISK;
    pub const OPEN: &str = FOLDER_OPEN;
    pub const NEW: &str = FILE_PLUS;
    pub const DELETE: &str = TRASH;
    pub const EDIT: &str = PENCIL;
    pub const COPY: &str = super::COPY;
    pub const EXPAND: &str = ARROWS_OUT;
    pub const COLLAPSE: &str = ARROWS_IN;
    pub const ZOOM_IN: &str = MAGNIFYING_GLASS_PLUS;
    pub const ZOOM_OUT: &str = MAGNIFYING_GLASS_MINUS;
    pub const FIT: &str = ARROWS_IN_CARDINAL;
}

pub mod status {
    use super::*;
    pub const SUCCESS: &str = CHECK_CIRCLE;
    pub const ERROR: &str = X_CIRCLE;
    pub const WARNING: &str = WARNING_CIRCLE;
    pub const INFO: &str = super::INFO;
    pub const LOADING: &str = SPINNER;
    pub const CONNECTED: &str = WIFI_HIGH;
    pub const DISCONNECTED: &str = WIFI_SLASH;
}

pub mod device {
    use super::*;
    pub const CAMERA: &str = super::CAMERA;
    pub const MOTOR: &str = GEAR_SIX;
    pub const SENSOR: &str = THERMOMETER;
    pub const LASER: &str = LIGHTNING;
    pub const DAQ: &str = CHART_BAR;
    pub const GENERIC: &str = CPU;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nav_icons_are_non_empty() {
        assert!(!nav::DEVICES.is_empty());
        assert!(!nav::SCRIPTS.is_empty());
        assert!(!nav::SCANS.is_empty());
        assert!(!nav::STORAGE.is_empty());
        assert!(!nav::IMAGE_VIEWER.is_empty());
        assert!(!nav::SIGNAL_PLOTTER.is_empty());
        assert!(!nav::LOGGING.is_empty());
    }

    #[test]
    fn test_action_icons_are_non_empty() {
        assert!(!action::CONNECT.is_empty());
        assert!(!action::DISCONNECT.is_empty());
        assert!(!action::START.is_empty());
        assert!(!action::STOP.is_empty());
        assert!(!action::REFRESH.is_empty());
    }

    #[test]
    fn test_status_icons_are_non_empty() {
        assert!(!status::SUCCESS.is_empty());
        assert!(!status::ERROR.is_empty());
        assert!(!status::WARNING.is_empty());
        assert!(!status::INFO.is_empty());
    }

    #[test]
    fn test_device_icons_are_non_empty() {
        assert!(!device::CAMERA.is_empty());
        assert!(!device::MOTOR.is_empty());
        assert!(!device::SENSOR.is_empty());
        assert!(!device::GENERIC.is_empty());
    }
}
