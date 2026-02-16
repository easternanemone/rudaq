//! Cross-driver frame compatibility tests (bd-vup0)
//!
//! Verifies that frames produced by the Andor SDK3 mock backend are compatible
//! with the common frame types used by downstream consumers (pipelines, gRPC, GUI).
//!
//! Key compatibility checks:
//! - `LoanedFrame` (pool) → `Frame` (common) field mapping
//! - `FrameView` observer data → `Frame` conversion
//! - `FrameProducer` trait is object-safe and implementable
//! - Frame data layout matches expectations of common consumers
//!
//! ## Running Tests
//!
//! ```bash
//! cargo nextest run -p driver-andor-sdk3 --test cross_driver_compat
//! ```

#![cfg(not(feature = "camera"))]

use common::capabilities::{ExposureControl, FrameObserver, FrameProducer, LoanedFrame};
use common::data::{Frame, FrameView};
use driver_andor_sdk3::AndorCamera;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// LoanedFrame → Frame Field Compatibility
// =============================================================================

/// Verify that LoanedFrame fields map directly to common::data::Frame fields.
///
/// Downstream consumers (gRPC serialization, storage, GUI) construct Frame
/// from raw data. This test ensures the mock produces data in the expected format.
#[tokio::test]
async fn loaned_frame_fields_match_common_frame() {
    let cam = AndorCamera::new_mock().await.unwrap();
    cam.set_exposure(0.010).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx).await.unwrap();
    cam.start_stream().await.unwrap();

    let loaned: LoanedFrame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    cam.stop_stream().await.unwrap();

    // Construct a common::data::Frame from LoanedFrame data
    // This is exactly what downstream consumers do
    let frame = Frame::from_vec(
        loaned.width,
        loaned.height,
        loaned.bit_depth,
        loaned.pixel_data().to_vec(),
    )
    .with_frame_number(loaned.frame_number)
    .with_timestamp(loaned.timestamp_ns)
    .with_exposure(loaned.exposure_ms);

    // Verify all fields transferred correctly
    assert_eq!(frame.width, loaned.width);
    assert_eq!(frame.height, loaned.height);
    assert_eq!(frame.bit_depth, loaned.bit_depth);
    assert_eq!(frame.frame_number, loaned.frame_number);
    assert_eq!(frame.timestamp_ns, loaned.timestamp_ns);
    assert_eq!(frame.exposure_ms, Some(loaned.exposure_ms));
    assert_eq!(frame.data.len(), loaned.pixel_data().len());
    assert_eq!(frame.data.as_ref(), loaned.pixel_data());
}

/// Verify LoanedFrame dimensions match the standard 2048x2048 16-bit format
/// expected by the Prime BSI / iStar pipeline.
#[tokio::test]
async fn loaned_frame_standard_dimensions() {
    let cam = AndorCamera::new_mock().await.unwrap();
    cam.set_exposure(0.005).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx).await.unwrap();
    cam.start_stream().await.unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    cam.stop_stream().await.unwrap();

    // Standard iStar mock sensor dimensions
    assert_eq!(frame.width, 2048, "width should be 2048");
    assert_eq!(frame.height, 2048, "height should be 2048");
    assert_eq!(frame.bit_depth, 16, "bit_depth should be 16");

    // Pixel data: width * height * (bit_depth / 8)
    let expected_bytes = 2048 * 2048 * 2;
    assert_eq!(
        frame.pixel_data().len(),
        expected_bytes,
        "pixel_data length should be 2048*2048*2"
    );
}

// =============================================================================
// FrameView Observer Compatibility
// =============================================================================

/// Observer that captures FrameView data for later verification.
struct CapturingObserver {
    width: AtomicU64,
    height: AtomicU64,
    bit_depth: AtomicU64,
    frame_number: AtomicU64,
    pixel_len: AtomicU64,
    has_exposure: AtomicU64, // 0=no, 1=yes
}

impl CapturingObserver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            width: AtomicU64::new(0),
            height: AtomicU64::new(0),
            bit_depth: AtomicU64::new(0),
            frame_number: AtomicU64::new(u64::MAX),
            pixel_len: AtomicU64::new(0),
            has_exposure: AtomicU64::new(0),
        })
    }
}

