// =============================================================================
// Device Lifecycle State (bd-oqo7.9)
// =============================================================================

/// PVCAM camera lifecycle state for SurrealDB persistence and health monitoring.
///
/// Derived from `get_controller_alive()` and `get_ccs_status()` polling.
/// State transitions are logged and persisted to the `device_runtime_state` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PvcamDeviceState {
    /// Camera not connected or controller not responding.
    Offline,
    /// `pl_cam_open` in progress (SDK initialization).
    Initializing,
    /// Camera initialized and idle (controller alive, CCS idle).
    Ready,
    /// Acquisition active (CCS running or continuously clearing).
    Streaming,
    /// Controller alive but CCS reports a fault condition.
    Error,
    /// `pl_cam_close` in progress (graceful shutdown).
    ShuttingDown,
}

impl PvcamDeviceState {
    /// Derive the device state from controller_alive and CCS status values.
    ///
    /// CCS status values (from PVCAM SDK):
    /// - 0: idle
    /// - 1: initializing
    /// - 2: running
    /// - 3: continuously clearing
    pub fn from_health_check(controller_alive: bool, ccs_status: i16) -> Self {
        if !controller_alive {
            return Self::Offline;
        }
        match ccs_status {
            0 => Self::Ready,
            1 => Self::Initializing,
            2 | 3 => Self::Streaming,
            _ => Self::Error,
        }
    }
}

impl std::fmt::Display for PvcamDeviceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline => write!(f, "offline"),
            Self::Initializing => write!(f, "initializing"),
            Self::Ready => write!(f, "ready"),
            Self::Streaming => write!(f, "streaming"),
            Self::Error => write!(f, "error"),
            Self::ShuttingDown => write!(f, "shutting_down"),
        }
    }
}

impl TryFrom<&str> for PvcamDeviceState {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, <Self as TryFrom<&str>>::Error> {
        match s {
            "offline" => Ok(Self::Offline),
            "initializing" => Ok(Self::Initializing),
            "ready" => Ok(Self::Ready),
            "streaming" => Ok(Self::Streaming),
            "error" => Ok(Self::Error),
            "shutting_down" | "ShuttingDown" => Ok(Self::ShuttingDown),
            _ => Err(format!("Invalid PVCAM device state: {s}")),
        }
    }
}

// =============================================================================
// Fan Speed
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanSpeed {
    High,
    Medium,
    Low,
    Off,
}

impl FanSpeed {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FanSpeed::High,
            1 => FanSpeed::Medium,
            2 => FanSpeed::Low,
            3 => FanSpeed::Off,
            _ => FanSpeed::High,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            FanSpeed::High => 0,
            FanSpeed::Medium => 1,
            FanSpeed::Low => 2,
            FanSpeed::Off => 3,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "High" => FanSpeed::High,
            "Medium" => FanSpeed::Medium,
            "Low" => FanSpeed::Low,
            "Off" => FanSpeed::Off,
            _ => FanSpeed::High,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FanSpeed::High => "High",
            FanSpeed::Medium => "Medium",
            FanSpeed::Low => "Low",
            FanSpeed::Off => "Off",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["High".into(), "Medium".into(), "Low".into(), "Off".into()]
    }
}

#[derive(Debug, Clone)]
pub struct PPFeature {
    pub index: u16,
    pub id: u16,
    pub name: String,
    pub params: Vec<PPParam>,
}

#[derive(Debug, Clone)]
pub struct PPParam {
    pub index: u16,
    pub id: u16,
    pub name: String,
    pub value: u32,
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CentroidsMode {
    Locate,
    Track,
    Blob,
}

impl CentroidsMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => CentroidsMode::Locate,
            1 => CentroidsMode::Track,
            2 => CentroidsMode::Blob,
            _ => CentroidsMode::Locate,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            CentroidsMode::Locate => 0,
            CentroidsMode::Track => 1,
            CentroidsMode::Blob => 2,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Locate" => Self::Locate,
            "Track" => Self::Track,
            "Blob" => Self::Blob,
            _ => Self::Locate,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Locate => "Locate",
            Self::Track => "Track",
            Self::Blob => "Blob",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["Locate".into(), "Track".into(), "Blob".into()]
    }
}

#[derive(Debug, Clone)]
pub struct CentroidsConfig {
    pub mode: CentroidsMode,
    pub radius: u16,
    pub max_count: u16,
    pub threshold: u32,
}

// =============================================================================
// Smart Streaming Types (bd-0zge)
// =============================================================================

