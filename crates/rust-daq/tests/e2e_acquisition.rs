#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
//! End-to-End Acquisition Tests
//!
//! Tests that simulate full acquisition sessions with multiple instruments,
//! including failure injection scenarios.
//!
//! Run with: cargo test --test e2e_acquisition

use anyhow::Result;
use rust_daq::hardware::capabilities::{FrameProducer, Movable, Readable, Triggerable};
use rust_daq::hardware::mock::{MockCamera, MockPowerMeter, MockStage};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// =============================================================================
// Mock with Configurable Failures
// =============================================================================

/// Mock stage with configurable failure injection
pub struct FailableStage {
    inner: MockStage,
    fail_after_moves: Arc<AtomicU32>,
    moves_count: Arc<AtomicU32>,
    should_fail: Arc<AtomicBool>,
}

impl FailableStage {
    pub fn new() -> Self {
        Self {
            inner: MockStage::new(),
            fail_after_moves: Arc::new(AtomicU32::new(u32::MAX)),
            moves_count: Arc::new(AtomicU32::new(0)),
            should_fail: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Configure stage to fail after N moves
    pub fn fail_after(&self, moves: u32) {
        self.fail_after_moves.store(moves, Ordering::SeqCst);
    }

    /// Set immediate failure mode
    pub fn set_fail(&self, fail: bool) {
        self.should_fail.store(fail, Ordering::SeqCst);
    }

    pub fn moves_count(&self) -> u32 {
        self.moves_count.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Movable for FailableStage {
    async fn move_abs(&self, target: f64) -> Result<()> {
        if self.should_fail.load(Ordering::SeqCst) {
            anyhow::bail!("FailableStage: Simulated hardware failure");
        }

        let count = self.moves_count.fetch_add(1, Ordering::SeqCst);
        if count >= self.fail_after_moves.load(Ordering::SeqCst) {
            anyhow::bail!("FailableStage: Failed after {} moves", count);
        }

        self.inner.move_abs(target).await
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = self.position().await?;
        self.move_abs(current + distance).await
    }

    async fn position(&self) -> Result<f64> {
        if self.should_fail.load(Ordering::SeqCst) {
            anyhow::bail!("FailableStage: Simulated hardware failure");
        }
        self.inner.position().await
    }

    async fn wait_settled(&self) -> Result<()> {
        self.inner.wait_settled().await
    }
}

/// Mock camera with configurable failure injection
pub struct FailableCamera {
    inner: MockCamera,
    fail_after_frames: Arc<AtomicU32>,
    should_fail: Arc<AtomicBool>,
}

impl FailableCamera {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            inner: MockCamera::new(width, height),
            fail_after_frames: Arc::new(AtomicU32::new(u32::MAX)),
            should_fail: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Configure camera to fail after N frames
    pub fn fail_after(&self, frames: u32) {
        self.fail_after_frames.store(frames, Ordering::SeqCst);
    }

    /// Set immediate failure mode
    pub fn set_fail(&self, fail: bool) {
        self.should_fail.store(fail, Ordering::SeqCst);
    }

    pub fn frame_count(&self) -> u64 {
        self.inner.get_frame_count()
    }
}

#[async_trait::async_trait]
impl Triggerable for FailableCamera {
    async fn arm(&self) -> Result<()> {
        if self.should_fail.load(Ordering::SeqCst) {
            anyhow::bail!("FailableCamera: Simulated hardware failure");
        }
        self.inner.arm().await
    }

    async fn trigger(&self) -> Result<()> {
        if self.should_fail.load(Ordering::SeqCst) {
            anyhow::bail!("FailableCamera: Simulated hardware failure");
        }

        let count = self.inner.get_frame_count();
        if count >= self.fail_after_frames.load(Ordering::SeqCst) as u64 {
            anyhow::bail!("FailableCamera: Failed after {} frames", count);
        }

        self.inner.trigger().await
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.inner.is_armed().await)
    }
}

// =============================================================================
// Test Data Collection Helper
// =============================================================================

#[derive(Debug, Clone)]
#[expect(
    dead_code,
    reason = "AcquisitionResult fields used for validation in end-to-end tests"
)]
struct AcquisitionResult {
    position: f64,
    power: f64,
    frame_index: u64,
    timestamp_ms: u128,
}

struct AcquisitionSession {
    results: Arc<RwLock<Vec<AcquisitionResult>>>,
    errors: Arc<RwLock<Vec<String>>>,
    start_time: Instant,
}

impl AcquisitionSession {
    fn new() -> Self {
        Self {
            results: Arc::new(RwLock::new(Vec::new())),
            errors: Arc::new(RwLock::new(Vec::new())),
            start_time: Instant::now(),
        }
    }

