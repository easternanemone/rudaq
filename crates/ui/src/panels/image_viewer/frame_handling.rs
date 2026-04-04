//! Frame handling - RGBA conversion, echelle extraction, streaming, recording.
//!
//! Background processing, frame pipelines, and stream lifecycle.

use super::*;

impl ImageViewerPanel {
    pub(super) fn poll_echelle_profile_cache(&mut self) {
        match self.echelle_profile_cache.poll_reload_if_changed() {
            EchelleProfileCacheEvent::Unchanged => {}
            EchelleProfileCacheEvent::Loaded(path) => {
                self.mark_echelle_run_engine_sync_dirty();
                self.error = None;
                self.echelle_preview_error = None;
                let path_str = path.display().to_string();
                self.echelle_cal_ui.save_as_path_text.clone_from(&path_str);
                self.echelle_cal_ui.record_recent_profile_path(&path_str);
                if !self.echelle_cal_ui.editor_dirty {
                    if let Some(profile) = self.echelle_profile_cache.profile() {
                        self.echelle_cal_ui.editor_profile = Some((**profile).clone());
                        self.echelle_cal_ui.editor_last_loaded_path = Some(path.clone());
                        self.echelle_cal_ui.status_message =
                            Some(format!("Editor synced from {}", path.display()));
                        self.echelle_cal_ui.last_error = None;
                    }
                } else {
                    self.echelle_cal_ui.status_message = Some(format!(
                        "Active profile reloaded from {} (editor has unsaved changes)",
                        path.display()
                    ));
                }
                self.status = Some(format!("Echelle profile loaded: {}", path.display()));
            }
            EchelleProfileCacheEvent::Error(msg) => {
                // Preserve last-good profile inside the cache; only surface the error.
                self.error = Some(msg);
            }
            EchelleProfileCacheEvent::Cleared => {
                self.mark_echelle_run_engine_sync_dirty();
                self.echelle_cal_ui.editor_last_loaded_path = None;
                self.status = Some("Echelle profile cleared".to_string());
            }
        }
    }

    /// Spawn background thread for RGBA conversion (bd-xifj)
    ///
    /// This moves CPU-intensive pixel conversion off the UI thread to prevent
    /// UI freezes on 4K 16-bit images at high frame rates.
    ///
    /// Returns true if the converter thread was spawned successfully, false otherwise.
    /// On failure, RGBA conversion will fall back to synchronous mode.
    fn spawn_rgba_converter(&mut self) -> bool {
        // Use bounded channel to prevent unbounded queue growth
        // Queue size of 2 is sufficient: 1 processing, 1 waiting
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<RgbaConversionRequest>(2);
        let (result_tx, result_rx) = std::sync::mpsc::channel::<RgbaConversionResult>();
        // Channel for recycling buffers from UI thread back to converter (bd-wdx3)
        let (recycle_tx, recycle_rx) = std::sync::mpsc::channel::<Vec<u8>>();

        // Spawn dedicated thread for RGBA conversion
        let spawn_result = std::thread::Builder::new()
            .name("rgba-converter".into())
            .spawn(move || {
                tracing::debug!("RGBA converter thread started");

                while let Ok(req) = request_rx.recv() {
                    // Get a buffer to reuse: prefer recycled, else allocate new (bd-wdx3)
                    let mut buffer = recycle_rx
                        .try_recv()
                        .unwrap_or_else(|_| Vec::with_capacity(1920 * 1080 * 4));

                    // Perform CPU-intensive conversion
                    let (computed_min, computed_max) =
                        convert_frame_to_rgba_into(&req, &mut buffer);

                    // Send result back to UI thread - move buffer ownership (no clone!)
                    let result = RgbaConversionResult {
                        rgba: buffer,
                        width: req.width,
                        height: req.height,
                        frame_number: req.frame_number,
                        computed_min,
                        computed_max,
                    };

                    if result_tx.send(result).is_err() {
                        // Receiver dropped, exit thread
                        tracing::debug!("RGBA converter result receiver dropped, exiting");
                        break;
                    }
                }

                tracing::debug!("RGBA converter thread exiting");
            });

        match spawn_result {
            Ok(_handle) => {
                self.rgba_request_tx = Some(request_tx);
                self.rgba_rx = Some(result_rx);
                self.rgba_recycle_tx = Some(recycle_tx);
                true
            }
            Err(e) => {
                tracing::error!(
                    "Failed to spawn RGBA converter thread: {}. Falling back to synchronous conversion.",
                    e
                );
                self.rgba_sync_mode = true;
                false
            }
        }
    }