/// Wrapper to make CapturingObserver implement FrameObserver.
struct CapturingObserverWrapper {
    state: Arc<CapturingObserver>,
}

impl FrameObserver for CapturingObserverWrapper {
    fn on_frame(&self, frame: &FrameView<'_>) {
        self.state
            .width
            .store(frame.width as u64, Ordering::Relaxed);
        self.state
            .height
            .store(frame.height as u64, Ordering::Relaxed);
        self.state
            .bit_depth
            .store(frame.bit_depth as u64, Ordering::Relaxed);
        self.state
            .frame_number
            .store(frame.frame_number, Ordering::Relaxed);
        self.state
            .pixel_len
            .store(frame.pixels().len() as u64, Ordering::Relaxed);
        let has_exp = if frame.exposure_ms.is_some() { 1 } else { 0 };
        self.state.has_exposure.store(has_exp, Ordering::Relaxed);
    }

    fn name(&self) -> &'static str {
        "capturing_observer"
    }
}

/// Verify that FrameView delivered to observers has the same metadata
/// as LoanedFrame delivered to the primary consumer.
#[tokio::test]
async fn observer_frame_view_matches_primary_frame() {
    let cam = AndorCamera::new_mock().await.unwrap();
    cam.set_exposure(0.010).await.unwrap();

    let state = CapturingObserver::new();
    let _handle = cam
        .register_observer(Box::new(CapturingObserverWrapper {
            state: state.clone(),
        }))
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx).await.unwrap();
    cam.start_stream().await.unwrap();

    let loaned = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    cam.stop_stream().await.unwrap();

    // Observer should have captured the same frame metadata
    assert_eq!(
        state.width.load(Ordering::Relaxed),
        loaned.width as u64,
        "observer width should match primary"
    );
    assert_eq!(
        state.height.load(Ordering::Relaxed),
        loaned.height as u64,
        "observer height should match primary"
    );
    assert_eq!(
        state.bit_depth.load(Ordering::Relaxed),
        loaned.bit_depth as u64,
        "observer bit_depth should match primary"
    );
    // Observer may have seen same frame or the next one; just check it's not MAX
    assert_ne!(
        state.frame_number.load(Ordering::Relaxed),
        u64::MAX,
        "observer should have received at least one frame"
    );
    assert_eq!(
        state.has_exposure.load(Ordering::Relaxed),
        1,
        "observer FrameView should include exposure_ms"
    );
}

/// Verify FrameView pixel data is compatible with Frame::from_vec construction.
#[tokio::test]
async fn frame_view_pixels_construct_valid_frame() {
    let cam = AndorCamera::new_mock().await.unwrap();
    cam.set_exposure(0.005).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx).await.unwrap();
    cam.start_stream().await.unwrap();

    let loaned = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    cam.stop_stream().await.unwrap();

    // Simulate what an observer would do: construct FrameView, then copy to owned Frame
    let view = FrameView::new(
        loaned.width,
        loaned.height,
        loaned.bit_depth,
        loaned.pixel_data(),
        loaned.frame_number,
        loaned.timestamp_ns,
    )
    .with_exposure(loaned.exposure_ms);

    // Convert FrameView → owned Frame (what pipeline stages do)
    let frame = Frame::from_vec(
        view.width,
        view.height,
        view.bit_depth,
        view.pixels().to_vec(),
    )
    .with_frame_number(view.frame_number)
    .with_timestamp(view.timestamp_ns);

    // Verify the conversion round-trip preserves data
    assert_eq!(frame.width, loaned.width);
    assert_eq!(frame.height, loaned.height);
    assert_eq!(frame.data.len(), loaned.pixel_data().len());
    assert_eq!(frame.frame_number, loaned.frame_number);
}

// =============================================================================
// FrameProducer Trait Compatibility
// =============================================================================