    async fn record(&self, position: f64, power: f64, frame_index: u64) {
        let result = AcquisitionResult {
            position,
            power,
            frame_index,
            timestamp_ms: self.start_time.elapsed().as_millis(),
        };
        self.results.write().await.push(result);
    }

    async fn record_error(&self, error: String) {
        self.errors.write().await.push(error);
    }

    async fn result_count(&self) -> usize {
        self.results.read().await.len()
    }

    async fn error_count(&self) -> usize {
        self.errors.read().await.len()
    }
}

// =============================================================================
// E2E Test: Full Acquisition Workflow
// =============================================================================

#[tokio::test]
async fn test_full_acquisition_session() {
    let stage = MockStage::new();
    let camera = MockCamera::new(1920, 1080);
    let power_meter = MockPowerMeter::new(2.5);
    let session = AcquisitionSession::new();

    // Positions to scan
    let positions = [0.0, 5.0, 10.0, 15.0, 20.0];

    // Arm camera
    camera.arm().await.unwrap();

    // Run acquisition
    for (i, &pos) in positions.iter().enumerate() {
        // Move stage
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        // Verify position
        let actual_pos = stage.position().await.unwrap();
        assert!(
            (actual_pos - pos).abs() < 0.001,
            "Position mismatch: expected {}, got {}",
            pos,
            actual_pos
        );

        // Read power
        let power = power_meter.read().await.unwrap();

        // Trigger camera
        camera.trigger().await.unwrap();

        // Record result
        session.record(pos, power, (i + 1) as u64).await;
    }

    // Verify all data collected
    assert_eq!(session.result_count().await, 5);
    assert_eq!(camera.frame_count(), 5);
    assert_eq!(session.error_count().await, 0);

    println!(
        "Full acquisition complete: {} positions, {} frames",
        session.result_count().await,
        camera.frame_count()
    );
}

// =============================================================================
// E2E Test: Multi-Instrument Coordination
// =============================================================================

#[tokio::test]
async fn test_coordinated_multi_instrument() {
    let stage1 = Arc::new(MockStage::new());
    let stage2 = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));
    let power_meter = Arc::new(MockPowerMeter::new(2.5));

    // Arm camera
    camera.arm().await.unwrap();

    // Run coordinated acquisition with parallel instrument access
    let positions = vec![(0.0, 0.0), (5.0, 10.0), (10.0, 20.0), (15.0, 30.0)];

    let results = Arc::new(RwLock::new(Vec::new()));

    for (pos1, pos2) in positions {
        // Move both stages in parallel
        let s1 = stage1.clone();
        let s2 = stage2.clone();

        let (r1, r2) = tokio::join!(s1.move_abs(pos1), s2.move_abs(pos2));

        r1.unwrap();
        r2.unwrap();

        // Wait for both to settle
        let (r1, r2) = tokio::join!(stage1.wait_settled(), stage2.wait_settled());
        r1.unwrap();
        r2.unwrap();

        // Verify positions
        let actual1 = stage1.position().await.unwrap();
        let actual2 = stage2.position().await.unwrap();

        assert!((actual1 - pos1).abs() < 0.001, "Stage1 position mismatch");
        assert!((actual2 - pos2).abs() < 0.001, "Stage2 position mismatch");

        // Read power and trigger camera
        let power = power_meter.read().await.unwrap();
        camera.trigger().await.unwrap();

        results.write().await.push((actual1, actual2, power));
    }

    // Verify all data collected
    assert_eq!(results.read().await.len(), 4);
    assert_eq!(camera.frame_count(), 4);

    println!(
        "Coordinated acquisition complete: {} synchronized readings",
        results.read().await.len()
    );
}

