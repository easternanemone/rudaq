//! Hardware-agnostic traits for instrument types
//!
//! Meta-instrument interfaces that any instrument actor must implement.
//! Enables runtime polymorphism and instrument assignment.

// Phase 1A-1C traits
pub mod power_meter;
pub mod tunable_laser;

// Phase 1D traits
pub mod camera_sensor;
pub mod motion_controller;
pub mod scpi_endpoint;

// Phase 1A-1C exports
pub use self::power_meter::{PowerMeter, PowerMeasurement, PowerUnit, Wavelength};
pub use self::tunable_laser::{
    LaserMeasurement, ShutterState, TunableLaser, Wavelength as LaserWavelength,
};

// Phase 1D exports
pub use self::camera_sensor::{
    BinningConfig, CameraCapabilities, CameraSensor, CameraStreamConfig, CameraTiming, Frame,
    PixelFormat, RegionOfInterest, TriggerMode,
};
pub use self::motion_controller::{
    AxisPosition, AxisState, MotionConfig, MotionController, MotionEvent,
};
pub use self::scpi_endpoint::{ScpiEndpoint, ScpiEvent};