    /// Poll for completed RGBA conversions from background thread (bd-xifj)
    fn poll_rgba_results(&mut self) {
        if let Some(rx) = &self.rgba_rx {
            // Drain all available results, keeping only the most recent
            let mut latest: Option<RgbaConversionResult> = None;
            while let Ok(result) = rx.try_recv() {
                latest = Some(result);
            }
            if latest.is_some() {
                self.pending_rgba = latest;
            }
        }
    }

    /// Submit frame for background RGBA conversion (bd-xifj)
    ///
    /// Returns true if frame was submitted, false if queue is full (frame dropped)
    fn submit_for_rgba_conversion(&mut self, frame: &FrameUpdate) -> bool {
        // Spawn converter thread lazily on first use (skip if already known unavailable)
        if self.rgba_request_tx.is_none() && !self.rgba_sync_mode {
            self.spawn_rgba_converter();
        }

        let request = RgbaConversionRequest {
            data: frame.data.clone(),
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            frame_number: frame.frame_number,
            colormap: self.colormap,
            scale_mode: self.scale_mode,
            display_min: self.display_min,
            display_max: self.display_max,
            auto_contrast: self.auto_contrast,
            contrast_mode: self.contrast_mode,
            percentile_low: self.percentile_low,
            percentile_high: self.percentile_high,
            colorbar_midpoint: self.colorbar.midpoint,
        };

        if let Some(tx) = &self.rgba_request_tx {
            match tx.try_send(request) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_)) => {
                    // Queue full, frame will be dropped (normal under load)
                    false
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    // Thread died, clear sender to trigger respawn
                    self.rgba_request_tx = None;
                    false
                }
            }
        } else {
            // No background thread (e.g., WASM): convert synchronously on the UI thread
            let mut buffer = Vec::with_capacity(frame.width as usize * frame.height as usize * 4);
            let (computed_min, computed_max) = convert_frame_to_rgba_into(&request, &mut buffer);
            self.pending_rgba = Some(RgbaConversionResult {
                rgba: buffer,
                width: request.width,
                height: request.height,
                frame_number: request.frame_number,
                computed_min,
                computed_max,
            });
            true
        }
    }

    /// Apply pending RGBA result to texture (bd-xifj)
    fn apply_pending_rgba(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.pending_rgba.take() {
            // Update auto-contrast display values
            if self.auto_contrast {
                self.display_min = result.computed_min;
                self.display_max = result.computed_max;
            }

            // Create or update texture
            let size = [result.width as usize, result.height as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &result.rgba);

            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.texture =
                    Some(ctx.load_texture("camera_frame", image, egui::TextureOptions::NEAREST));
            }

            // Recycle the buffer back to the converter thread (bd-wdx3)
            if let Some(tx) = &self.rgba_recycle_tx {
                let _ = tx.send(result.rgba);
            }
        }
    }

    // -- Background Echelle Extraction (bd-fwyp) --

    /// Spawn a dedicated thread for echelle extraction.
    ///
    /// Mirrors the RGBA converter pattern: bounded request channel, unbounded result channel.
    /// The worker thread owns its own u16 scratch buffer for 12/16-bit decode.
    fn spawn_echelle_extractor(&mut self) -> bool {
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<EchelleExtractionRequest>(2);
        let (result_tx, result_rx) = std::sync::mpsc::channel::<EchelleExtractionResult>();

        let spawn_result = std::thread::Builder::new()
            .name("echelle-extractor".into())
            .spawn(move || {
                tracing::debug!("Echelle extractor thread started");
                let mut u16_scratch = Vec::new();

                while let Ok(req) = request_rx.recv() {
                    let t0 = std::time::Instant::now();
                    let preview = extract_preview_with_u16_scratch(
                        &req.profile,
                        &req.data,
                        req.width,
                        req.height,
                        req.bit_depth,
                        req.frame_number,
                        &mut u16_scratch,
                    );
                    let extract_ms = t0.elapsed().as_secs_f64() * 1000.0;

                    let result = EchelleExtractionResult {
                        preview,
                        extract_ms,
                        frame_number: req.frame_number,
                    };

                    if result_tx.send(result).is_err() {
                        tracing::debug!("Echelle extractor result receiver dropped, exiting");
                        break;
                    }
                }

                tracing::debug!("Echelle extractor thread exiting");
            });

        match spawn_result {
            Ok(_handle) => {
                self.echelle_extract_tx = Some(request_tx);
                self.echelle_extract_rx = Some(result_rx);
                true
            }
            Err(e) => {
                tracing::error!(
                    "Failed to spawn echelle extractor thread: {}. Falling back to synchronous extraction.",
                    e
                );
                self.echelle_sync_mode = true;
                false
            }
        }
    }

    /// Poll for completed echelle extractions from background thread (bd-fwyp)
    fn poll_echelle_results(&mut self) {
        if let Some(rx) = &self.echelle_extract_rx {
            let mut latest: Option<EchelleExtractionResult> = None;
            while let Ok(result) = rx.try_recv() {
                latest = Some(result);
            }
            if latest.is_some() {
                self.pending_echelle = latest;
            }
        }
    }

    /// Apply pending echelle extraction result to panel state (bd-fwyp)
    fn apply_pending_echelle(&mut self) {
        if let Some(result) = self.pending_echelle.take() {
            self.echelle_last_extract_ms = Some(result.extract_ms);
            match result.preview {
                Ok(preview) => {
                    self.echelle_extract_runs = self.echelle_extract_runs.saturating_add(1);
                    let order_count = preview.orders.len();
                    if order_count == 0 {
                        self.echelle_preview = None;
                        self.echelle_preview_error = Some(
                            "Echelle profile has no enabled orders for extraction".to_string(),
                        );
                        return;
                    }
                    if self.echelle_selected_order_plot >= order_count {
                        self.echelle_selected_order_plot = 0;
                    }
                    self.echelle_preview_measurements = preview.to_measurements();
                    self.echelle_preview = Some(preview);
                    self.echelle_preview_error = None;
                }
                Err(err) => {
                    self.echelle_extract_errors = self.echelle_extract_errors.saturating_add(1);
                    // Preserve the last-good preview (bd-zy7y.2) so users
                    // see a stale spectrum rather than a blank panel during
                    // transient extraction failures (e.g., frame timing jitter,
                    // partial frames, or temporary stream interruption).
                    self.echelle_preview_error = Some(err);
                }
            }
        }
    }

    /// Submit frame for background echelle extraction (bd-fwyp)
    ///
    /// Handles decimation gating, profile lookup, and submission.
    /// Falls back to synchronous extraction on WASM or thread spawn failure.
    fn submit_for_echelle_extraction(&mut self, frame: &FrameUpdate) {
        if !self.echelle_extraction_enabled {
            return;
        }

        let decimation = u64::from(self.echelle_extract_every_n_frames.max(1));
        if decimation > 1 && !frame.frame_number.is_multiple_of(decimation) {
            self.echelle_extract_skipped_frames =
                self.echelle_extract_skipped_frames.saturating_add(1);
            return;
        }

        let Some(profile) = self.echelle_profile_cache.profile().cloned() else {
            self.echelle_preview = None;
            self.echelle_preview_error = None;
            return;
        };

        // Check frame/profile compatibility using structured diagnostics (bd-qe8p.1.2).
        // Incompatible profiles are rejected with a user-visible diagnostic instead of
        // silently patching dimensions (the old approach destroyed calibration geometry).
        if frame.width > 0 && frame.height > 0 {
            let compat = profile.check_frame_compatibility(&echelle::EchelleFrameContext {
                width: frame.width,
                height: frame.height,
                bit_depth: Some(frame.bit_depth),
                ..Default::default()
            });
            if !compat.is_usable() {
                let msgs: Vec<String> = compat
                    .diagnostics()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                self.echelle_preview = None;
                self.echelle_preview_error = Some(format!(
                    "Profile incompatible with current frame: {}",
                    msgs.join("; ")
                ));
                return;
            }
        }

        // Spawn extractor thread lazily on first use
        if self.echelle_extract_tx.is_none() && !self.echelle_sync_mode {
            self.spawn_echelle_extractor();
        }

        let request = EchelleExtractionRequest {
            data: frame.data.clone(),
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            frame_number: frame.frame_number,
            profile,
        };

        if let Some(tx) = &self.echelle_extract_tx {
            match tx.try_send(request) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    // Queue full, frame dropped (acceptable under load)
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.echelle_extract_tx = None;
                }
            }
        } else {
            // No background thread (e.g., WASM): extract synchronously
            let t0 = Instant::now();
            let preview = extract_preview_with_u16_scratch(
                &request.profile,
                &request.data,
                request.width,
                request.height,
                request.bit_depth,
                request.frame_number,
                &mut self.echelle_decode_scratch_u16,
            );
            self.pending_echelle = Some(EchelleExtractionResult {
                preview,
                extract_ms: t0.elapsed().as_secs_f64() * 1000.0,
                frame_number: request.frame_number,
            });
            self.apply_pending_echelle();
        }
    }

    /// Attempt a one-shot extraction on the last received frame (bd-zy7y.3).
    ///
    /// Called after profile activation to give immediate feedback on whether
    /// the new profile is compatible with the current frame data.
    pub(super) fn try_immediate_echelle_extraction(&mut self) {
        if !self.echelle_extraction_enabled {
            return;
        }
        let Some(data) = self.last_frame_data.clone() else {
            return;
        };
        if self.width == 0 || self.height == 0 {
            return;
        }
        let Some(profile) = self.echelle_profile_cache.profile().cloned() else {
            return;
        };

        // Check compatibility before extracting
        let compat = profile.check_frame_compatibility(&echelle::EchelleFrameContext {
            width: self.width,
            height: self.height,
            bit_depth: Some(self.bit_depth),
            ..Default::default()
        });
        if !compat.is_usable() {
            let msgs: Vec<String> = compat
                .diagnostics()
                .iter()
                .map(ToString::to_string)
                .collect();
            self.echelle_preview_error = Some(format!(
                "Profile incompatible with current frame: {}",
                msgs.join("; ")
            ));
            return;
        }

        let t0 = crate::time::Instant::now();
        let preview = extract_preview_with_u16_scratch(
            &profile,
            &data,
            self.width,
            self.height,
            self.bit_depth,
            self.frame_count,
            &mut self.echelle_decode_scratch_u16,
        );
        self.pending_echelle = Some(EchelleExtractionResult {
            preview,
            extract_ms: t0.elapsed().as_secs_f64() * 1000.0,
            frame_number: self.frame_count,
        });
        self.apply_pending_echelle();
    }

    /// Get sender for async frame updates (for external frame producers)
    ///
    /// Allows external code to push frames directly without going through gRPC.
    /// Useful for local frame sources or testing.
    #[allow(dead_code)]
    pub fn get_sender(&self) -> Option<FrameUpdateSender> {
        self.frame_tx.clone()
    }

    /// Start streaming frames from a device (public API for external control)
    pub fn start_stream(&mut self, device_id: &str, client: &mut DaqClient, runtime: &Runtime) {
        // Cancel existing subscription and stop server-side stream (non-blocking).
        // The streaming task's cleanup checks stream_generation to avoid killing the new stream.
        if let Some(sub) = self.subscription.take() {
            let cancel_tx = sub.cancel_tx.clone();
            let mut client = client.clone();
            let old_device_id = sub.device_id.clone();
            tracing::info!(
                old_device = %old_device_id,
                new_device = %device_id,
                "Cancelling existing stream before starting new one"
            );
            // Non-blocking cancellation: fire-and-forget the cancel signal and
            // stop_stream. The streaming task's cleanup checks stream_generation
            // to avoid killing the new stream.
            let new_device_id = device_id.to_string();
            runtime.spawn(async move {
                let _ = cancel_tx.send(()).await;
                // Skip server-side stop when reconnecting to the same device —
                // otherwise this background stop could kill the newly started stream.
                if old_device_id != new_device_id
                    && let Err(e) = client.stop_stream(&old_device_id).await
                {
                    tracing::debug!(
                        device = %old_device_id,
                        error = %e,
                        "Error stopping old stream (may already be stopped)"
                    );
                }
            });
        }

        self.device_id = Some(device_id.to_string());
        self.mark_echelle_run_engine_sync_dirty();
        self.error = None;
        self.status = Some(format!("Connecting to {}...", device_id));
        // bd-12qt: Update connection state
        self.connection_state = ConnectionState::Reconnecting;

        let Some(frame_tx) = self.frame_tx.clone() else {
            self.error = Some("Internal error: no frame channel".to_string());
            return;
        };

        let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut client = client.clone();
        let action_tx = self.action_tx.clone();
        let device_id_clone = device_id.to_string();
        let max_fps = self.max_fps;
        let stream_quality = self.stream_quality;
        let generation = self.stream_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let stream_gen = self.stream_generation.clone();

        runtime.spawn(async move {
            use futures::StreamExt;

            // 1. Start hardware-side streaming on the daemon
            // Treat "already streaming" as success (idempotent behavior)
            let start_result = client.start_stream(&device_id_clone, None).await;
            if let Err(e) = &start_result {
                // Check if this is "already streaming" - treat as non-fatal
                let error_str = e.to_string().to_lowercase();
                let is_already_streaming = error_str.contains("already streaming")
                    || error_str.contains("failedprecondition");

                if is_already_streaming {
                    tracing::info!(
                        device_id = %device_id_clone,
                        "Device already streaming; proceeding to subscribe"
                    );
                } else {
                    tracing::error!(device_id = %device_id_clone, error = %e, "Failed to start hardware stream");
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to start hardware stream: {}",
                        e
                    )));
                    return;
                }
            }

            // 2. Subscribe to the frame stream with quality setting
            let stream = match client.stream_frames(&device_id_clone, max_fps, stream_quality).await {
                Ok(s) => s,
                Err(e) => {
                    // Clean up: stop stream if we started it successfully
                    if start_result.is_ok() {
                        let _ = client.stop_stream(&device_id_clone).await;
                    }
                    tracing::error!(device_id = %device_id_clone, error = %e, "Failed to subscribe to frame stream");
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to subscribe to frames: {}",
                        e
                    )));
                    return;
                }
            };

            tokio::pin!(stream);

            tracing::info!(
                device_id = %device_id_clone,
                max_fps = max_fps,
                quality = ?stream_quality,
                "Frame streaming started - entering receive loop"
            );

            let mut frames_received = 0u64;
            let mut frames_dropped = 0u64;
            // Reusable decompression buffer — avoids per-frame Vec allocation
            let mut decompress_buf = Vec::new();

            // Timeout for stream inactivity (30s) to prevent hanging on network faults (bd-7rk0)
            const STREAM_TIMEOUT: Duration = Duration::from_secs(30);

            // Track why the loop exited for debugging
            let exit_reason: &str;

            loop {
                tokio::select! {
                    _ = cancel_rx.recv() => {
                        tracing::info!(
                            device_id = %device_id_clone,
                            frames_received = frames_received,
                            "Frame stream cancelled by user/system"
                        );
                        exit_reason = "cancelled";
                        break;
                    }
                    () = crate::runtime::sleep(STREAM_TIMEOUT) => {
                        tracing::warn!(
                            device_id = %device_id_clone,
                            timeout_secs = STREAM_TIMEOUT.as_secs(),
                            frames_received = frames_received,
                            "Frame stream timeout - no frames received in timeout period"
                        );
                        let _ = action_tx.send(ImageViewerAction::Error(format!(
                            "Frame stream timeout (no frames for {}s)", STREAM_TIMEOUT.as_secs()
                        )));
                        exit_reason = "timeout";
                        break;
                    }
                    frame_result = stream.next() => {
                        match frame_result {
                            Some(Ok(mut frame_data)) => {
                                frames_received += 1;

                                // Log EVERY frame for the first 10 frames to debug early disconnect
                                if frames_received <= 10 {
                                    tracing::info!(
                                        device_id = %device_id_clone,
                                        frame = frames_received,
                                        frame_number = frame_data.frame_number,
                                        bytes = frame_data.data.len(),
                                        width = frame_data.width,
                                        height = frame_data.height,
                                        compressed = frame_data.compression != 0,
                                        "Received frame from gRPC (early frame debug)"
                                    );
                                }

                                // Decompress frame if compressed (bd-7rk0: gRPC improvements)
                                // Uses buffer reuse to avoid per-frame allocation
                                if let Err(e) = decompress_frame_into(&mut frame_data, &mut decompress_buf) {
                                    tracing::warn!(
                                        device_id = %device_id_clone,
                                        frame = frames_received,
                                        error = %e,
                                        "Frame decompression failed, skipping frame"
                                    );
                                    continue;
                                }

                                if frames_received > 10 && frames_received.is_multiple_of(30) {
                                    tracing::debug!(
                                        device_id = %device_id_clone,
                                        frame = frames_received,
                                        bytes = frame_data.data.len(),
                                        "Received frame from gRPC"
                                    );
                                }

                                let update = FrameUpdate::from(frame_data);
                                // Use try_send to avoid blocking when queue is full
                                // Dropping frames is preferred over blocking the stream
                                match frame_tx.try_send(update) {
                                    Ok(()) => {
                                        if frames_received <= 10 {
                                            tracing::info!(
                                                device_id = %device_id_clone,
                                                frame = frames_received,
                                                "Frame queued to UI successfully"
                                            );
                                        }
                                    }
                                    Err(mpsc::TrySendError::Full(_)) => {
                                        frames_dropped += 1;
                                        if frames_dropped.is_multiple_of(10) {
                                            tracing::warn!(
                                                device_id = %device_id_clone,
                                                dropped = frames_dropped,
                                                "Frame dropped - UI queue full (slow render loop?)"
                                            );
                                        }
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => {
                                        // Receiver dropped - this shouldn't happen during normal operation
                                        tracing::error!(
                                            device_id = %device_id_clone,
                                            frames_received = frames_received,
                                            "Frame receiver disconnected unexpectedly - UI channel closed"
                                        );
                                        exit_reason = "receiver_disconnected";
                                        break;
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                // Log detailed error info
                                tracing::error!(
                                    device_id = %device_id_clone,
                                    frames_received = frames_received,
                                    error = %e,
                                    error_debug = ?e,
                                    "Frame stream error from gRPC"
                                );
                                let _ = action_tx.send(ImageViewerAction::Error(format!(
                                    "Frame stream error: {}", e
                                )));
                                exit_reason = "grpc_error";
                                break;
                            }
                            None => {
                                // Stream ended normally (server closed)
                                tracing::warn!(
                                    device_id = %device_id_clone,
                                    frames_received = frames_received,
                                    "Frame stream ended - server closed connection"
                                );
                                let _ = action_tx.send(ImageViewerAction::Error(format!(
                                    "Frame stream from {} ended unexpectedly", device_id_clone
                                )));
                                exit_reason = "stream_ended";
                                break;
                            }
                        }
                    }
                }
            }

            tracing::info!(
                device_id = %device_id_clone,
                exit_reason = exit_reason,
                frames_received = frames_received,
                frames_dropped = frames_dropped,
                "Frame stream loop exited"
            );

            // Cleanup: Only stop the server-side stream if this task is still the current generation.
            // If a newer stream has started (generation changed), the new stream's cancellation or
            // its own cleanup will handle stopping.
            if stream_gen.load(Ordering::Relaxed) == generation {
                let _ = client.stop_stream(&device_id_clone).await;
            } else {
                tracing::debug!(
                    device_id = %device_id_clone,
                    task_generation = generation,
                    "Skipping stop_stream - superseded by newer stream"
                );
            }
        });

        self.subscription = Some(FrameStreamSubscription {
            cancel_tx,
            device_id: device_id.to_string(),
        });
    }

    /// Stop streaming and notify server to stop hardware capture
    pub fn stop_stream(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Only bump generation when we can issue stop_stream ourselves.
        // When client is None, let the streaming task's cleanup handle stopping.
        if client.is_some() {
            self.stream_generation.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(sub) = self.subscription.take() {
            let cancel_tx = sub.cancel_tx.clone();
            let device_id = sub.device_id.clone();

            // If client is available, also tell server to stop hardware capture
            if let Some(client) = client {
                let mut client = client.clone();
                runtime.spawn(async move {
                    let _ = cancel_tx.send(()).await;
                    let _ = client.stop_stream(&device_id).await;
                });
            } else {
                runtime.spawn(async move {
                    let _ = cancel_tx.send(()).await;
                });
            }
        }
        self.status = Some("Stream stopped".to_string());
    }

    // -- Recording Methods (bd-3pdi.5.3) --

    /// Start recording camera frames to HDF5
    pub(super) fn start_recording(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        if self.recording_state != RecordingState::Idle {
            return;
        }

        self.recording_state = RecordingState::Starting;
        self.error = None;

        let action_tx = self.action_tx.clone();
        let mut client = client.clone();
        let name = if self.recording_name.is_empty() {
            // Generate name with device ID and timestamp
            let device_suffix = self
                .device_id
                .as_ref()
                .map(|d| format!("_{}", d.replace('/', "_")))
                .unwrap_or_default();
            format!(
                "camera{}_{}",
                device_suffix,
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            )
        } else {
            self.recording_name.clone()
        };

        runtime.spawn(async move {
            match client.start_recording(&name).await {
                Ok(response) => {
                    let _ = action_tx.send(ImageViewerAction::RecordingStarted {
                        output_path: response.output_path,
                    });
                }
                Err(e) => {
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to start recording: {}",
                        e
                    )));
                }
            }
        });
    }

    /// Stop recording camera frames
    pub(super) fn stop_recording(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        if self.recording_state != RecordingState::Recording {
            return;
        }

        self.recording_state = RecordingState::Stopping;
        self.error = None;

        let action_tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.stop_recording().await {
                Ok(response) => {
                    let _ = action_tx.send(ImageViewerAction::RecordingStopped {
                        output_path: response.output_path,
                        file_size_bytes: response.file_size_bytes,
                        total_samples: response.total_samples,
                    });
                }
                Err(e) => {
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to stop recording: {}",
                        e
                    )));
                }
            }
        });
    }

    /// Poll recording status from server
    pub(super) fn poll_recording_status(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        // Only poll every 500ms to avoid spamming
        let should_poll = self
            .last_recording_poll
            .is_none_or(|t| t.elapsed().as_millis() > 500);
        if !should_poll {
            return;
        }

        self.last_recording_poll = Some(Instant::now());

        let action_tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.get_recording_status().await {
                Ok(status) => {
                    let _ = action_tx.send(ImageViewerAction::RecordingStatus(Some(status)));
                }
                Err(_) => {
                    // Silently ignore status poll errors
                }
            }
        });
    }

    /// Drain pending frame updates, keeping only the most recent
    ///
    /// Fully drains the channel to prevent latency buildup.
    /// With bounded channel, producer blocks when queue is full.
    pub(super) fn drain_updates(&mut self, ctx: &egui::Context) {
        // bd-xifj: Poll for completed RGBA conversions from background thread
        self.poll_rgba_results();
        self.apply_pending_rgba(ctx);

        // bd-fwyp: Poll for completed echelle extractions from background thread
        self.poll_echelle_results();
        self.apply_pending_echelle();

        let Some(rx) = &self.frame_rx else { return };

        // Drain ALL pending frames, keeping only the last one
        // This ensures we always display the most recent frame
        let mut latest_frame: Option<FrameUpdate> = None;

        while let Ok(frame) = rx.try_recv() {
            latest_frame = Some(frame);
        }

        // Process only the latest frame
        if let Some(frame) = latest_frame {
            self.process_frame(ctx, frame);
        }
    }

    /// Process a single frame update
    fn process_frame(&mut self, _ctx: &egui::Context, mut frame: FrameUpdate) {
        // Validate frame belongs to currently selected device (bd-tjwm.3)
        if let Some(expected_device) = &self.device_id
            && &frame.device_id != expected_device
        {
            tracing::warn!(
                expected = %expected_device,
                received = %frame.device_id,
                "Dropping frame from unexpected device: mismatch"
            );
            return;
        }

        // Trace processed frames (throttled)
        if frame.frame_number.is_multiple_of(30) {
            tracing::debug!(
                frame = frame.frame_number,
                width = frame.width,
                height = frame.height,
                "Processing frame for display"
            );
        }

        self.fps_counter.tick();
        self.width = frame.width;
        self.height = frame.height;
        self.bit_depth = frame.bit_depth;
        self.frame_count = frame.frame_number;
        self.last_frame_timestamp_ns = frame.timestamp_ns;
        self.error = None;

        // bd-7rk0: Update stream metrics from server
        if frame.metrics.is_some() {
            self.stream_metrics = frame.metrics.take();
        }

        // bd-12qt: Update connection state when receiving frames
        if self.connection_state != ConnectionState::Connected {
            self.connection_state = ConnectionState::Connected;
            self.retry_count = 0;
            self.status = Some("Connected".to_string());
        } else if self.status.as_deref() == Some("Connected") {
            // Only clear the "Connected" status once steady-state is reached;
            // preserve other status messages (e.g., recording, saved).
            self.status = None;
        }

        // Store frame data for ROI statistics
        self.last_frame_data = Some(frame.data.clone());

        // Update ROI statistics if we have an active ROI
        self.roi_selector.update_statistics(
            &frame.data,
            frame.width,
            frame.height,
            frame.bit_depth,
        );

        // Compute pixel statistics when panel is visible (bd-li4i)
        if self.show_pixel_stats {
            self.pixel_statistics = Some(compute_pixel_statistics(&frame.data, frame.bit_depth));
        }

        // Update histogram
        self.histogram
            .from_frame_data(&frame.data, frame.width, frame.height, frame.bit_depth);

        // bd-fwyp: Submit for background echelle extraction (decimated)
        self.submit_for_echelle_extraction(&frame);

        // bd-07j1: Update colorbar range from frame data
        let bit_max = match frame.bit_depth {
            8 => 255.0,
            12 => 4095.0,
            16 => 65535.0,
            _ => 65535.0,
        };
        self.colorbar.min_value = 0.0;
        self.colorbar.max_value = bit_max;

        // bd-xifj: Submit frame for background RGBA conversion to prevent UI freezes
        // The converted RGBA will be applied to texture when polled in drain_updates
        let _submitted = self.submit_for_rgba_conversion(&frame);
        // Note: If submission fails (queue full), frame is dropped which is acceptable
        // under high load - we'll display the next successful frame
    }
}
