//! V3 Hardware Bindings for Rhai Scripts
//!
//! This module provides the bridge between V3 async Rust instruments and
//! synchronous Rhai scripts. It exposes V3 Instrument trait and meta traits
//! (Camera, PowerMeter, Stage, Laser) as Rhai-compatible types and methods.
//!
//! # Architecture
//!
//! - `V3CameraHandle` - Wraps instruments implementing `Camera` trait
//! - `V3PowerMeterHandle` - Wraps instruments implementing `PowerMeter` trait
//! - `V3StageHandle` - Wraps instruments implementing `Stage` trait
//! - `V3LaserHandle` - Wraps instruments implementing `Laser` trait
//! - `register_v3_hardware()` - Registers all V3 types and methods with Rhai engine
//!
//! # Async→Sync Bridge
//!
//! Uses `tokio::task::block_in_place()` to safely execute async V3 operations
//! from synchronous Rhai scripts. This allows scripts to call methods like
//! `camera.set_exposure(100.0)` without dealing with async/await.
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use rust_daq::scripting::bindings_v3::{register_v3_hardware, V3CameraHandle};
//! use rhai::{Engine, Scope};
//!
//! let mut engine = Engine::new();
//! register_v3_hardware(&mut engine);
//!
//! let camera = MockCameraV3::new("camera1");
//! let mut scope = Scope::new();
//! scope.push("camera", V3CameraHandle { instrument: Arc::new(Mutex::new(camera)) });
//!
//! let script = r#"
//!     camera.set_exposure(50.0);
//!     camera.start_acquisition();
//!     sleep(0.5);
//!     camera.stop_acquisition();
//! "#;
//!
//! engine.eval_with_scope(&mut scope, script)?;
//! ```

use rhai::{Dynamic, Engine};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::task::block_in_place;

use crate::core::{Camera, Laser, PowerMeter, Roi, Stage};

// =============================================================================
// Handle Types - V3 Rhai-Compatible Wrappers
// =============================================================================

/// Handle to a V3 Camera instrument for use in Rhai scripts
///
/// Wraps any instrument implementing the `Camera` trait (PVCAM, Mock, etc.).
/// Provides synchronous methods that scripts can call directly.
///
/// # Script Example
/// ```rhai
/// camera.set_exposure(100.0);
/// camera.set_binning(2, 2);
/// camera.start_acquisition();
/// sleep(1.0);
/// camera.stop_acquisition();
/// ```
#[derive(Clone)]
pub struct V3CameraHandle {
    pub instrument: Arc<Mutex<dyn Camera>>,
}

/// Handle to a V3 PowerMeter instrument for use in Rhai scripts
///
/// Wraps any instrument implementing the `PowerMeter` trait (Newport 1830-C, etc.).
/// Provides synchronous methods for power measurement.
///
/// # Script Example
/// ```rhai
/// power_meter.set_wavelength(800.0);
/// power_meter.zero();
/// let cmd = power_meter.start_command();
/// power_meter.execute(cmd);
/// ```
#[derive(Clone)]
pub struct V3PowerMeterHandle {
    pub instrument: Arc<Mutex<dyn PowerMeter>>,
}

/// Handle to a V3 Stage instrument for use in Rhai scripts
///
/// Wraps any instrument implementing the `Stage` trait (ESP300, etc.).
/// Provides synchronous methods for motion control.
///
/// # Script Example
/// ```rhai
/// stage.move_absolute(10.5);
/// stage.wait_settled(5.0);
/// let pos = stage.position();
/// print("Current position: " + pos + " mm");
/// ```
#[derive(Clone)]
pub struct V3StageHandle {
    pub instrument: Arc<Mutex<dyn Stage>>,
}

/// Handle to a V3 Laser instrument for use in Rhai scripts
///
/// Wraps any instrument implementing the `Laser` trait (Mai Tai, etc.).
/// Provides synchronous methods for laser control.
///
/// # Script Example
/// ```rhai
/// laser.set_wavelength(800.0);
/// laser.shutter_open();
/// sleep(1.0);
/// laser.shutter_close();
/// ```
#[derive(Clone)]
pub struct V3LaserHandle {
    pub instrument: Arc<Mutex<dyn Laser>>,
}

