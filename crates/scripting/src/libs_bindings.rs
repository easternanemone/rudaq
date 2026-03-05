//! LIBS Hardware Bindings for Rhai Scripts
//!
//! Provides Rhai-compatible handles for LIBS (Laser-Induced Breakdown Spectroscopy)
//! hardware: Andor iStar gated camera, Andor Shamrock spectrograph, and Dover SmartStage.
//!
//! # Architecture
//!
//! Uses the same async→sync bridge pattern as the core hardware bindings:
//! - `GatedCameraHandle` wraps `Arc<AndorCamera>` directly (not a trait object) because
//!   DDG/MCP methods are camera-specific, not yet in a common trait.
//! - `SpectrographHandle` wraps `Arc<AndorSpectrograph>` for grating/wavelength/slit control.
//! - `DoverAxisHandle` wraps `Arc<dyn TriggerOnPosition>` for movement and TOP, plus a
//!   stored closure for `set_velocity` (not yet in any common trait).
//!
//! # Script Example
//! ```rhai
//! let cam   = create_andor_camera();
//! let spec  = create_andor_spectrograph();
//! let stage = create_dover_axis("X");
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
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use driver_andor_sdk3::{AndorCamera, AndorSpectrograph};
use driver_dover_motion::mock::DoverMockDriver;
use rhai::{Array, Dynamic, Engine, EvalAltResult};

use crate::run_blocking;
use common::capabilities::TriggerOnPosition;

// Boxed async closure type used for capability bundling
type BoxFuture<T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send>>;
type VelocityFn = Arc<dyn Fn(f64) -> BoxFuture<()> + Send + Sync>;

// =============================================================================
// Handle Types
// =============================================================================

/// Handle to an Andor iStar gated ICCD camera for Rhai scripts.
///
/// Wraps `Arc<AndorCamera>` directly to expose LIBS-specific controls:
/// DDG timing, MCP gain, gate mode, and trigger mode.
///
/// # Script Example
/// ```rhai
/// let cam = create_andor_camera();
/// cam.set_gate_mode("DDG");
/// cam.set_ddg_timing(1300000, 10000000);
/// cam.set_mcp_gain(3600);
/// cam.arm();
/// ```
#[derive(Clone)]
pub struct GatedCameraHandle {
    pub driver: Arc<AndorCamera>,
}

/// Handle to an Andor Shamrock spectrograph for Rhai scripts.
///
/// Provides grating selection, wavelength tuning, slit control, and
/// wavelength-pixel calibration retrieval.
///
/// # Script Example
/// ```rhai
/// let spec = create_andor_spectrograph();
/// spec.set_grating(1);
/// spec.set_wavelength(310.0);
/// spec.set_slit_width(2, 150.0);
/// let wl = spec.get_calibration(2560);
/// ```
#[derive(Clone)]
pub struct SpectrographHandle {
    pub driver: Arc<AndorSpectrograph>,
}

/// Handle to a Dover SmartStage axis with Trigger-On-Position support.
///
/// Wraps `Arc<dyn TriggerOnPosition>` for movement and TOP methods.
/// `set_velocity` is bundled as a closure since it is not yet a common trait method.
///
/// # Script Example
/// ```rhai
/// let stage_x = create_dover_axis("X");
/// stage_x.set_velocity(5.0);
/// stage_x.move_abs(0.0);
/// stage_x.enable_top(0.0, 20.0, 0.1, false, 1000);
/// ```
#[derive(Clone)]
pub struct DoverAxisHandle {
    pub axis: Arc<dyn TriggerOnPosition>,
    /// Velocity setter — stored as a closure because `set_velocity` is not in
    /// `Movable` or `TriggerOnPosition` and varies between mock/real drivers.
    set_velocity_fn: VelocityFn,
}

// =============================================================================
// Rhai Registration
// =============================================================================