/// A single entry in a hardware-timed Smart Streaming sequence (bd-0zge)
///
/// Smart Streaming allows loading a sequence of varying exposure times
/// directly onto the camera FPGA, eliminating USB communication jitter
/// between frames. Useful for HDR imaging and time-lapse with varying exposures.
#[derive(Debug, Clone, Copy)]
pub struct SmartStreamEntry {
    /// Exposure time in milliseconds for this frame
    pub exposure_ms: u32,
}

/// Smart Streaming mode options (bd-0zge)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartStreamMode {
    /// Exposures only - varying exposure times per frame
    Exposures,
    /// Interleaved mode (if supported)
    Interleaved,
}

impl SmartStreamMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => SmartStreamMode::Exposures,
            1 => SmartStreamMode::Interleaved,
            _ => SmartStreamMode::Exposures,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            SmartStreamMode::Exposures => 0,
            SmartStreamMode::Interleaved => 1,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Exposures" => SmartStreamMode::Exposures,
            "Interleaved" => SmartStreamMode::Interleaved,
            _ => SmartStreamMode::Exposures,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SmartStreamMode::Exposures => "Exposures",
            SmartStreamMode::Interleaved => "Interleaved",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["Exposures".into(), "Interleaved".into()]
    }
}

// =============================================================================
// Shutter Control Types (bd-e8ah)
// =============================================================================

/// Shutter open mode settings (bd-e8ah)
///
/// Controls the physical shutter behavior or TTL "Expose Out" signal
/// for triggering external light sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutterMode {
    /// Normal operation - shutter opens during exposure
    Normal,
    /// Shutter always open (for external shutter control)
    Open,
    /// Shutter always closed (for dark frames)
    Closed,
    /// No shutter installed/disabled
    None,
    /// Open before trigger (pre-open mode)
    PreOpen,
}

impl ShutterMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ShutterMode::Normal,
            1 => ShutterMode::Open,
            2 => ShutterMode::Closed,
            3 => ShutterMode::None,
            4 => ShutterMode::PreOpen,
            _ => ShutterMode::Normal,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ShutterMode::Normal => 0,
            ShutterMode::Open => 1,
            ShutterMode::Closed => 2,
            ShutterMode::None => 3,
            ShutterMode::PreOpen => 4,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Normal" => ShutterMode::Normal,
            "Open" => ShutterMode::Open,
            "Closed" => ShutterMode::Closed,
            "None" => ShutterMode::None,
            "PreOpen" => ShutterMode::PreOpen,
            _ => ShutterMode::Normal,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ShutterMode::Normal => "Normal",
            ShutterMode::Open => "Open",
            ShutterMode::Closed => "Closed",
            ShutterMode::None => "None",
            ShutterMode::PreOpen => "PreOpen",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Normal".into(),
            "Open".into(),
            "Closed".into(),
            "None".into(),
            "PreOpen".into(),
        ]
    }
}

/// Shutter status reported by camera (bd-e8ah)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutterStatus {
    /// Shutter is closed
    Closed,
    /// Shutter is open
    Open,
    /// Shutter is opening
    Opening,
    /// Shutter is closing
    Closing,
    /// Shutter fault detected
    Fault,
    /// Status unknown
    Unknown,
}

impl ShutterStatus {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ShutterStatus::Closed,
            1 => ShutterStatus::Open,
            2 => ShutterStatus::Opening,
            3 => ShutterStatus::Closing,
            4 => ShutterStatus::Fault,
            _ => ShutterStatus::Unknown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ShutterStatus::Closed => "Closed",
            ShutterStatus::Open => "Open",
            ShutterStatus::Opening => "Opening",
            ShutterStatus::Closing => "Closing",
            ShutterStatus::Fault => "Fault",
            ShutterStatus::Unknown => "Unknown",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Closed".into(),
            "Open".into(),
            "Opening".into(),
            "Closing".into(),
            "Fault".into(),
            "Unknown".into(),
        ]
    }
}

// =============================================================================
// Triggering & Exposure Mode Types (bd-iai9)
// =============================================================================

/// Exposure mode settings (bd-iai9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureMode {
    /// Internal timing - camera controls exposure
    Timed,
    /// Strobe mode - external strobe signal
    Strobe,
    /// Bulb mode - exposure controlled by external signal duration
    Bulb,
    /// Trigger first - wait for trigger, then start exposure
    TriggerFirst,
    /// External edge trigger
    EdgeTrigger,
}

