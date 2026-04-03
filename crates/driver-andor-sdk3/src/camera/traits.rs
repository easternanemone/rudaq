//! Capability trait implementations: FrameProducer, Triggerable, ExposureControl, Parameterized, Commandable.

use super::{AndorCamera, AndorCameraInner, TapRegistry};
use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{
    Commandable, ExposureControl, FrameObserver, FrameProducer, LoanedFrame, ObserverHandle,
    Parameterized, Triggerable,
};
use common::data::FrameView;
use common::observable::ParameterSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(feature = "camera")]
use andor_sdk3_sys::*;

#[async_trait]
impl FrameProducer for AndorCamera {
    async fn start_stream(&self) -> Result<()> {
        if self.inner.streaming.load(Ordering::Relaxed) {
            anyhow::bail!("Camera is already streaming");
        }

        tracing::info!(
            sdk_handle = self.inner.handle,
            trigger_mode = %self.inner.trigger_mode.get(),
            gate_mode = %self.inner.gate_mode.get(),
            "Andor start_stream requested"
        );
        self.inner.streaming.store(true, Ordering::SeqCst);
        self.inner.frame_count.store(0, Ordering::Relaxed);
        self.inner.frames_dropped.store(0, Ordering::Relaxed);
        self.inner.last_hw_frame_nr.store(-1, Ordering::Relaxed);
        // bd-9id0: Clear sticky error so has_acquisition_error() is session-scoped.
        self.clear_error();
        // bd-zg9e.10: Update device lifecycle state
        let _ = self
            .inner
            .device_state
            .set(crate::types::DeviceState::Streaming)
            .await;

        #[cfg(feature = "camera")]
        {
            let inner = self.inner.clone();
            let handle = inner.handle;

            // ── Step 1: Configure all features BEFORE reading sizes or queuing buffers.
            let trigger_str = self.inner.trigger_mode.get().to_string();
            let gate_str = self.inner.gate_mode.get().to_string();
            let hw_ts_freq = crate::ffi_timeout::ffi_call({
                let inner = inner.clone();
                move || -> u64 {
                    if Self::is_feature_implemented(handle, features::METADATA_ENABLE)
                        .unwrap_or(false)
                    {
                        if let Err(e) =
                            Self::set_bool_feature(handle, features::METADATA_ENABLE, true)
                        {
                            tracing::debug!("Could not enable MetadataEnable: {e}");
                        }
                    }

                    if let Err(e) = Self::set_enum_feature(handle, "CycleMode", "Continuous") {
                        tracing::error!("Failed to set CycleMode: {e}");
                    }

                    if let Err(e) = Self::set_enum_feature(handle, "TriggerMode", &trigger_str) {
                        tracing::error!("Failed to set TriggerMode: {e}");
                    }

                    if let Err(e) = Self::set_enum_feature(handle, "GateMode", &gate_str) {
                        tracing::debug!("GateMode not available (non-gated camera): {e}");
                    }

                    // bd-zg9e.1: When DDG mode is active, ensure DDGOutputSelector
                    // targets the MCP gater so DDGOutputDelay/Width control the gate.
                    if inner.gate_mode.get() == crate::types::GateMode::DDG {
                        if let Err(e) = Self::set_enum_feature(handle, "DDGOutputSelector", "Gater")
                        {
                            tracing::debug!("DDGOutputSelector not available: {e}");
                        }
                        // bd-zg9e.2: Enable MCPIntelligate in DDG mode for UV safety
                        if inner.info.features.mcp_intelligate && inner.mcp_intelligate.get() {
                            if let Err(e) =
                                Self::set_bool_feature(handle, "MCPIntelligentGating", true)
                            {
                                tracing::debug!("MCPIntelligate not available: {e}");
                            }
                        }
                    }

                    if let Err(e) = Self::set_enum_feature(handle, "ShutterMode", "Open") {
                        tracing::debug!("ShutterMode not available: {e}");
                    }

                    if Self::is_feature_implemented(handle, features::TIMESTAMP_CLOCK_FREQUENCY)
                        .unwrap_or(false)
                    {
                        match Self::get_int_feature(handle, features::TIMESTAMP_CLOCK_FREQUENCY) {
                            Ok(freq) if freq > 0 => {
                                tracing::info!(freq, "Hardware timestamp clock frequency (Hz)");
                                freq as u64
                            }
                            Ok(_) => 0,
                            Err(e) => {
                                tracing::debug!("Could not read TimestampClockFrequency: {e}");
                                0
                            }
                        }
                    } else {
                        0
                    }
                }
            }, crate::ffi_timeout::FFI_ACQ_TIMEOUT, "start_stream:configure")
            .await
            .unwrap_or(0);
            inner.hw_timestamp_freq.store(hw_ts_freq, Ordering::Relaxed);

            // ── Step 2: Read frame dimensions AFTER all configuration is complete.
            let (image_size, aoi_width, aoi_height, aoi_stride, pixel_encoding) =
                crate::ffi_timeout::ffi_call(move || -> Result<(usize, u32, u32, usize, String)> {
                    let img_bytes = Self::get_int_feature(handle, "ImageSizeBytes")? as usize;
                    let w = Self::get_int_feature(handle, "AOIWidth")? as u32;
                    let h = Self::get_int_feature(handle, "AOIHeight")? as u32;
                    let stride = Self::get_int_feature(handle, "AOIStride")? as usize;
                    let encoding = Self::get_enum_string(handle, "PixelEncoding")
                        .unwrap_or_else(|_| "Unknown".to_string());
                    Ok((img_bytes, w, h, stride, encoding))
                }, crate::ffi_timeout::FFI_QUERY_TIMEOUT, "start_stream:read_dimensions")
                .await??;

            // Warn if AOI doesn't match full sensor — indicates stale or user-set crop.
            let expected_w = inner.info.sensor_width;
            let expected_h = inner.info.sensor_height;
            if aoi_width != expected_w || aoi_height != expected_h {
                tracing::warn!(
                    aoi_w = aoi_width,
                    aoi_h = aoi_height,
                    sensor_w = expected_w,
                    sensor_h = expected_h,
                    "AOI dimensions don't match sensor — frames will be cropped"
                );
            }

            let bytes_per_pixel: usize = match pixel_encoding.as_str() {
                "Mono16" => 2,
                "Mono12" => 2,
                "Mono12Packed" => {
                    tracing::warn!(
                        "Mono12Packed encoding detected — stride-aware extraction needed"
                    );
                    2
                }
                "Mono32" => 4,
                other => {
                    tracing::warn!(
                        pixel_encoding = other,
                        "Unknown pixel encoding, assuming 16-bit"
                    );
                    2
                }
            };

            tracing::info!(
                image_size,
                aoi_width,
                aoi_height,
                aoi_stride,
                %pixel_encoding,
                bytes_per_pixel,
                "SDK3 acquisition parameters"
            );

            // ── Step 3: Queue buffers and start acquisition.
            let buffer_count = crate::buffer::DEFAULT_BUFFER_COUNT;
            let sdk_buffers = Arc::new(crate::buffer::SdkBufferSet::new(buffer_count, image_size));

            crate::ffi_timeout::ffi_call({
                let sdk_buffers = sdk_buffers.clone();
                move || -> Result<()> {
                    use crate::error::sdk_result;
                    unsafe {
                        for buf in sdk_buffers.iter() {
                            let ret = AT_QueueBuffer(
                                handle,
                                buf.as_ptr(),
                                buf.size() as std::os::raw::c_int,
                            );
                            sdk_result(ret)?;
                        }

                        let feature = to_wide_string("AcquisitionStart");
                        tracing::debug!(
                            sdk_handle = handle,
                            buffer_count,
                            image_size,
                            "Issuing AT_Command(AcquisitionStart) after buffer queue"
                        );
                        let ret = AT_Command(handle, feature.as_ptr());
                        sdk_result(ret)?;
                    }
                    Ok(())
                }
            }, crate::ffi_timeout::FFI_ACQ_TIMEOUT, "start_stream:queue_and_start")
            .await??;

            // Store buffer set on inner so pause_apply_restart can flush/re-queue (bd-71sq)
            *self
                .inner
                .sdk_buffers
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(sdk_buffers.clone());

            let acq_inner = self.inner.clone();
            let acq_handle = tokio::task::spawn(Self::acquisition_loop(
                acq_inner,
                sdk_buffers,
                aoi_width,
                aoi_height,
                bytes_per_pixel,
            ));
            *self.inner.acq_task_handle.lock().await = Some(acq_handle);
        }

        #[cfg(not(feature = "camera"))]
        {
            let inner = self.inner.clone();
            let acq_handle = tokio::task::spawn(Self::mock_acquisition_loop(inner));
            *self.inner.acq_task_handle.lock().await = Some(acq_handle);
        }

        tracing::info!(
            sdk_handle = self.inner.handle,
            "Andor camera streaming started"
        );
        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        if !self.inner.streaming.load(Ordering::Relaxed) {
            tracing::debug!(
                sdk_handle = self.inner.handle,
                "Andor stop_stream called while already stopped"
            );
            return Ok(());
        }

        tracing::info!(
            sdk_handle = self.inner.handle,
            "Andor stop_stream requested"
        );

        self.inner.streaming.store(false, Ordering::SeqCst);
        // bd-zg9e.10: Update device lifecycle state
        let _ = self
            .inner
            .device_state
            .set(crate::types::DeviceState::Ready)
            .await;

        if let Some(handle) = self.inner.acq_task_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            crate::ffi_timeout::ffi_call(move || -> Result<()> {
                use crate::error::sdk_result;
                unsafe {
                    tracing::debug!(sdk_handle = handle, "Issuing AT_Command(AcquisitionStop)");
                    let feature = to_wide_string("AcquisitionStop");
                    let ret = AT_Command(handle, feature.as_ptr());
                    sdk_result(ret)?;

                    tracing::debug!(
                        sdk_handle = handle,
                        "Issuing AT_Flush after AcquisitionStop"
                    );
                    let ret = AT_Flush(handle);
                    sdk_result(ret)?;
                }
                Ok(())
            }, crate::ffi_timeout::FFI_ACQ_TIMEOUT, "stop_stream:stop_and_flush")
            .await??;

            // Clear stored buffer set (bd-71sq)
            *self
                .inner
                .sdk_buffers
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }

