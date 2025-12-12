//! PVCAM Acquisition Logic
//!
//! Handles streaming, circular buffers, and frame polling.

use anyhow::{anyhow, bail, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use daq_core::data::Frame;
use daq_core::parameter::Parameter;
use crate::components::connection::PvcamConnection;
use daq_core::core::Roi;
use std::time::Duration;
use tokio::time::timeout;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
use tokio::task::JoinHandle;

pub struct PvcamAcquisition {
    pub streaming: Parameter<bool>,
    pub frame_count: Arc<AtomicU64>,
    pub frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
    pub reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>>,
    
    #[cfg(feature = "pvcam_hardware")]
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    #[cfg(feature = "pvcam_hardware")]
    circ_buffer: Arc<Mutex<Option<Vec<u16>>>>,
    #[cfg(feature = "pvcam_hardware")]
    trigger_frame: Arc<Mutex<Option<Vec<u16>>>>,
}

impl PvcamAcquisition {
    pub fn new(streaming: Parameter<bool>) -> Self {
        let (frame_tx, _) = tokio::sync::broadcast::channel(16);
        Self {
            streaming,
            frame_count: Arc::new(AtomicU64::new(0)),
            frame_tx,
            reliable_tx: Arc::new(Mutex::new(None)),
            
            #[cfg(feature = "pvcam_hardware")]
            poll_handle: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            circ_buffer: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            trigger_frame: Arc::new(Mutex::new(None)),
        }
    }

    /// Start streaming frames
    pub async fn start_stream(
        &self, 
        conn: &PvcamConnection,
        roi: Roi,
        binning: (u16, u16),
        exposure_ms: f64
    ) -> Result<()> {
        // Avoid unused parameter warnings when hardware feature is disabled.
        let _ = conn;
        if self.streaming.get() {
            bail!("Already streaming");
        }

        self.streaming.set(true).await?;
        self.frame_count.store(0, Ordering::SeqCst);
        let reliable_tx = self.reliable_tx.lock().await.clone();

        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = conn.handle() {
            // Hardware path
            let (x_bin, y_bin) = binning;
            
            // Setup region
            let region = unsafe {
                let mut rgn: rgn_type = std::mem::zeroed();
                rgn.s1 = roi.x as uns16;
                rgn.s2 = (roi.x + roi.width - 1) as uns16;
                rgn.sbin = x_bin;
                rgn.p1 = roi.y as uns16;
                rgn.p2 = (roi.y + roi.height - 1) as uns16;
                rgn.pbin = y_bin;
                rgn
            };

            let mut frame_bytes: uns32 = 0;
            unsafe {
                if pl_exp_setup_cont(
                    h,
                    1,
                    &region as *const _,
                    TIMED_MODE,
                    exposure_ms as uns32,
                    &mut frame_bytes,
                    CIRC_NO_OVERWRITE,
                ) == 0 {
                    let _ = self.streaming.set(false).await;
                    return Err(anyhow!("Failed to setup continuous acquisition"));
                }
            }

            // Allocate buffer
            let binned_width = roi.width / x_bin as u32;
            let binned_height = roi.height / y_bin as u32;
            let frame_pixels = (binned_width * binned_height) as usize;
            let buffer_count = 8;
            let mut circ_buf = vec![0u16; frame_pixels * buffer_count];
            let circ_ptr = circ_buf.as_mut_ptr();
            let circ_size_bytes = (circ_buf.len() * 2) as uns32;

            unsafe {
                if pl_exp_start_cont(h, circ_ptr as *mut _, circ_size_bytes) == 0 {
                    let _ = self.streaming.set(false).await;
                    return Err(anyhow!("Failed to start continuous acquisition"));
                }
            }

            *self.circ_buffer.lock().await = Some(circ_buf);

            let streaming = self.streaming.clone();
            let frame_tx = self.frame_tx.clone();
            let frame_count = self.frame_count.clone();
            let width = binned_width;
            let height = binned_height;

            let poll_handle = tokio::task::spawn_blocking(move || {
                Self::poll_loop_hardware(
                    h,
                    streaming,
                    frame_tx,
                    reliable_tx,
                    frame_count,
                    frame_pixels,
                    width,
                    height,
                );
            });

            *self.poll_handle.lock().await = Some(poll_handle);
            return Ok(());
        }

        // Mock path (or no handle)
        #[cfg(not(feature = "pvcam_hardware"))]
        self.start_mock_stream(roi, binning, exposure_ms, reliable_tx).await?;
        
        // Handle case where hardware feature enabled but handle missing (mock fallback logic)
        #[cfg(feature = "pvcam_hardware")]
        if conn.handle().is_none() {
             self.start_mock_stream(roi, binning, exposure_ms, reliable_tx).await?;
        }

        Ok(())
    }

    /// Acquire a single frame by starting the stream, grabbing one frame, then stopping.
    pub async fn acquire_single_frame(
        &self,
        conn: &PvcamConnection,
        roi: Roi,
        binning: (u16, u16),
        exposure_ms: f64,
    ) -> Result<Frame> {
        let mut rx = self.frame_tx.subscribe();
        self.start_stream(conn, roi, binning, exposure_ms).await?;

        let frame = timeout(Duration::from_secs(5), rx.recv())
            .await
            .map_err(|_| anyhow!("Timed out waiting for frame"))?
            .map_err(|e| anyhow!("Frame channel closed: {e}"))?;

        let _ = self.stop_stream(conn).await;
        Ok((*frame).clone())
    }

    async fn start_mock_stream(
        &self, 
        roi: Roi, 
        binning: (u16, u16), 
        exposure_ms: f64,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>
    ) -> Result<()> {
        let streaming = self.streaming.clone();
        let frame_tx = self.frame_tx.clone();
        let frame_count = self.frame_count.clone();
        let (x_bin, y_bin) = binning;

        tokio::spawn(async move {
            let binned_width = roi.width / x_bin as u32;
            let binned_height = roi.height / y_bin as u32;
            let frame_size = (binned_width * binned_height) as usize;

            while streaming.get() {
                tokio::time::sleep(Duration::from_millis(exposure_ms as u64)).await;
                if !streaming.get() { break; }

                let frame_num = frame_count.fetch_add(1, Ordering::SeqCst);
                let mut pixels = vec![0u16; frame_size];
                for y in 0..binned_height {
                    for x in 0..binned_width {
                        let value = (((x + y + frame_num as u32) % 4096) as u16).saturating_add(100);
                        pixels[(y * binned_width + x) as usize] = value;
                    }
                }

                let frame = Arc::new(Frame::from_u16(binned_width, binned_height, &pixels));
                
                if let Some(ref tx) = reliable_tx {
                    let _ = tx.send(frame.clone()).await;
                }
                let _ = frame_tx.send(frame);
            }
        });
        Ok(())
    }

    pub async fn stop_stream(&self, conn: &PvcamConnection) -> Result<()> {
        // Avoid unused parameter warnings when hardware feature is disabled.
        let _ = conn;
        if !self.streaming.get() {
            return Ok(());
        }
        self.streaming.set(false).await?;

        #[cfg(feature = "pvcam_hardware")]
        {
            if let Some(handle) = self.poll_handle.lock().await.take() {
                let _ = handle.await;
            }
            if let Some(h) = conn.handle() {
                unsafe { pl_exp_stop_cont(h, CCS_HALT); }
            }
            *self.circ_buffer.lock().await = None;
        }
        Ok(())
    }

    #[cfg(feature = "pvcam_hardware")]
    fn poll_loop_hardware(
        hcam: i16,
        streaming: Parameter<bool>,
        frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        frame_count: Arc<AtomicU64>,
        frame_pixels: usize,
        width: u32,
        height: u32,
    ) {
        let mut status: i16 = 0;
        let mut bytes_arrived: uns32 = 0;
        let mut buffer_cnt: uns32 = 0;
        let mut no_frame_count: u32 = 0;
        const MAX_NO_FRAME_ITERATIONS: u32 = 5000;

        while streaming.get() {
            unsafe {
                if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) == 0 {
                    break;
                }

                match status {
                    s if s == READOUT_COMPLETE || s == EXPOSURE_IN_PROGRESS => {
                        let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                        if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) != 0 && !frame_ptr.is_null() {
                            let src = std::slice::from_raw_parts(frame_ptr as *const u16, frame_pixels);
                            let pixels = src.to_vec();
                            pl_exp_unlock_oldest_frame(hcam);

                            let frame = Frame::from_u16(width, height, &pixels);
                            frame_count.fetch_add(1, Ordering::SeqCst);
                            let frame_arc = Arc::new(frame);

                            if let Some(ref tx) = reliable_tx {
                                let _ = tx.blocking_send(frame_arc.clone());
                            }
                            let _ = frame_tx.send(frame_arc);
                            no_frame_count = 0;
                        } else {
                            std::thread::sleep(Duration::from_millis(1));
                            no_frame_count += 1;
                        }
                    }
                    s if s == READOUT_FAILED => break,
                    _ => {
                        std::thread::sleep(Duration::from_millis(1));
                        no_frame_count += 1;
                    }
                }

                if no_frame_count >= MAX_NO_FRAME_ITERATIONS {
                    break;
                }
            }
        }
        unsafe { pl_exp_stop_cont(hcam, CCS_HALT); }
    }
}
