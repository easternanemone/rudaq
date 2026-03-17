//! Trigger, exposure, laser, device command, and lifecycle endpoints.

use super::*;

pub(super) async fn arm(
    svc: &HardwareServiceImpl,
    request: Request<ArmRequest>,
) -> Result<Response<ArmResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let triggerable = require_capability!(
        svc,
        get_triggerable,
        &req.device_id,
        "not found or not triggerable"
    );

    match triggerable.arm().await {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(ArmResponse {
                success: true,
                error_message: String::new(),
                armed: true,
            }))
        }
        Err(e) => {
            let err_msg = e.to_string();
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            let status = map_hardware_error_to_status(&err_msg);
            Err(status)
        }
    }
}

pub(super) async fn trigger(
    svc: &HardwareServiceImpl,
    request: Request<TriggerRequest>,
) -> Result<Response<TriggerResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let triggerable = require_capability!(
        svc,
        get_triggerable,
        &req.device_id,
        "not found or not triggerable"
    );

    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    let timestamp_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    match triggerable.trigger().await {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(TriggerResponse {
                success: true,
                error_message: String::new(),
                trigger_timestamp_ns: timestamp_ns,
            }))
        }
        Err(e) => {
            let err_msg = e.to_string();
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            let status = map_hardware_error_to_status(&err_msg);
            Err(status)
        }
    }
}

pub(super) async fn set_exposure(
    svc: &HardwareServiceImpl,
    request: Request<SetExposureRequest>,
) -> Result<Response<SetExposureResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let exposure_ctrl = require_capability!(
        svc,
        get_exposure_control,
        &req.device_id,
        "not found or has no exposure control"
    );

    // Convert ms to seconds for the trait API
    let exposure_seconds = req.exposure_ms / 1000.0;

    match exposure_ctrl.set_exposure(exposure_seconds).await {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            // Convert seconds back to ms for response
            let actual_seconds = exposure_ctrl
                .get_exposure()
                .await
                .unwrap_or(exposure_seconds);
            Ok(Response::new(SetExposureResponse {
                success: true,
                error_message: String::new(),
                actual_exposure_ms: actual_seconds * 1000.0,
            }))
        }
        Err(e) => {
            let err_msg = e.to_string();
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            // Check for out-of-range errors
            if err_msg.contains("out of range")
                || err_msg.contains("bounds")
                || err_msg.contains("invalid")
            {
                Err(Status::invalid_argument(format!(
                    "Invalid exposure value: {}",
                    req.exposure_ms
                )))
            } else {
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
    }
}

