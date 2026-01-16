//! Frame tap system for single-owner + tap architecture (bd-0dax.4).
//!
//! This module provides a non-blocking observer pattern for frame processing:
//! - Primary consumer owns the frame via mpsc channel (storage/network)
//! - Taps borrow frames synchronously before primary send
//! - Taps use try_send/try_write - drop if slow, never block
//!
//! # Design Principles
//!
//! 1. **Primary consumer owns the frame** - Gets `LoanedFrame` via mpsc
//! 2. **Taps borrow synchronously** - Process `&FrameData` before primary send
//! 3. **Strict non-blocking** - Taps use `try_send`/`try_write`, drop if slow
//! 4. **Taps MUST copy** - To persist data, taps copy to their own buffer
//!
//! # Architecture
//!
//! ```text
//! Frame Loop
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────┐
//! │  tap_registry.apply(&frame)  [SYNCHRONOUS]  │
//! │    ├─ DecimatedTap → try_send(copy)         │
//! │    ├─ SnapshotTap → try_write(copy)         │
//! │    └─ MetricsTap → update counters          │
//! └─────────────────────────────────────────────┘
//!     │
//!     ▼
//! primary_tx.send(frame).await  [OWNERSHIP TRANSFER]
//!     │
//!     ▼
//! Pool slot returns when primary drops
//! ```
//!
//! # Example
//!
//! ```ignore
//! use daq_driver_pvcam::components::taps::{TapRegistry, DecimatedTap};
//!
//! // Create registry
//! let registry = Arc::new(TapRegistry::new());
//!
//! // Register a decimated tap for GUI preview
//! let (tx, rx) = tokio::sync::mpsc::channel(4);
//! let tap = DecimatedTap::new(10, tx); // Send every 10th frame
//! let handle = registry.register(Box::new(tap));
//!
//! // In frame loop:
//! registry.apply(&frame_data);
//! primary_tx.send(loaned_frame).await?;
//!
//! // When done:
//! registry.unregister(handle);
//! ```

use daq_core::capabilities::FrameObserver;
use daq_core::data::Frame;
use daq_pool::FrameData;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// ============================================================================
// FrameTap Trait
// ============================================================================

/// Trait for frame observers that process frames without blocking.
///
/// # Contract
///
/// - `inspect()` and `inspect_frame()` MUST NOT block
/// - These methods MUST complete quickly (< 1ms)
/// - To persist data, implementations MUST copy to their own buffer
/// - Implementations MUST handle backpressure internally (drop if slow)
///
/// # Safety
///
/// The references are only valid for the duration of the call.
/// Implementations must not store the reference or attempt to extend its lifetime.
///
/// # Deadlock Warning
///
/// **NEVER call `TapRegistry::unregister()` from within `inspect()`!**
///
/// The registry holds a read lock while calling `inspect()` on all taps.
/// Calling `unregister()` from within a tap's `inspect()` method will attempt
/// to acquire a write lock, causing a deadlock.
pub trait FrameTap: Send + Sync {
    /// Inspect a frame without blocking (for FrameData from pool).
    ///
    /// This is called synchronously from the frame loop for every frame.
    /// The implementation MUST NOT block and MUST complete quickly.
    ///
    /// # Arguments
    ///
    /// - `frame`: Reference to the frame data (valid only for this call)
    fn inspect(&self, frame: &FrameData);

    /// Inspect a Frame (daq_core::data::Frame) without blocking.
    ///
    /// This is a compatibility method for frame loops that still use the
    /// Frame type instead of FrameData. Default implementation creates a
    /// FrameSnapshot from the Frame.
    ///
    /// # Arguments
    ///
    /// - `frame`: Reference to the daq_core Frame (valid only for this call)
    fn inspect_frame(&self, frame: &Frame) {
        // Default: create FrameSnapshot which provides FrameData-like access
        let snapshot = FrameSnapshot::from_frame(frame);
        // Create a temporary FrameData for the inspect call
        // This is not ideal but maintains compatibility
        let frame_data = FrameData {
            pixels: snapshot.pixels.clone(),
            actual_len: snapshot.pixels.len(),
            frame_number: snapshot.frame_number,
            hw_frame_nr: -1,
            width: snapshot.width,
            height: snapshot.height,
            bit_depth: snapshot.bit_depth,
            timestamp_ns: snapshot.timestamp_ns,
            exposure_ms: snapshot.exposure_ms,
            roi_x: snapshot.roi_x,
            roi_y: snapshot.roi_y,
            temperature_c: None,
            binning: None,
        };
        self.inspect(&frame_data);
    }

