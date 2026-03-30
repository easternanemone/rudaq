//! LIBS Hardware Bindings for Rhai Scripts
//!
//! Provides Rhai-compatible handles for LIBS (Laser-Induced Breakdown Spectroscopy)
//! hardware: gated camera (ICCD), spectrograph, motion stage with Trigger-On-Position,
//! and the radiance calibration engine.
//!
//! # Architecture
//!
//! All handles use capability trait objects from `common::capabilities`:
//! - `GatedCameraHandle` wraps `Arc<dyn GatedCamera>` + `Arc<dyn Triggerable>` for
//!   DDG timing, MCP gain, gate mode, trigger mode, arm, and stop_stream.
//! - `SpectrographHandle` wraps `Arc<dyn SpectrometerControl>` for
//!   grating/wavelength/slit control and wavelength-pixel calibration.
//! - `DoverAxisHandle` wraps `Arc<dyn TriggerOnPosition>` for movement and TOP, plus a
//!   stored closure for `set_velocity` (not in any common trait).
//! - `CalibratorHandle` wraps `Arc<RadianceCalibrator>` for offline spectral correction.
//!
//! # Script Example
//! ```rhai
//! let cam   = create_gated_camera();
//! let spec  = create_spectrograph();
//! let stage = create_top_stage("X");
//!
//! cam.set_gate_mode("DDG");
//! cam.set_ddg_timing(1300000, 10000000);  // delay_ps, width_ps
//! cam.set_mcp_gain(3600);
//!
//! spec.set_grating(1);
//! spec.set_wavelength(310.0);
//!
//! stage.set_velocity(5.0);
//! stage.move_abs(10.0);
//! stage.enable_top(0.0, 20.0, 0.1, false, 1000);
//!
//! // Radiance calibration
//! let cal = create_radiance_calibrator("lamp.lmp", "g1.asc", "g2.asc");
//! let wl  = [300.0, 310.0, 320.0];
//! let raw = [100.0, 200.0, 150.0];
//! let corr = cal.calibrate(wl, raw, 1, false);
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use rhai::{Array, Dynamic, Engine, EvalAltResult};

use crate::run_blocking;
use common::capabilities::{GatedCamera, SpectrometerControl, TriggerOnPosition, Triggerable};
use common::processing::radiance_calibration::{RadianceCalibrator, Spectrum};

// Boxed async closure type used for capability bundling
type BoxFuture<T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send>>;
type VelocityFn = Arc<dyn Fn(f64) -> BoxFuture<()> + Send + Sync>;

// =============================================================================
// Handle Types
// =============================================================================

/// Handle to a gated ICCD camera for Rhai scripts.
///
/// Wraps trait objects for LIBS-specific controls:
/// DDG timing, MCP gain, gate mode, and trigger mode.
///
/// # Script Example
/// ```rhai
/// let cam = create_gated_camera();
/// cam.set_gate_mode("DDG");
/// cam.set_ddg_timing(1300000, 10000000);
/// cam.set_mcp_gain(3600);
/// cam.arm();
/// ```
#[derive(Clone)]
pub struct GatedCameraHandle {
    pub driver: Arc<dyn GatedCamera>,
    pub triggerable: Arc<dyn Triggerable>,
}

/// Handle to a spectrograph for Rhai scripts.
///
/// Provides grating selection, wavelength tuning, slit control, and
/// wavelength-pixel calibration retrieval.
///
/// # Script Example
/// ```rhai
/// let spec = create_spectrograph();
/// spec.set_grating(1);
/// spec.set_wavelength(310.0);
/// spec.set_slit_width(2, 150.0);
/// let wl = spec.get_calibration(2560);
/// ```
#[derive(Clone)]
pub struct SpectrographHandle {
    pub driver: Arc<dyn SpectrometerControl>,
}