pub(super) async fn get_exposure(
    svc: &HardwareServiceImpl,
    request: Request<GetExposureRequest>,
) -> Result<Response<GetExposureResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let exposure_ctrl = require_capability!(
        svc,
        get_exposure_control,
        &req.device_id,
        "not found or has no exposure control"
    );

    // Convert seconds to ms for response
    match exposure_ctrl.get_exposure().await {
        Ok(seconds) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(GetExposureResponse {
                exposure_ms: seconds * 1000.0,
            }))
        }
        Err(e) => {
            let err_msg = format!("Failed to get exposure: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

pub(super) async fn set_shutter(
    svc: &HardwareServiceImpl,
    request: Request<SetShutterRequest>,
) -> Result<Response<SetShutterResponse>, Status> {
    let req = request.into_inner();

    let shutter_ctrl = require_capability!(
        svc,
        get_shutter_control,
        &req.device_id,
        "not found or has no shutter control"
    );

    let open = req.open;
    match if open {
        shutter_ctrl.open_shutter().await
    } else {
        shutter_ctrl.close_shutter().await
    } {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(SetShutterResponse {
                success: true,
                error_message: String::new(),
                is_open: open,
            }))
        }
        Err(e) => {
            let err_msg = format!("Failed to set shutter: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

pub(super) async fn get_shutter(
    svc: &HardwareServiceImpl,
    request: Request<GetShutterRequest>,
) -> Result<Response<GetShutterResponse>, Status> {
    let req = request.into_inner();

    let shutter_ctrl = require_capability!(
        svc,
        get_shutter_control,
        &req.device_id,
        "not found or has no shutter control"
    );

    match shutter_ctrl.is_shutter_open().await {
        Ok(is_open) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(GetShutterResponse { is_open }))
        }
        Err(e) => {
            let err_msg = format!("Failed to get shutter state: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

pub(super) async fn set_wavelength(
    svc: &HardwareServiceImpl,
    request: Request<SetWavelengthRequest>,
) -> Result<Response<SetWavelengthResponse>, Status> {
    let req = request.into_inner();

    let wavelength_ctrl = require_capability!(
        svc,
        get_wavelength_tunable,
        &req.device_id,
        "not found or has no wavelength control"
    );

    let requested_nm = req.wavelength_nm;
    match wavelength_ctrl.set_wavelength(requested_nm).await {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(SetWavelengthResponse {
                success: true,
                error_message: String::new(),
                actual_wavelength_nm: requested_nm,
            }))
        }
        Err(e) => {
            let err_msg = format!("Failed to set wavelength: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

pub(super) async fn get_wavelength(
    svc: &HardwareServiceImpl,
    request: Request<GetWavelengthRequest>,
) -> Result<Response<GetWavelengthResponse>, Status> {
    let req = request.into_inner();

    let wavelength_ctrl = require_capability!(
        svc,
        get_wavelength_tunable,
        &req.device_id,
        "not found or has no wavelength control"
    );

    match wavelength_ctrl.get_wavelength().await {
        Ok(nm) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(GetWavelengthResponse { wavelength_nm: nm }))
        }
        Err(e) => {
            let err_msg = format!("Failed to get wavelength: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

pub(super) async fn set_emission(
    svc: &HardwareServiceImpl,
    request: Request<SetEmissionRequest>,
) -> Result<Response<SetEmissionResponse>, Status> {
    let req = request.into_inner();
    log::info!(
        ">>> set_emission RPC called: device={}, enabled={}",
        req.device_id,
        req.enabled
    );

    let emission_ctrl = require_capability!(
        svc,
        get_emission_control,
        &req.device_id,
        "not found or has no emission control"
    );

    let enabled = req.enabled;
    match if enabled {
        emission_ctrl.enable_emission().await
    } else {
        emission_ctrl.disable_emission().await
    } {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(SetEmissionResponse {
                success: true,
                error_message: String::new(),
                is_enabled: enabled,
            }))
        }
        Err(e) => {
            let err_msg = format!("Failed to set emission: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

pub(super) async fn get_emission(
    svc: &HardwareServiceImpl,
    request: Request<GetEmissionRequest>,
) -> Result<Response<GetEmissionResponse>, Status> {
    let req = request.into_inner();
    log::info!(">>> get_emission RPC called: device={}", req.device_id);

    let emission_ctrl = require_capability!(
        svc,
        get_emission_control,
        &req.device_id,
        "not found or has no emission control"
    );

    log::info!(">>> get_emission: calling is_emission_enabled()...");
    match emission_ctrl.is_emission_enabled().await {
        Ok(is_enabled) => {
            svc.registry.report_device_success(&req.device_id);
            log::info!(">>> get_emission: is_enabled={}", is_enabled);
            Ok(Response::new(GetEmissionResponse { is_enabled }))
        }
        Err(e) => {
            let err_msg = format!("Failed to get emission state: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

pub(super) async fn start_stream(
    svc: &HardwareServiceImpl,
    request: Request<StartStreamRequest>,
) -> Result<Response<StartStreamResponse>, Status> {
    let req = request.into_inner();

    // Extract Arc without lock before awaiting hardware
    let frame_producer = require_capability!(
        svc,
        get_frame_producer,
        &req.device_id,
        "not found or not a frame producer"
    );

    // Use frame_count from request (0 or None = continuous)
    let frame_limit = req.frame_count.filter(|&n| n > 0);

    match frame_producer.start_stream_finite(frame_limit).await {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            Ok(Response::new(StartStreamResponse {
                success: true,
                error_message: String::new(),
            }))
        }
        Err(e) => {
            let err_msg = e.to_string();
            // Idempotent: treat "already streaming" as success
            if err_msg.to_lowercase().contains("already streaming") {
                svc.registry.report_device_success(&req.device_id);
                tracing::info!(device_id = %req.device_id, "Device already streaming (idempotent success)");
                Ok(Response::new(StartStreamResponse {
                    success: true,
                    error_message: "Already streaming".to_string(),
                }))
            } else {
                svc.registry.report_device_failure(&req.device_id, &err_msg);
                let status = map_hardware_error_to_status(&err_msg);
                Err(status)
            }
        }
    }
}

pub(super) async fn stop_stream(
    svc: &HardwareServiceImpl,
    request: Request<StopStreamRequest>,
) -> Result<Response<StopStreamResponse>, Status> {
    let req = request.into_inner();
    tracing::debug!(device_id = %req.device_id, "stop_stream called");

    // Extract Arc without lock before awaiting hardware
    let frame_producer = require_capability!(
        svc,
        get_frame_producer,
        &req.device_id,
        "not found or not a frame producer"
    );

    match frame_producer.stop_stream().await {
        Ok(()) => {
            svc.registry.report_device_success(&req.device_id);
            // Get frame count from device
            let frames_captured = frame_producer.frame_count();
            Ok(Response::new(StopStreamResponse {
                success: true,
                frames_captured,
            }))
        }
        Err(e) => {
            let err_msg = format!("Failed to stop stream: {}", e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            Err(map_hardware_error_to_status(&err_msg))
        }
    }
}

/// Stream frames from a FrameProducer device to GUI clients (bd-0dax.6.3).
///
/// Uses the tap-based observer pattern (`register_observer`) to receive frames.
/// This is more efficient than the deprecated `subscribe_frames()` broadcast
/// approach because:
/// - Observers receive borrowed `FrameView` references (zero-copy from driver)
/// - Downsampling happens in the observer callback (before any channel send)
/// - Backpressure is handled locally in the observer
///
/// Supports optional rate limiting via max_fps.
///
/// Per-client rate limiting (bd-64hu): Each client IP is limited to
/// MAX_STREAMS_PER_CLIENT concurrent frame streams to prevent DoS.
pub(super) async fn stream_frames(
    svc: &HardwareServiceImpl,
    request: Request<StreamFramesRequest>,
) -> Result<Response<ReceiverStream<Result<FrameData, Status>>>, Status> {
    // Extract client IP for rate limiting (bd-64hu)
    let client_ip = request
        .remote_addr()
        .map(|addr| addr.ip())
        .unwrap_or_else(|| IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));

    // Check per-client stream limit (bd-64hu)
    let stream_slot = svc.stream_limiter.try_acquire_guard(client_ip)?;

    let req = request.into_inner();
    let device_id = req.device_id.clone();
    let max_fps = req.max_fps;
    let quality = req.quality();

    // Get frame producer
    let frame_producer = require_capability!(
        svc,
        get_frame_producer,
        &device_id,
        "not found or not a frame producer"
    );

    // Check if device supports observers (bd-0dax.6.3)
    if !frame_producer.supports_observers() {
        return Err(Status::unavailable(format!(
            "Device '{}' does not support frame observers. \
             The driver must implement register_observer() for tap-based streaming.",
            device_id
        )));
    }

    // Channel capacity constants (bd-rgnx.11: increased for streaming throughput)
    // Observer channel: bounded to handle backpressure in on_frame()
    // Increased from 16 to 32 to absorb burst frames and reduce drop rate
    const OBSERVER_CHANNEL_CAPACITY: usize = 32;
    // gRPC channel: buffer for network jitter (bd-7rk0)
    const GRPC_CHANNEL_CAPACITY: usize = 8;
    const GRPC_SKIP_THRESHOLD: usize = 6; // 75% full triggers frame skipping

    // Create channel from observer to forwarding task
    let (observer_tx, mut observer_rx) =
        tokio::sync::mpsc::channel::<ObserverFramePacket>(OBSERVER_CHANNEL_CAPACITY);

    // Create gRPC stream observer
    let observer = GrpcStreamObserver::new(observer_tx, quality, device_id.clone());

    // Register the observer with the frame producer
    let observer_handle = frame_producer
        .register_observer(Box::new(observer))
        .await
        .map_err(|e| {
            Status::internal(format!(
                "Failed to register frame observer for device '{}': {}",
                device_id, e
            ))
        })?;

    tracing::info!(
        device_id = %device_id,
        observer_handle = observer_handle.id(),
        max_fps = max_fps,
        quality = ?quality,
        "Registered gRPC stream observer"
    );

    // Create output channel for gRPC stream
    let (grpc_tx, grpc_rx) = tokio::sync::mpsc::channel(GRPC_CHANNEL_CAPACITY);

    // Calculate minimum interval between frames for rate limiting
    let min_interval = if max_fps > 0 {
        Some(Duration::from_secs_f64(1.0 / f64::from(max_fps)))
    } else {
        None
    };

    // Spawn task to forward frames from observer channel to gRPC stream
    // Use Arc<str> to avoid per-frame String clones in the hot path (bd-rgnx.11)
    let device_id_arc: Arc<str> = Arc::from(device_id.as_str());
    let device_id_clone = device_id.clone();
    let frame_producer_clone = frame_producer.clone();
    let registry_clone = svc.registry.clone();

    // Dedicated compression thread: receives ObserverFramePacket, compresses with
    // buffer reuse, and sends FrameData to the async forwarding task.
    // Eliminates per-frame spawn_blocking overhead (~50-200us) and per-frame
    // compression buffer allocation.
    let (compress_tx, mut compress_rx) = tokio::sync::mpsc::channel::<(
        ObserverFramePacket,
        StreamingMetrics,
    )>(GRPC_CHANNEL_CAPACITY);
    let device_id_for_compressor = Arc::clone(&device_id_arc);
    let (compressed_tx, mut compressed_rx) =
        tokio::sync::mpsc::channel::<(FrameData, usize, usize)>(GRPC_CHANNEL_CAPACITY);

    std::thread::Builder::new()
        .name(format!("lz4-compress-{device_id}"))
        .spawn({
            let compressed_tx = compressed_tx;
            move || {
                let mut compress_buf = Vec::new();

                while let Some((packet, metrics)) = compress_rx.blocking_recv() {
                    let mut frame_data = FrameData {
                        device_id: device_id_for_compressor.to_string(),
                        width: packet.width,
                        height: packet.height,
                        bit_depth: packet.bit_depth,
                        data: packet.data,
                        frame_number: packet.frame_number,
                        timestamp_ns: packet.timestamp_ns,
                        exposure_ms: packet.exposure_ms,
                        roi_x: packet.roi_x,
                        roi_y: packet.roi_y,
                        temperature_c: packet.temperature_c,
                        gain_mode: None,
                        readout_speed: None,
                        trigger_mode: None,
                        binning_x: packet.binning.map(|(x, _)| u32::from(x)),
                        binning_y: packet.binning.map(|(_, y)| u32::from(y)),
                        metadata: HashMap::new(),
                        metrics: Some(metrics),
                        compression: CompressionType::CompressionNone as i32,
                        uncompressed_size: 0,
                    };

                    let uncompressed_size = frame_data.data.len();
                    crate::grpc::compression::compress_frame_into(
                        &mut frame_data,
                        &mut compress_buf,
                    );
                    let compressed_size = frame_data.data.len();

                    if compressed_tx
                        .blocking_send((frame_data, uncompressed_size, compressed_size))
                        .is_err()
                    {
                        break; // Forwarding task dropped — exit
                    }
                }
            }
        })
        .expect("failed to spawn LZ4 compression thread");

    tokio::spawn(async move {
        // Hold the per-client stream slot for the lifetime of this forwarding task.
        // Dropping the guard releases the slot on every exit path (including panic unwind).
        let stream_slot_guard = stream_slot;
        // Initialize to allow first frame through immediately
        let mut last_frame_time = match min_interval {
            Some(interval) => std::time::Instant::now().checked_sub(interval).unwrap(),
            None => std::time::Instant::now(),
        };
        let mut frames_sent = 0u64;
        let mut frames_dropped = 0u64;
        let mut fps_window: VecDeque<std::time::Instant> = VecDeque::new();
        let mut avg_latency_ms = 0.0f64;
        let mut latency_samples = 0u64;
        // Track whether we have reported success this session.
        // Reporting on the first delivered frame resets any accumulated
        // failure count from a previous error, keeping the consecutive-
        // failure semantics of the health tracker meaningful.
        let mut success_reported = false;

        tracing::info!(
            device_id = %device_id_clone,
            max_fps = max_fps,
            quality = ?quality,
            observer_channel_capacity = OBSERVER_CHANNEL_CAPACITY,
            grpc_channel_capacity = GRPC_CHANNEL_CAPACITY,
            "Starting tap-based frame stream forwarding task"
        );

        let exit_reason: &str;

        loop {
            // Multiplex: read from observer (new frames), compression thread (compressed
            // frames ready to send), and watch for gRPC client disconnect.
            tokio::select! {
                // Handle compressed frames ready to send to gRPC client
                compressed_result = compressed_rx.recv() => {
                    match compressed_result {
                        Some((frame_data, uncompressed_size, compressed_size)) => {
                            // Log early frame sends
                            if frames_sent < 10 {
                                tracing::info!(
                                    device_id = %device_id_clone,
                                    frame = frames_sent + 1,
                                    frame_number = frame_data.frame_number,
                                    bytes = frame_data.data.len(),
                                    queue_capacity = grpc_tx.capacity(),
                                    "About to send frame to gRPC client (early frame debug)"
                                );
                            }

                            // Send to gRPC client
                            if grpc_tx.send(Ok(frame_data)).await.is_err() {
                                tracing::warn!(
                                    device_id = %device_id_clone,
                                    frames_sent = frames_sent,
                                    "Client disconnected from frame stream - gRPC send failed"
                                );

                                if let Err(e) = frame_producer_clone
                                    .unregister_observer(observer_handle)
                                    .await
                                {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        observer_handle = observer_handle.id(),
                                        error = %e,
                                        "Failed to unregister observer on client disconnect"
                                    );
                                } else {
                                    tracing::info!(
                                        device_id = %device_id_clone,
                                        observer_handle = observer_handle.id(),
                                        "Unregistered observer after client disconnect"
                                    );
                                }

                                exit_reason = "client_disconnected";
                                break;
                            }

                            // On the first successfully delivered frame, reset the device's
                            // consecutive-failure counter so that transient streaming errors
                            // do not accumulate toward Faulted once the stream recovers.
                            if !success_reported {
                                registry_clone.report_device_success(&device_id_clone);
                                success_reported = true;
                            }

                            // Log early frame sends success
                            if frames_sent <= 10 {
                                tracing::info!(
                                    device_id = %device_id_clone,
                                    frame = frames_sent,
                                    "Successfully sent frame to gRPC client (early frame debug)"
                                );
                            }

                            // Log compression stats periodically
                            if frames_sent > 10 && frames_sent.is_multiple_of(30) {
                                #[allow(clippy::cast_precision_loss)]
                                // SAFETY: precision loss acceptable for metrics/display
                                let ratio = if compressed_size > 0 {
                                    uncompressed_size as f64 / compressed_size as f64
                                } else {
                                    1.0
                                };
                                tracing::debug!(
                                    device_id = %device_id_clone,
                                    frames = frames_sent,
                                    uncompressed_kb = uncompressed_size / 1024,
                                    compressed_kb = compressed_size / 1024,
                                    compression_ratio = format!("{:.1}x", ratio),
                                    "Sent frame to client (LZ4 compressed)"
                                );
                            }
                        }
                        None => {
                            // Compression thread exited
                            tracing::info!(
                                device_id = %device_id_clone,
                                frames_sent = frames_sent,
                                "Compression thread exited"
                            );
                            exit_reason = "compressor_closed";
                            break;
                        }
                    }
                }
                // Handle new frames from the observer channel
                next_packet = observer_rx.recv() => {
                    match next_packet {
                        Some(mut packet) => {
                            // Log early frames for debugging
                            if frames_sent < 10 {
                                tracing::info!(
                                    device_id = %device_id_clone,
                                    frame_number = packet.frame_number,
                                    bytes = packet.data.len(),
                                    width = packet.width,
                                    height = packet.height,
                                    "Received frame from observer (early frame debug)"
                                );
                            }

                            // Rate limiting: skip frame if too soon
                            if let Some(interval) = min_interval {
                                let elapsed = last_frame_time.elapsed();
                                if elapsed < interval {
                                    frames_dropped = frames_dropped.saturating_add(1);
                                    continue;
                                }
                            }
                            last_frame_time = std::time::Instant::now();

                            // Backpressure handling: skip frames if gRPC channel is nearly full
                            let queue_len = GRPC_CHANNEL_CAPACITY - grpc_tx.capacity();
                            if queue_len >= GRPC_SKIP_THRESHOLD {
                                frames_dropped = frames_dropped.saturating_add(1);
                                if frames_dropped % 10 == 1 {
                                    tracing::debug!(
                                        device_id = %device_id_clone,
                                        queue_len,
                                        threshold = GRPC_SKIP_THRESHOLD,
                                        "Skipping frame due to gRPC backpressure"
                                    );
                                }
                                continue;
                            }

                            // Validate frame dimensions and normalize pixel format (bd-7rk0, bd-q2n6)
                            let pixel_count = (packet.width as usize)
                                .saturating_mul(packet.height as usize);
                            let expected_u16 = pixel_count.saturating_mul(2);
                            let expected_mono12_packed = pixel_count.saturating_mul(3) / 2;

                            if packet.data.len() == expected_u16 {
                                // Mono16 or 12-bit-in-16-bit container — normal path
                            } else if packet.data.len() == expected_mono12_packed
                                || packet.data.len() == expected_mono12_packed + (pixel_count % 2)
                            {
                                // Mono12Packed: 3 bytes per 2 pixels → unpack to u16
                                tracing::warn!(
                                    device_id = %device_id_clone,
                                    actual_size = packet.data.len(),
                                    "Unpacking Mono12 frame to u16 (camera using packed pixel encoding)"
                                );
                                let mut unpacked = Vec::with_capacity(expected_u16);
                                let src = &packet.data;
                                let mut i = 0;
                                while i + 2 < src.len() {
                                    // Mono12Packed: [P0_hi, P0_lo:P1_lo, P1_hi]
                                    let p0 = (u16::from(src[i]) << 4) | u16::from(src[i + 1] & 0x0F);
                                    let p1 = (u16::from(src[i + 2]) << 4) | u16::from(src[i + 1] >> 4);
                                    unpacked.extend_from_slice(&p0.to_le_bytes());
                                    unpacked.extend_from_slice(&p1.to_le_bytes());
                                    i += 3;
                                }
                                unpacked.truncate(expected_u16);
                                packet.data = unpacked;
                                packet.bit_depth = 16;
                            } else {
                                tracing::warn!(
                                    device_id = %device_id_clone,
                                    width = packet.width,
                                    height = packet.height,
                                    bit_depth = packet.bit_depth,
                                    actual_size = packet.data.len(),
                                    expected_u16,
                                    expected_mono12_packed,
                                    "Frame data size does not match any known pixel format, skipping"
                                );
                                frames_dropped = frames_dropped.saturating_add(1);
                                continue;
                            }

                            // Update FPS tracking
                            let now_instant = std::time::Instant::now();
                            fps_window.push_back(now_instant);
                            while let Some(front) = fps_window.front() {
                                if now_instant.duration_since(*front) > FPS_WINDOW {
                                    fps_window.pop_front();
                                } else {
                                    break;
                                }
                            }
                            #[allow(clippy::cast_precision_loss)]
                            // SAFETY: precision loss acceptable for metrics/display
                            let current_fps = fps_window.len() as f64;

                            // Update latency tracking
                            if packet.timestamp_ns > 0 {
                                #[allow(clippy::cast_precision_loss)]
                                // SAFETY: precision loss acceptable for metrics/display
                                let latency_ms =
                                    now_ns().saturating_sub(packet.timestamp_ns) as f64
                                        / 1_000_000.0;
                                latency_samples = latency_samples.saturating_add(1);
                                #[allow(clippy::cast_precision_loss)]
                                // SAFETY: precision loss acceptable for running average
                                let samples_f64 = latency_samples as f64;
                                avg_latency_ms += (latency_ms - avg_latency_ms) / samples_f64;
                            }

                            // Increment before building metrics (matches original behavior
                            // where frames_sent reflects "frames submitted for sending")
                            frames_sent = frames_sent.saturating_add(1);

                            let metrics = StreamingMetrics {
                                current_fps,
                                frames_sent,
                                frames_dropped,
                                avg_latency_ms,
                            };

                            // Non-blocking send: if the compressor is backlogged
                            // we drop this frame rather than awaiting, which would
                            // prevent the select! from draining compressed_rx and
                            // cause a deadlock between the two bounded channels.
                            match compress_tx.try_send((packet, metrics)) {
                                Ok(()) => {}
                                Err(
                                    tokio::sync::mpsc::error::TrySendError::Full(_),
                                ) => {
                                    frames_dropped =
                                        frames_dropped.saturating_add(1);
                                    tracing::debug!(
                                        device_id = %device_id_clone,
                                        frames_dropped,
                                        "Compressor channel full; dropping frame"
                                    );
                                }
                                Err(
                                    tokio::sync::mpsc::error::TrySendError::Closed(
                                        _,
                                    ),
                                ) => {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        "Compression thread channel closed"
                                    );
                                    exit_reason = "compressor_closed";
                                    break;
                                }
                            }
                        }
                        None => {
                            // Observer channel closed - producer stopped or observer was dropped.
                            // Check if this was due to a hardware error (e.g. USB/PCIe disconnect)
                            // so the supervisor can schedule an automatic reconnection attempt.
                            if frame_producer_clone.has_acquisition_error() {
                                tracing::warn!(
                                    device_id = %device_id_clone,
                                    frames_sent = frames_sent,
                                    "Observer channel closed due to acquisition error — \
                                     reporting failure for supervisor reconnection"
                                );
                                registry_clone.report_device_failure(
                                    &device_id_clone,
                                    "acquisition loop exited with hardware error",
                                );
                            } else {
                                tracing::info!(
                                    device_id = %device_id_clone,
                                    frames_sent = frames_sent,
                                    "Observer channel closed - producer stopped streaming"
                                );
                            }

                            // Clean up observer registration
                            if let Err(e) = frame_producer_clone
                                .unregister_observer(observer_handle)
                                .await
                            {
                                tracing::debug!(
                                    device_id = %device_id_clone,
                                    observer_handle = observer_handle.id(),
                                    error = %e,
                                    "Failed to unregister observer (may already be unregistered)"
                                );
                            }

                            exit_reason = "observer_channel_closed";
                            break;
                        }
                    }
                }
                // Handle gRPC client disconnect
                () = grpc_tx.closed() => {
                    tracing::info!(
                        device_id = %device_id_clone,
                        frames_sent = frames_sent,
                        "gRPC frame stream receiver dropped; stopping forwarding task"
                    );

                    if let Err(e) = frame_producer_clone
                        .unregister_observer(observer_handle)
                        .await
                    {
                        tracing::debug!(
                            device_id = %device_id_clone,
                            observer_handle = observer_handle.id(),
                            error = %e,
                            "Failed to unregister observer after gRPC receiver drop (may already be unregistered)"
                        );
                    } else {
                        tracing::info!(
                            device_id = %device_id_clone,
                            observer_handle = observer_handle.id(),
                            "Unregistered observer after gRPC receiver drop"
                        );
                    }

                    exit_reason = "grpc_receiver_dropped";
                    break;
                }
            }
        }
        // Release stream slot (bd-64hu)
        // Dropping compress_tx closes the compression thread channel, causing it to exit
        drop(compress_tx);
        drop(stream_slot_guard);

        // Final summary log
        tracing::info!(
            device_id = %device_id_clone,
            exit_reason = exit_reason,
            frames_sent = frames_sent,
            frames_dropped = frames_dropped,
            client_ip = %client_ip,
            "Tap-based frame stream forwarding task ended"
        );
    });

    Ok(Response::new(ReceiverStream::new(grpc_rx)))
}

pub(super) async fn stage_device(
    svc: &HardwareServiceImpl,
    request: Request<StageDeviceRequest>,
) -> Result<Response<StageDeviceResponse>, Status> {
    let req = request.into_inner();
    let stageable = svc.registry.get_stageable(&req.device_id);
    let exists = svc.registry.contains(&req.device_id);

    // Verify device exists
    if !exists {
        return Err(Status::not_found(format!(
            "Device '{}' not found",
            req.device_id
        )));
    }

    // If device implements Stageable, call stage()
    if let Some(stageable) = stageable {
        stageable.stage().await.map_err(|e| {
            let err_msg = format!("Failed to stage device '{}': {}", req.device_id, e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            map_hardware_error_to_status(&err_msg)
        })?;
        svc.registry.report_device_success(&req.device_id);
        tracing::info!("Staged device '{}' successfully", req.device_id);
    } else {
        // No-op for devices that don't implement Stageable
        tracing::debug!(
            "Staged device '{}' (no Stageable impl, no-op)",
            req.device_id
        );
    }

    Ok(Response::new(StageDeviceResponse {
        success: true,
        error_message: String::new(),
        staged: true,
    }))
}

pub(super) async fn unstage_device(
    svc: &HardwareServiceImpl,
    request: Request<UnstageDeviceRequest>,
) -> Result<Response<UnstageDeviceResponse>, Status> {
    let req = request.into_inner();
    let stageable = svc.registry.get_stageable(&req.device_id);
    let exists = svc.registry.contains(&req.device_id);

    // Verify device exists
    if !exists {
        return Err(Status::not_found(format!(
            "Device '{}' not found",
            req.device_id
        )));
    }

    // If device implements Stageable, call unstage()
    if let Some(stageable) = stageable {
        stageable.unstage().await.map_err(|e| {
            let err_msg = format!("Failed to unstage device '{}': {}", req.device_id, e);
            svc.registry.report_device_failure(&req.device_id, &err_msg);
            map_hardware_error_to_status(&err_msg)
        })?;
        svc.registry.report_device_success(&req.device_id);
        tracing::info!("Unstaged device '{}' successfully", req.device_id);
    } else {
        // No-op for devices that don't implement Stageable
        tracing::debug!(
            "Unstaged device '{}' (no Stageable impl, no-op)",
            req.device_id
        );
    }

    Ok(Response::new(UnstageDeviceResponse {
        success: true,
        error_message: String::new(),
    }))
}

pub(super) async fn execute_device_command(
    svc: &HardwareServiceImpl,
    request: Request<DeviceCommandRequest>,
) -> Result<Response<DeviceCommandResponse>, Status> {
    let req = request.into_inner();

    // Try the new generic Commandable interface first
    if let Some(device) = svc.registry.get_commandable(&req.device_id) {
        // Parse arguments as JSON
        const MAX_ARGS_LEN: usize = 64 * 1024; // 64KB
        if req.args.len() > MAX_ARGS_LEN {
            return Err(Status::invalid_argument(format!(
                "Arguments too large: {} bytes (max {})",
                req.args.len(),
                MAX_ARGS_LEN
            )));
        }

        let args = if req.args.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&req.args).map_err(|e| {
                Status::invalid_argument(format!("Failed to parse command arguments: {}", e))
            })?
        };

        let result = device
            .execute_command(&req.command, args)
            .await
            .map_err(|e| {
                let err_msg = format!("Command execution failed: {}", e);
                svc.registry.report_device_failure(&req.device_id, &err_msg);
                map_hardware_error_to_status(&err_msg)
            })?;

        svc.registry.report_device_success(&req.device_id);
        return Ok(Response::new(DeviceCommandResponse {
            success: true,
            error_message: String::new(),
            results: result.to_string(),
        }));
    }

    // Device doesn't implement Commandable trait
    Err(Status::unimplemented(format!(
        "Device '{}' does not support commands. Use capability-specific endpoints \
         (e.g., SetEmission for emission control) or implement Commandable trait.",
        req.device_id
    )))
}
