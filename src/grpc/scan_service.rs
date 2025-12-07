//! ScanService implementation for coordinated multi-axis scans (bd-4le6)
//!
//! This module provides gRPC endpoints for creating and executing coordinated
//! scans across multiple motion axes with synchronized data acquisition.
//!
//! # Scan Flow
//! 1. `CreateScan` - Validate and store scan configuration
//! 2. `StartScan` - Begin scan execution (spawns background task)
//! 3. `StreamScanProgress` - Monitor progress in real-time
//! 4. `PauseScan`/`ResumeScan` - Control execution
//! 5. `StopScan` - Abort scan
//!
//! # Data Persistence (The Mullet Strategy)
//!
//! When configured with a RingBuffer, scan data is automatically persisted:
//! - Scan progress and measurements are serialized to the ring buffer
//! - HDF5Writer background task flushes to disk at 1 Hz
//! - Scientists see HDF5 files compatible with Python/MATLAB/Igor

use crate::data::ring_buffer::RingBuffer;
use crate::grpc::proto::{
    scan_service_server::ScanService, CreateScanRequest, CreateScanResponse, GetScanStatusRequest,
    ListScansRequest, ListScansResponse, PauseScanRequest, PauseScanResponse, ResumeScanRequest,
    ResumeScanResponse, ScanConfig, ScanDataPoint, ScanProgress, ScanState, ScanStatus, ScanType,
    StartScanRequest, StartScanResponse, StopScanRequest, StopScanResponse,
    StreamScanProgressRequest,
};
use crate::hardware::registry::DeviceRegistry;
use log::warn;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

/// Internal scan state for tracking execution
struct ScanExecution {
    config: ScanConfig,
    state: i32, // ScanState as i32
    current_point: u32,
    total_points: u32,
    start_time: Option<Instant>,
    start_time_ns: u64,
    error_message: Option<String>,
    /// Channel for progress updates (shared sender for multiple subscribers)
    progress_tx: mpsc::Sender<ScanProgress>,
    progress_rx: Arc<Mutex<Option<mpsc::Receiver<ScanProgress>>>>,
    /// Handle to the background task (if running)
    task_handle: Option<JoinHandle<()>>,
    /// Flag to signal pause request
    pause_requested: Arc<std::sync::atomic::AtomicBool>,
    /// Flag to signal stop request
    stop_requested: Arc<std::sync::atomic::AtomicBool>,
}

