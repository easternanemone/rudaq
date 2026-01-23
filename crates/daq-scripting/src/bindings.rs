//! Hardware Bindings for Rhai Scripts
//!
//! This module provides the bridge between async Rust hardware traits and
//! synchronous Rhai scripts. It exposes hardware capabilities (Movable, Triggerable,
//! FrameProducer) as Rhai-compatible types and methods.
//!
//! # Architecture
//!
//! - `StageHandle` - Wraps devices implementing `Movable`
//! - `CameraHandle` - Wraps devices implementing `Triggerable + FrameProducer`
//! - `register_hardware()` - Registers all types and methods with Rhai engine
//!
//! # Async→Sync Bridge
//!
//! Uses a guarded `block_in_place` helper that only runs on Tokio's multi-thread
//! scheduler (current-thread would deadlock). This allows scripts to call
//! `stage.move_abs(10.0)` without dealing with async/await while keeping runtime
//! safety explicit.
//!
//! # Example Usage
//!
//! ```rust,ignore
//! let mut engine = Engine::new();
//! register_hardware(&mut engine);
//!
//! let mut scope = Scope::new();
//! scope.push("stage", StageHandle { driver: Arc::new(mock_stage) });
//!
//! let script = r#"
//!     stage.move_abs(10.0);
//!     let pos = stage.position();
//!     print("Position: " + pos);
//! "#;
//!
//! engine.eval_with_scope(&mut scope, script)?;
//! ```

use chrono::Utc;
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, NativeCallContext, Position};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::sync::broadcast;
use tokio::task::block_in_place;

use daq_core::core::Measurement;
use daq_hardware::capabilities::{Camera, Movable, Readable, ShutterControl};

// Helper to execute async hardware calls from synchronous Rhai functions without
// risking deadlock on a current-thread runtime. Returns a Rhai-compatible error
// with a clear message when the runtime flavor is unsupported.
fn run_blocking<Fut, T, E>(label: &str, fut: Fut) -> Result<T, Box<EvalAltResult>>
where
    Fut: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: std::fmt::Display,
{
    let handle = Handle::try_current().map_err(|e| {
        Box::new(EvalAltResult::ErrorRuntime(
            format!("{}: missing Tokio runtime ({})", label, e).into(),
            Position::NONE,
        ))
    })?;

    if handle.runtime_flavor() == RuntimeFlavor::CurrentThread {
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!(
                "{}: Tokio current-thread runtime cannot run blocking hardware calls. \
                 Use the multi-thread runtime (#[tokio::main(flavor = \"multi_thread\")]).",
                label
            )
            .into(),
            Position::NONE,
        )));
    }

    block_in_place(|| handle.block_on(fut)).map_err(|e| {
        Box::new(EvalAltResult::ErrorRuntime(
            format!("{}: {}", label, e).into(),
            Position::NONE,
        ))
    })
}

// =============================================================================
// Handle Types - Rhai-Compatible Wrappers
// =============================================================================

/// Soft position limits for stage safety (bd-jnfu.4)
///
/// These limits are enforced by the scripting layer BEFORE commands
/// are sent to hardware, preventing scripts from driving stages to
/// unsafe positions.
#[derive(Clone, Debug)]
pub struct SoftLimits {
    /// Minimum allowed position (None = no limit)
    pub min: Option<f64>,
    /// Maximum allowed position (None = no limit)
    pub max: Option<f64>,
}

impl SoftLimits {
    /// Create unlimited soft limits (no restrictions)
    pub fn unlimited() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// Create soft limits with min and max bounds
    pub fn new(min: f64, max: f64) -> Self {
        Self {
            min: Some(min),
            max: Some(max),
        }
    }

    /// Check if a position is within soft limits
    pub fn validate(&self, position: f64) -> Result<(), String> {
        if let Some(min) = self.min {
            if position < min {
                return Err(format!(
                    "Position {} is below soft limit minimum {}",
                    position, min
                ));
            }
        }
        if let Some(max) = self.max {
            if position > max {
                return Err(format!(
                    "Position {} exceeds soft limit maximum {}",
                    position, max
                ));
            }
        }
        Ok(())
    }
}

impl Default for SoftLimits {
    fn default() -> Self {
        Self::unlimited()
    }
}

/// Handle to a stage device that can be used in Rhai scripts
///
/// Wraps any device implementing `Movable` trait (stages, actuators, goniometers).
/// Provides synchronous methods that scripts can call directly.
///
/// # Safety (bd-jnfu.4)
/// Soft limits can be configured to prevent scripts from commanding
/// hardware to unsafe positions. These are checked BEFORE any hardware
/// command is issued.
///
/// # Script Example
/// ```rhai
/// stage.move_abs(10.0);
/// stage.wait_settled();
/// let pos = stage.position();
/// print("Current position: " + pos + "mm");
/// ```
#[derive(Clone)]
pub struct StageHandle {
    /// Hardware driver implementing the Movable trait.
    ///
    /// This is typically an Arc-wrapped driver (e.g., `Arc<MockStage>`, `Arc<Esp300>`)
    /// that provides position control methods. The Arc enables sharing the driver
    /// across multiple script handles and async tasks.
    pub driver: Arc<dyn Movable>,
    /// Optional data sender for broadcasting measurements to RingBuffer/gRPC clients
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
    /// Soft position limits (bd-jnfu.4) - validated before hardware commands
    pub soft_limits: SoftLimits,
}