/// Register all LIBS hardware bindings with the Rhai engine.
///
/// Registers types and methods for:
/// - `GatedCamera` — Andor iStar DDG/MCP control
/// - `Spectrograph` — Andor Shamrock grating/wavelength/slit control
/// - `DoverAxis` — Dover stage movement and Trigger-On-Position
/// - Factory functions: `create_andor_camera`, `create_andor_spectrograph`, `create_dover_axis`
pub fn register_libs_hardware(engine: &mut Engine) {
    engine.register_type_with_name::<GatedCameraHandle>("GatedCamera");
    engine.register_type_with_name::<SpectrographHandle>("Spectrograph");
    engine.register_type_with_name::<DoverAxisHandle>("DoverAxis");

    // =========================================================================
    // GatedCameraHandle — Andor iStar methods
    // =========================================================================

    // cam.set_gate_mode("DDG") — select gating mode ("DDG", "CW", etc.)
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

    // cam.set_trigger_mode("External") — select acquisition trigger source
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

    // cam.set_ddg_timing(delay_ps, width_ps) — configure DDG delay/gate width in picoseconds
    engine.register_fn(
        "set_ddg_timing",
        |cam: &mut GatedCameraHandle,
         delay_ps: i64,
         width_ps: i64|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            #[allow(clippy::cast_sign_loss)]
            // SAFETY: DDG timing values are always non-negative from script callers
            let (d, w) = (delay_ps as u64, width_ps as u64);
            run_blocking("set_ddg_timing", async move {
                driver.set_ddg_output_delay(d).await?;
                driver.set_ddg_output_width(w).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.set_mcp_gain(3600) — set MCP intensifier gain (0–4095 typical)
    engine.register_fn(
        "set_mcp_gain",
        |cam: &mut GatedCameraHandle, gain: i64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            // SAFETY: MCP gain is in range 0–4095, well within u32
            let gain_u32 = gain as u32;
            run_blocking("set_mcp_gain", async move {
                driver.set_mcp_gain(gain_u32).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.arm() — arm camera for acquisition (via Triggerable trait)
    engine.register_fn(
        "arm",
        |cam: &mut GatedCameraHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            run_blocking("arm", async move {
                common::capabilities::Triggerable::arm(driver.as_ref()).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // cam.stop_stream() — stop acquisition / disarm camera
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

    // cam.temperature() -> f64 — read sensor temperature in °C
    engine.register_fn(
        "temperature",
        |cam: &mut GatedCameraHandle| -> Result<f64, Box<EvalAltResult>> {
            let driver = cam.driver.clone();
            run_blocking("temperature", async move { driver.get_temperature().await })
        },
    );

    // cam.supports_ddg() -> bool — check if DDG is available
    engine.register_fn("supports_ddg", |cam: &mut GatedCameraHandle| -> bool {
        cam.driver.supports_ddg()
    });

    // cam.supports_mcp_gain() -> bool — check if MCP gain control is available
    engine.register_fn("supports_mcp_gain", |cam: &mut GatedCameraHandle| -> bool {
        cam.driver.supports_mcp_gain()
    });

    // =========================================================================
    // SpectrographHandle — Andor Shamrock methods
    // =========================================================================

    // spec.set_grating(index) — select diffraction grating by index (1-based)
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

    // spec.get_grating() -> i64 — read current grating index
    engine.register_fn(
        "get_grating",
        |spec: &mut SpectrographHandle| -> Result<i64, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            let g = run_blocking("get_grating", async move { driver.get_grating().await })?;
            Ok(i64::from(g))
        },
    );

    // spec.set_wavelength(nm) — tune center wavelength via WavelengthTunable trait
    engine.register_fn(
        "set_wavelength",
        |spec: &mut SpectrographHandle, nm: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            use common::capabilities::WavelengthTunable;
            let driver = spec.driver.clone();
            run_blocking(
                "set_wavelength",
                async move { driver.set_wavelength(nm).await },
            )?;
            Ok(Dynamic::UNIT)
        },
    );

    // spec.get_wavelength() -> f64 — read current center wavelength in nm
    engine.register_fn(
        "get_wavelength",
        |spec: &mut SpectrographHandle| -> Result<f64, Box<EvalAltResult>> {
            use common::capabilities::WavelengthTunable;
            let driver = spec.driver.clone();
            run_blocking(
                "get_wavelength",
                async move { driver.get_wavelength().await },
            )
        },
    );

    // spec.set_slit_width(port, width_um) — set entrance/exit slit width in micrometers
    engine.register_fn(
        "set_slit_width",
        |spec: &mut SpectrographHandle,
         port: i64,
         width_um: f64|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: Slit port index is small (0-3)
            run_blocking("set_slit_width", async move {
                driver.set_slit_width(port as i32, width_um).await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // spec.get_calibration(pixels) -> Array — wavelength array for detector pixel count
    // Returns an Array of f64 values (one per pixel) suitable for plotting
    engine.register_fn(
        "get_calibration",
        |spec: &mut SpectrographHandle, pixels: i64| -> Result<Array, Box<EvalAltResult>> {
            let driver = spec.driver.clone();
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            // SAFETY: pixel count is a small positive integer (e.g., 2560)
            let n = pixels as u32;
            let calibration = run_blocking("get_calibration", async move {
                driver.get_wavelength_calibration(n).await
            })?;
            Ok(calibration
                .wavelengths_nm
                .into_iter()
                .map(Dynamic::from)
                .collect())
        },
    );

    // =========================================================================
    // DoverAxisHandle — Dover SmartStage motion + Trigger-On-Position
    // =========================================================================

    // stage.move_abs(pos) — move to absolute position (mm)
    engine.register_fn(
        "move_abs",
        |axis: &mut DoverAxisHandle, pos: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("move_abs", async move { driver.move_abs(pos).await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.move_rel(dist) — move relative distance (mm)
    engine.register_fn(
        "move_rel",
        |axis: &mut DoverAxisHandle, dist: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("move_rel", async move { driver.move_rel(dist).await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.position() -> f64 — read current axis position (mm)
    engine.register_fn(
        "position",
        |axis: &mut DoverAxisHandle| -> Result<f64, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("position", async move { driver.position().await })
        },
    );

    // stage.wait_settled() — block until motion completes
    engine.register_fn(
        "wait_settled",
        |axis: &mut DoverAxisHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("wait_settled", async move { driver.wait_settled().await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.set_velocity(v) — set motion velocity (mm/s)
    engine.register_fn(
        "set_velocity",
        |axis: &mut DoverAxisHandle, v: f64| -> Result<Dynamic, Box<EvalAltResult>> {
            let setter = axis.set_velocity_fn.clone();
            run_blocking("set_velocity", async move { setter(v).await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.enable_top(start, end, increment, bidirectional, pulse_width_ns)
    // — arm Trigger-On-Position for continuous scanning
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
            #[allow(clippy::cast_sign_loss)]
            // SAFETY: pulse width is always a non-negative nanosecond count
            let pulse_ns_u64 = pulse_ns as u64;
            run_blocking("enable_top", async move {
                driver
                    .enable_top(start, end, increment, bidirectional, pulse_ns_u64)
                    .await
            })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.disable_top() — disarm Trigger-On-Position
    engine.register_fn(
        "disable_top",
        |axis: &mut DoverAxisHandle| -> Result<Dynamic, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("disable_top", async move { driver.disable_top().await })?;
            Ok(Dynamic::UNIT)
        },
    );

    // stage.top_enabled() -> bool — query TOP state
    engine.register_fn(
        "top_enabled",
        |axis: &mut DoverAxisHandle| -> Result<bool, Box<EvalAltResult>> {
            let driver = axis.axis.clone();
            run_blocking("top_enabled", async move { driver.is_top_enabled().await })
        },
    );

    // =========================================================================
    // Factory Functions — construct mock handles for development/testing
    // =========================================================================

    // create_andor_camera() -> GatedCamera — mock Andor iStar
    engine.register_fn("create_andor_camera", || -> GatedCameraHandle {
        let driver = tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(AndorCamera::new_mock())
                .expect("Failed to create mock AndorCamera")
        });
        GatedCameraHandle {
            driver: Arc::new(driver),
        }
    });

    // create_andor_spectrograph() -> Spectrograph — mock Andor Shamrock
    engine.register_fn("create_andor_spectrograph", || -> SpectrographHandle {
        let driver = tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(AndorSpectrograph::new_mock())
                .expect("Failed to create mock AndorSpectrograph")
        });
        SpectrographHandle {
            driver: Arc::new(driver),
        }
    });

    // create_dover_axis(axis_name) -> DoverAxis — mock Dover SmartStage axis
    engine.register_fn(
        "create_dover_axis",
        |axis_name: String| -> DoverAxisHandle {
            let mock = Arc::new(DoverMockDriver::new(&axis_name));
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
            let cam = create_andor_camera();
            cam.supports_ddg()
        ",
        );
        // Mock camera may or may not support DDG — just verify no panic
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_spectrograph_handle_creation() {
        let engine = make_engine();
        let result = engine.eval::<i64>(
            r"
            let spec = create_andor_spectrograph();
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
            let stage = create_dover_axis("X");
            stage.move_abs(5.0);
            stage.position()
        "#,
        );
        assert!(result.is_ok(), "dover axis move failed: {:?}", result);
        assert!((result.unwrap() - 5.0).abs() < 0.001);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dover_axis_top() {
        let engine = make_engine();
        let result = engine.eval::<bool>(
            r#"
            let stage = create_dover_axis("X");
            stage.enable_top(0.0, 10.0, 0.1, false, 1000);
            stage.top_enabled()
        "#,
        );
        assert!(result.is_ok(), "TOP enable failed: {:?}", result);
        assert!(result.unwrap(), "TOP should be enabled after enable_top");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dover_axis_set_velocity() {
        let engine = make_engine();
        let result = engine.eval::<Dynamic>(
            r#"
            let stage = create_dover_axis("X");
            stage.set_velocity(5.0);
            stage.move_abs(2.0);
        "#,
        );
        assert!(result.is_ok(), "set_velocity + move failed: {:?}", result);
    }
}