/// Handle to a motion stage with Trigger-On-Position support.
///
/// Wraps `Arc<dyn TriggerOnPosition>` for movement and TOP methods.
/// `set_velocity` is bundled as a closure since it is not a common trait method.
///
/// # Script Example
/// ```rhai
/// let stage_x = create_top_stage("X");
/// stage_x.set_velocity(5.0);
/// stage_x.move_abs(0.0);
/// stage_x.enable_top(0.0, 20.0, 0.1, false, 1000);
/// ```
#[derive(Clone)]
pub struct DoverAxisHandle {
    pub axis: Arc<dyn TriggerOnPosition>,
    /// Velocity setter -- stored as a closure because `set_velocity` is not in
    /// `Movable` or `TriggerOnPosition` and varies between mock/real drivers.
    set_velocity_fn: VelocityFn,
}

/// Coordinates spectrograph grating changes with camera stop/re-arm.
///
/// Grating rotation causes the camera to lose its acquisition window, so the
/// correct sequence is: **stop_stream -> set_grating -> set_wavelength -> arm**.
/// `ScanController` encapsulates this sequence as a single `set_config` call,
/// preventing accidental acquisitions during grating motion.
///
/// # Script Example
/// ```rhai
/// let sc = create_scan_controller(camera, spectrograph);
/// sc.set_config(1, 310.0);   // grating 1, 310 nm -- stops, tunes, re-arms
/// sc.set_config(2, 450.0);   // grating 2, 450 nm -- same coordinated sequence
/// ```
#[derive(Clone)]
pub struct ScanController {
    camera: Arc<dyn GatedCamera>,
    triggerable: Arc<dyn Triggerable>,
    spectrograph: Arc<dyn SpectrometerControl>,
}

/// Handle to a [`RadianceCalibrator`] for Rhai scripts.
///
/// Wraps `Arc<RadianceCalibrator>` for thread-safe sharing across script calls.
/// All computation is synchronous (no hardware I/O) so no async bridge is needed.
///
/// # Script Example
/// ```rhai
/// let cal = create_radiance_calibrator("lamp.lmp", "g1.asc", "g2.asc");
/// // wavelengths and raw intensities as Rhai arrays
/// let corrected = cal.calibrate(wavelengths, values, 1, false);
/// ```
#[derive(Clone)]
pub struct CalibratorHandle {
    pub calibrator: Arc<RadianceCalibrator>,
}

// =============================================================================
// Rhai Registration
// =============================================================================