// =============================================================================
// E2E Test: Failure Recovery - Stage Failure Mid-Acquisition
// =============================================================================

#[tokio::test]
async fn test_stage_failure_recovery() {
    let stage = FailableStage::new();
    let camera = MockCamera::new(1920, 1080);
    let session = AcquisitionSession::new();

    // Configure stage to fail after 3 moves
    stage.fail_after(3);

    camera.arm().await.unwrap();

    let positions = vec![0.0, 5.0, 10.0, 15.0, 20.0]; // 5 positions but fail after 3
    let mut successful_moves = 0;

    for &pos in &positions {
        match stage.move_abs(pos).await {
            Ok(_) => {
                stage.wait_settled().await.unwrap();
                camera.trigger().await.unwrap();
                session.record(pos, 0.0, camera.frame_count()).await;
                successful_moves += 1;
            }
            Err(e) => {
                session.record_error(e.to_string()).await;
                // In a real system, we might retry or abort gracefully
                break;
            }
        }
    }

    // Should have completed 3 moves before failure
    assert_eq!(successful_moves, 3);
    assert_eq!(session.result_count().await, 3);
    assert_eq!(session.error_count().await, 1);

    println!(
        "Stage failure test: {} successful, {} errors",
        session.result_count().await,
        session.error_count().await
    );
}

// =============================================================================
// E2E Test: Failure Recovery - Camera Failure Mid-Acquisition
// =============================================================================

#[tokio::test]
async fn test_camera_failure_recovery() {
    let stage = MockStage::new();
    let camera = FailableCamera::new(1920, 1080);
    let session = AcquisitionSession::new();

    // Configure camera to fail after 2 frames
    camera.fail_after(2);

    camera.arm().await.unwrap();

    let positions = vec![0.0, 5.0, 10.0, 15.0, 20.0];
    let mut successful_frames = 0;

    for &pos in &positions {
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        match camera.trigger().await {
            Ok(_) => {
                session.record(pos, 0.0, camera.frame_count()).await;
                successful_frames += 1;
            }
            Err(e) => {
                session.record_error(e.to_string()).await;
                break;
            }
        }
    }

    // Should have completed 2 frames before failure
    assert_eq!(successful_frames, 2);
    assert_eq!(session.result_count().await, 2);
    assert!(session.error_count().await >= 1);

    println!(
        "Camera failure test: {} successful, {} errors",
        session.result_count().await,
        session.error_count().await
    );
}

// =============================================================================
// E2E Test: Concurrent Hardware Operations
// =============================================================================

#[tokio::test]
async fn test_concurrent_hardware_stress() {
    let stage = Arc::new(MockStage::with_speed(100.0)); // Fast for stress test
    let camera = Arc::new(MockCamera::new(640, 480)); // Smaller for faster test

    camera.arm().await.unwrap();

    // Spawn concurrent tasks
    let stage_task = {
        let stage = stage.clone();
        tokio::spawn(async move {
            let mut moves = 0;
            for i in 0..20 {
                let pos = (i % 5) as f64 * 2.0;
                stage.move_abs(pos).await.unwrap();
                moves += 1;
            }
            moves
        })
    };

    let camera_task = {
        let camera = camera.clone();
        tokio::spawn(async move {
            let mut frames = 0;
            for _ in 0..20 {
                camera.trigger().await.unwrap();
                frames += 1;
            }
            frames
        })
    };

    // Wait for both tasks
    let (stage_moves, camera_frames) = tokio::join!(stage_task, camera_task);

    assert_eq!(stage_moves.unwrap(), 20);
    assert_eq!(camera_frames.unwrap(), 20);
    assert_eq!(camera.frame_count(), 20);

    println!("Concurrent stress test: 20 moves + 20 frames completed");
}

