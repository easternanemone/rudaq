//! PVCAM Hardware Adapter for V4 Architecture
//!
//! Lightweight wrapper around PVCAM SDK for V4 actors.
//! Provides mock mode for testing without hardware.

use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Camera handle (wraps PVCAM SDK handle)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraHandle(pub i16);

/// Single frame from PVCAM camera
#[derive(Debug, Clone)]
pub struct PvcamFrame {
    /// Raw pixel data (u16 pixels)
    pub data: Vec<u16>,
    /// Frame number
    pub frame_number: u32,
    /// Software timestamp
    pub timestamp_ns: i64,
    /// Exposure time (ms)
    pub exposure_ms: f64,
    /// ROI: (x, y, width, height)
    pub roi: (u16, u16, u16, u16),
}

/// Region of interest
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRegion {
    pub s1: u16,    // Start column
    pub s2: u16,    // End column
    pub sbin: u16,  // Column binning
    pub p1: u16,    // Start row
    pub p2: u16,    // End row
    pub pbin: u16,  // Row binning
}

/// PVCAM adapter trait
pub trait PvcamAdapter: Send + Sync {
    /// Initialize SDK
    fn init(&self) -> Result<()>;

    /// Uninitialize SDK
    fn uninit(&self) -> Result<()>;

    /// Open camera by name
    fn open_camera(&self, name: &str) -> Result<CameraHandle>;

    /// Close camera
    fn close_camera(&self, handle: CameraHandle) -> Result<()>;

    /// Set exposure time (ms)
    fn set_exposure(&self, handle: CameraHandle, exposure_ms: u16) -> Result<()>;

    /// Set gain
    fn set_gain(&self, handle: CameraHandle, gain: u16) -> Result<()>;

    /// Set ROI
    fn set_roi(&self, handle: CameraHandle, roi: PxRegion) -> Result<()>;

    /// Start continuous acquisition
    fn start_acquisition(
        self: Arc<Self>,
        handle: CameraHandle,
    ) -> Result<(mpsc::Receiver<PvcamFrame>, AcquisitionGuard)>;

    /// Stop acquisition
    fn stop_acquisition(&self, handle: CameraHandle) -> Result<()>;
}

/// RAII guard that stops acquisition when dropped
pub struct AcquisitionGuard {
    sdk: Arc<dyn PvcamAdapter>,
    handle: CameraHandle,
}

impl Drop for AcquisitionGuard {
    fn drop(&mut self) {
        if let Err(e) = self.sdk.stop_acquisition(self.handle) {
            log::error!("Failed to stop acquisition for {:?}: {}", self.handle, e);
        }
    }
}

// ============================================================================
// Mock PVCAM Adapter
// ============================================================================

struct MockCameraState {
    name: String,
    exposure_ms: u16,
    gain: u16,
    roi: PxRegion,
    acquisition_task: Option<JoinHandle<()>>,
    stop_tx: Option<oneshot::Sender<()>>,
}

pub struct MockPvcamAdapter {
    initialized: Arc<Mutex<bool>>,
    cameras: Arc<Mutex<std::collections::HashMap<CameraHandle, MockCameraState>>>,
    next_handle: Arc<Mutex<i16>>,
}

impl MockPvcamAdapter {
    pub fn new() -> Self {
        Self {
            initialized: Arc::new(Mutex::new(false)),
            cameras: Arc::new(Mutex::new(std::collections::HashMap::new())),
            next_handle: Arc::new(Mutex::new(1)),
        }
    }
}

impl Default for MockPvcamAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PvcamAdapter for MockPvcamAdapter {
    fn init(&self) -> Result<()> {
        let mut init = self.initialized.lock().unwrap();
        if *init {
            anyhow::bail!("Already initialized");
        }
        *init = true;
        log::info!("Mock PVCAM SDK initialized");
        Ok(())
    }

    fn uninit(&self) -> Result<()> {
        let mut init = self.initialized.lock().unwrap();
        if !*init {
            anyhow::bail!("Not initialized");
        }
        self.cameras.lock().unwrap().clear();
        *init = false;
        log::info!("Mock PVCAM SDK uninitialized");
        Ok(())
    }

    fn open_camera(&self, name: &str) -> Result<CameraHandle> {
        if !*self.initialized.lock().unwrap() {
            anyhow::bail!("SDK not initialized");
        }

        let mut next_id = self.next_handle.lock().unwrap();
        let handle = CameraHandle(*next_id);
        *next_id += 1;

        let state = MockCameraState {
            name: name.to_string(),
            exposure_ms: 100,
            gain: 1,
            roi: PxRegion {
                s1: 0,
                s2: 2047,
                sbin: 1,
                p1: 0,
                p2: 2047,
                pbin: 1,
            },
            acquisition_task: None,
            stop_tx: None,
        };

        self.cameras.lock().unwrap().insert(handle, state);
        log::info!("Mock camera '{}' opened with handle {:?}", name, handle);
        Ok(handle)
    }