/// Handle to a camera device that can be used in Rhai scripts
///
/// Wraps any device implementing `Camera` trait (which combines Triggerable + FrameProducer).
/// Provides synchronous methods for camera control.
///
/// # Script Example
/// ```rhai
/// camera.arm();
/// camera.trigger();
/// let res = camera.resolution();
/// print("Resolution: " + res[0] + "x" + res[1]);
/// ```
#[derive(Clone)]
pub struct CameraHandle {
    /// Hardware driver implementing the Camera trait.
    ///
    /// This is typically an Arc-wrapped driver (e.g., `Arc<MockCamera>`, `Arc<PvcamDriver>`)
    /// that provides camera control and frame acquisition methods. The Arc enables sharing
    /// the driver across multiple script handles and async tasks.
    pub driver: Arc<dyn Camera>,
    /// Optional data sender for broadcasting measurements to RingBuffer/gRPC clients
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
}

/// Handle to a readable device (power meters, sensors) for Rhai scripts
///
/// Wraps any device implementing `Readable` trait.
/// Provides synchronous methods for scalar value acquisition.
///
/// # Script Example
/// ```rhai
/// let power = power_meter.read();
/// print("Power: " + power + " W");
///
/// let avg = power_meter.read_averaged(10);
/// print("Averaged power: " + avg + " W");
/// ```
#[derive(Clone)]
pub struct ReadableHandle {
    /// Hardware driver implementing the Readable trait.
    pub driver: Arc<dyn Readable>,
    /// Optional data sender for broadcasting measurements
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
}

/// Newport 1830-C specific handle with zeroing capability
///
/// This handle extends ReadableHandle with Newport-specific methods like zero().
#[cfg(feature = "hardware_factories")]
#[derive(Clone)]
pub struct Newport1830CHandle {
    /// Newport 1830-C driver with direct access to all methods
    pub driver: Arc<daq_driver_newport::Newport1830CDriver>,
    /// Optional data sender for broadcasting measurements
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
}

/// Handle to a shutter device (laser shutters) for Rhai scripts
///
/// Wraps any device implementing `ShutterControl` trait.
/// Provides synchronous methods for shutter control with safety guarantees.
///
/// # Script Example
/// ```rhai
/// laser.open();
/// // ... do work with beam ...
/// laser.close();
///
/// // Or use the safety wrapper:
/// with_shutter_open(laser, || {
///     // Beam is available here
///     // Shutter closes automatically, even on error
/// });
/// ```
#[derive(Clone)]
pub struct ShutterHandle {
    /// Hardware driver implementing the ShutterControl trait.
    pub driver: Arc<dyn ShutterControl>,
}

/// Handle to an HDF5 file for data storage in Rhai scripts
///
/// Provides methods to write datasets and attributes to HDF5 files.
///
/// # Script Example
/// ```rhai
/// let hdf5 = create_hdf5("experiment_data.h5");
/// hdf5.write_attr("experiment", "polarization_scan");
/// hdf5.write_array("power_data", data_array);
/// hdf5.close();
/// ```
#[cfg(feature = "hdf5_scripting")]
#[derive(Clone)]
pub struct Hdf5Handle {
    /// HDF5 file handle protected by mutex
    file: Arc<tokio::sync::Mutex<Option<hdf5::File>>>,
    /// File path for reference
    path: std::path::PathBuf,
}

// =============================================================================
// Hardware Registration
// =============================================================================