impl ExposureMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ExposureMode::Timed,
            1 => ExposureMode::Strobe,
            2 => ExposureMode::Bulb,
            3 => ExposureMode::TriggerFirst,
            4 => ExposureMode::EdgeTrigger,
            _ => ExposureMode::Timed,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ExposureMode::Timed => 0,
            ExposureMode::Strobe => 1,
            ExposureMode::Bulb => 2,
            ExposureMode::TriggerFirst => 3,
            ExposureMode::EdgeTrigger => 4,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Timed" => ExposureMode::Timed,
            "Strobe" => ExposureMode::Strobe,
            "Bulb" => ExposureMode::Bulb,
            "TriggerFirst" => ExposureMode::TriggerFirst,
            "EdgeTrigger" => ExposureMode::EdgeTrigger,
            _ => ExposureMode::Timed,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExposureMode::Timed => "Timed",
            ExposureMode::Strobe => "Strobe",
            ExposureMode::Bulb => "Bulb",
            ExposureMode::TriggerFirst => "TriggerFirst",
            ExposureMode::EdgeTrigger => "EdgeTrigger",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Timed".into(),
            "Strobe".into(),
            "Bulb".into(),
            "TriggerFirst".into(),
            "EdgeTrigger".into(),
        ]
    }
}

/// Clear mode settings for CCD clearing (bd-iai9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearMode {
    /// Never clear
    Never,
    /// Clear before each exposure
    PreExposure,
    /// Clear before sequence
    PreSequence,
    /// Clear after sequence
    PostSequence,
    /// Clear before and after exposure
    PrePostSequence,
    /// Clear before each frame in sequence
    PreExposurePostSequence,
}