    /// Optional: Return a descriptive name for this tap (for debugging).
    fn name(&self) -> &str {
        "unnamed_tap"
    }
}

// ============================================================================
// TapHandle
// ============================================================================

/// Handle returned when registering a tap, used for unregistration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TapHandle(u64);

impl TapHandle {
    /// Get the internal ID (for debugging or conversion).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.0
    }

    /// Create a TapHandle from an ID.
    ///
    /// This is used when converting from generic ObserverHandle back to TapHandle.
    #[must_use]
    pub fn from_id(id: u64) -> Self {
        Self(id)
    }
}

// ============================================================================
// TapRegistry
// ============================================================================

/// Registry for frame tap consumers.
///
/// Thread-safe registry that allows dynamic registration and unregistration
/// of taps. The `apply()` method iterates through all registered taps and
/// calls their `inspect()` method.
///
/// # Performance
///
/// - Registration/unregistration: Takes write lock (rare operation)
/// - apply(): Takes read lock (frequent operation, optimized path)
///
/// Uses `parking_lot::RwLock` for better performance than `std::sync::RwLock`.
pub struct TapRegistry {
    /// Registered taps with their handles.
    taps: RwLock<Vec<(u64, Box<dyn FrameTap>)>>,
    /// Counter for generating unique tap IDs.
    next_id: AtomicU64,
    /// Total frames processed (for metrics).
    frames_processed: AtomicU64,
}