/// Register all LIBS hardware bindings with the Rhai engine.
///
/// Registers types and methods for:
/// - `GatedCamera` -- ICCD DDG/MCP control
/// - `Spectrograph` -- grating/wavelength/slit control
/// - `DoverAxis` -- stage movement and Trigger-On-Position
/// - `RadianceCalibrator` -- offline spectral correction engine
/// - Factory functions: `create_gated_camera`, `create_spectrograph`,
///   `create_top_stage`, `create_radiance_calibrator`
pub fn register_libs_hardware(engine: &mut Engine) {
    engine.register_type_with_name::<GatedCameraHandle>("GatedCamera");
    engine.register_type_with_name::<SpectrographHandle>("Spectrograph");
    engine.register_type_with_name::<DoverAxisHandle>("DoverAxis");
    engine.register_type_with_name::<CalibratorHandle>("RadianceCalibrator");
    engine.register_type_with_name::<ScanController>("ScanController");

    // =========================================================================
    // GatedCameraHandle -- gated camera methods via trait objects
    // =========================================================================

    // cam.set_gate_mode("DDG") -- select gating mode ("DDG", "CW", etc.)
    engine.register_fn(
        "set_gate_mode",
        |cam: &mut GatedCameraHandle, mode: String| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            run_blocking(
                "set_gate_mode",
                async move { driver.set_gate_mode(&mode).await },
            )?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.set_trigger_mode("External") -- select acquisition trigger source
    engine.register_fn(
        "set_trigger_mode",
        |cam: &mut GatedCameraHandle, mode: String| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            run_blocking("set_trigger_mode", async move {
                driver.set_trigger_mode(&mode).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.set_ddg_timing(delay_ps, width_ps) -- configure DDG delay/gate width in picoseconds
    engine.register_fn(
        "set_ddg_timing",
        |cam: &mut GatedCameraHandle,
         delay_ps: i64,
         width_ps: i64|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            let d = u64::try_from(delay_ps).map_err(|_| {
                crate::rhai_error_str("set_ddg_timing: delay_ps must be non-negative")
            })?;
            let w = u64::try_from(width_ps).map_err(|_| {
                crate::rhai_error_str("set_ddg_timing: width_ps must be non-negative")
            })?;
            run_blocking("set_ddg_timing", async move {
                driver.set_ddg_timing(d, w).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.set_mcp_gain(3600) -- set MCP intensifier gain (0-4095 typical)
    engine.register_fn(
        "set_mcp_gain",
        |cam: &mut GatedCameraHandle, gain: i64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            if !(0..=4095).contains(&gain) {
                return Err(crate::rhai_error_str(
                    "set_mcp_gain: gain must be in range 0-4095",
                ));
            }
            let gain_u32 = u32::try_from(gain).unwrap_or(0);
            run_blocking("set_mcp_gain", async move {
                driver.set_mcp_gain(gain_u32).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.arm() -- arm camera for acquisition (via Triggerable trait)
    engine.register_fn(
        "arm",
        |cam: &mut GatedCameraHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            let triggerable = cam.triggerable.clone();
            run_blocking("arm", async move { triggerable.arm().await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.stop_stream() -- stop acquisition / disarm camera
    engine.register_fn(
        "stop_stream",
        |cam: &mut GatedCameraHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            run_blocking("stop_stream", async move {
                common::capabilities::FrameProducer::stop_stream(driver.as_ref()).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.temperature() -> f64 -- read sensor temperature in C
    engine.register_fn(
        "temperature",
        |cam: &mut GatedCameraHandle| -> Result<f64, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            run_blocking("temperature", async move { driver.get_temperature().await })
        },
    );

    // cam.supports_ddg() -> bool -- check if DDG is available
    engine.register_fn("supports_ddg", |cam: &mut GatedCameraHandle| -> bool {
        cam.driver.supports_ddg()
    });

    // cam.supports_mcp_gain() -> bool -- check if MCP gain control is available
    engine.register_fn("supports_mcp_gain", |cam: &mut GatedCameraHandle| -> bool {
        cam.driver.supports_mcp_gain()
    });

    // =========================================================================
    // SpectrographHandle -- spectrograph methods via SpectrometerControl trait
    // =========================================================================

    // spec.set_grating(index) -- select diffraction grating by index (1-based)
    engine.register_fn(
        "set_grating",
        |spec: &mut SpectrographHandle, grating: i64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: Grating indices are small integers (<10 for any real spectrograph)
            run_blocking("set_grating", async move {
                driver.set_grating(grating as i32).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // spec.get_grating() -> i64 -- read current grating index
    engine.register_fn(
        "get_grating",
        |spec: &mut SpectrographHandle| -> Result<i64, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            let g = run_blocking("get_grating", async move { driver.get_grating().await })?;
            Ok(i64::from(g))
        },
    );

    // spec.set_wavelength(nm) -- tune center wavelength
    engine.register_fn(
        "set_wavelength",
        |spec: &mut SpectrographHandle, nm: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            run_blocking(
                "set_wavelength",
                async move { driver.set_wavelength(nm).await },
            )?;
            Ok(Dynamic::UNIT)
        },
    );

    // spec.get_wavelength() -> f64 -- read current center wavelength in nm
    engine.register_fn(
        "get_wavelength",
        |spec: &mut SpectrographHandle| -> Result<f64, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            run_blocking(
                "get_wavelength",
                async move { driver.get_wavelength().await },
            )
        },
    );

    // spec.set_slit_width(port, width_um) -- set entrance/exit slit width in micrometers
    engine.register_fn(
        "set_slit_width",
        |spec: &mut SpectrographHandle,
         port: i64,
         width_um: f64|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            let port_i32 = i32::try_from(port).map_err(|_| {
                crate::rhai_error_str("set_slit_width: port must be a valid integer")
            })?;
            run_blocking("set_slit_width", async move {
                driver.set_slit_width(port_i32, width_um).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // spec.get_calibration(pixels) -> Array -- wavelength array for detector pixel count
    // Returns an Array of f64 values (one per pixel) suitable for plotting
    engine.register_fn(
        "get_calibration",
        |spec: &mut SpectrographHandle, pixels: i64| -> Result<Array, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            let n = usize::try_from(pixels).map_err(|_| {
                crate::rhai_error_str("get_calibration: pixels must be a positive integer")
            })?;
            let calibration =
                run_blocking(
                    "get_calibration",
                    async move { driver.get_calibration(n).await },
                )?;
            Ok(calibration.into_iter().map(Dynamic::from).collect())
        },
    );

    // =========================================================================
    // DoverAxisHandle -- motion stage + Trigger-On-Position
    // =========================================================================

    // stage.move_abs(pos) -- move to absolute position (mm)
    engine.register_fn(
        "move_abs",
        |axis: &mut DoverAxisHandle, pos: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("move_abs", async move { driver.move_abs(pos).await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.move_rel(dist) -- move relative distance (mm)
    engine.register_fn(
        "move_rel",
        |axis: &mut DoverAxisHandle, dist: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("move_rel", async move { driver.move_rel(dist).await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.position() -> f64 -- read current axis position (mm)
    engine.register_fn(
        "position",
        |axis: &mut DoverAxisHandle| -> Result<f64, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("position", async move { driver.position().await })
        },
    );

    // stage.wait_settled() -- block until motion completes
    engine.register_fn(
        "wait_settled",
        |axis: &mut DoverAxisHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("wait_settled", async move { driver.wait_settled().await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.set_velocity(v) -- set motion velocity (mm/s)
    engine.register_fn(
        "set_velocity",
        |axis: &mut DoverAxisHandle, v: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let setter = axis.set_velocity_fn.clone();
            run_blocking("set_velocity", async move { setter(v).await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.enable_top(start, end, increment, bidirectional, pulse_width_ns)
    // -- arm Trigger-On-Position for continuous scanning
    engine.register_fn(
        "enable_top",
        |axis: &mut DoverAxisHandle,
         start: f64,
         end: f64,
         increment: f64,
         bidirectional: bool,
         pulse_ns: i64|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            let pulse_ns_u64 = u64::try_from(pulse_ns)
                .map_err(|_| crate::rhai_error_str("enable_top: pulse_ns must be non-negative"))?;
            run_blocking("enable_top", async move {
                driver
                    .enable_top(start, end, increment, bidirectional, pulse_ns_u64)
                    .await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.disable_top() -- disarm Trigger-On-Position
    engine.register_fn(
        "disable_top",
        |axis: &mut DoverAxisHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("disable_top", async move { driver.disable_top().await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.top_enabled() -> bool -- query TOP state
    engine.register_fn(
        "top_enabled",
        |axis: &mut DoverAxisHandle| -> Result<bool, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("top_enabled", async move { driver.is_top_enabled().await })
        },
    );

    // =========================================================================
    // CalibratorHandle -- RadianceCalibrator methods
    // =========================================================================

    // cal.calibrate(wavelengths, values, grating, normalize) -> Array
    // Applies radiometric correction and returns corrected intensity values.
    // wavelengths and values must be Rhai arrays of equal length.
    engine.register_fn(
        "calibrate",
        |cal: &mut CalibratorHandle,
         wavelengths: Array,
         values: Array,
         grating: i64,
         normalize: bool|
         -> Result<Array, Box<EvalAltResult>> {
            let wl: Vec<f64> = wavelengths.into_iter().map(|v| v.cast::<f64>()).collect();
            let vals: Vec<f64> = values.into_iter().map(|v| v.cast::<f64>()).collect();
            let spectrum = Spectrum::new(wl, vals)
                .map_err(|e| crate::rhai_error("calibrate: invalid spectrum", e))?;
            let grating_u8 = u8::try_from(grating).map_err(|_| {
                crate::rhai_error_str("calibrate: grating index must be in range 0-255")
            })?;
            let result = cal
                .calibrator
                .calibrate(&spectrum, grating_u8, normalize)
                .map_err(|e| crate::rhai_error("calibrate", e))?;
            Ok(result.values.into_iter().map(Dynamic::from).collect())
        },
    );

    // cal.lamp_wavelengths() -> Array -- wavelength axis of the lamp spectrum
    engine.register_fn("lamp_wavelengths", |cal: &mut CalibratorHandle| -> Array {
        cal.calibrator
            .lamp_spectrum()
            .wavelengths
            .iter()
            .copied()
            .map(Dynamic::from)
            .collect()
    });

    // cal.has_grating(index) -> bool -- check if grating calibration is loaded
    engine.register_fn(
        "has_grating",
        |cal: &mut CalibratorHandle, grating: i64| -> bool {
            u8::try_from(grating)
                .ok()
                .and_then(|g| cal.calibrator.grating_calibration(g))
                .is_some()
        },
    );

    // =========================================================================
    // Factory Functions -- construct mock handles for development/testing
    // =========================================================================

    // create_gated_camera() -> GatedCamera -- mock gated ICCD camera
    // Also registers legacy name create_andor_camera for backwards compatibility
    engine.register_fn(
        "create_gated_camera",
        || -> Result<GatedCameraHandle, Box<EvalAltResult>> {
            let mock = Arc::new(driver_mock::MockGatedCamera::new());
            Ok(GatedCameraHandle {
                driver: mock.clone() as Arc<dyn GatedCamera>,
                triggerable: mock as Arc<dyn Triggerable>,
            })
        },
    );
    engine.register_fn(
        "create_andor_camera",
        || -> Result<GatedCameraHandle, Box<EvalAltResult>> {
            let mock = Arc::new(driver_mock::MockGatedCamera::new());
            Ok(GatedCameraHandle {
                driver: mock.clone() as Arc<dyn GatedCamera>,
                triggerable: mock as Arc<dyn Triggerable>,
            })
        },
    );

    // create_spectrograph() -> Spectrograph -- mock spectrograph
    // Also registers legacy name create_andor_spectrograph for backwards compatibility
    engine.register_fn(
        "create_spectrograph",
        || -> Result<SpectrographHandle, Box<EvalAltResult>> {
            Ok(SpectrographHandle {
                driver: Arc::new(driver_mock::MockSpectrograph::new()),
            })
        },
    );
    engine.register_fn(
        "create_andor_spectrograph",
        || -> Result<SpectrographHandle, Box<EvalAltResult>> {
            Ok(SpectrographHandle {
                driver: Arc::new(driver_mock::MockSpectrograph::new()),
            })
        },
    );

    // create_top_stage(axis_name) -> DoverAxis -- mock motion stage with TOP
    // Also registers legacy name create_dover_axis for backwards compatibility
    engine.register_fn("create_top_stage", |axis_name: String| -> DoverAxisHandle {
        let mock = Arc::new(driver_mock::MockTopStage::new(&axis_name));
        let mock_vel = mock.clone();
        DoverAxisHandle {
            axis: mock,
            set_velocity_fn: Arc::new(move |v: f64| {
                let drv = mock_vel.clone();
                Box::pin(async move { drv.set_velocity(v).await })
            }),
        }
    });
    engine.register_fn(
        "create_dover_axis",
        |axis_name: String| -> DoverAxisHandle {
            let mock = Arc::new(driver_mock::MockTopStage::new(&axis_name));
            let mock_vel = mock.clone();
            DoverAxisHandle {
                axis: mock,
                set_velocity_fn: Arc::new(move |v: f64| {
                    let drv = mock_vel.clone();
                    Box::pin(async move { drv.set_velocity(v).await })
                }),
            }
        },
    );

    // =========================================================================
    // ScanController -- coordinated spectrograph/camera tuning
    // =========================================================================

    // sc.set_config(grating, wavelength_nm) -- stop camera, tune spectrograph, re-arm
    // This is the safe sequence for changing spectral range during a scan.
    engine.register_fn(
        "set_config",
        |sc: &mut ScanController,
         grating: i64,
         wavelength_nm: f64|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let camera = sc.camera.clone();
            let triggerable = sc.triggerable.clone();
            let spec = sc.spectrograph.clone();
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: grating index is a small integer
            let grating_i32 = grating as i32;
            run_blocking("set_config", async move {
                // 1. Stop camera acquisition during grating rotation
                common::capabilities::FrameProducer::stop_stream(camera.as_ref()).await?;
                // 2. Rotate grating and tune wavelength
                spec.set_grating(grating_i32).await?;
                spec.set_wavelength(wavelength_nm).await?;
                // 3. Re-arm camera -- ready to acquire at new spectral position
                triggerable.arm().await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // sc.set_wavelength(nm) -- tune wavelength only (no grating change, skip stop/re-arm)
    engine.register_fn(
        "set_wavelength",
        |sc: &mut ScanController, nm: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let spec = sc.spectrograph.clone();
            run_blocking(
                "set_wavelength",
                async move { spec.set_wavelength(nm).await },
            )?;
            Ok(Dynamic::UNIT)
        },
    );

    // sc.camera() -> GatedCamera -- access the managed camera handle
    engine.register_fn("camera", |sc: &mut ScanController| -> GatedCameraHandle {
        GatedCameraHandle {
            driver: sc.camera.clone(),
            triggerable: sc.triggerable.clone(),
        }
    });

    // sc.spectrograph() -> Spectrograph -- access the managed spectrograph handle
    engine.register_fn(
        "spectrograph",
        |sc: &mut ScanController| -> SpectrographHandle {
            SpectrographHandle {
                driver: sc.spectrograph.clone(),
            }
        },
    );

    // create_scan_controller(camera, spectrograph) -> ScanController
    engine.register_fn(
        "create_scan_controller",
        |cam: GatedCameraHandle, spec: SpectrographHandle| -> ScanController {
            ScanController {
                camera: cam.driver,
                triggerable: cam.triggerable,
                spectrograph: spec.driver,
            }
        },
    );

    // create_radiance_calibrator(lamp_file, grating1_cal_file, grating2_cal_file) -> RadianceCalibrator
    // Loads calibration files from disk. Grating 1 = first cal file, grating 2 = second.
    // Returns an error if any file cannot be loaded.
    engine.register_fn(
        "create_radiance_calibrator",
        |lamp_file: String,
         g1_file: String,
         g2_file: String|
         -> Result<CalibratorHandle, Box<EvalAltResult>> {
            let calibrator = RadianceCalibrator::from_files(
                &lamp_file,
                &[(1u8, &g1_file), (2u8, &g2_file)],
            )
            .map_err(|e| crate::rhai_error("create_radiance_calibrator", e))?;

            let coverage = calibrator
                .loaded_gratings()
                .into_iter()
                .filter_map(|grating| {
                    calibrator.grating_wavelength_bounds(grating).map(|(min_nm, max_nm)| {
                        (
                            grating,
                            experiment::CalibrationWavelengthCoverage { min_nm, max_nm },
                        )
                    })
                })
                .collect();

            let mut snapshot = experiment::CalibrationFreshness::new(calibrator.device_type())
                .with_grating_wavelength_coverage(coverage);

            if let Some(calibration_timestamp) = calibrator.calibration_timestamp() {
                snapshot = snapshot.with_timestamp(calibration_timestamp);
            } else {
                tracing::warn!(
                    target: "calibration_staleness",
                    "Radiance calibration files were loaded without calibration_timestamp metadata; age-based staleness gate disabled but config-match gating remains active"
                );
            }
            crate::update_current_script_calibration(snapshot);

            Ok(CalibratorHandle {
                calibrator: Arc::new(calibrator),
            })
        },
    );
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::Engine;

    fn make_engine() -> Engine {
        let mut engine = Engine::new();
        register_libs_hardware(&mut engine);
        engine
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_gated_camera_handle_creation() {
        let engine = make_engine();
        let result = engine.eval::<Dynamic>(
            r"
            let cam = create_gated_camera();
            cam.supports_ddg()
        ",
        );
        assert!(
            result.is_ok(),
            "gated camera DDG support query failed: {:?}",
            result
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_gated_camera_legacy_factory() {
        let engine = make_engine();
        let result = engine.eval::<Dynamic>(
            r"
            let cam = create_andor_camera();
            cam.supports_ddg()
        ",
        );
        assert!(
            result.is_ok(),
            "legacy create_andor_camera factory failed: {:?}",
            result
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_spectrograph_handle_creation() {
        let engine = make_engine();
        let result = engine.eval::<i64>(
            r"
            let spec = create_spectrograph();
            spec.get_grating()
        ",
        );
        assert!(
            result.is_ok(),
            "spectrograph grating query failed: {:?}",
            result
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dover_axis_move() {
        let engine = make_engine();
        let result = engine.eval::<f64>(
            r#"
            let stage = create_top_stage("X");
            stage.move_abs(5.0);
            stage.position()
        "#,
        );
        assert!(result.is_ok(), "stage move failed: {:?}", result);
        assert!((result.expect("test verified Ok above") - 5.0).abs() < 0.001);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dover_axis_top() {
        let engine = make_engine();
        let result = engine.eval::<bool>(
            r#"
            let stage = create_top_stage("X");
            stage.enable_top(0.0, 10.0, 0.1, false, 1000);
            stage.top_enabled()
        "#,
        );
        assert!(result.is_ok(), "TOP enable failed: {:?}", result);
        assert!(
            result.expect("test verified Ok above"),
            "TOP should be enabled after enable_top"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dover_axis_set_velocity() {
        let engine = make_engine();
        let result = engine.eval::<Dynamic>(
            r#"
            let stage = create_top_stage("X");
            stage.set_velocity(5.0);
            stage.move_abs(2.0);
        "#,
        );
        assert!(result.is_ok(), "set_velocity + move failed: {:?}", result);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dover_axis_legacy_factory() {
        let engine = make_engine();
        let result = engine.eval::<Dynamic>(
            r#"
            let stage = create_dover_axis("X");
            stage.move_abs(1.0);
        "#,
        );
        assert!(
            result.is_ok(),
            "legacy create_dover_axis factory failed: {:?}",
            result
        );
    }

    #[test]
    fn test_calibrator_handle_in_memory() {
        use common::processing::radiance_calibration::Spectrum;
        use std::collections::HashMap;

        // Build an in-memory calibrator (flat lamp = flat cal -> unity correction)
        let n = 50usize;
        #[allow(clippy::cast_precision_loss)] // test indices are small
        let wl: Vec<f64> = (0..n).map(|i| 300.0 + i as f64).collect();
        let lamp = Spectrum::new(wl.clone(), vec![1000.0; n]).expect("valid spectrum");
        let cal = Spectrum::new(wl, vec![1000.0; n]).expect("valid spectrum");
        let mut cals = HashMap::new();
        cals.insert(1u8, cal);
        let handle = CalibratorHandle {
            calibrator: Arc::new(RadianceCalibrator::new(lamp, cals)),
        };

        assert!(handle.calibrator.grating_calibration(1).is_some());
        assert!(handle.calibrator.grating_calibration(2).is_none());
    }

    #[test]
    fn test_calibrator_rhai_api_unity() {
        use std::io::Write;

        // Write temp calibration files
        let write_cal = |content: &str| {
            let mut f = tempfile::NamedTempFile::new().expect("create tempfile");
            write!(f, "{content}").expect("write tempfile");
            f
        };
        let lamp_content = "300.0\t1000.0\n400.0\t1000.0\n500.0\t1000.0\n";
        let cal_content = "300.0\t1000.0\n400.0\t1000.0\n500.0\t1000.0\n";

        let lamp_f = write_cal(lamp_content);
        let g1_f = write_cal(cal_content);
        let g2_f = write_cal(cal_content);

        let lamp_path = lamp_f.path().to_str().expect("path").replace('\\', "/");
        let g1_path = g1_f.path().to_str().expect("path").replace('\\', "/");
        let g2_path = g2_f.path().to_str().expect("path").replace('\\', "/");

        let script = format!(
            r#"
            let cal = create_radiance_calibrator("{lamp_path}", "{g1_path}", "{g2_path}");
            let wl  = [300.0, 400.0, 500.0];
            let raw = [500.0, 500.0, 500.0];
            cal.calibrate(wl, raw, 1, false)
        "#
        );

        let mut engine = Engine::new();
        register_libs_hardware(&mut engine);

        let result: Result<Array, _> = engine.eval(&script);
        assert!(result.is_ok(), "calibrate Rhai call failed: {:?}", result);
        let arr = result.expect("test verified Ok above");
        for v in arr {
            let val = v.cast::<f64>();
            assert!(
                (val - 500.0).abs() < 1.0,
                "unity correction should return ~500.0, got {val}"
            );
        }
    }

    #[test]
    fn test_create_radiance_calibrator_registers_coverage_without_timestamp() {
        use std::io::Write;

        let context_id = format!(
            "libs-calibration-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        );
        crate::set_script_calibration_context(context_id.clone());

        let write_cal = |content: &str| {
            let mut f = tempfile::NamedTempFile::new().expect("create tempfile");
            write!(f, "{content}").expect("write tempfile");
            f
        };
        let lamp_f = write_cal("# device_type: spectroscopy\n300.0\t1000.0\n400.0\t1000.0\n");
        let g1_f = write_cal("# device_type: spectroscopy\n295.0\t900.0\n305.0\t900.0\n");
        let g2_f = write_cal("# device_type: spectroscopy\n450.0\t900.0\n480.0\t900.0\n");

        let script = format!(
            r#"
            create_radiance_calibrator("{}", "{}", "{}");
        "#,
            lamp_f.path().display(),
            g1_f.path().display(),
            g2_f.path().display()
        );

        let mut engine = Engine::new();
        register_libs_hardware(&mut engine);
        let _ = engine
            .eval::<Dynamic>(&script)
            .expect("create_radiance_calibrator should succeed");

        let snapshot = crate::current_script_calibration(&context_id, "spectroscopy")
            .expect("calibration snapshot should be registered");
        assert_eq!(snapshot.calibration_timestamp, None);
        assert_eq!(
            snapshot.grating_wavelength_coverage.get(&1),
            Some(&experiment::CalibrationWavelengthCoverage {
                min_nm: 295.0,
                max_nm: 305.0,
            })
        );
        assert_eq!(
            snapshot.grating_wavelength_coverage.get(&2),
            Some(&experiment::CalibrationWavelengthCoverage {
                min_nm: 450.0,
                max_nm: 480.0,
            })
        );

        crate::clear_script_calibrations(&context_id);
        crate::clear_script_calibration_context();
    }
}
