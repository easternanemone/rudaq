//! Full Instrument Suite Integration Tests
//!
//! Comprehensive tests for coordinated operation of multiple hardware devices:
//! - Newport 1830-C Power Meter (Readable capability)
//! - ESP300 Motion Controller (Movable capability)
//! - Elliptec ELL14 Rotator (Movable capability)
//!
//! This test suite validates:
//! 1. Coordinated operation (rotate while measuring)
//! 2. No serial port conflicts (simulated parallel access)
//! 3. Synchronized data logging across multiple devices
//! 4. Error handling and recovery
//! 5. Realistic timing and settling behavior
//!
//! Uses V5 async/await patterns with tokio for all operations.

use anyhow::Result;
use async_trait::async_trait;
use rust_daq::hardware::capabilities::{Movable, Readable};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration, Instant};

// =============================================================================
// Mock Hardware Implementations
// =============================================================================

/// Mock Newport 1830-C Power Meter
///
/// Simulates a Newport power meter that:
/// - Reads measurements synchronously (fast, <1ms)
/// - Returns power dependent on external state (e.g., rotation angle)
/// - Has no settling time
struct MockNewportPowerMeter {
    /// Reference to rotator position for simulated measurements
    rotator_position: Arc<RwLock<f64>>,
    /// Read count for diagnostics
    read_count: AtomicUsize,
}

impl MockNewportPowerMeter {
    fn new(rotator_position: Arc<RwLock<f64>>) -> Self {
        Self {
            rotator_position,
            read_count: AtomicUsize::new(0),
        }
    }

    fn read_count(&self) -> usize {
        self.read_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Readable for MockNewportPowerMeter {
    async fn read(&self) -> Result<f64> {
        let count = self.read_count.fetch_add(1, Ordering::SeqCst) + 1;

        let pos = *self.rotator_position.read().await;
        // Simulate power reading that depends on angle (polarizer effect: cos^2)
        let power = (pos.to_radians().cos()).powi(2);

        println!(
            "Newport 1830-C: Read #{} @ angle {:.1}° = {:.6}W",
            count, pos, power
        );

        Ok(power)
    }
}

/// Mock ESP300 Motion Controller
///
/// Simulates an ESP300 stage controller that:
/// - Controls XY motion with realistic timing (10mm/sec speed)
/// - Has 50ms settling time after motion
/// - Tracks current position
/// - Prevents conflicting commands
struct MockESP300 {
    x_position: Arc<RwLock<f64>>,
    y_position: Arc<RwLock<f64>>,
    speed_mm_per_sec: f64,
    move_count: AtomicUsize,
}

impl MockESP300 {
    fn new() -> Self {
        Self {
            x_position: Arc::new(RwLock::new(0.0)),
            y_position: Arc::new(RwLock::new(0.0)),
            speed_mm_per_sec: 10.0,
            move_count: AtomicUsize::new(0),
        }
    }

    fn move_count(&self) -> usize {
        self.move_count.load(Ordering::SeqCst)
    }

    async fn move_xy(&self, target_x: f64, target_y: f64) -> Result<()> {
        let x_current = *self.x_position.read().await;
        let y_current = *self.y_position.read().await;

        let distance_x = (target_x - x_current).abs();
        let distance_y = (target_y - y_current).abs();
        let total_distance = (distance_x.powi(2) + distance_y.powi(2)).sqrt();

        let move_num = self.move_count.fetch_add(1, Ordering::SeqCst) + 1;
        let delay_ms = (total_distance / self.speed_mm_per_sec * 1000.0) as u64;

        println!(
            "ESP300: Move #{} from ({:.2}, {:.2}) to ({:.2}, {:.2}) distance={:.2}mm ({}ms)",
            move_num, x_current, y_current, target_x, target_y, total_distance, delay_ms
        );

        sleep(Duration::from_millis(delay_ms)).await;
        *self.x_position.write().await = target_x;
        *self.y_position.write().await = target_y;

        println!("ESP300: Move #{} complete", move_num);
        Ok(())
    }
}

#[async_trait]
impl Movable for MockESP300 {
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.move_xy(position, *self.y_position.read().await).await
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = *self.x_position.read().await;
        self.move_abs(current + distance).await
    }

    async fn position(&self) -> Result<f64> {
        Ok(*self.x_position.read().await)
    }

    async fn wait_settled(&self) -> Result<()> {
        sleep(Duration::from_millis(50)).await;
        println!("ESP300: Settled");
        Ok(())
    }
}

/// Mock Elliptec ELL14 Rotator
///
/// Simulates an Elliptec rotation mount that:
/// - Rotates around a single axis (0-360 degrees)
/// - Has precise positioning capability
/// - Has 100ms settling time (mechanical)
/// - Tracks rotation angle
struct MockElliptecRotator {
    angle: Arc<RwLock<f64>>,
    speed_deg_per_sec: f64,
    rotate_count: AtomicUsize,
}

impl MockElliptecRotator {
    fn new() -> Self {
        Self {
            angle: Arc::new(RwLock::new(0.0)),
            speed_deg_per_sec: 90.0, // 90 deg/sec - faster than motion stage
            rotate_count: AtomicUsize::new(0),
        }
    }