        tracing::info!(
            sdk_handle = self.inner.handle,
            "Andor camera streaming stopped"
        );
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        (self.inner.info.sensor_width, self.inner.info.sensor_height)
    }

    fn supports_observers(&self) -> bool {
        true
    }

    fn has_acquisition_error(&self) -> bool {
        self.has_error()
    }

    async fn register_observer(&self, observer: Box<dyn FrameObserver>) -> Result<ObserverHandle> {
        let handle = ObserverHandle(self.inner.next_observer_id.fetch_add(1, Ordering::Relaxed));
        self.inner.tap_registry.register(handle, observer);
        Ok(handle)
    }

    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()> {
        self.inner.tap_registry.unregister(handle);
        Ok(())
    }

    async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<LoanedFrame>,
    ) -> Result<()> {
        *self.inner.primary_tx.lock().await = Some(tx);
        tracing::debug!("Primary output registered");
        Ok(())
    }
}

impl AndorCamera {
    /// Hardware acquisition loop (runs on a tokio task, uses spawn_blocking for FFI).
    #[cfg(feature = "camera")]
    pub(super) async fn acquisition_loop(
        inner: Arc<AndorCameraInner>,
        sdk_buffers: Arc<crate::buffer::SdkBufferSet>,
        aoi_width: u32,
        aoi_height: u32,
        bytes_per_pixel: usize,
    ) {
        let handle = inner.handle;
        let timeout_ms: std::os::raw::c_uint = 10_000;
        let mut transient_retries: u32 = 0;

        while inner.streaming.load(Ordering::SeqCst) {
            let wait_result = tokio::task::spawn_blocking({
                move || -> Result<(usize, usize), crate::error::AndorError> {
                    unsafe {
                        let mut ptr: *mut u8 = std::ptr::null_mut();
                        let mut size: std::os::raw::c_int = 0;
                        let ret = AT_WaitBuffer(handle, &mut ptr, &mut size, timeout_ms);
                        if ret != 0 {
                            return Err(crate::error::AndorError::from_code(ret));
                        }
                        Ok((ptr as usize, size as usize))
                    }
                }
            })
            .await;

            let wait_result = match wait_result {
                Ok(r) => r,
                Err(_) => break,
            };

            let (frame_ptr_addr, frame_size) = match wait_result {
                Ok((ptr_addr, size)) => {
                    transient_retries = 0;
                    (ptr_addr, size)
                }
                Err(e) if e.is_timeout() => {
                    tracing::warn!("AT_WaitBuffer timeout, retrying");
                    continue;
                }
                Err(e) => {
                    if !inner.streaming.load(Ordering::Relaxed) {
                        break;
                    }
                    transient_retries += 1;
                    if transient_retries > 20 {
                        let msg =
                            format!("AT_WaitBuffer error: {e} (after {transient_retries} retries)");
                        tracing::error!("{msg}");
                        Self::set_error(&inner, msg);
                        break;
                    }
                    tracing::debug!(
                        retries = transient_retries,
                        "AT_WaitBuffer transient error: {e}, retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
            };

            let frame_nr = inner.frame_count.fetch_add(1, Ordering::Relaxed) as u64;

            // Frame loss detection (bd-fami)
            {
                let hw_nr = Self::get_int_feature(handle, "FrameCount").unwrap_or(-1) as i32;
                if hw_nr >= 0 {
                    let prev = inner.last_hw_frame_nr.swap(hw_nr, Ordering::Relaxed);
                    if prev >= 0 && hw_nr > prev + 1 {
                        let lost = (hw_nr - prev - 1) as u64;
                        inner.frames_dropped.fetch_add(lost, Ordering::Relaxed);
                        tracing::warn!(hw_prev = prev, hw_now = hw_nr, lost, "Frame loss detected");
                    }
                }
            }

            let pool_result = inner.frame_pool.try_acquire();
            match pool_result {
                Some(mut loaned) => {
                    let pixel_bytes =
                        (aoi_width as usize) * (aoi_height as usize) * bytes_per_pixel;
                    let copy_len = pixel_bytes.min(frame_size).min(loaned.capacity());

                    unsafe {
                        loaned.copy_from_sdk(frame_ptr_addr as *const u8, copy_len);
                    }

                    loaned.frame_number = frame_nr;
                    loaned.width = aoi_width;
                    loaned.height = aoi_height;
                    loaned.bit_depth = 16;

                    let system_time_ns = || -> u64 {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as u64
                    };
                    let freq = inner.hw_timestamp_freq.load(Ordering::Relaxed);
                    loaned.timestamp_ns = if freq > 0 {
                        Self::get_int_feature(handle, features::TIMESTAMP_CLOCK)
                            .ok()
                            .filter(|&ticks| ticks >= 0)
                            .map(|ticks| ((ticks as u128) * 1_000_000_000 / (freq as u128)) as u64)
                            .unwrap_or_else(|| system_time_ns())
                    } else {
                        system_time_ns()
                    };
                    loaned.exposure_ms = inner.exposure_s.get() * 1000.0;
                    loaned.temperature_c = Some(inner.temperature_c.get());
                    let bin = inner.binning.get();
                    loaned.binning = Some((bin.0 as u16, bin.1 as u16));

                    let view = FrameView::new(
                        loaned.width,
                        loaned.height,
                        loaned.bit_depth,
                        loaned.pixel_data(),
                        loaned.frame_number,
                        loaned.timestamp_ns,
                    )
                    .with_exposure(loaned.exposure_ms);
                    inner.tap_registry.notify(&view);

                    if let Some(tx) = inner.primary_tx.lock().await.as_ref() {
                        if tx.try_send(loaned).is_err() {
                            let dropped = inner.frames_dropped.fetch_add(1, Ordering::Relaxed) + 1;
                            if dropped == 1 || dropped.is_multiple_of(100) {
                                tracing::warn!(
                                    frame = frame_nr,
                                    total_dropped = dropped,
                                    "Backpressure: consumer too slow, frame dropped"
                                );
                            }
                        }
                    }
                }
                None => {
                    tracing::warn!("Frame pool exhausted, frame {frame_nr} dropped");
                }
            }

            let requeue_result = tokio::task::spawn_blocking({
                let sdk_buffers = sdk_buffers.clone();
                move || -> Result<()> {
                    use crate::error::sdk_result;
                    let frame_ptr = frame_ptr_addr as *const u8;
                    if let Some(idx) = sdk_buffers.index_for_ptr(frame_ptr) {
                        if let Some(buf) = sdk_buffers.get(idx) {
                            unsafe {
                                let ret = AT_QueueBuffer(
                                    handle,
                                    buf.as_ptr(),
                                    buf.size() as std::os::raw::c_int,
                                );
                                sdk_result(ret)?;
                            }
                        }
                    } else {
                        tracing::error!("WaitBuffer returned unknown pointer");
                    }
                    Ok(())
                }
            })
            .await;

            if let Err(e) = requeue_result {
                tracing::error!("Failed to re-queue buffer: {e}");
                break;
            }
            if let Ok(Err(e)) = requeue_result {
                tracing::error!("AT_QueueBuffer failed: {e}");
                break;
            }
        }

        tracing::debug!("Acquisition loop exited");
    }

    /// Mock acquisition loop for non-hardware builds.
    #[cfg(not(feature = "camera"))]
    pub(super) async fn mock_acquisition_loop(inner: Arc<AndorCameraInner>) {
        use tokio::time::Duration;

        let width = inner.info.sensor_width;
        let height = inner.info.sensor_height;

        while inner.streaming.load(Ordering::Relaxed) {
            let exposure = inner.exposure_s.get();
            tokio::time::sleep(Duration::from_secs_f64(exposure)).await;

            let frame_nr = u64::from(inner.frame_count.fetch_add(1, Ordering::Relaxed));

            if let Some(mut loaned) = inner.frame_pool.try_acquire() {
                let pixel_count = (width as usize) * (height as usize);
                let byte_count = pixel_count * 2;
                let actual_len = byte_count.min(loaned.capacity());

                let offset = (frame_nr % 100) as u16;
                let buf = &mut loaned.pixels[..actual_len];
                #[allow(clippy::cast_possible_truncation)]
                for y in 0..height {
                    for x in 0..width {
                        let idx = ((y * width + x) as usize) * 2;
                        if idx + 1 < actual_len {
                            let value = ((x + y + u32::from(offset)) % 65535) as u16;
                            buf[idx] = value as u8;
                            buf[idx + 1] = (value >> 8) as u8;
                        }
                    }
                }
                loaned.actual_len = actual_len;

                loaned.frame_number = frame_nr;
                loaned.width = width;
                loaned.height = height;
                loaned.bit_depth = 16;
                #[allow(clippy::cast_possible_truncation)]
                {
                    loaned.timestamp_ns = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64;
                }
                loaned.exposure_ms = exposure * 1000.0;
                loaned.temperature_c = Some(inner.temperature_c.get());
                let bin = inner.binning.get();
                #[allow(clippy::cast_possible_truncation)]
                {
                    loaned.binning = Some((bin.0 as u16, bin.1 as u16));
                }

                let view = FrameView::new(
                    loaned.width,
                    loaned.height,
                    loaned.bit_depth,
                    loaned.pixel_data(),
                    loaned.frame_number,
                    loaned.timestamp_ns,
                )
                .with_exposure(loaned.exposure_ms);
                inner.tap_registry.notify(&view);

                if let Some(tx) = inner.primary_tx.lock().await.as_ref() {
                    if tx.try_send(loaned).is_err() {
                        let dropped = inner.frames_dropped.fetch_add(1, Ordering::Relaxed) + 1;
                        if dropped == 1 || dropped.is_multiple_of(100) {
                            tracing::warn!(
                                frame = frame_nr,
                                total_dropped = dropped,
                                "Backpressure: consumer too slow, frame dropped"
                            );
                        }
                    }
                }
            }
        }

        tracing::debug!("Mock acquisition loop exited");
    }
}

#[async_trait]
impl Triggerable for AndorCamera {
    async fn arm(&self) -> Result<()> {
        self.inner.armed.store(true, Ordering::Relaxed);
        tracing::debug!("Camera armed");
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        if !self.inner.armed.load(Ordering::Relaxed) {
            anyhow::bail!("Camera not armed");
        }

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let feature = to_wide_string("SoftwareTrigger");
                    let ret = AT_Command(handle, feature.as_ptr());
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        tracing::debug!("Camera triggered");
        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.inner.armed.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl ExposureControl for AndorCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        if seconds <= 0.0 {
            anyhow::bail!("Exposure must be positive, got {}", seconds);
        }

        self.inner.exposure_s.set(seconds).await?;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(self.inner.exposure_s.get())
    }
}

impl Parameterized for AndorCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}

#[async_trait]
impl Commandable for AndorCamera {
    async fn execute_command(
        &self,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        match command {
            "set_cooling" => {
                let enabled = args
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .ok_or_else(|| anyhow::anyhow!("Missing bool 'enabled' argument"))?;
                self.set_cooling(enabled).await?;
                Ok(serde_json::json!({"cooling": enabled}))
            }
            "set_target_temperature" => {
                let temp = args
                    .get("temperature_c")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow::anyhow!("Missing float 'temperature_c' argument"))?;
                self.set_target_temperature(temp).await?;
                Ok(serde_json::json!({"target_temperature_c": temp}))
            }
            "get_cooling_status" => {
                let status = self.get_cooling_status().await?;
                Ok(serde_json::json!({"status": format!("{:?}", status)}))
            }
            "get_temperature" => {
                let temp = self.get_temperature().await?;
                Ok(serde_json::json!({"temperature_c": temp}))
            }
            "get_frames_dropped" => {
                let dropped = self.frames_dropped();
                Ok(serde_json::json!({"frames_dropped": dropped}))
            }
            "software_trigger" => {
                self.trigger().await?;
                Ok(serde_json::json!({"triggered": true}))
            }
            _ => anyhow::bail!("Unknown command: {command}"),
        }
    }
}