impl ClearMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ClearMode::Never,
            1 => ClearMode::PreExposure,
            2 => ClearMode::PreSequence,
            3 => ClearMode::PostSequence,
            4 => ClearMode::PrePostSequence,
            5 => ClearMode::PreExposurePostSequence,
            _ => ClearMode::PreExposure,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ClearMode::Never => 0,
            ClearMode::PreExposure => 1,
            ClearMode::PreSequence => 2,
            ClearMode::PostSequence => 3,
            ClearMode::PrePostSequence => 4,
            ClearMode::PreExposurePostSequence => 5,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Never" => ClearMode::Never,
            "PreExposure" => ClearMode::PreExposure,
            "PreSequence" => ClearMode::PreSequence,
            "PostSequence" => ClearMode::PostSequence,
            "PrePostSequence" => ClearMode::PrePostSequence,
            "PreExposurePostSequence" => ClearMode::PreExposurePostSequence,
            _ => ClearMode::PreExposure,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ClearMode::Never => "Never",
            ClearMode::PreExposure => "PreExposure",
            ClearMode::PreSequence => "PreSequence",
            ClearMode::PostSequence => "PostSequence",
            ClearMode::PrePostSequence => "PrePostSequence",
            ClearMode::PreExposurePostSequence => "PreExposurePostSequence",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Never".into(),
            "PreExposure".into(),
            "PreSequence".into(),
            "PostSequence".into(),
            "PrePostSequence".into(),
            "PreExposurePostSequence".into(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureResolution {
    Milliseconds,
    Microseconds,
    Seconds,
}

impl ExposureResolution {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ExposureResolution::Milliseconds,
            1 => ExposureResolution::Microseconds,
            2 => ExposureResolution::Seconds,
            _ => ExposureResolution::Milliseconds,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ExposureResolution::Milliseconds => 0,
            ExposureResolution::Microseconds => 1,
            ExposureResolution::Seconds => 2,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Milliseconds" => Self::Milliseconds,
            "Microseconds" => Self::Microseconds,
            "Seconds" => Self::Seconds,
            _ => Self::Milliseconds,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Milliseconds => "Milliseconds",
            Self::Microseconds => "Microseconds",
            Self::Seconds => "Seconds",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Milliseconds".into(),
            "Microseconds".into(),
            "Seconds".into(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRotate {
    None,
    Rotate90CW,
    Rotate180CW,
    Rotate270CW,
}

impl FrameRotate {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FrameRotate::None,
            1 => FrameRotate::Rotate90CW,
            2 => FrameRotate::Rotate180CW,
            3 => FrameRotate::Rotate270CW,
            _ => FrameRotate::None,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            FrameRotate::None => 0,
            FrameRotate::Rotate90CW => 1,
            FrameRotate::Rotate180CW => 2,
            FrameRotate::Rotate270CW => 3,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "None" => FrameRotate::None,
            "90 CW" => FrameRotate::Rotate90CW,
            "180 CW" => FrameRotate::Rotate180CW,
            "270 CW" => FrameRotate::Rotate270CW,
            _ => FrameRotate::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FrameRotate::None => "None",
            FrameRotate::Rotate90CW => "90 CW",
            FrameRotate::Rotate180CW => "180 CW",
            FrameRotate::Rotate270CW => "270 CW",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "None".into(),
            "90 CW".into(),
            "180 CW".into(),
            "270 CW".into(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFlip {
    None,
    FlipX,
    FlipY,
    FlipXY,
}

impl FrameFlip {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FrameFlip::None,
            1 => FrameFlip::FlipX,
            2 => FrameFlip::FlipY,
            3 => FrameFlip::FlipXY,
            _ => FrameFlip::None,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            FrameFlip::None => 0,
            FrameFlip::FlipX => 1,
            FrameFlip::FlipY => 2,
            FrameFlip::FlipXY => 3,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "None" => FrameFlip::None,
            "X" => FrameFlip::FlipX,
            "Y" => FrameFlip::FlipY,
            "XY" => FrameFlip::FlipXY,
            _ => FrameFlip::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FrameFlip::None => "None",
            FrameFlip::FlipX => "X",
            FrameFlip::FlipY => "Y",
            FrameFlip::FlipXY => "XY",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["None".into(), "X".into(), "Y".into(), "XY".into()]
    }
}

/// Expose out mode - controls the expose_out signal (bd-iai9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposeOutMode {
    /// First row exposure timing
    FirstRow,
    /// All rows exposure timing
    AllRows,
    /// Any row exposure timing
    AnyRow,
    /// Rolling shutter mode
    RollingShutter,
    /// Line output mode
    LineOutput,
}

impl ExposeOutMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ExposeOutMode::FirstRow,
            1 => ExposeOutMode::AllRows,
            2 => ExposeOutMode::AnyRow,
            3 => ExposeOutMode::RollingShutter,
            4 => ExposeOutMode::LineOutput,
            _ => ExposeOutMode::FirstRow,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ExposeOutMode::FirstRow => 0,
            ExposeOutMode::AllRows => 1,
            ExposeOutMode::AnyRow => 2,
            ExposeOutMode::RollingShutter => 3,
            ExposeOutMode::LineOutput => 4,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "FirstRow" => ExposeOutMode::FirstRow,
            "First Row" => ExposeOutMode::FirstRow,
            "AllRows" => ExposeOutMode::AllRows,
            "All Rows" => ExposeOutMode::AllRows,
            "AnyRow" => ExposeOutMode::AnyRow,
            "Any Row" => ExposeOutMode::AnyRow,
            "RollingShutter" => ExposeOutMode::RollingShutter,
            "Rolling Shutter" => ExposeOutMode::RollingShutter,
            "LineOutput" => ExposeOutMode::LineOutput,
            "Line Output" => ExposeOutMode::LineOutput,
            _ => ExposeOutMode::FirstRow,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExposeOutMode::FirstRow => "FirstRow",
            ExposeOutMode::AllRows => "AllRows",
            ExposeOutMode::AnyRow => "AnyRow",
            ExposeOutMode::RollingShutter => "RollingShutter",
            ExposeOutMode::LineOutput => "LineOutput",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "FirstRow".into(),
            "AllRows".into(),
            "AnyRow".into(),
            "RollingShutter".into(),
            "LineOutput".into(),
        ]
    }
}

// ─── Edge Trigger (bd-oqo7.5) ──────────────────────────────────────────────

/// Edge trigger mode — selects which trigger edge initiates acquisition.
///
/// Maps to `PARAM_EDGE_TRIGGER` SDK values. Used for external laser
/// synchronization in LIBS experiments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeTrigger {
    /// Trigger on the first edge only
    First,
    /// Trigger on all edges
    All,
}

impl EdgeTrigger {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => EdgeTrigger::First,
            1 => EdgeTrigger::All,
            _ => EdgeTrigger::First,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            EdgeTrigger::First => 0,
            EdgeTrigger::All => 1,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "First" => EdgeTrigger::First,
            "All" => EdgeTrigger::All,
            _ => EdgeTrigger::First,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeTrigger::First => "First",
            EdgeTrigger::All => "All",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["First".into(), "All".into()]
    }
}

// ─── I/O & Diagnostics enums (bd-oqo7.6) ──────────────────────────────────