impl ScanExecution {
    fn new(config: ScanConfig, total_points: u32) -> Self {
        let (progress_tx, progress_rx) = mpsc::channel(100);
        Self {
            config,
            state: ScanState::ScanCreated.into(),
            current_point: 0,
            total_points,
            start_time: None,
            start_time_ns: 0,
            error_message: None,
            progress_tx,
            progress_rx: Arc::new(Mutex::new(Some(progress_rx))),
            task_handle: None,
            pause_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stop_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn to_status(&self, scan_id: &str) -> ScanStatus {
        let elapsed_ns = self
            .start_time
            .map(|t| t.elapsed().as_nanos() as u64)
            .unwrap_or(0);

        let progress = if self.total_points > 0 {
            (self.current_point as f64 / self.total_points as f64) * 100.0
        } else {
            0.0
        };

        // Estimate remaining time based on progress
        let estimated_remaining_ns = if self.current_point > 0 && elapsed_ns > 0 {
            let time_per_point = elapsed_ns / self.current_point as u64;
            let remaining_points = self.total_points - self.current_point;
            Some(time_per_point * remaining_points as u64)
        } else {
            None
        };

        ScanStatus {
            scan_id: scan_id.to_string(),
            state: self.state,
            current_point: self.current_point,
            total_points: self.total_points,
            progress_percent: progress,
            start_time_ns: self.start_time_ns,
            elapsed_time_ns: elapsed_ns,
            estimated_remaining_ns,
            error_message: self.error_message.clone().unwrap_or_default(),
        }
    }
}

/// ScanService gRPC implementation
///
/// Coordinates multi-axis scans with synchronized data acquisition.
///
/// # Data Persistence
///
/// When configured with a RingBuffer via `with_ring_buffer()`, scan data is
/// automatically persisted using The Mullet Strategy:
/// - Fast writes to memory-mapped ring buffer during scan
/// - HDF5Writer background task flushes to disk at 1 Hz
/// - Scientists see HDF5 files compatible with Python/MATLAB/Igor
pub struct ScanServiceImpl {
    registry: Arc<RwLock<DeviceRegistry>>,
    scans: Arc<Mutex<HashMap<String, ScanExecution>>>,
    next_scan_id: Arc<std::sync::atomic::AtomicU64>,
    /// Optional ring buffer for data persistence
    ring_buffer: Option<Arc<RingBuffer>>,
}

impl ScanServiceImpl {
    const RPC_TIMEOUT: Duration = Duration::from_secs(15);

    async fn with_request_deadline<F, T>(&self, operation: &str, fut: F) -> Result<T, Status>
    where
        F: Future<Output = Result<T, Status>> + Send,
        T: Send,
    {
        match tokio::time::timeout(Self::RPC_TIMEOUT, fut).await {
            Ok(result) => result,
            Err(_) => Err(Status::deadline_exceeded(format!(
                "{} timed out after {:?}",
                operation,
                Self::RPC_TIMEOUT
            ))),
        }
    }

    /// Create a new ScanService with the given device registry
    pub fn new(registry: Arc<RwLock<DeviceRegistry>>) -> Self {
        Self {
            registry,
            scans: Arc::new(Mutex::new(HashMap::new())),
            next_scan_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            ring_buffer: None,
        }
    }

    /// Configure data persistence via ring buffer
    ///
    /// When set, scan data will be written to the ring buffer in a format
    /// that HDF5Writer can persist to disk.
    pub fn with_ring_buffer(mut self, ring_buffer: Arc<RingBuffer>) -> Self {
        self.ring_buffer = Some(ring_buffer);
        self
    }

    /// Calculate total scan points from configuration
    fn calculate_total_points(config: &ScanConfig) -> u32 {
        if config.axes.is_empty() {
            return 0;
        }

        config
            .axes
            .iter()
            .map(|axis| axis.num_points.max(1))
            .product()
    }

    /// Generate scan point positions for all axes
    fn generate_scan_points(config: &ScanConfig) -> Vec<Vec<f64>> {
        // Generate positions for each axis
        let axis_positions: Vec<Vec<f64>> = config
            .axes
            .iter()
            .map(|axis| {
                let n = axis.num_points.max(1) as usize;
                if n == 1 {
                    vec![axis.start_position]
                } else {
                    let step = (axis.end_position - axis.start_position) / (n - 1) as f64;
                    (0..n)
                        .map(|i| axis.start_position + step * i as f64)
                        .collect()
                }
            })
            .collect();

        // Build combined points based on scan type
        let mut points = Vec::new();
        Self::build_scan_points_recursive(config, &axis_positions, 0, vec![], &mut points);
        points
    }

    fn build_scan_points_recursive(
        config: &ScanConfig,
        axis_positions: &[Vec<f64>],
        axis_idx: usize,
        current_point: Vec<f64>,
        result: &mut Vec<Vec<f64>>,
    ) {
        if axis_idx >= axis_positions.len() {
            result.push(current_point);
            return;
        }

        let positions = &axis_positions[axis_idx];
        let is_snake = config.scan_type() == ScanType::SnakeScan;

        // For snake scan, reverse direction on odd iterations of outer axes
        let should_reverse = is_snake && (result.len() / positions.len()) % 2 == 1;

        let iter: Box<dyn Iterator<Item = &f64>> = if should_reverse {
            Box::new(positions.iter().rev())
        } else {
            Box::new(positions.iter())
        };

        for &pos in iter {
            let mut next_point = current_point.clone();
            next_point.push(pos);
            Self::build_scan_points_recursive(
                config,
                axis_positions,
                axis_idx + 1,
                next_point,
                result,
            );
        }
    }

    /// Validate scan configuration against available devices
    async fn validate_config(&self, config: &ScanConfig) -> Result<(), String> {
        let registry = self.registry.read().await;

        // Validate axes
        if config.axes.is_empty() {
            return Err("Scan must have at least one axis".to_string());
        }

        for axis in &config.axes {
            if !registry.contains(&axis.device_id) {
                return Err(format!("Device not found: {}", axis.device_id));
            }
            if registry.get_movable(&axis.device_id).is_none() {
                return Err(format!("Device is not movable: {}", axis.device_id));
            }
            if axis.num_points == 0 {
                return Err(format!(
                    "Axis {} must have at least 1 point",
                    axis.device_id
                ));
            }
        }

        // Validate acquire devices
        for device_id in &config.acquire_device_ids {
            if !registry.contains(device_id) {
                return Err(format!("Acquire device not found: {}", device_id));
            }
            if registry.get_readable(device_id).is_none() {
                return Err(format!("Device is not readable: {}", device_id));
            }
        }

        // Validate camera if specified
        if let Some(camera_id) = &config.camera_device_id {
            if !registry.contains(camera_id) {
                return Err(format!("Camera device not found: {}", camera_id));
            }
            if registry.get_triggerable(camera_id).is_none() {
                return Err(format!("Camera is not triggerable: {}", camera_id));
            }
        }

        Ok(())
    }

    /// Execute scan in background task
    async fn run_scan(
        registry: Arc<RwLock<DeviceRegistry>>,
        scan_id: String,
        config: ScanConfig,
        scans: Arc<Mutex<HashMap<String, ScanExecution>>>,
        pause_requested: Arc<std::sync::atomic::AtomicBool>,
        stop_requested: Arc<std::sync::atomic::AtomicBool>,
        progress_tx: mpsc::Sender<ScanProgress>,
        ring_buffer: Option<Arc<RingBuffer>>,
    ) {
        use std::sync::atomic::Ordering;

        let points = Self::generate_scan_points(&config);
        let total_points = points.len() as u32;
        let dwell_ms = config.dwell_time_ms.max(0.0) as u64;
        let triggers = config.triggers_per_point.max(1);

        // Extract device references (lock once, release before async loop)
        let (movables, readables, triggerable) = {
            let reg = registry.read().await;
            let movables: Vec<_> = config
                .axes
                .iter()
                .filter_map(|a| {
                    reg.get_movable(&a.device_id)
                        .map(|m| (a.device_id.clone(), m))
                })
                .collect();
            let readables: Vec<_> = config
                .acquire_device_ids
                .iter()
                .filter_map(|id| reg.get_readable(id).map(|r| (id.clone(), r)))
                .collect();
            let triggerable = config
                .camera_device_id
                .as_ref()
                .and_then(|id| reg.get_triggerable(id).map(|t| (id.clone(), t)));
            (movables, readables, triggerable)
        };

        // Arm camera if configured
        if let Some((_, ref trig)) = triggerable {
            if config.arm_camera.unwrap_or(false) {
                if let Err(e) = trig.arm().await {
                    Self::set_scan_error(&scans, &scan_id, format!("Failed to arm camera: {}", e))
                        .await;
                    return;
                }
            }
        }

        // Execute scan points
        for (point_idx, positions) in points.iter().enumerate() {
            // Check for stop/pause
            if stop_requested.load(Ordering::SeqCst) {
                Self::set_scan_state(&scans, &scan_id, ScanState::ScanStopped, point_idx as u32)
                    .await;
                return;
            }

            while pause_requested.load(Ordering::SeqCst) {
                Self::set_scan_state(&scans, &scan_id, ScanState::ScanPaused, point_idx as u32)
                    .await;
                tokio::time::sleep(Duration::from_millis(100)).await;
                if stop_requested.load(Ordering::SeqCst) {
                    Self::set_scan_state(
                        &scans,
                        &scan_id,
                        ScanState::ScanStopped,
                        point_idx as u32,
                    )
                    .await;
                    return;
                }
            }

            // Set running state
            Self::set_scan_state(&scans, &scan_id, ScanState::ScanRunning, point_idx as u32).await;

            // Move all axes to position
            let mut axis_positions_map = HashMap::new();
            for (i, (device_id, movable)) in movables.iter().enumerate() {
                let target = positions[i];
                if let Err(e) = movable.move_abs(target).await {
                    Self::set_scan_error(
                        &scans,
                        &scan_id,
                        format!("Move failed on {}: {}", device_id, e),
                    )
                    .await;
                    return;
                }
                axis_positions_map.insert(device_id.clone(), target);
            }

            // Wait for all axes to settle
            for (device_id, movable) in &movables {
                if let Err(e) = movable.wait_settled().await {
                    Self::set_scan_error(
                        &scans,
                        &scan_id,
                        format!("Settle failed on {}: {}", device_id, e),
                    )
                    .await;
                    return;
                }
            }

            // Dwell time
            if dwell_ms > 0 {
                tokio::time::sleep(Duration::from_millis(dwell_ms)).await;
            }

            // Acquire data
            let mut data_points = Vec::new();
            for trigger_idx in 0..triggers {
                // Trigger camera if configured
                if let Some((_, ref trig)) = triggerable {
                    if let Err(e) = trig.trigger().await {
                        Self::set_scan_error(&scans, &scan_id, format!("Trigger failed: {}", e))
                            .await;
                        return;
                    }
                }

                // Read all acquisition devices
                let timestamp_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64;

                for (device_id, readable) in &readables {
                    match readable.read().await {
                        Ok(value) => {
                            data_points.push(ScanDataPoint {
                                device_id: device_id.clone(),
                                value,
                                timestamp_ns,
                                trigger_index: trigger_idx,
                            });
                        }
                        Err(e) => {
                            Self::set_scan_error(
                                &scans,
                                &scan_id,
                                format!("Read failed on {}: {}", device_id, e),
                            )
                            .await;
                            return;
                        }
                    }
                }
            }

            // Send progress update with backpressure handling (bd-6qaj)
            // Use try_send to avoid blocking if client is slow; drop updates rather than
            // accumulating spawned tasks or erroring the scan.
            let progress = ScanProgress {
                scan_id: scan_id.clone(),
                state: ScanState::ScanRunning.into(),
                point_index: point_idx as u32,
                total_points,
                timestamp_ns: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64,
                axis_positions: axis_positions_map,
                data_points,
            };

            // Persist scan data to ring buffer for HDF5 storage (The Mullet Strategy)
            // Write length-prefixed Protobuf messages for HDF5Writer to decode
            if let Some(ref rb) = ring_buffer {
                use prost::Message;
                let msg_len = progress.encoded_len();
                // Allocate buffer: 4 bytes for length + message bytes
                let mut buf = Vec::with_capacity(4 + msg_len);
                // Write length prefix (4 bytes, little-endian)
                buf.extend_from_slice(&(msg_len as u32).to_le_bytes());
                if let Err(e) = progress.encode(&mut buf) {
                    warn!("Failed to encode scan progress for persistence: {}", e);
                } else if let Err(e) = rb.write(&buf) {
                    warn!("Failed to write scan data to ring buffer: {}", e);
                }
            }

            match progress_tx.try_send(progress) {
                Ok(()) => {} // Progress sent successfully
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Channel full - client is slow, drop this update
                    // Client can use GetScanStatus to poll current state
                    warn!(
                        "Progress channel full for {} at point {}/{}, dropping update",
                        scan_id, point_idx, total_points
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    // Channel closed - client disconnected, but don't fail the scan
                    // The scan should continue to completion even without a listener
                    warn!(
                        "Progress channel closed for {} at point {}/{}, continuing scan",
                        scan_id, point_idx, total_points
                    );
                }
            }

            // Update current point
            {
                let mut scans_guard = scans.lock().await;
                if let Some(scan) = scans_guard.get_mut(&scan_id) {
                    scan.current_point = point_idx as u32 + 1;
                }
            }
        }

        // Scan completed successfully
        Self::set_scan_state(&scans, &scan_id, ScanState::ScanCompleted, total_points).await;
    }

    async fn set_scan_state(
        scans: &Arc<Mutex<HashMap<String, ScanExecution>>>,
        scan_id: &str,
        state: ScanState,
        current_point: u32,
    ) {
        let mut scans_guard = scans.lock().await;
        if let Some(scan) = scans_guard.get_mut(scan_id) {
            scan.state = state.into();
            scan.current_point = current_point;
        }
    }

    async fn set_scan_error(
        scans: &Arc<Mutex<HashMap<String, ScanExecution>>>,
        scan_id: &str,
        error: String,
    ) {
        let mut scans_guard = scans.lock().await;
        if let Some(scan) = scans_guard.get_mut(scan_id) {
            scan.state = ScanState::ScanError.into();
            scan.error_message = Some(error);
        }
    }

    async fn create_scan_inner(
        &self,
        req: CreateScanRequest,
    ) -> Result<Response<CreateScanResponse>, Status> {
        let config = req
            .config
            .ok_or_else(|| Status::invalid_argument("Missing scan config"))?;

        // Validate configuration
        if let Err(e) = self.validate_config(&config).await {
            return Ok(Response::new(CreateScanResponse {
                success: false,
                scan_id: String::new(),
                error_message: e,
                total_points: 0,
            }));
        }

        let total_points = Self::calculate_total_points(&config);
        let scan_id = format!(
            "scan-{}",
            self.next_scan_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );

        let execution = ScanExecution::new(config, total_points);
        self.scans.lock().await.insert(scan_id.clone(), execution);

        Ok(Response::new(CreateScanResponse {
            success: true,
            scan_id,
            error_message: String::new(),
            total_points,
        }))
    }

    async fn start_scan_inner(
        &self,
        req: StartScanRequest,
    ) -> Result<Response<StartScanResponse>, Status> {
        let mut scans_guard = self.scans.lock().await;
        let scan = scans_guard
            .get_mut(&req.scan_id)
            .ok_or_else(|| Status::not_found(format!("Scan not found: {}", req.scan_id)))?;

        let created_state: i32 = ScanState::ScanCreated.into();
        if scan.state != created_state {
            return Err(Status::failed_precondition(format!(
                "Scan is not in CREATED state (current: {:?})",
                ScanState::try_from(scan.state)
            )));
        }

        let start_time_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        scan.state = ScanState::ScanRunning.into();
        scan.start_time = Some(Instant::now());
        scan.start_time_ns = start_time_ns;

        // Clone what we need for the background task
        let registry = self.registry.clone();
        let scan_id = req.scan_id.clone();
        let config = scan.config.clone();
        let scans = self.scans.clone();
        let pause_requested = scan.pause_requested.clone();
        let stop_requested = scan.stop_requested.clone();
        let progress_tx = scan.progress_tx.clone();

        // Release first lock before spawning to avoid deadlock
        drop(scans_guard);

        // Spawn background task
        let handle = tokio::spawn(Self::run_scan(
            registry,
            scan_id.clone(),
            config,
            scans.clone(),
            pause_requested,
            stop_requested,
            progress_tx,
            self.ring_buffer.clone(),
        ));

        // Store handle (safe to lock now since we dropped the first guard)
        let mut scans_guard2 = self.scans.lock().await;
        if let Some(scan) = scans_guard2.get_mut(&req.scan_id) {
            scan.task_handle = Some(handle);
        }

        Ok(Response::new(StartScanResponse {
            success: true,
            error_message: String::new(),
            start_time_ns,
        }))
    }

    async fn pause_scan_inner(
        &self,
        req: PauseScanRequest,
    ) -> Result<Response<PauseScanResponse>, Status> {
        let scans_guard = self.scans.lock().await;
        let scan = scans_guard
            .get(&req.scan_id)
            .ok_or_else(|| Status::not_found(format!("Scan not found: {}", req.scan_id)))?;

        let running_state: i32 = ScanState::ScanRunning.into();
        if scan.state != running_state {
            return Err(Status::failed_precondition("Scan is not running"));
        }

        scan.pause_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);

        Ok(Response::new(PauseScanResponse {
            success: true,
            paused_at_point: scan.current_point,
        }))
    }

    async fn resume_scan_inner(
        &self,
        req: ResumeScanRequest,
    ) -> Result<Response<ResumeScanResponse>, Status> {
        let scans_guard = self.scans.lock().await;
        let scan = scans_guard
            .get(&req.scan_id)
            .ok_or_else(|| Status::not_found(format!("Scan not found: {}", req.scan_id)))?;

        let paused_state: i32 = ScanState::ScanPaused.into();
        if scan.state != paused_state {
            return Err(Status::failed_precondition("Scan is not paused"));
        }

        scan.pause_requested
            .store(false, std::sync::atomic::Ordering::SeqCst);

        Ok(Response::new(ResumeScanResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    async fn stop_scan_inner(
        &self,
        req: StopScanRequest,
    ) -> Result<Response<StopScanResponse>, Status> {
        let scans_guard = self.scans.lock().await;
        let scan = scans_guard
            .get(&req.scan_id)
            .ok_or_else(|| Status::not_found(format!("Scan not found: {}", req.scan_id)))?;

        let current_point = scan.current_point;

        // Signal stop
        scan.stop_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
        scan.pause_requested
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // If emergency stop, also stop all motion devices
        if req.emergency_stop {
            drop(scans_guard); // Release lock before async operations
            let registry = self.registry.read().await;

            // Get config to find axes
            let config = {
                let scans_guard = self.scans.lock().await;
                scans_guard.get(&req.scan_id).map(|s| s.config.clone())
            };

            if let Some(config) = config {
                for axis in &config.axes {
                    if let Some(movable) = registry.get_movable(&axis.device_id) {
                        let _ = movable.stop().await; // Best effort stop
                    }
                }
            }
        }

        Ok(Response::new(StopScanResponse {
            success: true,
            points_completed: current_point,
            error_message: String::new(),
        }))
    }
}

#[tonic::async_trait]
impl ScanService for ScanServiceImpl {
    async fn create_scan(
        &self,
        request: Request<CreateScanRequest>,
    ) -> Result<Response<CreateScanResponse>, Status> {
        let req = request.into_inner();
        self.with_request_deadline("CreateScan", self.create_scan_inner(req))
            .await
    }

    async fn start_scan(
        &self,
        request: Request<StartScanRequest>,
    ) -> Result<Response<StartScanResponse>, Status> {
        let req = request.into_inner();
        self.with_request_deadline("StartScan", self.start_scan_inner(req))
            .await
    }

    async fn pause_scan(
        &self,
        request: Request<PauseScanRequest>,
    ) -> Result<Response<PauseScanResponse>, Status> {
        let req = request.into_inner();
        self.with_request_deadline("PauseScan", self.pause_scan_inner(req))
            .await
    }

    async fn resume_scan(
        &self,
        request: Request<ResumeScanRequest>,
    ) -> Result<Response<ResumeScanResponse>, Status> {
        let req = request.into_inner();
        self.with_request_deadline("ResumeScan", self.resume_scan_inner(req))
            .await
    }

    async fn stop_scan(
        &self,
        request: Request<StopScanRequest>,
    ) -> Result<Response<StopScanResponse>, Status> {
        let req = request.into_inner();
        self.with_request_deadline("StopScan", self.stop_scan_inner(req))
            .await
    }

    async fn get_scan_status(
        &self,
        request: Request<GetScanStatusRequest>,
    ) -> Result<Response<ScanStatus>, Status> {
        let req = request.into_inner();

        let scans_guard = self.scans.lock().await;
        let scan = scans_guard
            .get(&req.scan_id)
            .ok_or_else(|| Status::not_found(format!("Scan not found: {}", req.scan_id)))?;

        Ok(Response::new(scan.to_status(&req.scan_id)))
    }

    async fn list_scans(
        &self,
        request: Request<ListScansRequest>,
    ) -> Result<Response<ListScansResponse>, Status> {
        let req = request.into_inner();

        let scans_guard = self.scans.lock().await;
        let mut statuses: Vec<ScanStatus> = Vec::new();

        for (scan_id, scan) in scans_guard.iter() {
            if let Some(filter) = req.state_filter {
                let filter_state: i32 = filter.into();
                if scan.state != filter_state {
                    continue;
                }
            }
            statuses.push(scan.to_status(scan_id));
        }

        Ok(Response::new(ListScansResponse { scans: statuses }))
    }

    type StreamScanProgressStream =
        Pin<Box<dyn Stream<Item = Result<ScanProgress, Status>> + Send>>;

    async fn stream_scan_progress(
        &self,
        request: Request<StreamScanProgressRequest>,
    ) -> Result<Response<Self::StreamScanProgressStream>, Status> {
        let req = request.into_inner();

        let scans_guard = self.scans.lock().await;
        let scan = scans_guard
            .get(&req.scan_id)
            .ok_or_else(|| Status::not_found(format!("Scan not found: {}", req.scan_id)))?;

        // Take the receiver (can only be taken once)
        let rx = scan
            .progress_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| Status::already_exists("Progress stream already taken"))?;

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mapped_stream = tokio_stream::StreamExt::map(stream, |progress| Ok(progress));

        Ok(Response::new(Box::pin(mapped_stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::proto::AxisConfig;

    #[test]
    fn test_calculate_total_points_single_axis() {
        let config = ScanConfig {
            axes: vec![AxisConfig {
                device_id: "stage".to_string(),
                start_position: 0.0,
                end_position: 10.0,
                num_points: 11,
            }],
            scan_type: ScanType::LineScan.into(),
            ..Default::default()
        };

        assert_eq!(ScanServiceImpl::calculate_total_points(&config), 11);
    }

    #[test]
    fn test_calculate_total_points_grid() {
        let config = ScanConfig {
            axes: vec![
                AxisConfig {
                    device_id: "x".to_string(),
                    start_position: 0.0,
                    end_position: 10.0,
                    num_points: 11,
                },
                AxisConfig {
                    device_id: "y".to_string(),
                    start_position: 0.0,
                    end_position: 5.0,
                    num_points: 6,
                },
            ],
            scan_type: ScanType::GridScan.into(),
            ..Default::default()
        };

        assert_eq!(ScanServiceImpl::calculate_total_points(&config), 66); // 11 * 6
    }

    #[test]
    fn test_generate_scan_points_line() {
        let config = ScanConfig {
            axes: vec![AxisConfig {
                device_id: "stage".to_string(),
                start_position: 0.0,
                end_position: 2.0,
                num_points: 3,
            }],
            scan_type: ScanType::LineScan.into(),
            ..Default::default()
        };

        let points = ScanServiceImpl::generate_scan_points(&config);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], vec![0.0]);
        assert_eq!(points[1], vec![1.0]);
        assert_eq!(points[2], vec![2.0]);
    }

    /// Test backpressure handling for progress channel (bd-6qaj)
    ///
    /// Verifies that try_send correctly handles:
    /// 1. Channel full - returns TrySendError::Full
    /// 2. Channel closed - returns TrySendError::Closed
    #[tokio::test]
    async fn test_progress_channel_backpressure() {
        // Create a tiny channel to easily trigger backpressure
        let (tx, mut rx) = mpsc::channel::<ScanProgress>(2);

        // Fill the channel
        let progress1 = ScanProgress {
            scan_id: "test-1".to_string(),
            point_index: 0,
            total_points: 10,
            ..Default::default()
        };
        let progress2 = ScanProgress {
            scan_id: "test-1".to_string(),
            point_index: 1,
            total_points: 10,
            ..Default::default()
        };
        let progress3 = ScanProgress {
            scan_id: "test-1".to_string(),
            point_index: 2,
            total_points: 10,
            ..Default::default()
        };

        // First two sends should succeed
        assert!(tx.try_send(progress1).is_ok());
        assert!(tx.try_send(progress2).is_ok());

        // Third send should fail with Full (channel capacity is 2)
        match tx.try_send(progress3) {
            Err(mpsc::error::TrySendError::Full(_)) => {} // Expected
            Ok(_) => panic!("Expected channel to be full"),
            Err(e) => panic!("Expected Full error, got {:?}", e),
        }

        // Drain one message
        let _ = rx.recv().await;

        // Now send should succeed again
        let progress4 = ScanProgress {
            scan_id: "test-1".to_string(),
            point_index: 3,
            total_points: 10,
            ..Default::default()
        };
        assert!(tx.try_send(progress4).is_ok());

        // Close the receiver
        drop(rx);

        // Send should fail with Closed
        let progress5 = ScanProgress {
            scan_id: "test-1".to_string(),
            point_index: 4,
            total_points: 10,
            ..Default::default()
        };
        match tx.try_send(progress5) {
            Err(mpsc::error::TrySendError::Closed(_)) => {} // Expected
            Ok(_) => panic!("Expected channel to be closed"),
            Err(e) => panic!("Expected Closed error, got {:?}", e),
        }
    }
}