/// Register all hardware bindings with the Rhai engine
///
/// This function registers:
/// - Custom types: `Stage`, `Camera`
/// - Stage methods: `move_abs`, `move_rel`, `position`, `wait_settled`
/// - Camera methods: `arm`, `trigger`, `resolution`
/// - Utility functions: `sleep`
///
/// # Arguments
/// * `engine` - Mutable reference to Rhai engine
///
/// # Example
/// ```rust,ignore
/// let mut engine = Engine::new();
/// register_hardware(&mut engine);
/// ```
pub fn register_hardware(engine: &mut Engine) {
    // Register custom types with human-readable names
    engine.register_type_with_name::<StageHandle>("Stage");
    engine.register_type_with_name::<CameraHandle>("Camera");

    // =========================================================================
    // Stage Methods - Motion Control
    // =========================================================================

    // stage.move_abs(10.0) - Move to absolute position
    // SAFETY (bd-jnfu.4): Validates against soft limits BEFORE hardware command
    // DEPRECATED (bd-94zq.4): Use yield_move() instead for Document emission
    engine.register_fn(
        "move_abs",
        move |stage: &mut StageHandle, pos: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            // Deprecation warning (bd-94zq.4) - emitted once per call
            tracing::warn!(
                target: "daq_scripting::deprecation",
                "Direct stage.move_abs() is DEPRECATED (v0.7.0). \
                 Use yield_move(device_id, position) instead for proper Document emission. \
                 Direct hardware commands will be removed in v0.9.0."
            );

            // Validate soft limits BEFORE issuing hardware command (bd-jnfu.4)
            if let Err(e) = stage.soft_limits.validate(pos) {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    format!("Stage move_abs: soft limit violation: {}", e).into(),
                    Position::NONE,
                )));
            }

            run_blocking("Stage move_abs", stage.driver.move_abs(pos))?;

            // Send measurement to broadcast channel if sender available
            if let Some(ref tx) = stage.data_tx {
                let measurement = Measurement::Scalar {
                    name: "stage_position".to_string(),
                    value: pos,
                    unit: "mm".to_string(),
                    timestamp: Utc::now(),
                };

                // Ignore errors if no receivers (non-critical)
                let _ = tx.send(measurement);
            }

            Ok(Dynamic::UNIT)
        },
    );

    // stage.move_rel(5.0) - Move relative distance
    // SAFETY (bd-jnfu.4): Validates resulting position against soft limits
    // FIX (bd-jnfu.17): Convert to atomic move_abs to prevent TOCTOU race condition
    // If position changes between read and move, we still land at a validated position
    // DEPRECATED (bd-94zq.4): Use yield_move() instead for Document emission
    engine.register_fn(
        "move_rel",
        move |stage: &mut StageHandle, dist: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            // Deprecation warning (bd-94zq.4)
            tracing::warn!(
                target: "daq_scripting::deprecation",
                "Direct stage.move_rel() is DEPRECATED (v0.7.0). \
                 Use yield_move(device_id, position) with absolute positioning instead. \
                 Direct hardware commands will be removed in v0.9.0."
            );

            // Get current position to calculate target, then validate soft limits (bd-jnfu.4)
            let current_pos = run_blocking("Stage position", stage.driver.position())?;
            let target_pos = current_pos + dist;

            if let Err(e) = stage.soft_limits.validate(target_pos) {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    format!(
                        "Stage move_rel: soft limit violation (current: {}, relative: {}, target: {}): {}",
                        current_pos, dist, target_pos, e
                    ).into(),
                    Position::NONE,
                )));
            }

            // Use move_abs instead of move_rel to eliminate TOCTOU race (bd-jnfu.17)
            // This ensures we move to the validated target position regardless of
            // any intermediate position changes (e.g., from concurrent controllers)
            run_blocking("Stage move_abs (atomic from move_rel)", stage.driver.move_abs(target_pos))?;
            Ok(Dynamic::UNIT)
        },
    );

    // let pos = stage.position() - Get current position
    engine.register_fn(
        "position",
        move |stage: &mut StageHandle| -> Result<f64, Box<EvalAltResult>> {
            run_blocking("Stage position", stage.driver.position())
        },
    );

    // stage.wait_settled() - Wait for motion to complete
    engine.register_fn(
        "wait_settled",
        move |stage: &mut StageHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("Stage wait_settled", stage.driver.wait_settled())?;
            Ok(Dynamic::UNIT)
        },
    );

    // =========================================================================
    // Camera Methods - Acquisition Control
    // =========================================================================

    // camera.arm() - Prepare camera for trigger
    // DEPRECATED (bd-94zq.4): Use yield-based plans instead for Document emission
    engine.register_fn(
        "arm",
        move |camera: &mut CameraHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            // Deprecation warning (bd-94zq.4)
            tracing::warn!(
                target: "daq_scripting::deprecation",
                "Direct camera.arm() is DEPRECATED (v0.7.0). \
                 Use yield-based plans (e.g., yield_plan(count(...))) instead for proper Document emission. \
                 Direct hardware commands will be removed in v0.9.0."
            );

            run_blocking("Camera arm", camera.driver.arm())?;
            Ok(Dynamic::UNIT)
        },
    );

    // camera.trigger() - Capture frame
    // DEPRECATED (bd-94zq.4): Use yield_trigger() instead for Document emission
    engine.register_fn(
        "trigger",
        move |camera: &mut CameraHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            // Deprecation warning (bd-94zq.4)
            tracing::warn!(
                target: "daq_scripting::deprecation",
                "Direct camera.trigger() is DEPRECATED (v0.7.0). \
                 Use yield_trigger(device_id) instead for proper Document emission. \
                 Direct hardware commands will be removed in v0.9.0."
            );

            run_blocking("Camera trigger", camera.driver.trigger())?;

            // Send measurement to broadcast channel if sender available
            if let Some(ref tx) = camera.data_tx {
                let measurement = Measurement::Scalar {
                    name: "camera_trigger".to_string(),
                    value: 1.0, // Trigger event indicator
                    unit: "event".to_string(),
                    timestamp: Utc::now(),
                };

                // Ignore errors if no receivers (non-critical)
                let _ = tx.send(measurement);
            }

            Ok(Dynamic::UNIT)
        },
    );

    // let res = camera.resolution() - Get [width, height] array
    engine.register_fn("resolution", move |camera: &mut CameraHandle| -> Dynamic {
        let (width, height) = camera.driver.resolution();
        Dynamic::from(vec![
            Dynamic::from(width as i64),
            Dynamic::from(height as i64),
        ])
    });

    // =========================================================================
    // Utility Functions
    // =========================================================================

    // sleep(0.5) - Async-aware sleep using Tokio (avoids blocking the runtime)
    engine.register_fn("sleep", |seconds: f64| {
        if let Ok(handle) = Handle::try_current() {
            if handle.runtime_flavor() == RuntimeFlavor::CurrentThread {
                // Avoid deadlock: fall back to std::thread::sleep on current-thread runtime
                std::thread::sleep(Duration::from_secs_f64(seconds));
                return;
            }

            block_in_place(|| {
                handle.block_on(tokio::time::sleep(Duration::from_secs_f64(seconds)))
            });
        } else {
            // No runtime available; use blocking sleep as a safe fallback
            std::thread::sleep(Duration::from_secs_f64(seconds));
        }
    });

    // =========================================================================
    // Mock Hardware Factories - For script testing and demos
    // =========================================================================

    // create_mock_stage() - Create a mock stage for testing (unlimited soft limits)
    engine.register_fn("create_mock_stage", || -> StageHandle {
        use daq_driver_mock::MockStage;
        StageHandle {
            driver: Arc::new(MockStage::new()),
            data_tx: None,
            soft_limits: SoftLimits::unlimited(),
        }
    });

    // create_mock_stage_limited(min, max) - Create a mock stage with soft limits (bd-jnfu.4)
    engine.register_fn(
        "create_mock_stage_limited",
        |min: f64, max: f64| -> StageHandle {
            use daq_driver_mock::MockStage;
            StageHandle {
                driver: Arc::new(MockStage::new()),
                data_tx: None,
                soft_limits: SoftLimits::new(min, max),
            }
        },
    );

    // create_mock_camera(width, height) - Create a mock camera for testing
    engine.register_fn(
        "create_mock_camera",
        |width: i64, height: i64| -> CameraHandle {
            use daq_driver_mock::MockCamera;
            CameraHandle {
                driver: Arc::new(MockCamera::new(width as u32, height as u32)),
                data_tx: None,
            }
        },
    );

    // create_mock_power_meter(base_power) - Create a mock power meter for testing
    // Returns a StageHandle (using Readable trait exposed as stage for simplicity)
    engine.register_fn(
        "create_mock_power_meter",
        |_base_power: f64| -> StageHandle {
            // Mock power meter uses MockStage for now (no dedicated mock)
            // Real scripts should use actual Newport 1830-C
            use daq_driver_mock::MockStage;
            StageHandle {
                driver: Arc::new(MockStage::new()),
                data_tx: None,
                soft_limits: SoftLimits::unlimited(),
            }
        },
    );

    // =========================================================================
    // Soft Limit Methods - Query and configure limits from scripts (bd-jnfu.4)
    // =========================================================================

    // stage.get_soft_limits() - Returns [min, max] or [] if unlimited
    engine.register_fn("get_soft_limits", |stage: &mut StageHandle| -> Dynamic {
        let limits = &stage.soft_limits;
        match (limits.min, limits.max) {
            (Some(min), Some(max)) => Dynamic::from(vec![Dynamic::from(min), Dynamic::from(max)]),
            (Some(min), None) => {
                Dynamic::from(vec![Dynamic::from(min), Dynamic::from(f64::INFINITY)])
            }
            (None, Some(max)) => {
                Dynamic::from(vec![Dynamic::from(f64::NEG_INFINITY), Dynamic::from(max)])
            }
            (None, None) => Dynamic::from(Vec::<Dynamic>::new()),
        }
    });

    // =========================================================================
    // Additional Stage Methods
    // =========================================================================

    // stage.home() - Home the stage to mechanical zero
    engine.register_fn(
        "home",
        move |stage: &mut StageHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            // Move to position 0.0 as a generic home operation
            // Note: For devices with true homing (like ELL14), use the specific driver
            run_blocking("Stage home", stage.driver.move_abs(0.0))?;
            run_blocking("Stage wait_settled", stage.driver.wait_settled())?;
            Ok(Dynamic::UNIT)
        },
    );

    // =========================================================================
    // Readable Methods - Power Meters and Sensors
    // =========================================================================

    engine.register_type_with_name::<ReadableHandle>("PowerMeter");

    // power_meter.read() - Read current value
    engine.register_fn(
        "read",
        move |readable: &mut ReadableHandle| -> Result<f64, Box<EvalAltResult>> {
            run_blocking("Readable read", readable.driver.read())
        },
    );

    // power_meter.read_averaged(samples) - Average multiple readings
    engine.register_fn(
        "read_averaged",
        move |readable: &mut ReadableHandle, samples: i64| -> Result<f64, Box<EvalAltResult>> {
            if samples < 1 {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    "read_averaged: samples must be >= 1".into(),
                    Position::NONE,
                )));
            }

            let mut sum = 0.0;
            for _ in 0..samples {
                let val = run_blocking("Readable read", readable.driver.read())?;
                sum += val;
                // Small delay between samples
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(sum / samples as f64)
        },
    );

    // =========================================================================
    // Shutter Methods - Laser Shutter Control
    // =========================================================================

    engine.register_type_with_name::<ShutterHandle>("Shutter");

    // shutter.open() - Open the shutter
    engine.register_fn(
        "open",
        move |shutter: &mut ShutterHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("Shutter open", shutter.driver.open_shutter())?;
            Ok(Dynamic::UNIT)
        },
    );

    // shutter.close() - Close the shutter
    engine.register_fn(
        "close",
        move |shutter: &mut ShutterHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("Shutter close", shutter.driver.close_shutter())?;
            Ok(Dynamic::UNIT)
        },
    );

    // shutter.is_open() - Query shutter state
    engine.register_fn(
        "is_open",
        move |shutter: &mut ShutterHandle| -> Result<bool, Box<EvalAltResult>> {
            run_blocking("Shutter is_open", shutter.driver.is_shutter_open())
        },
    );

    // =========================================================================
    // Global Utility Functions
    // =========================================================================

    // timestamp() - Get current ISO8601 timestamp (for filenames, logging)
    engine.register_fn("timestamp", || -> String {
        Utc::now().format("%Y%m%d_%H%M%S").to_string()
    });

    // timestamp_iso() - Get full ISO8601 timestamp with timezone
    engine.register_fn("timestamp_iso", || -> String {
        Utc::now().to_rfc3339()
    });

    // =========================================================================
    // Shutter Safety Wrapper
    // =========================================================================

    // with_shutter_open(shutter, callback) - Execute callback with shutter open
    // GUARANTEES shutter closure even on error (Rust's Drop semantics)
    engine.register_fn(
        "with_shutter_open",
        move |context: NativeCallContext,
              shutter: ShutterHandle,
              callback: FnPtr|
              -> Result<Dynamic, Box<EvalAltResult>> {
            // Open shutter
            run_blocking("Shutter open", shutter.driver.open_shutter())?;
            tracing::info!("with_shutter_open: Shutter opened");

            // Execute callback, capturing result or error
            let result = callback.call_within_context(&context, ());

            // ALWAYS close shutter - even on error
            let close_result = run_blocking("Shutter close", shutter.driver.close_shutter());

            match close_result {
                Ok(()) => tracing::info!("with_shutter_open: Shutter closed successfully"),
                Err(ref e) => tracing::error!("with_shutter_open: Failed to close shutter: {}", e),
            }

            // Handle results - prioritize callback error if both fail
            match (result, close_result) {
                (Ok(val), Ok(())) => Ok(val),
                (Err(e), Ok(())) => Err(e), // Callback failed, shutter closed ok
                (Ok(_), Err(e)) => Err(e),  // Callback ok, shutter close failed
                (Err(e1), Err(_)) => Err(e1), // Both failed, report callback error
            }
        },
    );

    // Register hardware factory functions (feature-gated)
    #[cfg(feature = "hardware_factories")]
    register_hardware_factories(engine);

    // Register HDF5 functions (feature-gated)
    #[cfg(feature = "hdf5_scripting")]
    register_hdf5_functions(engine);
}

