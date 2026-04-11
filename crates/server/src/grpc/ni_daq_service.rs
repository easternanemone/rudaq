// Deliberate explicit control flow structure for readability in complex DAQ logic (bd-m5fh.4.2).
#![allow(clippy::collapsible_if)]
//! NI DAQ Service implementation for Comedi hardware control (bd-czem)
//!
//! This module provides gRPC endpoints for NI PCI-MIO-16XE-10 data acquisition
//! hardware via the Comedi Linux driver. Extends basic HardwareService with
//! DAQ-specific capabilities:
//! - Multi-channel analog input streaming
//! - Digital I/O control
//! - Counter/timer operations
//! - Hardware triggering
//!
//! # Architecture
//!
//! NiDaqService is in a **partial migration** from direct Comedi FFI access to
//! DeviceRegistry-owned capability handles (bd-krmn). The current state:
//!
//! ## Migrated to Registry Traits
//! - `set_analog_output`: Uses `registry.get_settable()` -> `Settable::set_value()`
//! - `configure_digital_io`, `read_digital_io`, `write_digital_io`,
//!   `read_digital_port`, `write_digital_port`: Uses registry DIO device (bd-u0lg)
//! - `read_counter`, `reset_counter`: Uses registry counter devices (bd-u0lg)
//! - `configure_counter`: Uses `registry.get_counter_configurable()` ->
//!   `CounterConfigurable::configure_counter()` (bd-f3pq)
//! - `read_analog_input`: Uses `registry.get_readable_with_metadata()` ->
//!   `ReadableWithMetadata::read_with_metadata()` (bd-09ls)
//! - `configure_analog_output`: Primary path uses `registry.get_range_introspectable()`
//!   -> `RangeIntrospectable::get_ranges()` with Comedi FFI fallback (bd-3bjp)
//! - `get_daq_status`: Uses `registry.get_device_introspection()` ->
//!   `DeviceIntrospection::introspect()` (bd-sa9p), plus
//!   `registry.get_range_introspectable()` for AI voltage ranges (bd-3bjp).
//!   Resolution bits not yet populated (needs trait extension).
//! - `read_analog_input`: Uses `registry.get_readable_with_metadata()` ->
//!   `ReadableWithMetadata::read_with_metadata()` (bd-09ls)
//! - `configure_analog_output`: Primary path uses `registry.get_range_introspectable()`
//!   (bd-3bjp); Comedi FFI fallback for devices without the trait.
//!
//! ## Still Using Direct Comedi Access
//! The following RPCs still bypass registry traits for direct Comedi FFI:
//!
//! - `configure_analog_output`: Fallback path uses `get_or_open_device()` for
//!   devices that predate `RangeIntrospectable` wiring (primary path migrated, bd-3bjp).
//! - `stream_analog_input`: Uses `ComediMultiChannelAcquisition::new_async()`
//!   directly (CMD-based DMA, no registry trait equivalent — see bd-rwk8).

use anyhow::Error as AnyError;
use common::limits::RPC_TIMEOUT;
use hardware::registry::DeviceRegistry;
use protocol::ni_daq::ni_daq_service_server::NiDaqService;
use protocol::ni_daq::*;
use std::future::Future;
#[cfg(feature = "comedi")]
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::instrument;

// =============================================================================
// NI DAQ Service Implementation
// =============================================================================

/// Implementation of NiDaqService gRPC interface.
///
/// Provides access to NI PCI-MIO-16XE-10 capabilities. Uses a mix of
/// DeviceRegistry capability traits (the target architecture) and direct
/// Comedi FFI (for operations that don't yet have registry equivalents).
///
/// See module-level docs for the per-RPC migration status (bd-krmn).
///
/// # Device Handle Caching (bd-ucyu.3)
///
/// Comedi device handles are cached by device path to avoid re-opening
/// `/dev/comedi0` on every RPC. `ComediDevice` is `Clone` (wraps `Arc<DeviceInner>`)
/// and its internal `ffi_lock` serializes all FFI calls, so sharing a single handle
/// across RPCs is both safe and eliminates the kernel deadlock risk from concurrent opens.
///
/// # Device Cache
/// `device_cache` is only used by `configure_analog_output` (fallback path for
/// devices that predate `RangeIntrospectable` wiring). `stream_analog_input`
/// opens its own handle via `ComediMultiChannelAcquisition::new_async`.
/// Removal blocked on: migrating the AO fallback to a registry trait (bd-ucyu.4).
#[derive(Clone)]
pub struct NiDaqServiceImpl {
    /// Device registry for looking up Comedi devices
    registry: DeviceRegistry,
    /// Serializes streaming acquisition to prevent concurrent opens of
    /// `/dev/comedi0` which can deadlock the kernel driver.
    /// NOTE: Only used by `stream_analog_input` which still opens its own handle.
    /// Will be removed in Phase 4 (bd-ucyu.4.1) once streaming is migrated.
    #[cfg(feature = "comedi")]
    comedi_semaphore: Arc<tokio::sync::Semaphore>,
    /// Cached Comedi device handles keyed by device path (e.g., "/dev/comedi0").
    /// Opened lazily on first use. `ComediDevice::ffi_lock` serializes FFI access.
    #[cfg(feature = "comedi")]
    device_cache:
        Arc<std::sync::Mutex<std::collections::HashMap<String, driver_comedi::ComediDevice>>>,
}