/// I/O port type — identifies the kind of I/O port at the current address.
///
/// Maps to `PARAM_IO_TYPE` SDK values. Read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoType {
    /// TTL digital I/O
    Ttl,
    /// DAC analog output
    Dac,
}

impl IoType {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            1 => IoType::Dac,
            _ => IoType::Ttl,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            IoType::Ttl => 0,
            IoType::Dac => 1,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ttl => "TTL",
            Self::Dac => "DAC",
        }
    }
}

impl std::fmt::Display for IoType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ttl => write!(f, "TTL"),
            Self::Dac => write!(f, "DAC"),
        }
    }
}

/// Logic output mode — which internal camera signal is routed to the
/// programmable logic output connector.
///
/// Maps to `PARAM_LOGIC_OUTPUT` SDK values (13 modes). Read-write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOutput {
    NotScan,
    Shutter,
    NotReady,
    Logic0,
    Clearing,
    NotFtImageShift,
    Reserved,
    ExposeProg,
    Expose,
    ImageShift,
    Readout,
    Acquiring,
    WaitForTrig,
}

impl LogicOutput {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => LogicOutput::NotScan,
            1 => LogicOutput::Shutter,
            2 => LogicOutput::NotReady,
            3 => LogicOutput::Logic0,
            4 => LogicOutput::Clearing,
            5 => LogicOutput::NotFtImageShift,
            6 => LogicOutput::Reserved,
            7 => LogicOutput::ExposeProg,
            8 => LogicOutput::Expose,
            9 => LogicOutput::ImageShift,
            10 => LogicOutput::Readout,
            11 => LogicOutput::Acquiring,
            12 => LogicOutput::WaitForTrig,
            _ => LogicOutput::NotScan,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            LogicOutput::NotScan => 0,
            LogicOutput::Shutter => 1,
            LogicOutput::NotReady => 2,
            LogicOutput::Logic0 => 3,
            LogicOutput::Clearing => 4,
            LogicOutput::NotFtImageShift => 5,
            LogicOutput::Reserved => 6,
            LogicOutput::ExposeProg => 7,
            LogicOutput::Expose => 8,
            LogicOutput::ImageShift => 9,
            LogicOutput::Readout => 10,
            LogicOutput::Acquiring => 11,
            LogicOutput::WaitForTrig => 12,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotScan => "NotScan",
            Self::Shutter => "Shutter",
            Self::NotReady => "NotReady",
            Self::Logic0 => "Logic0",
            Self::Clearing => "Clearing",
            Self::NotFtImageShift => "NotFtImageShift",
            Self::Reserved => "Reserved",
            Self::ExposeProg => "ExposeProg",
            Self::Expose => "Expose",
            Self::ImageShift => "ImageShift",
            Self::Readout => "Readout",
            Self::Acquiring => "Acquiring",
            Self::WaitForTrig => "WaitForTrig",
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Self {
        match s {
            "NotScan" => Self::NotScan,
            "Shutter" => Self::Shutter,
            "NotReady" => Self::NotReady,
            "Logic0" => Self::Logic0,
            "Clearing" => Self::Clearing,
            "NotFtImageShift" => Self::NotFtImageShift,
            "Reserved" => Self::Reserved,
            "ExposeProg" => Self::ExposeProg,
            "Expose" => Self::Expose,
            "ImageShift" => Self::ImageShift,
            "Readout" => Self::Readout,
            "Acquiring" => Self::Acquiring,
            "WaitForTrig" => Self::WaitForTrig,
            _ => Self::NotScan,
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "NotScan".into(),
            "Shutter".into(),
            "NotReady".into(),
            "Logic0".into(),
            "Clearing".into(),
            "NotFtImageShift".into(),
            "Reserved".into(),
            "ExposeProg".into(),
            "Expose".into(),
            "ImageShift".into(),
            "Readout".into(),
            "Acquiring".into(),
            "WaitForTrig".into(),
        ]
    }
}

impl std::fmt::Display for LogicOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotScan => write!(f, "Not Scan"),
            Self::Shutter => write!(f, "Shutter"),
            Self::NotReady => write!(f, "Not Ready"),
            Self::Logic0 => write!(f, "Logic 0"),
            Self::Clearing => write!(f, "Clearing"),
            Self::NotFtImageShift => write!(f, "Not FT Image Shift"),
            Self::Reserved => write!(f, "Reserved"),
            Self::ExposeProg => write!(f, "Expose Prog"),
            Self::Expose => write!(f, "Expose"),
            Self::ImageShift => write!(f, "Image Shift"),
            Self::Readout => write!(f, "Readout"),
            Self::Acquiring => write!(f, "Acquiring"),
            Self::WaitForTrig => write!(f, "Wait For Trigger"),
        }
    }
}