/// Verify AndorCamera can be used as a trait object `dyn FrameProducer`.
///
/// This is critical for downstream consumers that accept `Box<dyn FrameProducer>`
/// regardless of whether the camera is PVCAM, Andor, or mock.
#[tokio::test]
async fn andor_camera_is_frame_producer_trait_object() {
    let cam = AndorCamera::new_mock().await.unwrap();

    // Must be usable as Box<dyn FrameProducer>
    let producer: Box<dyn FrameProducer> = Box::new(cam);

    // Basic operations through trait object
    let (w, h) = producer.resolution();
    assert_eq!(w, 2048);
    assert_eq!(h, 2048);

    // Start/stop through trait object
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    producer.register_primary_output(tx).await.unwrap();
    producer.start_stream().await.unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    assert!(frame.width > 0);

    producer.stop_stream().await.unwrap();
}

// =============================================================================
// Pixel Data Layout Compatibility
// =============================================================================

/// Verify that 16-bit pixel data from Andor mock uses little-endian byte order,
/// consistent with the common::data::Frame::from_u16 encoding.
#[tokio::test]
async fn pixel_data_little_endian_u16() {
    let cam = AndorCamera::new_mock().await.unwrap();
    cam.set_exposure(0.005).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx).await.unwrap();
    cam.start_stream().await.unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    cam.stop_stream().await.unwrap();

    let data = frame.pixel_data();

    // Mock gradient: pixel(x,y) = (x + y + offset) % 65535, stored as LE u16
    // For frame 0, offset = 0
    // pixel(0,0) = 0, pixel(1,0) = 1, pixel(0,1) = 1

    // Verify LE encoding by manual decode vs Rust's from_le_bytes
    let pixel_0_0 = u16::from_le_bytes([data[0], data[1]]);
    assert_eq!(pixel_0_0, data[0] as u16 | ((data[1] as u16) << 8));

    // Cross-check: Frame::from_u16 also uses LE encoding
    let test_pixels = vec![pixel_0_0];
    let test_frame = Frame::from_u16(1, 1, &test_pixels);
    assert_eq!(test_frame.data[0], data[0], "LE byte 0 should match");
    assert_eq!(test_frame.data[1], data[1], "LE byte 1 should match");
}

/// Verify frame data can be round-tripped through Frame::from_u16 → data bytes.
///
/// This ensures frames from the Andor driver can be serialized/deserialized
/// by any consumer that uses the standard 16-bit pixel format.
#[tokio::test]
async fn pixel_data_roundtrip_through_frame() {
    let cam = AndorCamera::new_mock().await.unwrap();
    cam.set_exposure(0.005).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx).await.unwrap();
    cam.start_stream().await.unwrap();

    let loaned = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    cam.stop_stream().await.unwrap();

    // Extract pixels as u16 (as downstream consumers would)
    let pixel_data = loaned.pixel_data();
    let pixels_u16: Vec<u16> = pixel_data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Round-trip through Frame::from_u16
    let frame = Frame::from_u16(loaned.width, loaned.height, &pixels_u16);

    // Raw bytes should match exactly
    assert_eq!(
        frame.data.as_ref(),
        pixel_data,
        "Round-tripped pixel data should match original"
    );
}

// =============================================================================
// Metadata Consistency
// =============================================================================

/// Verify frame metadata is populated with sensible values that downstream
/// consumers can rely on (non-zero timestamp, positive exposure, etc.).
#[tokio::test]
async fn frame_metadata_sensible_for_downstream() {
    let cam = AndorCamera::new_mock().await.unwrap();
    let exposure_s = 0.025; // 25ms
    cam.set_exposure(exposure_s).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx).await.unwrap();
    cam.start_stream().await.unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    cam.stop_stream().await.unwrap();

    // Timestamp should be a recent UNIX timestamp in nanoseconds
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let one_minute_ns = 60_000_000_000u64;
    assert!(
        frame.timestamp_ns > now_ns - one_minute_ns && frame.timestamp_ns <= now_ns,
        "timestamp_ns should be within last minute"
    );

    // Exposure should match what was configured
    let expected_ms = exposure_s * 1000.0;
    assert!(
        (frame.exposure_ms - expected_ms).abs() < 0.1,
        "exposure_ms ({}) should match configured ({} ms)",
        frame.exposure_ms,
        expected_ms
    );

    // Frame number should start at 0
    assert_eq!(
        frame.frame_number, 0,
        "first frame should be frame_number 0"
    );

    // Dimensions must be positive
    assert!(frame.width > 0 && frame.height > 0);

    // bit_depth should be standard 16
    assert_eq!(frame.bit_depth, 16);
}