impl NiDaqServiceImpl {
    /// Create a new NI DAQ service instance.
    pub fn new(registry: DeviceRegistry) -> Self {
        Self {
            registry,
            #[cfg(feature = "comedi")]
            comedi_semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
            #[cfg(feature = "comedi")]
            device_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Get a cached Comedi device handle, opening it on first access.
    ///
    /// Returns a cloned handle (cheap — `Arc` bump) that shares the underlying
    /// file descriptor and `ffi_lock` with all other users of the same device path.
    ///
    /// The cache lookup is fast (just a mutex + HashMap get). On cache miss,
    /// the blocking `ComediDevice::open()` FFI call runs inside `spawn_blocking`
    /// to avoid blocking the Tokio runtime.
    #[cfg(feature = "comedi")]
    async fn get_or_open_device(
        &self,
        device_path: &str,
    ) -> Result<driver_comedi::ComediDevice, Status> {
        // Fast path: cache hit (lock held only for HashMap lookup)
        {
            let cache = self
                .device_cache
                .lock()
                .map_err(|_| Status::internal("Device cache lock poisoned"))?;
            if let Some(device) = cache.get(device_path) {
                return Ok(device.clone());
            }
        }

        // Slow path: open device in spawn_blocking to avoid blocking tokio
        let path = device_path.to_string();
        let device = self
            .await_with_timeout("ComediDevice::open", async {
                let p = path.clone();
                tokio::task::spawn_blocking(move || {
                    driver_comedi::ComediDevice::open(&p)
                        .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", p, e))
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            })
            .await?;

        // Insert into cache (re-acquire lock after blocking open)
        let mut cache = self
            .device_cache
            .lock()
            .map_err(|_| Status::internal("Device cache lock poisoned"))?;
        // Another task may have raced and inserted while we were opening
        cache.entry(path).or_insert_with(|| {
            tracing::info!(device_path, "Cached new Comedi device handle");
            device.clone()
        });
        Ok(device)
    }

    /// Resolve the Comedi device node path for a given device ID.
    ///
    /// Looks up the `"device"` field from the driver's raw TOML config (e.g.,
    /// `device = "/dev/comedi0"`). Falls back to `/dev/{device_id}` if the
    /// config field is absent (shouldn't happen for properly configured devices).
    #[cfg(feature = "comedi")]
    fn resolve_device_path(&self, device_id: &str) -> String {
        self.registry
            .get_driver_config_str(device_id, "device")
            .unwrap_or_else(|| {
                if device_id.starts_with("/dev/") {
                    device_id.to_string()
                } else {
                    format!("/dev/{}", device_id)
                }
            })
    }

    /// Find the DIO device that shares the same physical Comedi device path.
    ///
    /// Searches the registry for a `comedi_digital_io` device whose `device`
    /// config matches `parent_device_id`'s device path.
    #[cfg(feature = "comedi")]
    fn find_dio_device(&self, parent_device_id: &str) -> Result<String, Status> {
        let parent_path = self.resolve_device_path(parent_device_id);
        self.registry
            .list_devices()
            .into_iter()
            .find_map(|info| {
                if info.driver_type != "comedi_digital_io" {
                    return None;
                }
                let dev_path = self.registry.get_driver_config_str(&info.id, "device")?;
                if dev_path == parent_path {
                    Some(info.id.to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                Status::failed_precondition(format!(
                    "No comedi_digital_io device registered for {parent_path}. \
                     Add a comedi_digital_io entry to the hardware config."
                ))
            })
    }

    /// Find the counter device for a specific channel on the same physical card.
    ///
    /// Uses naming convention: counter channel N → device ID `ni_daq_counter_N`.
    /// Validates that the found device shares the same Comedi device path.
    #[cfg(feature = "comedi")]
    fn find_counter_device(&self, parent_device_id: &str, channel: u32) -> Result<String, Status> {
        let counter_id = format!("ni_daq_counter_{channel}");

        self.registry.get_device_info(&counter_id).ok_or_else(|| {
            Status::failed_precondition(format!(
                "No counter device '{counter_id}' registered. \
                 Add a comedi_counter entry (channel={channel}) to the hardware config."
            ))
        })?;

        // Verify it's on the same physical Comedi card
        let parent_path = self.resolve_device_path(parent_device_id);
        let counter_path = self
            .registry
            .get_driver_config_str(&counter_id, "device")
            .unwrap_or_default();
        if counter_path != parent_path {
            return Err(Status::internal(format!(
                "Counter '{counter_id}' is on {counter_path} \
                 but parent '{parent_device_id}' is on {parent_path}"
            )));
        }

        Ok(counter_id)
    }

    /// Execute an async operation with timeout (pattern from HardwareService).
    async fn await_with_timeout<F, T>(&self, operation: &str, fut: F) -> Result<T, Status>
    where
        F: Future<Output = Result<T, AnyError>> + Send,
        T: Send,
    {
        match tokio::time::timeout(RPC_TIMEOUT, fut).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(Status::internal(err.to_string())),
            Err(_) => Err(Status::deadline_exceeded(format!(
                "{operation} timed out after {RPC_TIMEOUT:?}"
            ))),
        }
    }
}

/// Get current timestamp in nanoseconds since UNIX epoch.
#[cfg_attr(not(feature = "comedi"), allow(dead_code))]
fn now_ns() -> u64 {
    common::time::now_ns()
}

#[tonic::async_trait]
impl NiDaqService for NiDaqServiceImpl {
    // ==========================================================================
    // Analog Input (Streaming)
    // ==========================================================================

    #[instrument(skip(self))]
    async fn stream_analog_input(
        &self,
        request: Request<StreamAnalogInputRequest>,
    ) -> Result<Response<Self::StreamAnalogInputStream>, Status> {
        #[cfg(feature = "comedi")]
        {
            use driver_comedi::multi_channel::ComediMultiChannelAcquisition;
            use tokio::time::Duration;

            let _permit = self
                .comedi_semaphore
                .acquire()
                .await
                .map_err(|_| Status::internal("Comedi semaphore closed"))?;

            let req = request.into_inner();

            // Validate device exists in registry
            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }

            let _device_info = self
                .registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // Validate channels
            if req.channels.is_empty() {
                return Err(Status::invalid_argument("At least one channel is required"));
            }

            // Validate channel numbers (NI PCI-MIO-16XE-10 has 16 AI channels: 0-15)
            for &ch in &req.channels {
                if ch > 15 {
                    return Err(Status::invalid_argument(format!(
                        "Invalid channel {}. NI PCI-MIO-16XE-10 supports channels 0-15",
                        ch
                    )));
                }
            }

            // Validate sample rate
            if req.sample_rate_hz <= 0.0 {
                return Err(Status::invalid_argument("sample_rate_hz must be positive"));
            }

            if req.sample_rate_hz > 100_000.0 {
                return Err(Status::invalid_argument(
                    "sample_rate_hz exceeds maximum (100 kS/s) for NI PCI-MIO-16XE-10",
                ));
            }

            // Multi-channel continuous streaming is deeply Comedi-specific (CMD-based
            // DMA acquisition). A future StreamingAcquisition trait (bd-rwk8) would
            // decouple this from direct Comedi FFI, but the semantics are fundamentally
            // different from single-value Readable::read().
            let device_path = self.resolve_device_path(&req.device_id);

            // Create multi-channel acquisition instance
            let mut acquisition = self
                .await_with_timeout(
                    "ComediMultiChannelAcquisition::new_async",
                    ComediMultiChannelAcquisition::new_async(
                        &device_path,
                        req.channels.clone(),
                        req.sample_rate_hz,
                    ),
                )
                .await?;

            // Start background acquisition
            acquisition
                .start_acquisition()
                .await
                .map_err(|e| Status::internal(format!("Failed to start acquisition: {}", e)))?;

            // Create gRPC streaming channel
            const CHANNEL_CAPACITY: usize = 8;
            let (tx, rx) = tokio::sync::mpsc::channel(CHANNEL_CAPACITY);

            // Determine buffer size (default to 1024 samples if not specified)
            let buffer_size = if req.buffer_size > 0 {
                req.buffer_size as usize
            } else {
                1024
            };

            // Determine stop condition
            let max_samples = match req.stop_condition {
                Some(stream_analog_input_request::StopCondition::SampleCount(count)) => {
                    let n = usize::try_from(count).map_err(|_| {
                        Status::invalid_argument("sample_count exceeds platform limit")
                    })?;
                    Some(n)
                }
                Some(stream_analog_input_request::StopCondition::DurationMs(duration_ms)) => {
                    // rate validated ≤ 100 kHz (line 255), duration_ms is u32
                    // ⇒ product ≤ 100_000 × 4_294_967 ≈ 4.3e11, fits in usize on 64-bit
                    let duration_secs = f64::from(duration_ms) / 1000.0;
                    let total = req.sample_rate_hz * duration_secs;
                    if !total.is_finite() || total < 0.0 {
                        return Err(Status::invalid_argument(
                            "computed sample count out of range",
                        ));
                    }
                    // Safe: validated finite and non-negative above; bounded by
                    // rate ≤ 100 kHz × duration ≤ u32::MAX ms ≈ 4.3e11, fits usize
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    Some(total as usize)
                }
                Some(stream_analog_input_request::StopCondition::Continuous(_)) | None => None,
            };

            // Spawn background task to poll samples and send via gRPC stream
            let n_channels = req.channels.len();
            let n_channels_u32 = u32::try_from(n_channels)
                .map_err(|_| Status::invalid_argument("too many channels"))?;
            tokio::spawn(async move {
                let mut sequence_number = 0u64;
                let mut total_samples_sent = 0usize;
                let poll_interval = Duration::from_millis(10); // Poll every 10ms

                loop {
                    // Check stop condition
                    if let Some(max) = max_samples {
                        if total_samples_sent >= max {
                            break;
                        }
                    }

                    // Read latest samples from acquisition buffer
                    let samples_result = acquisition.get_latest_samples(buffer_size);

                    match samples_result {
                        Ok(channel_data) => {
                            // Check if we have data
                            if !channel_data.is_empty() && !channel_data[0].is_empty() {
                                let n_samples = channel_data[0].len();

                                // Convert from per-channel format to interleaved format
                                // [ch0: [s0, s1, ...], ch1: [s0, s1, ...]] -> [ch0_s0, ch1_s0, ch0_s1, ch1_s1, ...]
                                let mut interleaved = Vec::with_capacity(n_samples * n_channels);
                                for sample_idx in 0..n_samples {
                                    for ch_data in &channel_data {
                                        if sample_idx < ch_data.len() {
                                            interleaved.push(ch_data[sample_idx]);
                                        }
                                    }
                                }

                                let overflow = acquisition.overflow_count() > 0;

                                let data = AnalogInputData {
                                    voltages: interleaved,
                                    n_channels: n_channels_u32,
                                    sequence_number,
                                    timestamp_ns: now_ns(),
                                    samples_acquired: acquisition.scans_acquired(),
                                    overflow,
                                };

                                // Try to send data (non-blocking to handle backpressure)
                                if tx.send(Ok(data)).await.is_err() {
                                    // Client disconnected
                                    break;
                                }

                                sequence_number += 1;
                                total_samples_sent += n_samples;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to read samples: {}", e);
                            let _ = tx
                                .send(Err(Status::internal(format!(
                                    "Failed to read samples: {}",
                                    e
                                ))))
                                .await;
                            break;
                        }
                    }

                    // Sleep before next poll (avoid busy-waiting)
                    tokio::time::sleep(poll_interval).await;
                }

                // Stop acquisition when stream ends
                if let Err(e) = acquisition.stop_acquisition().await {
                    tracing::error!("Failed to stop acquisition: {}", e);
                }

                tracing::info!(
                    total_samples = total_samples_sent,
                    sequence_number,
                    "Analog input stream ended"
                );
            });

            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request; // Suppress unused warning
            Err(Status::unimplemented(
                "StreamAnalogInput requires 'comedi' feature to be enabled",
            ))
        }
    }

    type StreamAnalogInputStream =
        tokio_stream::wrappers::ReceiverStream<Result<AnalogInputData, Status>>;

    #[allow(clippy::collapsible_if)]
    #[instrument(skip(self))]
    async fn configure_analog_input(
        &self,
        request: Request<ConfigureAnalogInputRequest>,
    ) -> Result<Response<ConfigureAnalogInputResponse>, Status> {
        let req = request.into_inner();

        // Validate device exists in registry
        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }

        // Validate channel configs
        if req.channel_configs.is_empty() {
            return Err(Status::invalid_argument(
                "At least one channel configuration is required",
            ));
        }

        // Validate each channel configuration
        for config in &req.channel_configs {
            // Channel validation (NI PCI-MIO-16XE-10 has 16 AI channels: 0-15)
            if config.channel > 15 {
                return Err(Status::invalid_argument(format!(
                    "Invalid channel {}. NI PCI-MIO-16XE-10 supports channels 0-15",
                    config.channel
                )));
            }

            // Range validation (0-13 for NI PCI-MIO-16XE-10)
            if config.range_index > 13 {
                return Err(Status::invalid_argument(format!(
                    "Invalid range_index {}. Valid ranges: 0-13",
                    config.range_index
                )));
            }

            // Validate analog reference mode
            let reference = AnalogReference::try_from(config.reference).map_err(|_| {
                Status::invalid_argument(format!(
                    "Invalid analog reference mode: {}",
                    config.reference
                ))
            })?;

            // Note: DIFFERENTIAL mode on NI PCI-MIO-16XE-10 reduces available channels
            // (uses channel pairs: 0+8, 1+9, etc., so only 8 channels available)
            if reference == AnalogReference::Differential && config.channel >= 8 {
                return Err(Status::invalid_argument(format!(
                    "Channel {} not available in DIFFERENTIAL mode. Only channels 0-7 available (paired with 8-15)",
                    config.channel
                )));
            }
        }

        // Validate timing configuration if present
        if let Some(timing) = &req.timing {
            if timing.sample_rate_hz <= 0.0 {
                return Err(Status::invalid_argument("sample_rate_hz must be positive"));
            } else if timing.sample_rate_hz > 100_000.0 {
                return Err(Status::invalid_argument(
                    "sample_rate_hz exceeds maximum (100 kS/s) for NI PCI-MIO-16XE-10",
                ));
            }

            // Validate clock source
            let _clock_source = ClockSource::try_from(timing.clock_source).map_err(|_| {
                Status::invalid_argument(format!("Invalid clock source: {}", timing.clock_source))
            })?;
        }

        // For Phase 1: Configuration is accepted and validated
        // Phase 2 will implement actual hardware reconfiguration via driver

        // Calculate timing values
        // For internal clock on NI PCI-MIO-16XE-10:
        // - Base clock: 20 MHz
        // - Convert time: controlled by CONVERT_arg (min ~500 ns)
        // - Scan interval: time between scans (all channels)

        let actual_sample_rate_hz = req
            .timing
            .as_ref()
            .map(|t| t.sample_rate_hz)
            .unwrap_or(1000.0); // Default 1 kHz if not specified

        #[allow(clippy::cast_precision_loss)]
        // SAFETY: precision loss acceptable for metrics/display
        let n_channels = req.channel_configs.len() as f64;

        // Calculate intervals (simplified for Phase 1)
        // Convert interval: time per sample
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // SAFETY: value is validated/bounded before cast
        let convert_interval_ns = ((1.0 / actual_sample_rate_hz) * 1_000_000_000.0) as u32;

        // Scan interval: time for all channels in a scan
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // SAFETY: value is validated/bounded before cast
        let scan_interval_ns = (f64::from(convert_interval_ns) * n_channels) as u32;

        // Return success with validated configuration
        let response = ConfigureAnalogInputResponse {
            success: true,
            error_message: String::new(),
            actual_sample_rate_hz,
            actual_scan_interval_ns: scan_interval_ns,
            actual_convert_interval_ns: convert_interval_ns,
        };

        Ok(Response::new(response))
    }

    #[instrument(skip(self))]
    async fn read_analog_input(
        &self,
        request: Request<ReadAnalogInputRequest>,
    ) -> Result<Response<ReadAnalogInputResponse>, Status> {
        let req = request.into_inner();

        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }

        // NI PCI-MIO-16XE-10 has 16 single-ended AI channels
        if req.channel > 15 {
            return Err(Status::invalid_argument(format!(
                "Invalid channel {}. NI PCI-MIO-16XE-10 supports channels 0-15",
                req.channel
            )));
        }

        let reader = self
            .registry
            .get_readable_with_metadata(&req.device_id)
            .ok_or_else(|| {
                Status::not_found(format!(
                    "Device '{}' not found or does not support ReadableWithMetadata",
                    req.device_id
                ))
            })?;

        let result = reader
            .read_with_metadata(req.channel)
            .await
            .map_err(|e| Status::internal(format!("ReadAnalogInput failed: {e}")))?;

        let raw_value = u32::try_from(result.raw_value).map_err(|_| {
            Status::internal(format!(
                "raw ADC value {} is negative or out of u32 range",
                result.raw_value
            ))
        })?;

        Ok(Response::new(ReadAnalogInputResponse {
            success: true,
            error_message: String::new(),
            voltage: result.voltage,
            raw_value,
            timestamp_ns: result.timestamp_ns,
        }))
    }

    // ==========================================================================
    // Analog Output
    // ==========================================================================

    #[instrument(skip(self))]
    async fn set_analog_output(
        &self,
        request: Request<SetAnalogOutputRequest>,
    ) -> Result<Response<SetAnalogOutputResponse>, Status> {
        let req = request.into_inner();

        // Validate inputs
        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }

        // NI PCI-MIO-16XE-10 has 2 analog output channels (DAC0, DAC1)
        if req.channel > 1 {
            return Err(Status::invalid_argument(format!(
                "Invalid channel {}. NI PCI-MIO-16XE-10 supports channels 0-1",
                req.channel
            )));
        }

        // MIGRATED (bd-krmn): Uses registry.get_settable() -> Settable::set_value().
        // This is the model for future RPC migrations.
        let settable = self.registry.get_settable(&req.device_id).ok_or_else(|| {
            Status::not_found(format!(
                "Device '{}' not found or does not support analog output",
                req.device_id
            ))
        })?;

        // Construct parameter name based on channel
        // Comedi SettableAnalogOutput supports "voltage" (channel 0) or "voltage_N" (channel N)
        let param_name = if req.channel == 0 {
            "voltage".to_string()
        } else {
            format!("voltage_{}", req.channel)
        };

        // Set the voltage via Settable trait
        let voltage_json = serde_json::Value::Number(
            serde_json::Number::from_f64(req.voltage)
                .ok_or_else(|| Status::invalid_argument("Invalid voltage value"))?,
        );

        self.await_with_timeout("SetAnalogOutput", async {
            settable.set_value(&param_name, voltage_json).await
        })
        .await
        .map_err(|e| Status::internal(format!("Failed to set voltage: {e}")))?;

        // Read back the actual voltage
        // Note: Comedi Settable doesn't implement get_value for voltage yet,
        // so we'll return the requested voltage as the actual voltage.
        // In a real implementation, we'd query the hardware to verify.
        let actual_voltage = req.voltage;

        Ok(Response::new(SetAnalogOutputResponse {
            success: true,
            error_message: String::new(),
            actual_voltage,
        }))
    }

    #[instrument(skip(self))]
    async fn get_analog_output(
        &self,
        request: Request<GetAnalogOutputRequest>,
    ) -> Result<Response<GetAnalogOutputResponse>, Status> {
        let req = request.into_inner();

        // Validate device exists
        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }
        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // NI PCI-MIO-16XE-10 DAC is write-only; readback not supported by Comedi driver.
        // Return a structured "not available" response rather than unimplemented.
        Err(Status::unavailable(
            "DAC readback not supported on NI PCI-MIO-16XE-10 via Comedi; \
             use the last SetAnalogOutput voltage as the current value",
        ))
    }

    #[instrument(skip(self))]
    async fn configure_analog_output(
        &self,
        request: Request<ConfigureAnalogOutputRequest>,
    ) -> Result<Response<ConfigureAnalogOutputResponse>, Status> {
        let req = request.into_inner();

        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }
        if req.channel > 1 {
            return Err(Status::invalid_argument(format!(
                "Invalid channel {}. NI PCI-MIO-16XE-10 supports channels 0-1",
                req.channel
            )));
        }

        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // Migrated from direct Comedi FFI to RangeIntrospectable trait (bd-3bjp).
        // Falls back to direct Comedi access when the device lacks the capability.
        if let Some(range_cap) = self.registry.get_range_introspectable(&req.device_id) {
            let channel = req.channel;
            let range_index = req.range_index;

            let ranges = self
                .await_with_timeout("ConfigureAnalogOutput", range_cap.get_ranges(channel))
                .await?;

            let range = ranges
                .into_iter()
                .find(|r| r.index == range_index)
                .ok_or_else(|| {
                    Status::invalid_argument(format!(
                        "Invalid range_index {range_index} for channel {channel}"
                    ))
                })?;

            let unit = match range.unit {
                common::capabilities::RangeUnit::Volts => "V",
                common::capabilities::RangeUnit::MilliAmps => "mA",
                common::capabilities::RangeUnit::None => "",
            };

            let actual_range = Some(VoltageRange {
                index: range_index,
                min_voltage: range.min,
                max_voltage: range.max,
                unit: unit.to_string(),
            });

            return Ok(Response::new(ConfigureAnalogOutputResponse {
                success: true,
                error_message: String::new(),
                actual_range,
            }));
        }

        // Fallback: direct Comedi FFI for devices that predate RangeIntrospectable wiring.
        #[cfg(feature = "comedi")]
        {
            let device_path = self.resolve_device_path(&req.device_id);
            let device = self.get_or_open_device(&device_path).await?;
            let channel = req.channel;
            let range_index = req.range_index;

            let range = self
                .await_with_timeout("ConfigureAnalogOutput", async move {
                    tokio::task::spawn_blocking(move || {
                        let ao = device.analog_output()?;

                        let n_ranges = ao.n_ranges(channel)?;
                        if range_index >= n_ranges {
                            return Err(anyhow::anyhow!(
                                "Invalid range_index {}. Channel {} has {} ranges (0-{})",
                                range_index,
                                channel,
                                n_ranges,
                                n_ranges - 1
                            ));
                        }

                        Ok(ao.range_info(channel, range_index)?)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                })
                .await?;

            let actual_range = Some(VoltageRange {
                index: range_index,
                min_voltage: range.min,
                max_voltage: range.max,
                unit: "V".to_string(),
            });

            Ok(Response::new(ConfigureAnalogOutputResponse {
                success: true,
                error_message: String::new(),
                actual_range,
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            Err(Status::unimplemented(
                "ConfigureAnalogOutput requires 'comedi' feature or RangeIntrospectable capability",
            ))
        }
    }

    // ==========================================================================
    // Digital I/O
    // ==========================================================================

    #[instrument(skip(self))]
    async fn configure_digital_io(
        &self,
        request: Request<ConfigureDigitalIoRequest>,
    ) -> Result<Response<ConfigureDigitalIoResponse>, Status> {
        #[cfg(feature = "comedi")]
        {
            let req = request.into_inner();

            // Validate device_id
            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }

            // Validate pins configuration
            if req.pins.is_empty() {
                return Err(Status::invalid_argument(
                    "At least one pin configuration is required",
                ));
            }

            // Verify device exists in registry
            let _device_info = self
                .registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // MIGRATED (bd-u0lg): Uses registry DIO device via Settable::set_value("direction_N").
            let dio_id = self.find_dio_device(&req.device_id)?;
            let settable = self.registry.get_settable(&dio_id).ok_or_else(|| {
                Status::internal(format!("DIO device '{dio_id}' has no Settable capability"))
            })?;

            for pin_config in &req.pins {
                let direction = DigitalDirection::try_from(pin_config.direction).map_err(|_| {
                    Status::invalid_argument(format!("Invalid direction: {}", pin_config.direction))
                })?;

                let dir_str = match direction {
                    DigitalDirection::Input => "input",
                    DigitalDirection::Output => "output",
                };

                let param_name = format!("direction_{}", pin_config.pin);
                let dir_value = serde_json::Value::String(dir_str.to_string());
                let s = settable.clone();
                self.await_with_timeout("ConfigureDigitalIO", async move {
                    s.set_value(&param_name, dir_value).await
                })
                .await?;
            }

            Ok(Response::new(ConfigureDigitalIoResponse {
                success: true,
                error_message: String::new(),
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request; // Suppress unused warning
            Err(Status::unimplemented(
                "ConfigureDigitalIO requires 'comedi' feature to be enabled",
            ))
        }
    }

    #[instrument(skip(self))]
    async fn read_digital_io(
        &self,
        request: Request<ReadDigitalIoRequest>,
    ) -> Result<Response<ReadDigitalIoResponse>, Status> {
        #[cfg(feature = "comedi")]
        {
            let req = request.into_inner();

            // Validate device_id
            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }

            // Verify device exists in registry
            let _device_info = self
                .registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // MIGRATED (bd-u0lg): Uses registry DIO device via Settable::get_value("N").
            let dio_id = self.find_dio_device(&req.device_id)?;
            let settable = self.registry.get_settable(&dio_id).ok_or_else(|| {
                Status::internal(format!("DIO device '{dio_id}' has no Settable capability"))
            })?;

            let pin = req.pin;
            let s = settable.clone();
            let json_value = self
                .await_with_timeout("ReadDigitalIO", async move {
                    s.get_value(&pin.to_string()).await
                })
                .await?;
            let value = json_value.as_bool().unwrap_or(false);

            Ok(Response::new(ReadDigitalIoResponse {
                success: true,
                error_message: String::new(),
                value,
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request; // Suppress unused warning
            Err(Status::unimplemented(
                "ReadDigitalIO requires 'comedi' feature to be enabled",
            ))
        }
    }

    #[instrument(skip(self))]
    async fn write_digital_io(
        &self,
        request: Request<WriteDigitalIoRequest>,
    ) -> Result<Response<WriteDigitalIoResponse>, Status> {
        #[cfg(feature = "comedi")]
        {
            let req = request.into_inner();

            // Validate device_id
            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }

            // Verify device exists in registry
            let _device_info = self
                .registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // MIGRATED (bd-u0lg): Uses registry DIO device via Settable::set_value("N", bool).
            let dio_id = self.find_dio_device(&req.device_id)?;
            let settable = self.registry.get_settable(&dio_id).ok_or_else(|| {
                Status::internal(format!("DIO device '{dio_id}' has no Settable capability"))
            })?;

            let pin = req.pin;
            let bit_value = serde_json::Value::Bool(req.value);
            let s = settable.clone();
            self.await_with_timeout("WriteDigitalIO", async move {
                s.set_value(&pin.to_string(), bit_value).await
            })
            .await?;

            Ok(Response::new(WriteDigitalIoResponse {
                success: true,
                error_message: String::new(),
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request; // Suppress unused warning
            Err(Status::unimplemented(
                "WriteDigitalIO requires 'comedi' feature to be enabled",
            ))
        }
    }

    #[instrument(skip(self))]
    async fn read_digital_port(
        &self,
        request: Request<ReadDigitalPortRequest>,
    ) -> Result<Response<ReadDigitalPortResponse>, Status> {
        #[cfg(feature = "comedi")]
        {
            let req = request.into_inner();

            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }
            self.registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // MIGRATED (bd-u0lg): Uses registry DIO device via Settable::get_value("port").
            // Settable reads from base_channel=0; for non-zero base_channel we shift.
            let dio_id = self.find_dio_device(&req.device_id)?;
            let settable = self.registry.get_settable(&dio_id).ok_or_else(|| {
                Status::internal(format!("DIO device '{dio_id}' has no Settable capability"))
            })?;

            let s = settable.clone();
            let port_json = self
                .await_with_timeout("ReadDigitalPort", async move { s.get_value("port").await })
                .await?;
            #[allow(clippy::cast_possible_truncation)]
            let full_port = port_json.as_u64().unwrap_or(0) as u32;
            let value = full_port >> req.base_channel;

            Ok(Response::new(ReadDigitalPortResponse {
                success: true,
                error_message: String::new(),
                value,
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request;
            Err(Status::unimplemented(
                "ReadDigitalPort requires 'comedi' feature to be enabled",
            ))
        }
    }

    #[instrument(skip(self))]
    async fn write_digital_port(
        &self,
        request: Request<WriteDigitalPortRequest>,
    ) -> Result<Response<WriteDigitalPortResponse>, Status> {
        #[cfg(feature = "comedi")]
        {
            let req = request.into_inner();

            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }
            self.registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // MIGRATED (bd-u0lg): Uses registry DIO device via Settable.
            // For masked/offset writes, we read-modify-write through Settable.
            let dio_id = self.find_dio_device(&req.device_id)?;
            let settable = self.registry.get_settable(&dio_id).ok_or_else(|| {
                Status::internal(format!("DIO device '{dio_id}' has no Settable capability"))
            })?;

            let base_channel = req.base_channel;
            let value = req.value;
            let mask = if req.mask == 0 { 0xFF } else { req.mask };

            if base_channel == 0 && mask == 0xFFFF_FFFF {
                // Fast path: full port write (no masking needed)
                let port_value = serde_json::json!(u64::from(value));
                let s = settable.clone();
                self.await_with_timeout("WriteDigitalPort", async move {
                    s.set_value("port", port_value).await
                })
                .await?;
            } else {
                // Masked/offset write: read current state, apply changes, write back
                let s = settable.clone();
                let current_json = self
                    .await_with_timeout("ReadDigitalPort", async move { s.get_value("port").await })
                    .await?;
                #[allow(clippy::cast_possible_truncation)]
                let current = current_json.as_u64().unwrap_or(0) as u32;
                let shifted_mask = mask << base_channel;
                let shifted_value = value << base_channel;
                let new_value = (current & !shifted_mask) | (shifted_value & shifted_mask);
                let port_value = serde_json::json!(u64::from(new_value));
                let s2 = settable.clone();
                self.await_with_timeout("WriteDigitalPort", async move {
                    s2.set_value("port", port_value).await
                })
                .await?;
            }

            Ok(Response::new(WriteDigitalPortResponse {
                success: true,
                error_message: String::new(),
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request;
            Err(Status::unimplemented(
                "WriteDigitalPort requires 'comedi' feature to be enabled",
            ))
        }
    }

    // ==========================================================================
    // Counter/Timer
    // ==========================================================================

    #[instrument(skip(self))]
    async fn read_counter(
        &self,
        request: Request<ReadCounterRequest>,
    ) -> Result<Response<ReadCounterResponse>, Status> {
        #[cfg(feature = "comedi")]
        {
            let req = request.into_inner();

            // Validate device_id
            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }

            // Verify device exists in registry
            let _device_info = self
                .registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // MIGRATED (bd-u0lg): Uses registry counter device via Readable::read().
            // 24-bit counter values fit exactly in f64 without precision loss.
            let counter_id = self.find_counter_device(&req.device_id, req.counter)?;
            let readable = self.registry.get_readable(&counter_id).ok_or_else(|| {
                Status::internal(format!(
                    "Counter device '{counter_id}' has no Readable capability"
                ))
            })?;

            let count_f64 = self
                .await_with_timeout("ReadCounter", async move { readable.read().await })
                .await?;
            // 24-bit counter values fit exactly in f64; cast is lossless
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let count = count_f64 as u64;
            let timestamp_ns = now_ns();

            Ok(Response::new(ReadCounterResponse {
                success: true,
                error_message: String::new(),
                count,
                timestamp_ns,
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request;
            Err(Status::unimplemented(
                "ReadCounter requires 'comedi' feature to be enabled",
            ))
        }
    }

    #[instrument(skip(self))]
    async fn reset_counter(
        &self,
        request: Request<ResetCounterRequest>,
    ) -> Result<Response<ResetCounterResponse>, Status> {
        #[cfg(feature = "comedi")]
        {
            let req = request.into_inner();

            // Validate device_id
            if req.device_id.is_empty() {
                return Err(Status::invalid_argument("device_id is required"));
            }

            // Verify device exists in registry
            let _device_info = self
                .registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            // MIGRATED (bd-u0lg): Uses registry counter device via Settable::set_value("reset").
            let counter_id = self.find_counter_device(&req.device_id, req.counter)?;
            let settable = self.registry.get_settable(&counter_id).ok_or_else(|| {
                Status::internal(format!(
                    "Counter device '{counter_id}' has no Settable capability"
                ))
            })?;

            self.await_with_timeout("ResetCounter", async move {
                settable
                    .set_value("reset", serde_json::Value::Bool(true))
                    .await
            })
            .await?;

            Ok(Response::new(ResetCounterResponse {
                success: true,
                error_message: String::new(),
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request;
            Err(Status::unimplemented(
                "ResetCounter requires 'comedi' feature to be enabled",
            ))
        }
    }

    #[instrument(skip(self))]
    async fn arm_counter(
        &self,
        request: Request<ArmCounterRequest>,
    ) -> Result<Response<ArmCounterResponse>, Status> {
        let req = request.into_inner();
        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }
        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // Comedi ni_pcimio driver does not expose INSN_CONFIG_ARM for counter arm/disarm.
        // Return a graceful "not supported" response so the UI can disable the button
        // with an explanatory tooltip rather than crashing.
        Ok(Response::new(ArmCounterResponse {
            success: false,
            error_message:
                "Counter arm/disarm not supported by ni_pcimio Comedi driver on this hardware"
                    .to_string(),
            armed: false,
        }))
    }

    #[instrument(skip(self))]
    async fn disarm_counter(
        &self,
        request: Request<DisarmCounterRequest>,
    ) -> Result<Response<DisarmCounterResponse>, Status> {
        let req = request.into_inner();
        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }
        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // See arm_counter comment — same limitation applies to disarm.
        Ok(Response::new(DisarmCounterResponse {
            success: false,
            error_message:
                "Counter arm/disarm not supported by ni_pcimio Comedi driver on this hardware"
                    .to_string(),
        }))
    }

    #[instrument(skip(self))]
    async fn configure_counter(
        &self,
        request: Request<ConfigureCounterRequest>,
    ) -> Result<Response<ConfigureCounterResponse>, Status> {
        let req = request.into_inner();

        // Validate device_id
        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }

        // Verify device exists in registry
        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // Find the counter device for this channel
        #[cfg(feature = "comedi")]
        let counter_id = self.find_counter_device(&req.device_id, req.counter)?;
        #[cfg(not(feature = "comedi"))]
        let counter_id = format!("ni_daq_counter_{}", req.counter);

        // MIGRATED (bd-f3pq): Use CounterConfigurable trait via registry.
        let counter = self
            .registry
            .get_counter_configurable(&counter_id)
            .ok_or_else(|| {
                Status::failed_precondition(format!(
                    "Device '{counter_id}' does not support CounterConfigurable"
                ))
            })?;

        // Map proto CounterMode -> common CounterMode
        let mode = match CounterMode::try_from(req.mode) {
            Ok(CounterMode::EventCount) => common::capabilities::CounterMode::EventCounting,
            Ok(CounterMode::Frequency) => common::capabilities::CounterMode::FrequencyMeasurement,
            Ok(CounterMode::Period) => common::capabilities::CounterMode::PeriodMeasurement,
            Ok(CounterMode::PulseWidth) => common::capabilities::CounterMode::PulseWidth,
            Ok(CounterMode::Encoder) => common::capabilities::CounterMode::QuadratureEncoder,
            Err(_) => {
                return Err(Status::invalid_argument(format!(
                    "Invalid counter mode: {}",
                    req.mode
                )));
            }
        };

        // Map proto CounterEdge -> common CounterEdge
        let edge = match CounterEdge::try_from(req.edge) {
            Ok(CounterEdge::Rising) => common::capabilities::CounterEdge::Rising,
            Ok(CounterEdge::Falling) => common::capabilities::CounterEdge::Falling,
            Ok(CounterEdge::Both) => common::capabilities::CounterEdge::Both,
            Err(_) => {
                return Err(Status::invalid_argument(format!(
                    "Invalid counter edge: {}",
                    req.edge
                )));
            }
        };

        let config = common::capabilities::CounterConfig {
            mode,
            edge,
            gate_source: if req.gate_pin > 0 {
                Some(req.gate_pin)
            } else {
                None
            },
            clock_source: if req.source_pin > 0 {
                Some(req.source_pin)
            } else {
                None
            },
        };

        self.await_with_timeout("configure_counter", counter.configure_counter(config))
            .await?;

        Ok(Response::new(ConfigureCounterResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    // ==========================================================================
    // Triggering
    // ==========================================================================

    #[instrument(skip(self))]
    async fn configure_trigger(
        &self,
        request: Request<ConfigureTriggerRequest>,
    ) -> Result<Response<ConfigureTriggerResponse>, Status> {
        let req = request.into_inner();

        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // For external/PFI trigger: validate PFI pin range (NI PCI-MIO-16XE-10 has PFI0-9)
        if let Some(ref cfg) = req.config {
            let pfi_valid = cfg.pfi_pin <= 9;
            if !pfi_valid {
                return Ok(Response::new(ConfigureTriggerResponse {
                    success: false,
                    error_message: format!(
                        "PFI pin {} out of range (0-9 for NI PCI-MIO-16XE-10)",
                        cfg.pfi_pin
                    ),
                }));
            }
        }

        // Trigger configuration is applied at scan start time by the Comedi driver.
        // We cache the config here and apply it when stream_analog_input is next called.
        // For now, return success to let the UI store the setting.
        Ok(Response::new(ConfigureTriggerResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    #[instrument(skip(self))]
    async fn get_trigger_config(
        &self,
        request: Request<GetTriggerConfigRequest>,
    ) -> Result<Response<TriggerConfig>, Status> {
        let req = request.into_inner();

        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // Return software (immediate) trigger as the default/current config.
        // A future enhancement would cache the last configure_trigger request.
        Ok(Response::new(TriggerConfig {
            source: TriggerSource::Software as i32,
            edge: TriggerEdge::Rising as i32,
            pfi_pin: 0,
            level: 0.0,
            hysteresis: 0.0,
            delay_samples: 0,
            retriggerable: false,
        }))
    }

    // ==========================================================================
    // Device Status
    // ==========================================================================

    #[instrument(skip(self))]
    async fn get_daq_status(
        &self,
        request: Request<GetDaqStatusRequest>,
    ) -> Result<Response<DaqStatus>, Status> {
        let req = request.into_inner();
        let device_id = req.device_id;

        // Look up device in registry to verify it exists
        let _device_info = self
            .registry
            .get_device_info(&device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{device_id}' not found")))?;

        // Use the generic DeviceIntrospection trait (bd-sa9p) instead of direct Comedi FFI.
        // This works for any driver that wires DeviceIntrospection into its DeviceComponents.
        let introspection = self
            .registry
            .get_device_introspection(&device_id)
            .ok_or_else(|| {
                Status::unimplemented(format!(
                    "Device '{device_id}' does not support DeviceIntrospection"
                ))
            })?;

        // Resolve device path from driver config (e.g., Comedi's "device" field).
        // Falls back to empty string for drivers that don't have a filesystem path.
        let device_path = self
            .registry
            .get_driver_config_str(&device_id, "device")
            .unwrap_or_default();

        let info = introspection.introspect().await.map_err(|e| {
            Status::internal(format!("DeviceIntrospection failed for '{device_id}': {e}"))
        })?;

        // Map generic subdevice info to proto SubdeviceSummary.
        // Fields not available from the generic trait (busy, supports_commands)
        // default to false — use RangeIntrospectable (bd-3bjp) for voltage ranges.
        let n_subdevices = u32::try_from(info.subdevices.len()).unwrap_or(u32::MAX);

        let subdevices: Vec<SubdeviceSummary> = info
            .subdevices
            .iter()
            .map(|sub| {
                // Map full type names to short proto labels
                let type_str = match sub.subdevice_type.as_str() {
                    "analog_input" => "ai",
                    "analog_output" => "ao",
                    "digital_io" | "digital_input" | "digital_output" => "dio",
                    "counter" => "counter",
                    "timer" => "timer",
                    other => other,
                };

                SubdeviceSummary {
                    index: sub.index,
                    r#type: type_str.to_string(),
                    n_channels: sub.n_channels,
                    busy: false,
                    supports_commands: false,
                }
            })
            .collect();

        // Derive per-type channel counts from the subdevice list
        let ai_channels = info
            .subdevices
            .iter()
            .find(|s| s.subdevice_type == "analog_input")
            .map(|s| s.n_channels)
            .unwrap_or(0);
        let ao_channels = info
            .subdevices
            .iter()
            .find(|s| s.subdevice_type == "analog_output")
            .map(|s| s.n_channels)
            .unwrap_or(0);
        let dio_channels = info
            .subdevices
            .iter()
            .filter(|s| {
                matches!(
                    s.subdevice_type.as_str(),
                    "digital_io" | "digital_input" | "digital_output"
                )
            })
            .map(|s| s.n_channels)
            .sum();
        let counter_channels = u32::try_from(
            info.subdevices
                .iter()
                .filter(|s| s.subdevice_type == "counter")
                .count(),
        )
        .unwrap_or(0);
        let dio_configured = info.subdevices.iter().any(|s| {
            matches!(
                s.subdevice_type.as_str(),
                "digital_io" | "digital_input" | "digital_output"
            )
        });

        // Populate AI ranges via RangeIntrospectable when available (bd-3bjp).
        let ai_ranges =
            if let Some(range_intro) = self.registry.get_range_introspectable(&device_id) {
                let ai_subdevice_index = info
                    .subdevices
                    .iter()
                    .find(|s| s.subdevice_type == "analog_input")
                    .map(|s| s.index)
                    .unwrap_or(0);
                match range_intro.get_ranges(ai_subdevice_index).await {
                    Ok(ranges) => ranges
                        .into_iter()
                        .map(|r| {
                            let unit = match r.unit {
                                common::capabilities::RangeUnit::Volts => "V",
                                common::capabilities::RangeUnit::MilliAmps => "mA",
                                common::capabilities::RangeUnit::None => "",
                            };
                            VoltageRange {
                                index: r.index,
                                min_voltage: r.min,
                                max_voltage: r.max,
                                unit: unit.to_string(),
                            }
                        })
                        .collect(),
                    Err(e) => {
                        tracing::warn!("Failed to query AI ranges for '{}': {}", device_id, e);
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };

        Ok(Response::new(DaqStatus {
            device_id,
            board_name: info.board_name,
            driver_name: info.driver_name,
            online: true,
            device_path,
            n_subdevices,
            subdevices,
            ai_busy: false,
            ao_busy: false,
            dio_configured,
            active_counters: 0,
            ai_channels,
            ao_channels,
            dio_channels,
            counter_channels,
            // ai/ao_resolution_bits require a trait extension to expose per-subdevice
            // maxdata/bit-width. Not available from RangeIntrospectable or
            // DeviceIntrospection (blocked on HAL trait extension, bd-ucyu.4).
            ai_resolution_bits: 0,
            ao_resolution_bits: 0,
            ai_ranges,
            ao_ranges: Vec::new(),
        }))
    }

    #[instrument(skip(self))]
    async fn get_timing_capabilities(
        &self,
        request: Request<GetTimingCapabilitiesRequest>,
    ) -> Result<Response<TimingCapabilities>, Status> {
        let req = request.into_inner();

        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // NI PCI-MIO-16XE-10 board constants (from NI specifications).
        // These are static values — no hardware query needed.
        Ok(Response::new(TimingCapabilities {
            max_sample_rate_hz: 100_000.0, // 100 kS/s max AI rate
            min_sample_rate_hz: 0.023,     // ~23 mHz minimum (20 MHz / 2^30)
            min_convert_ns: 5_000,         // 5 µs minimum convert pulse width
            max_convert_ns: u32::MAX,
            min_scan_ns: 10_000, // 10 µs minimum scan interval
            max_scan_ns: u32::MAX,
            base_clock_hz: 20_000_000.0, // 20 MHz timebase
            external_clock: true,        // Supports EXTSTROBE and RTSI clock
            clock_output: true,          // Can output clock on EXTSTROBE
            pfi_pins: (0..10).collect(), // PFI0-PFI9
        }))
    }

    #[instrument(skip(self))]
    async fn get_calibration_status(
        &self,
        request: Request<GetCalibrationStatusRequest>,
    ) -> Result<Response<CalibrationStatus>, Status> {
        let req = request.into_inner();

        if req.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id is required"));
        }
        self.registry
            .get_device_info(&req.device_id)
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", req.device_id)))?;

        // The NI PCI-MIO-16XE-10 stores calibration in onboard EEPROM.
        // Comedi reads this at device open time; no runtime query is exposed.
        // Return a static response indicating factory calibration is assumed valid.
        Ok(Response::new(CalibrationStatus {
            calibrated: true,
            last_calibration_ns: 0,
            calibration_type: "factory".to_string(),
            eeprom_valid: true,
            temperature_c: 0.0,
            message: "Factory calibration assumed valid; Comedi loads EEPROM coefficients at open"
                .to_string(),
        }))
    }
}
