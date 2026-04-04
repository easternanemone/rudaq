//! Output channel registration and buffer management for PVCAM acquisition.

use super::PvcamAcquisition;
use super::PvcamConnection;
#[cfg(feature = "pvcam_sdk")]
use super::get_pvcam_error;
use anyhow::Result;
#[cfg(feature = "pvcam_sdk")]
use anyhow::anyhow;
#[cfg(feature = "pvcam_sdk")]
use pvcam_sys::*;

impl PvcamAcquisition {
    /// Register the primary output channel for zero-allocation frame delivery (bd-0dax.5).
    ///
    /// Only ONE primary consumer is allowed - subsequent calls replace the previous consumer.
    /// Call BEFORE `start_stream()` to ensure frames are delivered from the start.
    ///
    /// # Arguments
    /// * `tx` - Channel sender that will receive `LoanedFrame` ownership
    pub async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<common::capabilities::LoanedFrame>,
    ) -> anyhow::Result<()> {
        let mut primary = self.primary_tx.lock().await;
        *primary = Some(tx);
        tracing::debug!(target: "pvcam", "Primary output channel registered");
        Ok(())
    }

    /// Calculate optimal circular buffer frame count (bd-ek9n.4)
    ///
    /// Uses PARAM_FRAME_BUFFER_SIZE when available, with heuristic fallback:
    /// - Minimum 32 frames for reliability
    /// - At least 1 second of buffer at current frame rate
    /// - Capped at 255 frames (matches PVCAM example defaults)
    ///
    /// # Arguments
    ///
    /// * `hcam` - Open camera handle
    /// * `frame_bytes` - Size of one frame in bytes
    /// * `exposure_ms` - Exposure time in milliseconds (for frame rate calculation)
    #[cfg(feature = "pvcam_sdk")]
    pub(super) fn calculate_buffer_count(hcam: i16, frame_bytes: usize, exposure_ms: f64) -> usize {
        // PVCAM examples default to 255-frame circular buffers for full-frame streaming.
        // We align with that default but still clamp to a sane upper bound to avoid
        // excessive host memory use on large frames.
        const MIN_BUFFER_FRAMES: usize = 32;
        const MAX_BUFFER_FRAMES: usize = 255;
        const ONE_SECOND_MS: f64 = 1000.0;

        // Try to query PARAM_FRAME_BUFFER_SIZE from SDK
        // This returns recommended buffer size in bytes for current acquisition settings
        // SAFETY: `hcam` is a valid camera handle. All output parameters
        // (`avail`, `recommended_bytes`) are stack-allocated with correct
        // types. These are read-only parameter queries with no side effects.
        let sdk_recommended = unsafe {
            let mut avail: rs_bool = 0;
            // Check if parameter is available
            if pl_get_param(
                hcam,
                PARAM_FRAME_BUFFER_SIZE,
                ATTR_AVAIL as i16,
                &mut avail as *mut _ as *mut _,
            ) != 0
                && avail != 0
            {
                // Get the default (recommended) value
                let mut recommended_bytes: u64 = 0;
                if pl_get_param(
                    hcam,
                    PARAM_FRAME_BUFFER_SIZE,
                    ATTR_DEFAULT as i16,
                    &mut recommended_bytes as *mut _ as *mut _,
                ) != 0
                {
                    Some(recommended_bytes as usize)
                } else {
                    tracing::debug!("PARAM_FRAME_BUFFER_SIZE is not available on this camera");
                    None
                }
            } else {
                tracing::debug!("PARAM_FRAME_BUFFER_SIZE is not available, using heuristics");
                None
            }
        };

        // Calculate frame count from SDK recommendation
        let sdk_frames = sdk_recommended
            .map(|bytes| bytes / frame_bytes.max(1))
            .unwrap_or(0);

        // Calculate frames needed for ~1 second of buffer based on exposure time
        // Frame period ~= exposure_ms (simplified; ignores readout time)
        let fps_estimate = if exposure_ms > 0.0 {
            ONE_SECOND_MS / exposure_ms
        } else {
            100.0 // Default assumption: 100 FPS
        };
        let one_second_frames = fps_estimate.ceil() as usize;

        // Choose the larger of SDK recommendation and 1-second heuristic,
        // then clamp to reasonable bounds. Default to SDK guidance when available
        // (typical Prime BSI recommendation is 255 frames at full frame).
        let target = sdk_frames.max(one_second_frames).max(MIN_BUFFER_FRAMES);
        let clamped = target.min(MAX_BUFFER_FRAMES);

        tracing::debug!(
            "Buffer sizing: SDK={:?} frames, 1sec={} frames, target={}, clamped={}",
            sdk_recommended.map(|b| b / frame_bytes.max(1)),
            one_second_frames,
            target,
            clamped
        );

        clamped
    }

    /// Get the number of ROIs supported by the camera (bd-vcbd)
    ///
    /// Returns the maximum number of regions of interest (ROIs) that can be
    /// configured for acquisition. Useful for multi-region readout modes.
    ///
    /// # SDK Pattern (bd-vcbd)
    /// Checks PARAM_ROI_COUNT availability before access.
    #[cfg(feature = "pvcam_sdk")]
    pub fn get_roi_count(conn: &PvcamConnection) -> Result<u16> {
        if let Some(h) = conn.handle() {
            // SDK Pattern: Check availability before access
            let mut avail: rs_bool = 0;
            // SAFETY: `h` is a valid camera handle. `avail` is a stack-allocated
            // rs_bool output parameter. Read-only availability query.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_ROI_COUNT,
                    ATTR_AVAIL as i16,
                    &mut avail as *mut _ as *mut _,
                ) == 0
                {
                    // Failed to query availability
                    return Err(anyhow!(
                        "Failed to query PARAM_ROI_COUNT availability: {}",
                        get_pvcam_error()
                    ));
                }

                if avail == 0 {
                    return Err(anyhow!("PARAM_ROI_COUNT is not available on this camera"));
                }

                let mut count: uns16 = 0;
                // SAFETY: h is valid handle; count is writable uns16 on stack.
                if pl_get_param(
                    h,
                    PARAM_ROI_COUNT,
                    ATTR_CURRENT as i16,
                    &mut count as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get ROI count: {}", get_pvcam_error()));
                }
                return Ok(count);
            }
        }
        Err(anyhow!("Camera not connected"))
    }

    /// Get the number of ROIs supported by the camera (mock mode) (bd-vcbd)
    ///
    /// Mock version that returns a default value when hardware is not available.
    #[cfg(not(feature = "pvcam_sdk"))]
    pub fn get_roi_count(_conn: &PvcamConnection) -> Result<u16> {
        // Mock mode default: 1 ROI (single region)
        Ok(1)
    }
}
