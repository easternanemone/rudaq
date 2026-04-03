//! Frame streaming observer and per-client rate limiter.

use crate::grpc::proto::StreamQuality;
use common::capabilities::FrameObserver;
use common::data::FrameView;
use common::limits::MAX_STREAMS_PER_CLIENT;
use protocol::downsample::{downsample_2x2_into, downsample_4x4_into};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tonic::Status;

// =============================================================================
// Frame Observer for gRPC Streaming (bd-0dax.6.3)
// =============================================================================

/// Internal frame data packet sent through the observer channel.
///
/// Contains pre-processed frame data ready for gRPC transmission.
pub(super) struct ObserverFramePacket {
    pub(super) data: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) bit_depth: u32,
    pub(super) frame_number: u64,
    pub(super) timestamp_ns: u64,
    pub(super) exposure_ms: Option<f64>,
    pub(super) roi_x: u32,
    pub(super) roi_y: u32,
    pub(super) temperature_c: Option<f64>,
    pub(super) binning: Option<(u16, u16)>,
}

/// Observer that sends frames to gRPC stream (bd-0dax.6.3).
///
/// This observer receives `FrameView` references from the frame loop and
/// forwards them (after optional downsampling) to a gRPC client via an
/// mpsc channel.
///
/// # Contract
///
/// - `on_frame()` MUST NOT block - uses `try_send()` with bounded channel
/// - Frame data is copied during `on_frame()` (required - can't hold reference)
/// - Backpressure is handled by dropping frames when channel is full
///
/// # Quality Modes
///
/// - `Full`: No downsampling, full resolution frames
/// - `Preview`: 2x2 binning, ~75% bandwidth reduction
/// - `Fast`: 4x4 binning, ~94% bandwidth reduction
pub(super) struct GrpcStreamObserver {
    /// Channel sender for frame packets (bounded to handle backpressure)
    tx: tokio::sync::mpsc::Sender<ObserverFramePacket>,
    /// Quality setting for server-side downsampling
    quality: StreamQuality,
    /// Device ID for logging
    device_id: String,
    /// Frame counter for logging
    frames_received: AtomicU64,
    /// Frames dropped due to backpressure
    frames_dropped: AtomicU64,
    /// Reusable buffer for downsample/copy output, avoiding per-frame allocation churn.
    /// Wrapped in `Mutex` because `on_frame` takes `&self` (interior mutability needed).
    /// Contention is negligible: only one frame-loop thread calls `on_frame` per observer.
    frame_buffer: Mutex<Vec<u8>>,
}

impl GrpcStreamObserver {
    /// Create a new gRPC stream observer.
    pub(super) fn new(
        tx: tokio::sync::mpsc::Sender<ObserverFramePacket>,
        quality: StreamQuality,
        device_id: String,
    ) -> Self {
        Self {
            tx,
            quality,
            device_id,
            frames_received: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            frame_buffer: Mutex::new(Vec::new()),
        }
    }
}

impl FrameObserver for GrpcStreamObserver {
    fn on_frame(&self, frame: &FrameView<'_>) {
        let frame_count = self.frames_received.fetch_add(1, Ordering::Relaxed);

        // Log early frames for debugging
        if frame_count < 10 {
            tracing::debug!(
                device_id = %self.device_id,
                frame_number = frame.frame_number,
                width = frame.width,
                height = frame.height,
                quality = ?self.quality,
                "GrpcStreamObserver received frame (early frame debug)"
            );
        }

        // Apply server-side downsampling based on quality setting.
        // Uses the buffer-reuse _into variants to avoid per-frame allocation churn.
        // The Mutex is uncontended (single frame-loop thread per observer).
        let mut buf = self.frame_buffer.lock().unwrap_or_else(|poisoned| {
            tracing::error!("GrpcStreamObserver frame_buffer mutex poisoned, recovering");
            poisoned.into_inner()
        });

        let (effective_width, effective_height) = match self.quality {
            StreamQuality::Preview => {
                downsample_2x2_into(frame.pixels(), frame.width, frame.height, &mut buf)
            }
            StreamQuality::Fast => {
                downsample_4x4_into(frame.pixels(), frame.width, frame.height, &mut buf)
            }
            StreamQuality::Full => {
                buf.clear();
                buf.extend_from_slice(frame.pixels());
                (frame.width, frame.height)
            }
        };

        // Clone the buffer contents for the packet. The buffer's *capacity*
        // persists across frames (clear + extend/reserve reuse it), eliminating
        // the alloc/dealloc churn of the intermediate downsample buffer.
        let frame_data = buf.clone();
        drop(buf); // Release lock before channel send

        let packet = ObserverFramePacket {
            data: frame_data,
            width: effective_width,
            height: effective_height,
            bit_depth: frame.bit_depth,
            frame_number: frame.frame_number,
            timestamp_ns: frame.timestamp_ns,
            exposure_ms: frame.exposure_ms,
            roi_x: frame.roi_x,
            roi_y: frame.roi_y,
            temperature_c: frame.temperature_c,
            binning: frame.binning,
        };

        // Non-blocking send - drop frame if channel is full (backpressure)
        if self.tx.try_send(packet).is_err() {
            let dropped = self.frames_dropped.fetch_add(1, Ordering::Relaxed);
            if dropped.is_multiple_of(10) {
                tracing::debug!(
                    device_id = %self.device_id,
                    frames_dropped = dropped + 1,
                    "GrpcStreamObserver dropping frame due to backpressure"
                );
            }
        }
    }