impl TapRegistry {
    /// Create a new empty tap registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            taps: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
            frames_processed: AtomicU64::new(0),
        }
    }

    /// Register a new tap and return a handle for unregistration.
    ///
    /// # Arguments
    ///
    /// - `tap`: The tap implementation to register
    ///
    /// # Returns
    ///
    /// A `TapHandle` that can be used to unregister the tap later.
    pub fn register(&self, tap: Box<dyn FrameTap>) -> TapHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let name = tap.name();

        tracing::debug!(tap_id = id, tap_name = %name, "Registering tap");

        let mut taps = self.taps.write();
        taps.push((id, tap));

        TapHandle(id)
    }

    /// Unregister a tap by its handle.
    ///
    /// # Arguments
    ///
    /// - `handle`: The handle returned from `register()`
    ///
    /// # Returns
    ///
    /// `true` if the tap was found and removed, `false` otherwise.
    pub fn unregister(&self, handle: TapHandle) -> bool {
        let mut taps = self.taps.write();
        let initial_len = taps.len();
        taps.retain(|(id, _)| *id != handle.0);
        let removed = taps.len() < initial_len;

        if removed {
            tracing::debug!(tap_id = handle.0, "Unregistered tap");
        } else {
            tracing::warn!(tap_id = handle.0, "Tap not found for unregistration");
        }

        removed
    }

    /// Apply all registered taps to a frame.
    ///
    /// Calls `inspect()` on each registered tap in registration order.
    /// This method is designed to be called from the frame loop and
    /// takes a read lock (allowing concurrent reads).
    ///
    /// # Arguments
    ///
    /// - `frame`: Reference to the frame data to inspect
    #[inline]
    pub fn apply(&self, frame: &FrameData) {
        self.frames_processed.fetch_add(1, Ordering::Relaxed);

        let taps = self.taps.read();
        for (_, tap) in taps.iter() {
            tap.inspect(frame);
        }
    }

    /// Apply all registered taps to a Frame (daq_core::data::Frame).
    ///
    /// This is a compatibility method that converts the Frame to a temporary
    /// FrameData view for tap inspection. This is useful during the transition
    /// period where the frame loop still uses Frame instead of FrameData.
    ///
    /// # Arguments
    ///
    /// - `frame`: Reference to the daq_core Frame
    ///
    /// # Performance Note
    ///
    /// This method creates a temporary FrameData on the stack without copying
    /// pixel data - it creates a view into the Frame's data. The FrameData's
    /// `pixels` Vec is empty; use `pixel_data()` which will return empty slice.
    /// Taps that need pixel data should use `apply_frame_with_pixels()` instead
    /// or work directly with the Frame.
    #[inline]
    pub fn apply_frame(&self, frame: &Frame) {
        self.frames_processed.fetch_add(1, Ordering::Relaxed);

        // For efficiency, we create a minimal FrameData with just metadata
        // Taps that need pixel data can access it through FrameSnapshot
        let frame_data = FrameData {
            pixels: Vec::new(), // Empty - taps should not rely on this
            actual_len: 0,
            frame_number: frame.frame_number,
            hw_frame_nr: -1,
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            timestamp_ns: frame.timestamp_ns,
            exposure_ms: frame.exposure_ms.unwrap_or(0.0),
            roi_x: frame.roi_x,
            roi_y: frame.roi_y,
            temperature_c: frame.metadata.as_ref().and_then(|m| m.temperature_c),
            binning: frame.metadata.as_ref().and_then(|m| m.binning),
        };

        let taps = self.taps.read();
        for (_, tap) in taps.iter() {
            tap.inspect(&frame_data);
        }
    }

    /// Apply all registered taps to a Frame with full pixel data access.
    ///
    /// This method provides taps with access to the actual pixel data from
    /// the Frame. More expensive than `apply_frame()` as it creates a FrameData
    /// that references the pixel slice.
    ///
    /// # Arguments
    ///
    /// - `frame`: Reference to the daq_core Frame
    #[inline]
    pub fn apply_frame_with_pixels(&self, frame: &Frame) {
        self.frames_processed.fetch_add(1, Ordering::Relaxed);

        // Create FrameData with a copy of pixel data reference info
        // Note: We can't directly use frame.data as pixels Vec, so taps
        // will use FrameSnapshot which copies the data anyway
        let taps = self.taps.read();
        for (_, tap) in taps.iter() {
            tap.inspect_frame(frame);
        }
    }

    /// Get the number of registered taps.
    #[must_use]
    pub fn tap_count(&self) -> usize {
        self.taps.read().len()
    }

    /// Get the total number of frames processed.
    #[must_use]
    pub fn frames_processed(&self) -> u64 {
        self.frames_processed.load(Ordering::Relaxed)
    }

    /// Check if there are any registered taps.
    #[must_use]
    pub fn has_taps(&self) -> bool {
        !self.taps.read().is_empty()
    }
}

impl Default for TapRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// FrameSnapshot - Lightweight frame copy for SnapshotTap
// ============================================================================

/// Lightweight copy of frame data for snapshot access.
///
/// Contains a copy of the pixel data and essential metadata.
/// Used by `SnapshotTap` to provide access to the latest frame.
#[derive(Debug, Clone)]
pub struct FrameSnapshot {
    /// Copied pixel data.
    pub pixels: Vec<u8>,
    /// Frame dimensions.
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    /// Frame identity.
    pub frame_number: u64,
    pub timestamp_ns: u64,
    /// Acquisition parameters.
    pub exposure_ms: f64,
    pub roi_x: u32,
    pub roi_y: u32,
}

impl FrameSnapshot {
    /// Create a snapshot from FrameData by copying the pixel data.
    #[must_use]
    pub fn from_frame_data(frame: &FrameData) -> Self {
        Self {
            pixels: frame.pixel_data().to_vec(),
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            frame_number: frame.frame_number,
            timestamp_ns: frame.timestamp_ns,
            exposure_ms: frame.exposure_ms,
            roi_x: frame.roi_x,
            roi_y: frame.roi_y,
        }
    }