// =============================================================================
// Hardware Factory Functions (feature-gated)
// =============================================================================

#[cfg(feature = "hardware_factories")]
fn register_hardware_factories(engine: &mut Engine) {
    use daq_driver_newport::Newport1830CDriver;
    use daq_driver_spectra_physics::MaiTaiDriver;
    use daq_driver_thorlabs::Ell14Driver;

    // =========================================================================
    // ELL14 Rotator Factory
    // =========================================================================

    // create_elliptec(port, address) - Create ELL14 rotator driver
    // Note: ELL14 uses 115200 baud (not 9600), so we use get_or_open_port_115200
    // If calibration times out (device not responding), falls back to uncalibrated mode
    engine.register_fn(
        "create_elliptec",
        |port: &str, address: &str| -> Result<StageHandle, Box<EvalAltResult>> {
            let port = port.to_string();
            let address = address.to_string();

            let driver = run_blocking("ELL14 create", async move {
                use daq_driver_thorlabs::shared_ports::get_or_open_port_115200;
                use tokio::time::{timeout, Duration};

                eprintln!("  [DEBUG] Opening port {}...", port);
                let shared_port = get_or_open_port_115200(&port).await?;
                eprintln!("  [DEBUG] Port opened, attempting calibration for address {}...", address);

                // Try calibrated driver with 3s timeout
                let driver: Ell14Driver = match timeout(
                    Duration::from_secs(3),
                    Ell14Driver::with_shared_port_calibrated(shared_port.clone(), &address),
                )
                .await
                {
                    Ok(Ok(driver)) => {
                        eprintln!("  [DEBUG] ELL14 calibrated successfully");
                        driver
                    }
                    Ok(Err(e)) => {
                        eprintln!("  [DEBUG] ELL14 calibration failed: {}, using defaults", e);
                        Ell14Driver::with_shared_port(shared_port, &address)
                    }
                    Err(_) => {
                        eprintln!("  [DEBUG] ELL14 calibration timed out, using defaults");
                        Ell14Driver::with_shared_port(shared_port, &address)
                    }
                };
                Ok::<_, anyhow::Error>(driver)
            })?;

            Ok(StageHandle {
                driver: Arc::new(driver),
                data_tx: None,
                soft_limits: SoftLimits::new(0.0, 360.0), // Rotator: 0-360°
            })
        },
    );

    // =========================================================================
    // Newport 1830-C Power Meter Factory
    // =========================================================================

    // Register Newport1830CHandle type
    engine.register_type_with_name::<Newport1830CHandle>("Newport1830C");

    // create_newport_1830c(port) - Create Newport 1830-C power meter driver
    engine.register_fn(
        "create_newport_1830c",
        |port: &str| -> Result<Newport1830CHandle, Box<EvalAltResult>> {
            let port = port.to_string();

            let driver =
                run_blocking("Newport 1830-C create", Newport1830CDriver::new_async(&port))?;

            Ok(Newport1830CHandle {
                driver: Arc::new(driver),
                data_tx: None,
            })
        },
    );

    // power_meter.read() - Read power value
    engine.register_fn(
        "read",
        |pm: &mut Newport1830CHandle| -> Result<f64, Box<EvalAltResult>> {
            run_blocking("Newport 1830-C read", pm.driver.read())
        },
    );

    // power_meter.read_averaged(samples) - Average multiple readings
    engine.register_fn(
        "read_averaged",
        |pm: &mut Newport1830CHandle, samples: i64| -> Result<f64, Box<EvalAltResult>> {
            if samples < 1 {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    "read_averaged: samples must be >= 1".into(),
                    Position::NONE,
                )));
            }

            let mut sum = 0.0;
            for _ in 0..samples {
                let val = run_blocking("Newport 1830-C read", pm.driver.read())?;
                sum += val;
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(sum / samples as f64)
        },
    );

    // power_meter.zero() - Zero the power meter (no attenuator)
    engine.register_fn(
        "zero",
        |pm: &mut Newport1830CHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("Newport 1830-C zero", pm.driver.zero(false))?;
            Ok(Dynamic::UNIT)
        },
    );

    // power_meter.zero_with_attenuator() - Zero the power meter with attenuator
    engine.register_fn(
        "zero_with_attenuator",
        |pm: &mut Newport1830CHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("Newport 1830-C zero", pm.driver.zero(true))?;
            Ok(Dynamic::UNIT)
        },
    );

    // =========================================================================
    // MaiTai Laser Shutter Factory
    // =========================================================================

    // create_maitai(port) - Create MaiTai laser driver with default baud (115200)
    engine.register_fn(
        "create_maitai",
        |port: &str| -> Result<ShutterHandle, Box<EvalAltResult>> {
            let port = port.to_string();

            // Default baud rate for USB-to-USB connection is 115200
            let driver = run_blocking("MaiTai create", MaiTaiDriver::new_async_default(&port))?;

            Ok(ShutterHandle {
                driver: Arc::new(driver),
            })
        },
    );

    // create_maitai_with_baud(port, baud_rate) - Create MaiTai with custom baud rate
    engine.register_fn(
        "create_maitai_with_baud",
        |port: &str, baud_rate: i64| -> Result<ShutterHandle, Box<EvalAltResult>> {
            let port = port.to_string();
            let baud = baud_rate as u32;

            let driver = run_blocking("MaiTai create", MaiTaiDriver::new_async(&port, baud))?;

            Ok(ShutterHandle {
                driver: Arc::new(driver),
            })
        },
    );
}

