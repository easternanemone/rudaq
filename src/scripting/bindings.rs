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
//! Uses `tokio::task::block_in_place()` to safely execute async hardware operations
//! from synchronous Rhai scripts. This allows scripts to call `stage.move_abs(10.0)`
//! without dealing with async/await.
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

use rhai::{Dynamic, Engine};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::task::block_in_place;
use chrono::Utc;

use crate::hardware::capabilities::{Camera, Movable};
use crate::core::Measurement;

// =============================================================================
// Handle Types - Rhai-Compatible Wrappers
// =============================================================================

/// Handle to a stage device that can be used in Rhai scripts
///
/// Wraps any device implementing `Movable` trait (stages, actuators, goniometers).
/// Provides synchronous methods that scripts can call directly.
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
    pub driver: Arc<dyn Movable>,
    /// Optional data sender for broadcasting measurements to RingBuffer/gRPC clients
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
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
    pub driver: Arc<dyn Camera>,
    /// Optional data sender for broadcasting measurements to RingBuffer/gRPC clients
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
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
    engine.register_fn("move_abs", move |stage: &mut StageHandle, pos: f64| {
        block_in_place(|| Handle::current().block_on(stage.driver.move_abs(pos))).unwrap();

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
    });

    // stage.move_rel(5.0) - Move relative distance
    engine.register_fn("move_rel", move |stage: &mut StageHandle, dist: f64| {
        block_in_place(|| Handle::current().block_on(stage.driver.move_rel(dist))).unwrap()
    });

    // let pos = stage.position() - Get current position
    engine.register_fn("position", move |stage: &mut StageHandle| -> f64 {
        block_in_place(|| Handle::current().block_on(stage.driver.position())).unwrap()
    });

    // stage.wait_settled() - Wait for motion to complete
    engine.register_fn("wait_settled", move |stage: &mut StageHandle| {
        block_in_place(|| Handle::current().block_on(stage.driver.wait_settled())).unwrap()
    });

    // =========================================================================
    // Camera Methods - Acquisition Control
    // =========================================================================

    // camera.arm() - Prepare camera for trigger
    engine.register_fn(
        "arm",
        move |camera: &mut CameraHandle| match block_in_place(|| {
            Handle::current().block_on(camera.driver.arm())
        }) {
            Ok(_) => (),
            Err(e) => panic!("Camera arm failed: {}", e),
        },
    );

    // camera.trigger() - Capture frame
    engine.register_fn(
        "trigger",
        move |camera: &mut CameraHandle| match block_in_place(|| {
            Handle::current().block_on(camera.driver.trigger())
        }) {
            Ok(_) => {
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
            }
            Err(e) => panic!("Camera trigger failed: {}", e),
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
    use crate::hardware::mock::{MockCamera, MockStage};
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

    #[test]
    fn test_sleep_function() {
        let mut engine = Engine::new();
        register_hardware(&mut engine);

        let start = std::time::Instant::now();
        engine.eval::<()>("sleep(0.1)").unwrap();
        let elapsed = start.elapsed();

        // Should sleep for ~100ms (allow some tolerance)
        assert!(elapsed.as_millis() >= 95);
        assert!(elapsed.as_millis() <= 150);
    }
}
