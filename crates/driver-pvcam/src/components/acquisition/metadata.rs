//! Metadata decoding control for PVCAM acquisition.

#[cfg(feature = "pvcam_sdk")]
use super::FrameMetadata;
use super::PvcamAcquisition;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::Ordering;

impl PvcamAcquisition {
    /// Enable metadata decoding and set the metadata channel (Gemini SDK review).
    ///
    /// When enabled, PVCAM embeds hardware timestamps in frame buffers which are
    /// decoded using `pl_md_frame_decode`. This provides microsecond-precision
    /// timing from the FPGA for correlating frames with other hardware events.
    ///
    /// # Arguments
    ///
    /// * `tx` - Channel to receive `FrameMetadata` for each frame
    ///
    /// # Note
    ///
    /// Must be called before `start_stream()`. The metadata channel will receive
    /// one `FrameMetadata` per frame in sync with the frame delivery.
    #[cfg(feature = "pvcam_sdk")]
    pub async fn enable_metadata(&self, tx: tokio::sync::mpsc::Sender<FrameMetadata>) {
        let mut guard = self.metadata_tx.lock().await;
        *guard = Some(tx);
        self.metadata_enabled.store(true, Ordering::Release);
        tracing::info!("Metadata decoding enabled for acquisition");
    }

    /// Disable metadata decoding (Gemini SDK review).
    #[cfg(feature = "pvcam_sdk")]
    pub async fn disable_metadata(&self) {
        let mut guard = self.metadata_tx.lock().await;
        *guard = None;
        self.metadata_enabled.store(false, Ordering::Release);
    }

    /// Toggle metadata decoding without changing the channel (bd-32f4).
    ///
    /// Called by the driver's `metadata_enabled` parameter write callback to
    /// sync the acquisition's decoding flag with the SDK parameter. When enabled
    /// without a channel, frames are still decoded (useful for data integrity)
    /// but decoded metadata is silently dropped.
    #[cfg(feature = "pvcam_sdk")]
    pub fn set_metadata_decoding(&self, enabled: bool) {
        self.metadata_enabled.store(enabled, Ordering::Release);
        tracing::debug!("Metadata decoding flag set to {}", enabled);
    }
}