    fn close_camera(&self, handle: CameraHandle) -> Result<()> {
        let mut cameras = self.cameras.lock().unwrap();
        if let Some(mut state) = cameras.remove(&handle) {
            if let Some(tx) = state.stop_tx.take() {
                let _ = tx.send(());
            }
            log::info!("Mock camera {:?} closed", handle);
            Ok(())
        } else {
            anyhow::bail!("Camera not open: {:?}", handle);
        }
    }

    fn set_exposure(&self, handle: CameraHandle, exposure_ms: u16) -> Result<()> {
        let mut cameras = self.cameras.lock().unwrap();
        if let Some(state) = cameras.get_mut(&handle) {
            state.exposure_ms = exposure_ms;
            log::debug!("Set exposure to {}ms for {:?}", exposure_ms, handle);
            Ok(())
        } else {
            anyhow::bail!("Camera not open: {:?}", handle);
        }
    }

    fn set_gain(&self, handle: CameraHandle, gain: u16) -> Result<()> {
        let mut cameras = self.cameras.lock().unwrap();
        if let Some(state) = cameras.get_mut(&handle) {
            state.gain = gain;
            log::debug!("Set gain to {} for {:?}", gain, handle);
            Ok(())
        } else {
            anyhow::bail!("Camera not open: {:?}", handle);
        }
    }

    fn set_roi(&self, handle: CameraHandle, roi: PxRegion) -> Result<()> {
        let mut cameras = self.cameras.lock().unwrap();
        if let Some(state) = cameras.get_mut(&handle) {
            state.roi = roi;
            log::debug!("Set ROI {:?} for {:?}", roi, handle);
            Ok(())
        } else {
            anyhow::bail!("Camera not open: {:?}", handle);
        }
    }

    fn start_acquisition(
        self: Arc<Self>,
        handle: CameraHandle,
    ) -> Result<(mpsc::Receiver<PvcamFrame>, AcquisitionGuard)> {
        let (exposure_ms, roi) = {
            let cameras = self.cameras.lock().unwrap();
            let state = cameras
                .get(&handle)
                .ok_or_else(|| anyhow::anyhow!("Camera not open: {:?}", handle))?;

            if state.acquisition_task.is_some() {
                anyhow::bail!("Acquisition already in progress for {:?}", handle);
            }

            (state.exposure_ms, state.roi)
        };

        let (tx, rx) = mpsc::channel(16);
        let (stop_tx, mut stop_rx) = oneshot::channel();

        let width = (roi.s2 - roi.s1 + 1) / roi.sbin;
        let height = (roi.p2 - roi.p1 + 1) / roi.pbin;

        // Spawn mock acquisition task
        let task = tokio::spawn(async move {
            let mut frame_count = 0u32;

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(exposure_ms as u64)) => {
                        // Generate mock frame data
                        let mut data = Vec::with_capacity((width as usize) * (height as usize));
                        for y in 0..height {
                            for x in 0..width {
                                // Prevent overflow by using u32 arithmetic and clamping result
                                let val = ((x as u32 + y as u32 + frame_count) % 256) * 100;
                                data.push(val.min(u16::MAX as u32) as u16);
                            }
                        }

                        let frame = PvcamFrame {
                            data,
                            frame_number: frame_count,
                            timestamp_ns: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                            exposure_ms: exposure_ms as f64,
                            roi: (roi.s1, roi.p1, width, height),
                        };

                        if tx.send(frame).await.is_err() {
                            log::info!("Mock acquisition receiver dropped");
                            break;
                        }

                        frame_count += 1;
                    }
                    _ = &mut stop_rx => {
                        log::info!("Mock acquisition stopped via signal");
                        break;
                    }
                }
            }
        });

        // Update state
        {
            let mut cameras = self.cameras.lock().unwrap();
            if let Some(state) = cameras.get_mut(&handle) {
                state.acquisition_task = Some(task);
                state.stop_tx = Some(stop_tx);
            }
        }

        let guard = AcquisitionGuard {
            sdk: self.clone(),
            handle,
        };

        log::info!("Mock acquisition started for {:?}", handle);
        Ok((rx, guard))
    }

    fn stop_acquisition(&self, handle: CameraHandle) -> Result<()> {
        let mut cameras = self.cameras.lock().unwrap();
        if let Some(state) = cameras.get_mut(&handle) {
            if let Some(tx) = state.stop_tx.take() {
                let _ = tx.send(());
            }
            state.acquisition_task = None;
            log::info!("Mock acquisition stopped for {:?}", handle);
        }
        Ok(())
    }
}