// =============================================================================
// HDF5 Functions (feature-gated)
// =============================================================================

#[cfg(feature = "hdf5_scripting")]
fn register_hdf5_functions(engine: &mut Engine) {
    engine.register_type_with_name::<Hdf5Handle>("Hdf5File");

    // create_hdf5(path) - Create new HDF5 file for writing
    engine.register_fn(
        "create_hdf5",
        |path: &str| -> Result<Hdf5Handle, Box<EvalAltResult>> {
            let file = hdf5::File::create(path).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(
                    format!("Failed to create HDF5 file '{}': {}", path, e).into(),
                    Position::NONE,
                ))
            })?;

            Ok(Hdf5Handle {
                file: Arc::new(tokio::sync::Mutex::new(Some(file))),
                path: std::path::PathBuf::from(path),
            })
        },
    );

    // hdf5.path() - Get file path
    engine.register_fn("path", |hdf5: &mut Hdf5Handle| -> String {
        hdf5.path.display().to_string()
    });

    // hdf5.write_attr(name, value) - Write string attribute
    engine.register_fn(
        "write_attr",
        |hdf5: &mut Hdf5Handle, name: &str, value: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            let name = name.to_string();
            let value = value.to_string();
            run_blocking("HDF5 write_attr", async {
                let guard = hdf5.file.lock().await;
                let file = guard.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HDF5 file already closed")
                })?;
                // Parse string to VarLenUnicode (following hdf5_writer.rs pattern)
                let vlu: hdf5::types::VarLenUnicode = value.parse()
                    .map_err(|_| anyhow::anyhow!("Failed to parse string as VarLenUnicode"))?;
                file.new_attr::<hdf5::types::VarLenUnicode>()
                    .shape(())
                    .create(name.as_str())
                    .map_err(|e| anyhow::anyhow!("Failed to create attr '{}': {}", name, e))?
                    .write_scalar(&vlu)
                    .map_err(|e| anyhow::anyhow!("Failed to write attr '{}': {}", name, e))?;
                Ok::<_, anyhow::Error>(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // hdf5.write_attr_f64(name, value) - Write float attribute
    engine.register_fn(
        "write_attr_f64",
        |hdf5: &mut Hdf5Handle, name: &str, value: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("HDF5 write_attr_f64", async {
                let guard = hdf5.file.lock().await;
                let file = guard.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HDF5 file already closed")
                })?;
                file.new_attr::<f64>()
                    .shape(())
                    .create(name)
                    .map_err(|e| anyhow::anyhow!("Failed to create attr '{}': {}", name, e))?
                    .write_scalar(&value)
                    .map_err(|e| anyhow::anyhow!("Failed to write attr '{}': {}", name, e))?;
                Ok::<_, anyhow::Error>(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // hdf5.write_attr_i64(name, value) - Write integer attribute
    engine.register_fn(
        "write_attr_i64",
        |hdf5: &mut Hdf5Handle, name: &str, value: i64| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("HDF5 write_attr_i64", async {
                let guard = hdf5.file.lock().await;
                let file = guard.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HDF5 file already closed")
                })?;
                file.new_attr::<i64>()
                    .shape(())
                    .create(name)
                    .map_err(|e| anyhow::anyhow!("Failed to create attr '{}': {}", name, e))?
                    .write_scalar(&value)
                    .map_err(|e| anyhow::anyhow!("Failed to write attr '{}': {}", name, e))?;
                Ok::<_, anyhow::Error>(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // hdf5.write_array_1d(name, data) - Write 1D f64 array as dataset
    engine.register_fn(
        "write_array_1d",
        |hdf5: &mut Hdf5Handle,
         name: &str,
         data: rhai::Array|
         -> Result<Dynamic, Box<EvalAltResult>> {
            // Convert Rhai array to Vec<f64>
            let values: Vec<f64> = data
                .iter()
                .map(|v| {
                    v.as_float()
                        .or_else(|_| v.as_int().map(|i| i as f64))
                        .map_err(|_| {
                            Box::new(EvalAltResult::ErrorRuntime(
                                format!("Array element is not a number: {:?}", v).into(),
                                Position::NONE,
                            ))
                        })
                })
                .collect::<Result<Vec<f64>, _>>()?;

            run_blocking("HDF5 write_array_1d", async {
                let guard = hdf5.file.lock().await;
                let file = guard.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HDF5 file already closed")
                })?;

                // Create dataset
                let dataset = file
                    .new_dataset::<f64>()
                    .shape([values.len()])
                    .create(name)
                    .map_err(|e| anyhow::anyhow!("Failed to create dataset '{}': {}", name, e))?;

                dataset
                    .write(&values)
                    .map_err(|e| anyhow::anyhow!("Failed to write dataset '{}': {}", name, e))?;

                Ok::<_, anyhow::Error>(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // hdf5.write_array_2d(name, data) - Write 2D array (array of [angle, power] pairs)
    engine.register_fn(
        "write_array_2d",
        |hdf5: &mut Hdf5Handle,
         name: &str,
         data: rhai::Array|
         -> Result<Dynamic, Box<EvalAltResult>> {
            // Convert Rhai array of arrays to flat Vec<f64> + dimensions
            let mut values: Vec<f64> = Vec::new();
            let mut ncols = 0usize;

            for (i, row) in data.iter().enumerate() {
                let row_arr = row.clone().into_array().map_err(|_| {
                    Box::new(EvalAltResult::ErrorRuntime(
                        format!("Row {} is not an array", i).into(),
                        Position::NONE,
                    ))
                })?;

                if i == 0 {
                    ncols = row_arr.len();
                } else if row_arr.len() != ncols {
                    return Err(Box::new(EvalAltResult::ErrorRuntime(
                        format!(
                            "Row {} has {} columns, expected {}",
                            i,
                            row_arr.len(),
                            ncols
                        )
                        .into(),
                        Position::NONE,
                    )));
                }

                for v in row_arr {
                    let val = v
                        .as_float()
                        .or_else(|_| v.as_int().map(|i| i as f64))
                        .map_err(|_| {
                            Box::new(EvalAltResult::ErrorRuntime(
                                format!("Array element is not a number: {:?}", v).into(),
                                Position::NONE,
                            ))
                        })?;
                    values.push(val);
                }
            }

            let nrows = data.len();

            run_blocking("HDF5 write_array_2d", async {
                let guard = hdf5.file.lock().await;
                let file = guard.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HDF5 file already closed")
                })?;

                // Create or get group for nested paths
                let (group, dataset_name) = if name.contains('/') {
                    let parts: Vec<&str> = name.rsplitn(2, '/').collect();
                    let ds_name = parts[0];
                    let group_path = parts[1];

                    // Create nested groups
                    let group = file
                        .create_group(group_path)
                        .or_else(|_| file.group(group_path))
                        .map_err(|e| anyhow::anyhow!("Failed to create group '{}': {}", group_path, e))?;

                    (Some(group), ds_name)
                } else {
                    (None, name)
                };

                // Create dataset with 2D shape
                let builder = match &group {
                    Some(g) => g.new_dataset::<f64>(),
                    None => file.new_dataset::<f64>(),
                };

                let dataset = builder
                    .shape([nrows, ncols])
                    .create(dataset_name)
                    .map_err(|e| anyhow::anyhow!("Failed to create dataset '{}': {}", name, e))?;

                // Write as raw slice - hdf5 handles the 2D shape internally
                dataset
                    .write_raw(&values)
                    .map_err(|e| anyhow::anyhow!("Failed to write dataset '{}': {}", name, e))?;

                Ok::<_, anyhow::Error>(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // hdf5.create_group(name) - Create a group
    engine.register_fn(
        "create_group",
        |hdf5: &mut Hdf5Handle, name: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("HDF5 create_group", async {
                let guard = hdf5.file.lock().await;
                let file = guard.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HDF5 file already closed")
                })?;
                file.create_group(name)
                    .map_err(|e| anyhow::anyhow!("Failed to create group '{}': {}", name, e))?;
                Ok::<_, anyhow::Error>(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // hdf5.close() - Close the file
    engine.register_fn(
        "close",
        |hdf5: &mut Hdf5Handle| -> Result<Dynamic, Box<EvalAltResult>> {
            run_blocking("HDF5 close", async {
                let mut guard = hdf5.file.lock().await;
                if let Some(file) = guard.take() {
                    file.flush()
                        .map_err(|e| anyhow::anyhow!("Failed to flush HDF5 file: {}", e))?;
                    // File is closed on drop
                }
                Ok::<_, anyhow::Error>(())
            })?;
            Ok(Dynamic::UNIT)
        },
    );
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use daq_driver_mock::{MockCamera, MockStage};
    use rhai::Scope;

    #[test]
    fn test_register_hardware_succeeds() {
        let mut engine = Engine::new();
        register_hardware(&mut engine); // Should not panic
    }

    #[test]
    fn test_stage_handle_clone() {
        let stage = Arc::new(MockStage::new());
        let handle1 = StageHandle {
            driver: stage.clone(),
            data_tx: None,
            soft_limits: SoftLimits::unlimited(),
        };
        let handle2 = handle1.clone();

        // Both handles should point to same underlying driver
        assert!(Arc::ptr_eq(&handle1.driver, &handle2.driver));
    }

    #[test]
    fn test_camera_handle_clone() {
        let camera = Arc::new(MockCamera::new(1920, 1080));
        let handle1 = CameraHandle {
            driver: camera.clone(),
            data_tx: None,
        };
        let handle2 = handle1.clone();

        assert!(Arc::ptr_eq(&handle1.driver, &handle2.driver));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stage_methods_available() {
        let mut engine = Engine::new();
        register_hardware(&mut engine);

        let stage = Arc::new(MockStage::new());
        let mut scope = Scope::new();
        scope.push(
            "stage",
            StageHandle {
                driver: stage,
                data_tx: None,
                soft_limits: SoftLimits::unlimited(),
            },
        );

        // Test that all stage methods are registered and callable
        let script = r#"
            stage.move_abs(5.0);
            stage.move_rel(2.0);
            let pos = stage.position();
            stage.wait_settled();
            pos
        "#;

        let result = engine.eval_with_scope::<f64>(&mut scope, script).unwrap();
        assert_eq!(result, 7.0); // 5.0 + 2.0
    }

    /// Test that soft limits prevent out-of-range moves (bd-jnfu.4)
    #[tokio::test(flavor = "multi_thread")]
    async fn test_soft_limits_enforcement() {
        let mut engine = Engine::new();
        register_hardware(&mut engine);

        let stage = Arc::new(MockStage::new());
        let mut scope = Scope::new();
        scope.push(
            "stage",
            StageHandle {
                driver: stage,
                data_tx: None,
                soft_limits: SoftLimits::new(0.0, 100.0), // Limit: 0 to 100
            },
        );

        // Valid move should succeed
        let result = engine.eval_with_scope::<()>(&mut scope, "stage.move_abs(50.0);");
        assert!(result.is_ok(), "Move within limits should succeed");

        // Move above max should fail
        let result = engine.eval_with_scope::<()>(&mut scope, "stage.move_abs(150.0);");
        assert!(result.is_err(), "Move above max should fail");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("soft limit"),
            "Error should mention soft limit: {}",
            err
        );

        // Move below min should fail
        let result = engine.eval_with_scope::<()>(&mut scope, "stage.move_abs(-10.0);");
        assert!(result.is_err(), "Move below min should fail");

        // Relative move that would exceed limits should fail
        let result = engine.eval_with_scope::<()>(&mut scope, "stage.move_rel(60.0);"); // 50 + 60 = 110 > 100
        assert!(
            result.is_err(),
            "Relative move exceeding limits should fail"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_camera_methods_available() {
        let mut engine = Engine::new();
        register_hardware(&mut engine);

        let camera = Arc::new(MockCamera::new(1920, 1080));
        let mut scope = Scope::new();
        scope.push(
            "camera",
            CameraHandle {
                driver: camera,
                data_tx: None,
            },
        );

        // Test camera methods
        let script = r#"
            camera.arm();
            camera.trigger();
            let res = camera.resolution();
            res[0]
        "#;

        let result = engine.eval_with_scope::<i64>(&mut scope, script).unwrap();
        assert_eq!(result, 1920);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sleep_function() {
        let mut engine = Engine::new();
        register_hardware(&mut engine);

        let start = std::time::Instant::now();
        // We need to run this in a spawn_blocking or similar if calling from async test directly,
        // but rhai_fn is registered with block_in_place.
        // However, calling engine.eval from async context is tricky if the fn calls block_in_place.
        // Actually, block_in_place works inside a multithreaded runtime.

        // Note: engine.eval is synchronous.
        engine.eval::<()>("sleep(0.1)").unwrap();

        let elapsed = start.elapsed();

        // Should sleep for ~100ms (allow some tolerance)
        assert!(elapsed.as_millis() >= 95);
        assert!(elapsed.as_millis() <= 150);
    }
}