// =============================================================================
// V3 Hardware Registration
// =============================================================================

/// Register all V3 hardware bindings with the Rhai engine
///
/// This function registers:
/// - Custom types: `V3Camera`, `V3PowerMeter`, `V3Stage`, `V3Laser`
/// - Camera methods: `set_exposure`, `set_roi`, `set_binning`, `start/stop_acquisition`, `arm_trigger`, `trigger`
/// - PowerMeter methods: `set_wavelength`, `set_range`, `zero`
/// - Stage methods: `move_absolute`, `move_relative`, `position`, `stop_motion`, `is_moving`, `home`, `set_velocity`, `wait_settled`
/// - Laser methods: `set_wavelength`, `wavelength`, `set_power`, `power`, `shutter_open`, `shutter_close`
/// - Common Instrument methods: `id`, `state`, `initialize`, `shutdown`
/// - Utility functions: `sleep`
///
/// # Arguments
/// * `engine` - Mutable reference to Rhai engine
///
/// # Example
/// ```rust,ignore
/// let mut engine = Engine::new();
/// register_v3_hardware(&mut engine);
/// ```
pub fn register_v3_hardware(engine: &mut Engine) {
    // Register custom types with human-readable names
    engine.register_type_with_name::<V3CameraHandle>("V3Camera");
    engine.register_type_with_name::<V3PowerMeterHandle>("V3PowerMeter");
    engine.register_type_with_name::<V3StageHandle>("V3Stage");
    engine.register_type_with_name::<V3LaserHandle>("V3Laser");

    // =========================================================================
    // Common Instrument Methods
    // =========================================================================

    // camera.id() - Get instrument ID
    engine.register_fn("id", |camera: &mut V3CameraHandle| -> String {
        let inst = block_in_place(|| camera.instrument.blocking_lock());
        inst.id().to_string()
    });

    engine.register_fn("id", |pm: &mut V3PowerMeterHandle| -> String {
        let inst = block_in_place(|| pm.instrument.blocking_lock());
        inst.id().to_string()
    });

    engine.register_fn("id", |stage: &mut V3StageHandle| -> String {
        let inst = block_in_place(|| stage.instrument.blocking_lock());
        inst.id().to_string()
    });

    engine.register_fn("id", |laser: &mut V3LaserHandle| -> String {
        let inst = block_in_place(|| laser.instrument.blocking_lock());
        inst.id().to_string()
    });

    // camera.state() - Get instrument state as string
    engine.register_fn("state", |camera: &mut V3CameraHandle| -> String {
        let inst = block_in_place(|| camera.instrument.blocking_lock());
        format!("{:?}", inst.state())
    });

    engine.register_fn("state", |pm: &mut V3PowerMeterHandle| -> String {
        let inst = block_in_place(|| pm.instrument.blocking_lock());
        format!("{:?}", inst.state())
    });

    engine.register_fn("state", |stage: &mut V3StageHandle| -> String {
        let inst = block_in_place(|| stage.instrument.blocking_lock());
        format!("{:?}", inst.state())
    });

    engine.register_fn("state", |laser: &mut V3LaserHandle| -> String {
        let inst = block_in_place(|| laser.instrument.blocking_lock());
        format!("{:?}", inst.state())
    });

    // camera.initialize() - Initialize hardware connection
    engine.register_fn("initialize", |camera: &mut V3CameraHandle| {
        let inst = camera.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.initialize().await
            })
        })
        .unwrap()
    });

    engine.register_fn("initialize", |pm: &mut V3PowerMeterHandle| {
        let inst = pm.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.initialize().await
            })
        })
        .unwrap()
    });

    engine.register_fn("initialize", |stage: &mut V3StageHandle| {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.initialize().await
            })
        })
        .unwrap()
    });

    engine.register_fn("initialize", |laser: &mut V3LaserHandle| {
        let inst = laser.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.initialize().await
            })
        })
        .unwrap()
    });

    // =========================================================================
    // Camera Methods - V3 Camera Trait
    // =========================================================================

    // camera.set_exposure(100.0) - Set exposure time in milliseconds
    engine.register_fn("set_exposure", |camera: &mut V3CameraHandle, ms: f64| {
        let inst = camera.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.set_exposure(ms).await
            })
        })
        .unwrap()
    });

    // camera.set_binning(2, 2) - Set pixel binning
    engine.register_fn(
        "set_binning",
        |camera: &mut V3CameraHandle, h: i64, v: i64| {
            let inst = camera.instrument.clone();
            block_in_place(|| {
                Handle::current().block_on(async {
                    let mut locked = inst.lock().await;
                    locked.set_binning(h as u32, v as u32).await
                })
            })
            .unwrap()
        },
    );

    // camera.start_acquisition() - Start continuous acquisition
    engine.register_fn("start_acquisition", |camera: &mut V3CameraHandle| {
        let inst = camera.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.start_acquisition().await
            })
        })
        .unwrap()
    });

    // camera.stop_acquisition() - Stop acquisition
    engine.register_fn("stop_acquisition", |camera: &mut V3CameraHandle| {
        let inst = camera.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.stop_acquisition().await
            })
        })
        .unwrap()
    });

    // camera.arm_trigger() - Arm for triggered acquisition
    engine.register_fn("arm_trigger", |camera: &mut V3CameraHandle| {
        let inst = camera.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.arm_trigger().await
            })
        })
        .unwrap()
    });

    // camera.trigger() - Software trigger
    engine.register_fn("trigger", |camera: &mut V3CameraHandle| {
        let inst = camera.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.trigger().await
            })
        })
        .unwrap()
    });

    // camera.roi() - Get current ROI as array [x, y, width, height]
    engine.register_fn("roi", |camera: &mut V3CameraHandle| -> Dynamic {
        let inst = camera.instrument.clone();
        let roi = block_in_place(|| {
            Handle::current().block_on(async {
                let locked = inst.lock().await;
                locked.roi().await
            })
        });
        Dynamic::from(vec![
            Dynamic::from(roi.x as i64),
            Dynamic::from(roi.y as i64),
            Dynamic::from(roi.width as i64),
            Dynamic::from(roi.height as i64),
        ])
    });

    // camera.set_roi(x, y, width, height) - Set region of interest
    engine.register_fn(
        "set_roi",
        |camera: &mut V3CameraHandle, x: i64, y: i64, width: i64, height: i64| {
            let inst = camera.instrument.clone();
            let roi = Roi {
                x: x as u32,
                y: y as u32,
                width: width as u32,
                height: height as u32,
            };
            block_in_place(|| {
                Handle::current().block_on(async {
                    let mut locked = inst.lock().await;
                    locked.set_roi(roi).await
                })
            })
            .unwrap()
        },
    );

    // =========================================================================
    // PowerMeter Methods - V3 PowerMeter Trait
    // =========================================================================

    // power_meter.set_wavelength(800.0) - Set wavelength for calibration
    engine.register_fn("set_wavelength", |pm: &mut V3PowerMeterHandle, nm: f64| {
        let inst = pm.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.set_wavelength(nm).await
            })
        })
        .unwrap()
    });

    // power_meter.set_range(0.001) - Set measurement range in watts
    engine.register_fn("set_range", |pm: &mut V3PowerMeterHandle, watts: f64| {
        let inst = pm.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.set_range(watts).await
            })
        })
        .unwrap()
    });

    // power_meter.zero() - Zero/calibrate sensor
    engine.register_fn("zero", |pm: &mut V3PowerMeterHandle| {
        let inst = pm.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.zero().await
            })
        })
        .unwrap()
    });

    // =========================================================================
    // Stage Methods - V3 Stage Trait
    // =========================================================================

    // stage.move_absolute(10.5) - Move to absolute position in mm
    engine.register_fn("move_absolute", |stage: &mut V3StageHandle, pos: f64| {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.move_absolute(pos).await
            })
        })
        .unwrap()
    });

    // stage.move_relative(2.5) - Move relative distance in mm
    engine.register_fn("move_relative", |stage: &mut V3StageHandle, dist: f64| {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.move_relative(dist).await
            })
        })
        .unwrap()
    });

    // let pos = stage.position() - Get current position in mm
    engine.register_fn("position", |stage: &mut V3StageHandle| -> f64 {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let locked = inst.lock().await;
                locked.position().await
            })
        })
        .unwrap()
    });

    // stage.stop_motion() - Stop motion immediately
    engine.register_fn("stop_motion", |stage: &mut V3StageHandle| {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.stop_motion().await
            })
        })
        .unwrap()
    });

    // let moving = stage.is_moving() - Check if stage is moving
    engine.register_fn("is_moving", |stage: &mut V3StageHandle| -> bool {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let locked = inst.lock().await;
                locked.is_moving().await
            })
        })
        .unwrap()
    });

    // stage.home() - Home stage (find reference position)
    engine.register_fn("home", |stage: &mut V3StageHandle| {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.home().await
            })
        })
        .unwrap()
    });

    // stage.set_velocity(10.0) - Set velocity in mm/s
    engine.register_fn("set_velocity", |stage: &mut V3StageHandle, mm_per_sec: f64| {
        let inst = stage.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.set_velocity(mm_per_sec).await
            })
        })
        .unwrap()
    });

    // stage.wait_settled(5.0) - Wait for motion to settle (timeout in seconds)
    engine.register_fn("wait_settled", |stage: &mut V3StageHandle, timeout_sec: f64| {
        let inst = stage.instrument.clone();
        let timeout = std::time::Duration::from_secs_f64(timeout_sec);
        block_in_place(|| {
            Handle::current().block_on(async {
                let locked = inst.lock().await;
                locked.wait_settled(timeout).await
            })
        })
        .unwrap()
    });

    // =========================================================================
    // Laser Methods - V3 Laser Trait
    // =========================================================================

    // laser.set_wavelength(800.0) - Set wavelength in nm
    engine.register_fn("set_wavelength", |laser: &mut V3LaserHandle, nm: f64| {
        let inst = laser.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.set_wavelength(nm).await
            })
        })
        .unwrap()
    });

    // let wl = laser.get_wavelength() - Get wavelength in nm
    engine.register_fn("get_wavelength", |laser: &mut V3LaserHandle| -> f64 {
        let inst = laser.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let locked = inst.lock().await;
                locked.wavelength().await
            })
        })
        .unwrap()
    });

    // laser.set_power(2.5) - Set output power in watts
    engine.register_fn("set_power", |laser: &mut V3LaserHandle, watts: f64| {
        let inst = laser.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.set_power(watts).await
            })
        })
        .unwrap()
    });

    // let pwr = laser.power() - Get current power in watts
    engine.register_fn("power", |laser: &mut V3LaserHandle| -> f64 {
        let inst = laser.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let locked = inst.lock().await;
                locked.power().await
            })
        })
        .unwrap()
    });

    // laser.shutter_open() - Open shutter (calls enable_shutter)
    engine.register_fn("shutter_open", |laser: &mut V3LaserHandle| {
        let inst = laser.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.enable_shutter().await
            })
        })
        .unwrap()
    });

    // laser.shutter_close() - Close shutter (calls disable_shutter)
    engine.register_fn("shutter_close", |laser: &mut V3LaserHandle| {
        let inst = laser.instrument.clone();
        block_in_place(|| {
            Handle::current().block_on(async {
                let mut locked = inst.lock().await;
                locked.disable_shutter().await
            })
        })
        .unwrap()
    });

    // =========================================================================
    // Utility Functions
    // =========================================================================

    // sleep(0.5) - Sleep for seconds (uses std::thread::sleep, safe in Rhai context)
    engine.register_fn("sleep", |seconds: f64| {
        use std::thread;
        use std::time::Duration;
        thread::sleep(Duration::from_secs_f64(seconds));
    });
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_v3_hardware_succeeds() {
        let mut engine = Engine::new();
        register_v3_hardware(&mut engine); // Should not panic
    }

    // Note: Full integration tests require mock V3 instruments
    // See examples/scripting_v3_demo.rs for complete examples
}