// =============================================================================
// E2E Test: Graceful Shutdown During Acquisition
// =============================================================================

#[tokio::test]
async fn test_graceful_shutdown() {
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    camera.arm().await.unwrap();

    // Start acquisition in background
    let acquisition_task = {
        let stage = stage.clone();
        let camera = camera.clone();
        let shutdown = shutdown_flag.clone();

        tokio::spawn(async move {
            let mut completed = 0;
            for i in 0..100 {
                // Check shutdown flag
                if shutdown.load(Ordering::SeqCst) {
                    println!("Shutdown requested, stopping acquisition");
                    break;
                }

                let pos = i as f64;
                stage.move_abs(pos).await.unwrap();
                camera.trigger().await.unwrap();
                completed += 1;
            }
            completed
        })
    };

    // Request shutdown after 500ms
    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown_flag.store(true, Ordering::SeqCst);

    // Wait for task to complete
    let completed = acquisition_task.await.unwrap();

    println!(
        "Graceful shutdown: {} frames completed before shutdown",
        completed
    );

    // Should have completed some frames but not all 100
    assert!(completed > 0, "Should have completed some frames");
    assert!(completed < 100, "Should have stopped before completing all");
}

// =============================================================================
// E2E Test: High-Throughput Acquisition
// =============================================================================

#[tokio::test(start_paused = true)]
async fn test_high_throughput_acquisition() {
    use tokio::time::Instant;

    let camera = MockCamera::new(640, 480);
    let power_meter = MockPowerMeter::new(1.0);

    camera.arm().await.unwrap();

    let start = Instant::now();
    let num_samples = 100;

    for _ in 0..num_samples {
        let (_power, _trigger) = tokio::join!(power_meter.read(), camera.trigger());
    }

    let elapsed = start.elapsed();
    let samples_per_sec = num_samples as f64 / elapsed.as_secs_f64();

    println!(
        "High-throughput test: {} samples in {:?} ({:.1} samples/sec, simulated time)",
        num_samples, elapsed, samples_per_sec
    );

    // With start_paused, time is deterministic:
    // - MockCamera trigger takes 33ms (30fps simulation)
    // - MockPowerMeter read is near-instant
    // - 100 samples * 33ms = 3300ms total
    // - Expected throughput: ~30.3 samples/sec
    assert_eq!(
        elapsed.as_millis(),
        3300,
        "Expected 3300ms for 100 samples at 33ms each, got {}ms",
        elapsed.as_millis()
    );
    assert_eq!(camera.frame_count(), 100, "Should have captured 100 frames");
}

// =============================================================================
// E2E Test: Data Integrity Verification
// =============================================================================

#[tokio::test]
async fn test_data_integrity() {
    let stage = MockStage::new();
    let power_meter = MockPowerMeter::new(2.5);
    let camera = MockCamera::new(1920, 1080);

    camera.arm().await.unwrap();

    let mut results: Vec<(f64, f64, u64)> = Vec::new();
    let expected_positions = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0];

    for &expected_pos in &expected_positions {
        stage.move_abs(expected_pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        let actual_pos = stage.position().await.unwrap();
        let power = power_meter.read().await.unwrap();
        camera.trigger().await.unwrap();
        let frame = camera.frame_count();

        results.push((actual_pos, power, frame));
    }

    // Verify data integrity
    assert_eq!(results.len(), expected_positions.len());

    for (i, (pos, power, frame)) in results.iter().enumerate() {
        // Position should match expected
        assert!(
            (*pos - expected_positions[i]).abs() < 0.001,
            "Position {} mismatch at index {}",
            pos,
            i
        );

        // Power should be in valid range
        assert!(
            *power > 0.0 && *power < 10.0,
            "Power {} out of range at index {}",
            power,
            i
        );

        // Frame count should be sequential
        assert_eq!(
            *frame,
            (i + 1) as u64,
            "Frame count mismatch at index {}: expected {}, got {}",
            i,
            i + 1,
            frame
        );
    }

    println!("Data integrity verified for {} samples", results.len());
}