#[async_trait]
impl common::capabilities::GatedCamera for AndorCamera {
    async fn set_gate_mode(&self, mode: &str) -> Result<()> {
        self.set_gate_mode(mode).await
    }

    async fn set_trigger_mode(&self, mode: &str) -> Result<()> {
        self.set_trigger_mode(mode).await
    }

    async fn set_ddg_timing(&self, delay_ps: u64, width_ps: u64) -> Result<()> {
        self.set_ddg_output_delay(delay_ps).await?;
        self.set_ddg_output_width(width_ps).await
    }

    async fn set_mcp_gain(&self, gain: u32) -> Result<()> {
        self.set_mcp_gain(gain).await
    }

    async fn set_intelligate(&self, _enabled: bool) -> Result<()> {
        anyhow::bail!("IntelliGate not yet implemented for Andor SDK3 cameras")
    }

    async fn get_temperature_status(&self) -> Result<common::capabilities::TemperatureStatus> {
        let status = self.get_cooling_status().await?;
        Ok(match status {
            crate::types::CoolingStatus::Stabilised => {
                common::capabilities::TemperatureStatus::Stabilized
            }
            crate::types::CoolingStatus::Stabilising => {
                common::capabilities::TemperatureStatus::Cooling
            }
            _ => common::capabilities::TemperatureStatus::NotStabilized,
        })
    }

    async fn get_temperature(&self) -> Result<f64> {
        self.get_temperature().await
    }

    fn supports_ddg(&self) -> bool {
        self.supports_ddg()
    }

    fn supports_mcp_gain(&self) -> bool {
        self.supports_mcp_gain()
    }
}