    fn name(&self) -> &'static str {
        "grpc_stream_observer"
    }
}

// =============================================================================
// Per-Client Stream Rate Limiter (bd-64hu)
// =============================================================================

/// Tracks active frame streams per client IP for DoS prevention.
///
/// Each client IP is limited to `MAX_STREAMS_PER_CLIENT` concurrent frame streams.
/// Returns `ResourceExhausted` when the limit is exceeded.
#[derive(Debug, Default)]
pub struct StreamLimiter {
    /// Map of client IP to active stream count
    active_streams: std::sync::Mutex<HashMap<IpAddr, usize>>,
}

/// RAII guard that releases a previously acquired stream slot on drop.
#[must_use = "Dropping the guard releases the stream slot"]
pub struct StreamSlotGuard {
    limiter: Arc<StreamLimiter>,
    client_ip: IpAddr,
}

impl Drop for StreamSlotGuard {
    fn drop(&mut self) {
        self.limiter.release(self.client_ip);
    }
}

impl StreamLimiter {
    /// Create a new stream limiter.
    pub fn new() -> Self {
        Self {
            active_streams: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Try to acquire a stream slot for the given client IP.
    ///
    /// Returns `Ok(())` if the client is under the limit, or `Err(Status)` if exceeded.
    #[allow(clippy::result_large_err)] // tonic::Status (176 bytes) is the standard gRPC error type
    pub fn try_acquire(&self, client_ip: IpAddr) -> Result<(), Status> {
        let mut streams = self.active_streams.lock().map_err(|_| {
            tracing::error!("StreamLimiter mutex poisoned in try_acquire");
            Status::internal("Stream limiter internal error")
        })?;
        let count = streams.entry(client_ip).or_insert(0);

        if *count >= MAX_STREAMS_PER_CLIENT {
            tracing::warn!(
                client_ip = %client_ip,
                active_streams = *count,
                max_allowed = MAX_STREAMS_PER_CLIENT,
                "Client exceeded maximum concurrent streams"
            );
            return Err(Status::resource_exhausted(format!(
                "Maximum concurrent streams ({}) exceeded for client {}",
                MAX_STREAMS_PER_CLIENT, client_ip
            )));
        }

        *count += 1;
        tracing::debug!(
            client_ip = %client_ip,
            active_streams = *count,
            "Acquired stream slot"
        );
        Ok(())
    }

    /// Try to acquire a stream slot and return an RAII guard that releases it on drop.
    #[allow(clippy::result_large_err)] // tonic::Status (176 bytes) is the standard gRPC error type
    pub fn try_acquire_guard(
        self: &Arc<Self>,
        client_ip: IpAddr,
    ) -> Result<StreamSlotGuard, Status> {
        self.try_acquire(client_ip)?;
        Ok(StreamSlotGuard {
            limiter: Arc::clone(self),
            client_ip,
        })
    }

    /// Release a stream slot for the given client IP.
    pub fn release(&self, client_ip: IpAddr) {
        let mut streams = match self.active_streams.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::error!("StreamLimiter mutex poisoned in release, recovering");
                poisoned.into_inner()
            }
        };

        // Use Entry API for single-lookup access (avoids double borrow)
        if let Entry::Occupied(mut entry) = streams.entry(client_ip) {
            let count = entry.get_mut();
            *count = count.saturating_sub(1);
            tracing::debug!(
                client_ip = %client_ip,
                active_streams = *count,
                "Released stream slot"
            );
            if *count == 0 {
                entry.remove();
            }
        }
    }

    /// Check if client has any tracked streams (test helper)
    #[cfg(test)]
    pub(super) fn has_streams(&self, client_ip: IpAddr) -> bool {
        self.active_streams
            .lock()
            .map(|streams| streams.contains_key(&client_ip))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::StreamLimiter;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    #[test]
    fn stream_slot_guard_releases_on_drop() {
        let limiter = Arc::new(StreamLimiter::new());
        let client_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);

        {
            let _guard = limiter.try_acquire_guard(client_ip).expect("acquire guard");
            assert!(limiter.has_streams(client_ip));
        }

        assert!(!limiter.has_streams(client_ip));
    }

    #[test]
    fn stream_slot_guard_releases_on_early_return_path() {
        fn early_return(limiter: Arc<StreamLimiter>, client_ip: IpAddr) {
            let _guard = limiter.try_acquire_guard(client_ip).expect("acquire guard");
        }

        let limiter = Arc::new(StreamLimiter::new());
        let client_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        early_return(Arc::clone(&limiter), client_ip);
        assert!(!limiter.has_streams(client_ip));
    }
}
