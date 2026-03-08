//! Well-known `panel_kind` string constants for explicit UI panel routing.
//!
//! These constants are used by drivers to set `DeviceMetadata::panel_kind` and by the
//! UI dispatch layer to route devices to their correct control panel without heuristics.
//!
//! # Usage
//!
//! In a driver's `build()` method:
//! ```rust,ignore
//! use common::panel_kind;
//!
//! components.metadata.panel_kind = Some(panel_kind::PVCAM.to_string());
//! ```

/// Photometrics PVCAM camera panel (image viewer + camera controls)
pub const PVCAM: &str = "pvcam";

/// Andor iStar / sCMOS camera panel (gated camera, DDG, MCP)
pub const ANDOR_CAMERA: &str = "andor_camera";

/// Andor Shamrock spectrograph panel (grating, wavelength, slit)
pub const ANDOR_SHAMROCK: &str = "andor_shamrock";

/// Comedi DAQ panel (analog input, analog output, DIO, counters)
pub const COMEDI: &str = "comedi";

/// Dover SmartStage panel (motion + trigger-on-position)
pub const DOVER_STAGE: &str = "dover_stage";

/// MaiTai Ti:Sapphire laser panel (wavelength, emission, shutter)
pub const MAITAI: &str = "maitai";

/// Power meter panel (scalar readout sensor)
pub const POWER_METER: &str = "power_meter";

/// Rotator panel (ELL14-style rotation mount)
pub const ROTATOR: &str = "rotator";

/// Generic linear/XY stage panel
pub const STAGE: &str = "stage";

/// Generic device panel (fallback for unrecognized devices)
pub const GENERIC: &str = "generic";
