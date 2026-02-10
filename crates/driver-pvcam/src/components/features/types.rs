// =============================================================================
// Data Structures
// =============================================================================

/// Comprehensive camera information (bd-565x)
#[derive(Debug, Clone)]
pub struct CameraInfo {
    /// Camera serial number (alphanumeric)
    pub serial_number: String,
    /// Firmware version string
    pub firmware_version: String,
    /// Sensor chip name (e.g., "GS2020" for Prime BSI)
    pub chip_name: String,
    /// Current sensor temperature in Celsius
    pub temperature_c: f64,
    /// Bit depth for current speed mode
    pub bit_depth: u16,
    /// Pixel readout time in nanoseconds
    pub pixel_time_ns: u32,
    /// Pixel size in nanometers (width, height)
    pub pixel_size_nm: (u32, u32),
    /// Sensor size in pixels (width, height)
    pub sensor_size: (u32, u32),
    /// Current gain mode name
    pub gain_name: String,
    /// Current speed mode name
    pub speed_name: String,
    /// Current readout port name
    pub port_name: String,
    /// Current gain index
    pub gain_index: u16,
    /// Current speed table index
    pub speed_index: u16,
}

#[derive(Debug, Clone)]
pub struct GainMode {
    pub index: u16,
    pub name: String,
}

/// Speed mode entry from the camera's speed table (bd-v54z)
#[derive(Debug, Clone)]
pub struct SpeedMode {
    /// Speed table index
    pub index: u16,
    /// Display name (e.g., "100 MHz")
    pub name: String,
    /// Pixel readout time in nanoseconds
    pub pixel_time_ns: u32,
    /// Bit depth at this speed
    pub bit_depth: u16,
    /// Associated readout port index
    pub port_index: u16,
}

/// Readout port entry (bd-v54z)
#[derive(Debug, Clone)]
pub struct ReadoutPort {
    /// Port index
    pub index: u16,
    /// Port name (e.g., "Sensitivity", "Speed")
    pub name: String,
}