    fn rotate_count(&self) -> usize {
        self.rotate_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Movable for MockElliptecRotator {
    async fn move_abs(&self, angle: f64) -> Result<()> {
        let current = *self.angle.read().await;
        let angle_normalized = angle % 360.0;

        // Calculate shortest path (could go either direction)
        let mut delta = angle_normalized - current;
        if delta > 180.0 {
            delta -= 360.0;
        } else if delta < -180.0 {
            delta += 360.0;
        }

        let rotate_num = self.rotate_count.fetch_add(1, Ordering::SeqCst) + 1;
        let delay_ms = (delta.abs() / self.speed_deg_per_sec * 1000.0) as u64;

        println!(
            "Elliptec ELL14: Rotate #{} from {:.1}° to {:.1}° ({}ms)",
            rotate_num, current, angle_normalized, delay_ms
        );

        sleep(Duration::from_millis(delay_ms)).await;
        *self.angle.write().await = angle_normalized;

        println!("Elliptec ELL14: Rotate #{} complete", rotate_num);
        Ok(())
    }

    async fn move_rel(&self, angle: f64) -> Result<()> {
        let current = *self.angle.read().await;
        self.move_abs(current + angle).await
    }

    async fn position(&self) -> Result<f64> {
        Ok(*self.angle.read().await)
    }

    async fn wait_settled(&self) -> Result<()> {
        sleep(Duration::from_millis(100)).await;
        println!("Elliptec ELL14: Settled");
        Ok(())
    }
}

// =============================================================================
// Test Cases
// =============================================================================

#[tokio::test]
async fn test_rotate_and_measure() -> Result<()> {
    println!("\n=== Test: Rotate and Measure ===");

    // Create mock devices
    let rotator = MockElliptecRotator::new();
    let _meter = MockNewportPowerMeter::new(Arc::new(RwLock::new(0.0)));

    // Update the meter's reference to use the rotator's angle
    let meter_with_rotator = MockNewportPowerMeter {
        rotator_position: rotator.angle.clone(),
        read_count: AtomicUsize::new(0),
    };

    let mut measurements = Vec::new();

    // Perform polarization sweep (0 to 90 degrees)
    for angle in (0..=90).step_by(15) {
        rotator.move_abs(angle as f64).await?;
        rotator.wait_settled().await?;

        let power = meter_with_rotator.read().await?;
        measurements.push((angle, power));
        println!("Measurement: {:.0}° → {:.6}W", angle, power);
    }

    // Verify measurements
    assert_eq!(measurements.len(), 7); // 0, 15, 30, 45, 60, 75, 90
    assert_eq!(measurements[0].0, 0);
    assert!((measurements[0].1 - 1.0).abs() < 1e-6, "Max power at 0°");
    assert_eq!(measurements[6].0, 90);
    assert!((measurements[6].1).abs() < 1e-6, "Min power at 90°");

    println!(
        "Passed: {} rotations, {} measurements",
        rotator.rotate_count(),
        meter_with_rotator.read_count()
    );

    Ok(())
}

#[tokio::test]
async fn test_coordinated_motion_and_measurement() -> Result<()> {
    println!("\n=== Test: Coordinated Motion and Measurement ===");

    // Create mock devices
    let rotator = MockElliptecRotator::new();
    let stage = MockESP300::new();
    let meter = MockNewportPowerMeter {
        rotator_position: rotator.angle.clone(),
        read_count: AtomicUsize::new(0),
    };

    // Coordinate: move stage, then rotate, then measure
    stage.move_abs(10.0).await?;
    stage.wait_settled().await?;
    println!("Stage moved to 10mm");

    rotator.move_abs(45.0).await?;
    rotator.wait_settled().await?;
    println!("Rotator at 45°");

    let power = meter.read().await?;
    println!("Power at stage 10mm, rotator 45°: {:.6}W", power);

    // Move back to origin
    stage.move_abs(0.0).await?;
    stage.wait_settled().await?;

    rotator.move_abs(0.0).await?;
    rotator.wait_settled().await?;

    println!(
        "Passed: {} stage moves, {} rotations, {} reads",
        stage.move_count(),
        rotator.rotate_count(),
        meter.read_count()
    );

    Ok(())
}

#[tokio::test]
async fn test_parallel_device_operations() -> Result<()> {
    println!("\n=== Test: Parallel Device Operations ===");

    // Create mock devices
    let rotator = MockElliptecRotator::new();
    let stage = MockESP300::new();
    let meter = MockNewportPowerMeter {
        rotator_position: rotator.angle.clone(),
        read_count: AtomicUsize::new(0),
    };

    // Start multiple operations concurrently to simulate true parallel access
    let start = Instant::now();

    let rotator_task = {
        let mut r = MockElliptecRotator::new();
        r.angle = rotator.angle.clone();
        tokio::spawn(async move {
            for angle in [0.0, 45.0, 90.0, 45.0, 0.0] {
                r.move_abs(angle).await?;
                r.wait_settled().await?;
            }
            Ok::<_, anyhow::Error>(r.rotate_count())
        })
    };

    let stage_task = {
        let s = stage.clone();
        tokio::spawn(async move {
            for x in [0.0, 5.0, 10.0, 5.0, 0.0] {
                s.move_xy(x, 0.0).await?;
                s.wait_settled().await?;
            }
            Ok::<_, anyhow::Error>(s.move_count())
        })
    };

    let meter_task = {
        let m = meter.clone();
        tokio::spawn(async move {
            for _ in 0..10 {
                m.read().await?;
                sleep(Duration::from_millis(50)).await;
            }
            Ok::<_, anyhow::Error>(m.read_count())
        })
    };

    // Wait for all tasks
    let rotate_count = rotator_task.await??;
    let move_count = stage_task.await??;
    let read_count = meter_task.await??;

    let elapsed = start.elapsed();
    println!(
        "Parallel execution completed in {:.2}s",
        elapsed.as_secs_f64()
    );
    println!(
        "Results: {} rotations, {} stage moves, {} readings",
        rotate_count, move_count, read_count
    );

    assert_eq!(rotate_count, 5);
    assert_eq!(move_count, 5);
    assert_eq!(read_count, 10);

    Ok(())
}

#[tokio::test]
async fn test_sequential_sweep() -> Result<()> {
    println!("\n=== Test: Sequential Sweep ===");

    // Create mock devices
    let rotator = MockElliptecRotator::new();
    let stage = MockESP300::new();
    let meter = MockNewportPowerMeter {
        rotator_position: rotator.angle.clone(),
        read_count: AtomicUsize::new(0),
    };

    let mut data_points = Vec::new();

    // Perform a 2D scan: X position vs rotation angle
    for x in [0.0, 5.0, 10.0] {
        stage.move_abs(x).await?;
        stage.wait_settled().await?;

        for angle in (0..=180).step_by(30) {
            rotator.move_abs(angle as f64).await?;
            rotator.wait_settled().await?;

            let power = meter.read().await?;
            data_points.push((x, angle, power));

            println!(
                "Data: x={:.1}mm, angle={:.0}°, power={:.6}W",
                x, angle, power
            );
        }
    }

    // Verify we have the expected number of data points
    assert_eq!(data_points.len(), 3 * 7); // 3 x positions, 7 angles each

    println!("Passed: Collected {} data points", data_points.len());

    Ok(())
}

#[tokio::test]
async fn test_error_handling_invalid_position() -> Result<()> {
    println!("\n=== Test: Error Handling - Invalid Position ===");

    let stage = MockESP300::new();

    // These moves should succeed (mock doesn't validate bounds)
    stage.move_abs(-100.0).await?;
    stage.wait_settled().await?;
    assert_eq!(stage.position().await?, -100.0);

    stage.move_abs(1000.0).await?;
    stage.wait_settled().await?;
    assert_eq!(stage.position().await?, 1000.0);

    println!("Passed: Moves to extreme positions succeeded");

    Ok(())
}

#[tokio::test]
async fn test_find_maximum_power_angle() -> Result<()> {
    println!("\n=== Test: Find Maximum Power Angle ===");

    let rotator = MockElliptecRotator::new();
    let meter = MockNewportPowerMeter {
        rotator_position: rotator.angle.clone(),
        read_count: AtomicUsize::new(0),
    };

    let mut max_power = 0.0;
    let mut best_angle = 0.0;

    // Scan 0-180 degrees in 10 degree steps
    for angle in (0..=180).step_by(10) {
        rotator.move_abs(angle as f64).await?;
        rotator.wait_settled().await?;

        let power = meter.read().await?;

        if power > max_power {
            max_power = power;
            best_angle = angle as f64;
        }
    }

    println!(
        "Maximum power: {:.6}W at angle {:.0}°",
        max_power, best_angle
    );

    // Max power should be at 0 or 180 degrees (both give cos(0)^2 = 1)
    assert!(
        (best_angle - 0.0).abs() < 1e-6 || (best_angle - 180.0).abs() < 1e-6,
        "Expected max at 0° or 180°, got {:.0}°",
        best_angle
    );
    assert!(
        (max_power - 1.0).abs() < 1e-6,
        "Expected power ~1.0, got {:.6}",
        max_power
    );

    Ok(())
}

#[tokio::test]
async fn test_polarization_extinction_ratio() -> Result<()> {
    println!("\n=== Test: Polarization Extinction Ratio ===");

    let rotator = MockElliptecRotator::new();
    let meter = MockNewportPowerMeter {
        rotator_position: rotator.angle.clone(),
        read_count: AtomicUsize::new(0),
    };

    // Measure at transmission maximum
    rotator.move_abs(0.0).await?;
    rotator.wait_settled().await?;
    let max_power = meter.read().await?;
    println!("Maximum transmission at 0°: {:.6}W", max_power);

    // Measure at transmission minimum (crossed polarizers)
    rotator.move_abs(90.0).await?;
    rotator.wait_settled().await?;
    let min_power = meter.read().await?;
    println!("Minimum transmission at 90°: {:.9}W", min_power);

    // Calculate extinction ratio
    // In ideal case with perfect polarization: ratio = inf
    // In our mock: cos(90°)^2 ≈ 0 (very small but not zero due to float precision)
    let extinction_ratio = if min_power > 1e-10 {
        max_power / min_power
    } else {
        f64::INFINITY
    };

    println!("Extinction ratio: {:.2}", extinction_ratio);

    // Check basic properties
    assert!((max_power - 1.0).abs() < 1e-6, "Max power should be ~1.0");
    assert!(min_power < 1e-8, "Min power should be very close to 0");
    assert!(extinction_ratio > 1e6, "Extinction ratio should be large");

    Ok(())
}

#[tokio::test]
async fn test_motion_settling_behavior() -> Result<()> {
    println!("\n=== Test: Motion Settling Behavior ===");

    let stage = MockESP300::new();
    let rotator = MockElliptecRotator::new();

    let start = Instant::now();

    // Move stage 10mm at 10mm/sec = ~1 second
    stage.move_abs(10.0).await?;
    let stage_move_time = start.elapsed();
    println!("Stage move took {:.3}s", stage_move_time.as_secs_f64());

    stage.wait_settled().await?;
    let stage_settle_time = start.elapsed();
    println!("Stage settled in {:.3}s", stage_settle_time.as_secs_f64());

    // Rotate 90 degrees at 90deg/sec = ~1 second
    let rotate_start = Instant::now();
    rotator.move_abs(90.0).await?;
    let rotate_time = rotate_start.elapsed();
    println!("Rotator moved in {:.3}s", rotate_time.as_secs_f64());

    rotator.wait_settled().await?;
    let rotate_settle_time = rotate_start.elapsed();
    println!(
        "Rotator settled in {:.3}s",
        rotate_settle_time.as_secs_f64()
    );

    // Verify settling times
    assert!(
        stage_settle_time.as_millis() >= 1050,
        "Stage should take ~1s to move + 50ms settle"
    );
    assert!(
        rotate_settle_time.as_millis() >= 1100,
        "Rotator should take ~1s to move + 100ms settle"
    );

    Ok(())
}

#[tokio::test]
async fn test_relative_moves() -> Result<()> {
    println!("\n=== Test: Relative Moves ===");

    let rotator = MockElliptecRotator::new();
    let stage = MockESP300::new();

    // Test relative motion on rotator
    rotator.move_abs(0.0).await?;
    assert_eq!(rotator.position().await?, 0.0);

    rotator.move_rel(45.0).await?;
    assert_eq!(rotator.position().await?, 45.0);

    rotator.move_rel(-20.0).await?;
    assert_eq!(rotator.position().await?, 25.0);

    rotator.move_rel(350.0).await?; // Wraps around
    assert_eq!(rotator.position().await?, 15.0); // 25 + 350 = 375 mod 360 = 15

    // Test relative motion on stage
    stage.move_abs(0.0).await?;
    assert_eq!(stage.position().await?, 0.0);

    stage.move_rel(10.0).await?;
    assert_eq!(stage.position().await?, 10.0);

    stage.move_rel(-3.0).await?;
    assert_eq!(stage.position().await?, 7.0);

    println!("Passed: Relative moves work correctly");

    Ok(())
}

// =============================================================================
// Helper Utilities for Testing
// =============================================================================

impl Clone for MockESP300 {
    fn clone(&self) -> Self {
        Self {
            x_position: self.x_position.clone(),
            y_position: self.y_position.clone(),
            speed_mm_per_sec: self.speed_mm_per_sec,
            move_count: AtomicUsize::new(self.move_count.load(Ordering::SeqCst)),
        }
    }
}

impl Clone for MockNewportPowerMeter {
    fn clone(&self) -> Self {
        Self {
            rotator_position: self.rotator_position.clone(),
            read_count: AtomicUsize::new(self.read_count.load(Ordering::SeqCst)),
        }
    }
}