    /// Create a snapshot from a daq_core Frame by copying the pixel data.
    ///
    /// This is a compatibility method for code that uses the Frame type
    /// instead of FrameData.
    #[must_use]
    pub fn from_frame(frame: &Frame) -> Self {
        Self {
            pixels: frame.data.to_vec(),
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            frame_number: frame.frame_number,
            timestamp_ns: frame.timestamp_ns,
            exposure_ms: frame.exposure_ms.unwrap_or(0.0),
            roi_x: frame.roi_x,
            roi_y: frame.roi_y,
        }
    }

    /// Get the pixel data as a slice.
    #[inline]
    #[must_use]
    pub fn pixel_data(&self) -> &[u8] {
        &self.pixels
    }
}

// ============================================================================
// DecimatedTap - Send every Nth frame to a channel
// ============================================================================

/// Tap that sends a copy of every Nth frame to a channel.
///
/// Useful for GUI preview where full frame rate isn't needed.
/// Uses `try_send` to avoid blocking - drops frames if channel is full.
///
/// # Example
///
/// ```ignore
/// let (tx, mut rx) = tokio::sync::mpsc::channel(4);
/// let tap = DecimatedTap::new(10, tx); // Send every 10th frame
/// registry.register(Box::new(tap));
///
/// // Receive decimated frames
/// while let Some(snapshot) = rx.recv().await {
///     display_preview(snapshot);
/// }
/// ```
pub struct DecimatedTap {
    /// Send every `interval`th frame (1 = every frame).
    interval: u64,
    /// Frame counter (incremented for each frame seen).
    count: AtomicU64,
    /// Channel to send frame snapshots.
    tx: tokio::sync::mpsc::Sender<FrameSnapshot>,
    /// Number of frames successfully sent.
    sent: AtomicU64,
    /// Number of frames dropped due to channel full.
    dropped: AtomicU64,
}