// ─── PrimeLocate (bd-oqo7.10) ─────────────────────────────────────────────

/// A single localization event from PrimeLocate FPGA processing.
///
/// When PrimeLocate is active, the camera FPGA evaluates each frame on-chip
/// and outputs only the coordinates and intensity of detected spots. This
/// reduces data bandwidth by orders of magnitude compared to full-frame readout,
/// enabling high-throughput single-molecule tracking.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalizationEvent {
    /// X coordinate of the localized spot (sub-pixel, sensor coordinates).
    pub x: f32,
    /// Y coordinate of the localized spot (sub-pixel, sensor coordinates).
    pub y: f32,
    /// Integrated intensity of the localized spot (ADU).
    pub intensity: f32,
}

// =============================================================================
// Scan Mode (Programmable Scan Mode)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    Auto,
    ProgrammableLineDelay,
    ProgrammableScanWidth,
}

impl ScanMode {
    pub fn from_pvcam(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Auto),
            1 => Some(Self::ProgrammableLineDelay),
            2 => Some(Self::ProgrammableScanWidth),
            _ => None,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            Self::Auto => 0,
            Self::ProgrammableLineDelay => 1,
            Self::ProgrammableScanWidth => 2,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "line_delay" | "programmable_line_delay" => Some(Self::ProgrammableLineDelay),
            "scan_width" | "programmable_scan_width" => Some(Self::ProgrammableScanWidth),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::ProgrammableLineDelay => "ProgrammableLineDelay",
            Self::ProgrammableScanWidth => "ProgrammableScanWidth",
        }
    }

    pub fn all_choices() -> &'static [&'static str] {
        &["Auto", "ProgrammableLineDelay", "ProgrammableScanWidth"]
    }
}

impl std::fmt::Display for ScanMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// =============================================================================
// Scan Direction (Programmable Scan Mode)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanDirection {
    Down,
    Up,
    DownUpAlternate,
}

impl ScanDirection {
    pub fn from_pvcam(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Down),
            1 => Some(Self::Up),
            2 => Some(Self::DownUpAlternate),
            _ => None,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            Self::Down => 0,
            Self::Up => 1,
            Self::DownUpAlternate => 2,
        }
    }

    #[expect(clippy::should_implement_trait, reason = "infallible from_str returns default on unknown input; cannot implement FromStr which returns Result")]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "down" => Some(Self::Down),
            "up" => Some(Self::Up),
            "down_up" | "down_up_alternate" | "alternate" => Some(Self::DownUpAlternate),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Down => "Down",
            Self::Up => "Up",
            Self::DownUpAlternate => "DownUpAlternate",
        }
    }

    pub fn all_choices() -> &'static [&'static str] {
        &["Down", "Up", "DownUpAlternate"]
    }
}

impl std::fmt::Display for ScanDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// =============================================================================
// Buffer Mode (bd-vw77y)
// =============================================================================

/// Acquisition buffer mode for PVCAM streaming.
///
/// Controls how the PVCAM SDK manages the circular DMA buffer
/// during continuous acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BufferMode {
    /// Circular buffer with overwrite: DMA continues writing, old frames discarded.
    /// Currently force-disabled at runtime (bd-g3ap) due to unsafe unlock-before-copy
    /// pattern, but kept as an option for future re-enablement.
    Overwrite,
    /// Circular buffer without overwrite: DMA stops when buffer is full (FIFO).
    #[default]
    #[serde(rename = "No Overwrite", alias = "NoOverwrite")]
    NoOverwrite,
    /// Sequence mode: batch-based non-circular acquisition via
    /// `pl_exp_setup_seq`/`pl_exp_start_seq`. Useful for single-frame capture
    /// workflows or diagnostics (bd-9pel).
    Sequence,
}

impl std::str::FromStr for BufferMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Overwrite" => Ok(Self::Overwrite),
            "No Overwrite" | "NoOverwrite" => Ok(Self::NoOverwrite),
            "Sequence" => Ok(Self::Sequence),
            _ => Err(anyhow::anyhow!("unknown buffer mode: {s:?}")),
        }
    }
}

impl BufferMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Overwrite => "Overwrite",
            Self::NoOverwrite => "No Overwrite",
            Self::Sequence => "Sequence",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["Overwrite".into(), "No Overwrite".into(), "Sequence".into()]
    }
}

impl std::fmt::Display for BufferMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