impl DecimatedTap {
    /// Create a new decimated tap.
    ///
    /// # Arguments
    ///
    /// - `interval`: Send every Nth frame (1 = every frame, 10 = every 10th)
    /// - `tx`: Channel sender for frame snapshots
    ///
    /// # Panics
    ///
    /// Panics if `interval` is 0.
    #[must_use]
    pub fn new(interval: u64, tx: tokio::sync::mpsc::Sender<FrameSnapshot>) -> Self {
        assert!(interval > 0, "interval must be > 0");
        Self {
            interval,
            count: AtomicU64::new(0),
            tx,
            sent: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Get the number of frames successfully sent.
    #[must_use]
    pub fn sent_count(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }

    /// Get the number of frames dropped due to channel full.
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl FrameTap for DecimatedTap {
    fn inspect(&self, frame: &FrameData) {
        let count = self.count.fetch_add(1, Ordering::Relaxed);

        // Only process frames at the specified interval
        if !count.is_multiple_of(self.interval) {
            return;
        }

        // Check if channel has capacity before copying (optimization)
        if self.tx.capacity() == 0 {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // Copy frame data and try to send
        let snapshot = FrameSnapshot::from_frame_data(frame);
        match self.tx.try_send(snapshot) {
            Ok(()) => {
                self.sent.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn name(&self) -> &str {
        "DecimatedTap"
    }
}

// ============================================================================
// SnapshotTap - Maintain latest frame for on-demand access
// ============================================================================

/// Tap that maintains the latest frame for on-demand access.
///
/// Uses `try_write` to avoid blocking - skips update if reader holds lock.
/// Useful for gRPC endpoints that need the current frame on request.
///
/// # Example
///
/// ```ignore
/// let tap = SnapshotTap::new();
/// let handle = tap.handle();
/// registry.register(Box::new(tap));
///
/// // Access latest frame from another thread
/// if let Some(snapshot) = handle.get() {
///     process_latest(snapshot);
/// }
/// ```
pub struct SnapshotTap {
    /// Latest frame snapshot (protected by RwLock).
    latest: Arc<RwLock<Option<FrameSnapshot>>>,
    /// Number of successful updates.
    updates: AtomicU64,
    /// Number of skipped updates (reader held lock).
    skipped: AtomicU64,
}

impl SnapshotTap {
    /// Create a new snapshot tap.
    #[must_use]
    pub fn new() -> Self {
        Self {
            latest: Arc::new(RwLock::new(None)),
            updates: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }

    /// Get a handle for reading the latest snapshot.
    ///
    /// The handle can be cloned and shared across threads.
    #[must_use]
    pub fn handle(&self) -> SnapshotHandle {
        SnapshotHandle {
            latest: Arc::clone(&self.latest),
        }
    }

    /// Get the number of successful updates.
    #[must_use]
    pub fn update_count(&self) -> u64 {
        self.updates.load(Ordering::Relaxed)
    }

    /// Get the number of skipped updates.
    #[must_use]
    pub fn skipped_count(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }
}

impl Default for SnapshotTap {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameTap for SnapshotTap {
    fn inspect(&self, frame: &FrameData) {
        // Use try_write to avoid blocking - skip if reader holds lock
        if let Some(mut guard) = self.latest.try_write() {
            *guard = Some(FrameSnapshot::from_frame_data(frame));
            self.updates.fetch_add(1, Ordering::Relaxed);
        } else {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn name(&self) -> &str {
        "SnapshotTap"
    }
}

/// Handle for reading the latest frame snapshot.
///
/// Can be cloned and shared across threads.
#[derive(Clone)]
pub struct SnapshotHandle {
    latest: Arc<RwLock<Option<FrameSnapshot>>>,
}

impl SnapshotHandle {
    /// Get a clone of the latest frame snapshot.
    ///
    /// Returns `None` if no frame has been captured yet.
    #[must_use]
    pub fn get(&self) -> Option<FrameSnapshot> {
        self.latest.read().clone()
    }

    /// Check if a snapshot is available.
    #[must_use]
    pub fn has_snapshot(&self) -> bool {
        self.latest.read().is_some()
    }
}

// ============================================================================
// MetricsTap - Collect frame statistics without copying
// ============================================================================

/// Tap that collects frame statistics without copying pixel data.
///
/// Useful for monitoring frame rate, frame gaps, and timing statistics.
pub struct MetricsTap {
    /// Total frames seen.
    frame_count: AtomicU64,
    /// Last frame number seen (for gap detection).
    last_frame_nr: AtomicU64,
    /// Total frame gaps detected.
    gap_count: AtomicU64,
    /// Total frames lost (sum of all gaps).
    lost_frames: AtomicU64,
}

impl MetricsTap {
    /// Create a new metrics tap.
    #[must_use]
    pub fn new() -> Self {
        Self {
            frame_count: AtomicU64::new(0),
            last_frame_nr: AtomicU64::new(0),
            gap_count: AtomicU64::new(0),
            lost_frames: AtomicU64::new(0),
        }
    }

    /// Get the total number of frames seen.
    #[must_use]
    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    /// Get the number of gap events detected.
    #[must_use]
    pub fn gap_count(&self) -> u64 {
        self.gap_count.load(Ordering::Relaxed)
    }

    /// Get the total number of frames lost.
    #[must_use]
    pub fn lost_frames(&self) -> u64 {
        self.lost_frames.load(Ordering::Relaxed)
    }

    /// Reset all counters.
    pub fn reset(&self) {
        self.frame_count.store(0, Ordering::Relaxed);
        self.last_frame_nr.store(0, Ordering::Relaxed);
        self.gap_count.store(0, Ordering::Relaxed);
        self.lost_frames.store(0, Ordering::Relaxed);
    }
}

impl Default for MetricsTap {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameTap for MetricsTap {
    fn inspect(&self, frame: &FrameData) {
        let count = self.frame_count.fetch_add(1, Ordering::Relaxed);

        // Gap detection (skip first frame)
        if count > 0 {
            let last_nr = self.last_frame_nr.load(Ordering::Relaxed);
            let current_nr = frame.frame_number;

            if current_nr > last_nr + 1 {
                let gap = current_nr - last_nr - 1;
                self.gap_count.fetch_add(1, Ordering::Relaxed);
                self.lost_frames.fetch_add(gap, Ordering::Relaxed);

                tracing::warn!(
                    last_frame = last_nr,
                    current_frame = current_nr,
                    gap,
                    "Frame gap detected"
                );
            }
        }

        self.last_frame_nr
            .store(frame.frame_number, Ordering::Relaxed);
    }

    fn name(&self) -> &str {
        "MetricsTap"
    }
}

// ============================================================================
// ObserverAdapter - Bridge from generic FrameObserver to internal FrameTap
// ============================================================================

/// Adapter that bridges from `daq_core::capabilities::FrameObserver` to internal `FrameTap`.
///
/// This allows external crates (like daq-server) to register generic frame observers
/// without depending on the internal tap system. The adapter converts each `inspect()`
/// call into an `on_frame()` call on the wrapped observer.
///
/// # Example
///
/// ```ignore
/// use daq_core::capabilities::FrameObserver;
/// use daq_core::data::FrameView;
///
/// struct MyObserver;
///
/// impl FrameObserver for MyObserver {
///     fn on_frame(&self, frame: &FrameView<'_>) {
///         println!("Got frame {}", frame.frame_number);
///     }
/// }
///
/// // In driver code:
/// let observer = Box::new(MyObserver);
/// let adapter = ObserverAdapter::new(observer);
/// let handle = tap_registry.register(Box::new(adapter));
/// ```
pub struct ObserverAdapter {
    /// The wrapped generic observer.
    observer: Box<dyn FrameObserver>,
}

impl ObserverAdapter {
    /// Create a new adapter wrapping a generic FrameObserver.
    #[must_use]
    pub fn new(observer: Box<dyn FrameObserver>) -> Self {
        Self { observer }
    }

    /// Get the observer's name.
    #[must_use]
    pub fn observer_name(&self) -> &str {
        self.observer.name()
    }
}

impl FrameTap for ObserverAdapter {
    fn inspect(&self, frame: &FrameData) {
        // Create zero-copy FrameView from FrameData (bd-gtvu optimization)
        // This borrows pixel data directly - no allocation!
        let mut view = daq_core::data::FrameView::new(
            frame.width,
            frame.height,
            frame.bit_depth,
            frame.pixel_data(),
            frame.frame_number,
            frame.timestamp_ns,
        )
        .with_exposure(frame.exposure_ms)
        .with_roi_offset(frame.roi_x, frame.roi_y);

        // Add optional metadata if present
        if let Some(temp) = frame.temperature_c {
            view = view.with_temperature(temp);
        }
        if let Some(binning) = frame.binning {
            view = view.with_binning(binning);
        }

        self.observer.on_frame(&view);
    }

    fn inspect_frame(&self, frame: &Frame) {
        // Convert Frame to FrameView (zero-copy borrow)
        let view = daq_core::data::FrameView::from_frame(frame);
        self.observer.on_frame(&view);
    }

    fn name(&self) -> &str {
        self.observer.name()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_frame(frame_number: u64) -> FrameData {
        let mut frame = FrameData::with_capacity(1024);
        frame.frame_number = frame_number;
        frame.width = 32;
        frame.height = 32;
        frame.bit_depth = 8;
        frame.actual_len = 1024;
        frame
    }

    #[test]
    fn test_tap_registry_register_unregister() {
        let registry = TapRegistry::new();
        assert_eq!(registry.tap_count(), 0);

        let tap = MetricsTap::new();
        let handle = registry.register(Box::new(tap));

        assert_eq!(registry.tap_count(), 1);

        let removed = registry.unregister(handle);
        assert!(removed);
        assert_eq!(registry.tap_count(), 0);

        // Unregistering again should return false
        let removed_again = registry.unregister(handle);
        assert!(!removed_again);
    }

    #[test]
    fn test_tap_registry_apply() {
        let registry = TapRegistry::new();
        let tap = MetricsTap::new();
        let tap_ref = &tap as *const _;

        registry.register(Box::new(tap));

        let frame = make_test_frame(1);
        registry.apply(&frame);

        assert_eq!(registry.frames_processed(), 1);
    }

    #[test]
    fn test_metrics_tap() {
        let tap = MetricsTap::new();

        // First frame
        let frame1 = make_test_frame(1);
        tap.inspect(&frame1);
        assert_eq!(tap.frame_count(), 1);
        assert_eq!(tap.gap_count(), 0);

        // Consecutive frame (no gap)
        let frame2 = make_test_frame(2);
        tap.inspect(&frame2);
        assert_eq!(tap.frame_count(), 2);
        assert_eq!(tap.gap_count(), 0);

        // Gap of 2 frames (3 -> 6)
        let frame3 = make_test_frame(5);
        tap.inspect(&frame3);
        assert_eq!(tap.frame_count(), 3);
        assert_eq!(tap.gap_count(), 1);
        assert_eq!(tap.lost_frames(), 2); // frames 3 and 4 lost
    }

    #[tokio::test]
    async fn test_decimated_tap() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let tap = DecimatedTap::new(3, tx); // Every 3rd frame

        // Send 10 frames
        for i in 0..10 {
            let frame = make_test_frame(i);
            tap.inspect(&frame);
        }

        // Should have sent frames 0, 3, 6, 9 = 4 frames
        assert_eq!(tap.sent_count(), 4);

        // Verify received frames
        let mut received = Vec::new();
        while let Ok(snapshot) = rx.try_recv() {
            received.push(snapshot.frame_number);
        }
        assert_eq!(received, vec![0, 3, 6, 9]);
    }

    #[tokio::test]
    async fn test_decimated_tap_backpressure() {
        let (tx, _rx) = tokio::sync::mpsc::channel(2); // Small buffer
        let tap = DecimatedTap::new(1, tx); // Every frame

        // Send more frames than buffer can hold
        for i in 0..10 {
            let frame = make_test_frame(i);
            tap.inspect(&frame);
        }

        // Some frames should be dropped
        assert!(tap.dropped_count() > 0);
        assert!(tap.sent_count() + tap.dropped_count() == 10);
    }

    #[test]
    fn test_snapshot_tap() {
        let tap = SnapshotTap::new();
        let handle = tap.handle();

        // No snapshot initially
        assert!(!handle.has_snapshot());
        assert!(handle.get().is_none());

        // Capture a frame
        let frame = make_test_frame(42);
        tap.inspect(&frame);

        // Snapshot should be available
        assert!(handle.has_snapshot());
        let snapshot = handle.get().unwrap();
        assert_eq!(snapshot.frame_number, 42);
        assert_eq!(tap.update_count(), 1);
    }

    #[test]
    fn test_frame_snapshot_from_frame_data() {
        let mut frame = FrameData::with_capacity(100);
        frame.frame_number = 123;
        frame.width = 10;
        frame.height = 10;
        frame.bit_depth = 8;
        frame.timestamp_ns = 987654321;
        frame.exposure_ms = 10.5;
        frame.roi_x = 5;
        frame.roi_y = 6;
        frame.actual_len = 100;

        // Fill with test data
        for (i, byte) in frame.pixels.iter_mut().enumerate().take(100) {
            *byte = i as u8;
        }

        let snapshot = FrameSnapshot::from_frame_data(&frame);

        assert_eq!(snapshot.frame_number, 123);
        assert_eq!(snapshot.width, 10);
        assert_eq!(snapshot.height, 10);
        assert_eq!(snapshot.bit_depth, 8);
        assert_eq!(snapshot.timestamp_ns, 987654321);
        assert_eq!(snapshot.exposure_ms, 10.5);
        assert_eq!(snapshot.roi_x, 5);
        assert_eq!(snapshot.roi_y, 6);
        assert_eq!(snapshot.pixels.len(), 100);
        assert_eq!(snapshot.pixels[0], 0);
        assert_eq!(snapshot.pixels[99], 99);
    }
}
