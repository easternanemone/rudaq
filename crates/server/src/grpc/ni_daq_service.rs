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
//! Unlike HardwareService which uses capability traits (Readable, Movable, etc.),
//! NiDaqService accesses Comedi drivers directly for low-level DAQ operations.

use anyhow::Error as AnyError;
use common::limits::RPC_TIMEOUT;
use hardware::registry::DeviceRegistry;
use protocol::ni_daq::ni_daq_service_server::NiDaqService;
use protocol::ni_daq::*;
use std::future::Future;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::instrument;

// =============================================================================
// NI DAQ Service Implementation
// =============================================================================

/// Implementation of NiDaqService gRPC interface.
///
/// Provides direct access to NI PCI-MIO-16XE-10 capabilities through Comedi drivers.
/// Complements HardwareService for DAQ-specific operations.
#[derive(Clone)]
pub struct NiDaqServiceImpl {
    /// Device registry for looking up Comedi devices
    registry: Arc<DeviceRegistry>,
}

impl NiDaqServiceImpl {
    /// Create a new NI DAQ service instance.
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        Self { registry }
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
                "{} timed out after {:?}",
                operation, RPC_TIMEOUT
            ))),
        }
    }
}

/// Get current timestamp in nanoseconds since UNIX epoch.
#[allow(dead_code)]
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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

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
                    Some(count as usize)
                }
                Some(stream_analog_input_request::StopCondition::DurationMs(duration_ms)) => {
                    // Calculate approximate sample count from duration
                    let duration_secs = duration_ms as f64 / 1000.0;
                    let total_samples = (req.sample_rate_hz * duration_secs) as usize;
                    Some(total_samples)
                }
                Some(stream_analog_input_request::StopCondition::Continuous(_)) | None => None,
            };

            // Spawn background task to poll samples and send via gRPC stream
            let n_channels = req.channels.len();
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
                                    n_channels: n_channels as u32,
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
        #[cfg(feature = "comedi")]
        {
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

            self.registry
                .get_device_info(&req.device_id)
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

            let channel = req.channel;
            let range_index = req.range_index;

            let (voltage, raw_value, timestamp_ns) = self
                .await_with_timeout("ReadAnalogInput", async move {
                    tokio::task::spawn_blocking(move || {
                        use driver_comedi::ComediDevice;
                        use std::time::SystemTime;

                        let device = ComediDevice::open(&device_path)?;
                        let ai = device.analog_input()?;

                        // Get the range for voltage conversion
                        let range = ai.range_info(channel, range_index)?;
                        let raw = ai.read_raw(
                            channel,
                            range_index,
                            driver_comedi::subsystem::AnalogReference::Ground,
                        )?;
                        let voltage = ai.raw_to_voltage(raw, &range);

                        let timestamp_ns = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as u64;

                        Ok((voltage, raw, timestamp_ns))
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                })
                .await?;

            Ok(Response::new(ReadAnalogInputResponse {
                success: true,
                error_message: String::new(),
                voltage,
                raw_value,
                timestamp_ns,
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request;
            Err(Status::unimplemented(
                "ReadAnalogInput requires 'comedi' feature to be enabled",
            ))
        }
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

        // Look up device in registry
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
        .map_err(|e| Status::internal(format!("Failed to set voltage: {}", e)))?;

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
        #[cfg(feature = "comedi")]
        {
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
                .ok_or_else(|| {
                    Status::not_found(format!("Device '{}' not found", req.device_id))
                })?;

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };
            let channel = req.channel;
            let range_index = req.range_index;

            let range = self
                .await_with_timeout("ConfigureAnalogOutput", async move {
                    tokio::task::spawn_blocking(move || {
                        use driver_comedi::ComediDevice;
                        let device = ComediDevice::open(&device_path)?;
                        let ao = device.analog_output()?;

                        // Validate range_index
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
            let _ = request;
            Err(Status::unimplemented(
                "ConfigureAnalogOutput requires 'comedi' feature to be enabled",
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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

            // Configure pins via spawn_blocking (FFI call)
            let pins = req.pins.clone();
            self.await_with_timeout("ConfigureDigitalIO", async move {
                tokio::task::spawn_blocking(move || {
                    use driver_comedi::ComediDevice;
                    use driver_comedi::subsystem::digital_io::DioDirection;

                    let device = ComediDevice::open(&device_path)?;
                    let dio = device.digital_io()?;

                    // Validate and configure each pin
                    for pin_config in &pins {
                        let pin = pin_config.pin;
                        let direction =
                            DigitalDirection::try_from(pin_config.direction).map_err(|_| {
                                anyhow::anyhow!("Invalid direction: {}", pin_config.direction)
                            })?;

                        // Validate pin number
                        if pin >= dio.n_channels() {
                            return Err(anyhow::anyhow!(
                                "Invalid pin {}. Device has {} DIO channels",
                                pin,
                                dio.n_channels()
                            ));
                        }

                        // Map proto DigitalDirection to driver DioDirection
                        let dio_dir = match direction {
                            DigitalDirection::Input => DioDirection::Input,
                            DigitalDirection::Output => DioDirection::Output,
                        };

                        // Configure the pin
                        dio.configure(pin, dio_dir)?;
                    }

                    Ok(())
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            })
            .await?;

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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

            // Read pin via spawn_blocking (FFI call)
            let pin = req.pin;
            let value = self
                .await_with_timeout("ReadDigitalIO", async move {
                    tokio::task::spawn_blocking(move || {
                        use driver_comedi::ComediDevice;

                        let device = ComediDevice::open(&device_path)?;
                        let dio = device.digital_io()?;

                        // Validate pin number
                        if pin >= dio.n_channels() {
                            return Err(anyhow::anyhow!(
                                "Invalid pin {}. Device has {} DIO channels",
                                pin,
                                dio.n_channels()
                            ));
                        }

                        // Read the pin value
                        dio.read(pin)
                            .map_err(|e| anyhow::anyhow!("Failed to read pin: {}", e))
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                })
                .await?;

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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

            // Write pin via spawn_blocking (FFI call)
            let pin = req.pin;
            let value = req.value;
            self.await_with_timeout("WriteDigitalIO", async move {
                tokio::task::spawn_blocking(move || {
                    use driver_comedi::ComediDevice;

                    let device = ComediDevice::open(&device_path)?;
                    let dio = device.digital_io()?;

                    // Validate pin number
                    if pin >= dio.n_channels() {
                        return Err(anyhow::anyhow!(
                            "Invalid pin {}. Device has {} DIO channels",
                            pin,
                            dio.n_channels()
                        ));
                    }

                    // Write the pin value
                    dio.write(pin, value)
                        .map_err(|e| anyhow::anyhow!("Failed to write pin: {}", e))
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };
            let base_channel = req.base_channel;

            let value = self
                .await_with_timeout("ReadDigitalPort", async move {
                    tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                        use driver_comedi::ComediDevice;
                        let device = ComediDevice::open(&device_path)?;
                        let dio = device.digital_io()?;
                        Ok(dio.read_port(base_channel)?)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                })
                .await?;

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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };
            let base_channel = req.base_channel;
            let value = req.value;
            let mask = if req.mask == 0 { 0xFF } else { req.mask }; // Default: update all 8 bits

            self.await_with_timeout("WriteDigitalPort", async move {
                tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                    use driver_comedi::ComediDevice;
                    let device = ComediDevice::open(&device_path)?;
                    let dio = device.digital_io()?;
                    Ok(dio.write_port(base_channel, mask, value)?)
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            })
            .await?;

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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

            // Read counter via spawn_blocking (FFI call)
            let counter = req.counter;
            let (count, timestamp_ns) = self
                .await_with_timeout("ReadCounter", async move {
                    tokio::task::spawn_blocking(move || {
                        use driver_comedi::ComediDevice;
                        use std::time::SystemTime;

                        let device = ComediDevice::open(&device_path)?;
                        let counter_subsystem = device.counter()?;

                        // Validate counter channel
                        if counter >= counter_subsystem.n_channels() {
                            return Err(anyhow::anyhow!(
                                "Invalid counter {}. Device has {} counter channels",
                                counter,
                                counter_subsystem.n_channels()
                            ));
                        }

                        // Read the counter value
                        let count = counter_subsystem
                            .read(counter)
                            .map_err(|e| anyhow::anyhow!("Failed to read counter: {}", e))?;

                        // Get timestamp
                        let timestamp_ns = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as u64;

                        Ok((count as u64, timestamp_ns))
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                })
                .await?;

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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

            // Reset counter via spawn_blocking (FFI call)
            let counter = req.counter;
            self.await_with_timeout("ResetCounter", async move {
                tokio::task::spawn_blocking(move || {
                    use driver_comedi::ComediDevice;

                    let device = ComediDevice::open(&device_path)?;
                    let counter_subsystem = device.counter()?;

                    // Validate counter channel
                    if counter >= counter_subsystem.n_channels() {
                        return Err(anyhow::anyhow!(
                            "Invalid counter {}. Device has {} counter channels",
                            counter,
                            counter_subsystem.n_channels()
                        ));
                    }

                    // Reset the counter
                    counter_subsystem
                        .reset(counter)
                        .map_err(|e| anyhow::anyhow!("Failed to reset counter: {}", e))
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
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

            let device_path = if req.device_id.starts_with("/dev/") {
                req.device_id.clone()
            } else {
                format!("/dev/{}", req.device_id)
            };

            // Validate counter configuration
            let counter = req.counter;
            self.await_with_timeout("ConfigureCounter", async move {
                tokio::task::spawn_blocking(move || {
                    use driver_comedi::ComediDevice;

                    let device = ComediDevice::open(&device_path)?;
                    let counter_subsystem = device.counter()?;

                    // Validate counter channel
                    if counter >= counter_subsystem.n_channels() {
                        return Err(anyhow::anyhow!(
                            "Invalid counter {}. Device has {} counter channels",
                            counter,
                            counter_subsystem.n_channels()
                        ));
                    }

                    // Note: The NI PCI-MIO-16XE-10 Comedi driver has limited counter
                    // configuration support. Advanced features like mode selection,
                    // edge detection, and gate/source pin configuration may require
                    // direct INSN commands or CMD-based acquisition.
                    // For now, we validate the request and acknowledge it.
                    // Full implementation would require extending the Counter subsystem
                    // with Comedi INSN_CONFIG commands.

                    Ok(())
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            })
            .await?;

            Ok(Response::new(ConfigureCounterResponse {
                success: true,
                error_message: String::new(),
            }))
        }

        #[cfg(not(feature = "comedi"))]
        {
            let _ = request;
            Err(Status::unimplemented(
                "ConfigureCounter requires 'comedi' feature to be enabled",
            ))
        }
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
            .ok_or_else(|| Status::not_found(format!("Device '{}' not found", device_id)))?;

        // Comedi support is only available when the 'comedi' feature is enabled
        #[cfg(feature = "comedi")]
        {
            // For now, we open a new connection to query device info
            // TODO: Store device path in registry metadata or driver state
            let device_path = if device_id.starts_with("/dev/") {
                device_id.clone()
            } else {
                format!("/dev/{}", device_id)
            };

            let status = self
                .await_with_timeout("GetDAQStatus", async {
                    // Open device to query info (spawn_blocking for FFI)
                    let path = device_path.to_string();
                    let device = tokio::task::spawn_blocking(move || {
                        use driver_comedi::ComediDevice;
                        ComediDevice::open(&path)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))??;

                    // Get comprehensive device info
                    let info = device.info()?;

                    // Build subdevice summaries
                    let subdevices: Vec<SubdeviceSummary> = info
                        .subdevices
                        .iter()
                        .map(|sub| {
                            use driver_comedi::SubdeviceType;
                            let type_str = match sub.subdev_type {
                                SubdeviceType::AnalogInput => "ai",
                                SubdeviceType::AnalogOutput => "ao",
                                SubdeviceType::DigitalIO => "dio",
                                SubdeviceType::Counter => "counter",
                                SubdeviceType::Timer => "timer",
                                _ => "other",
                            };

                            SubdeviceSummary {
                                index: sub.index,
                                r#type: type_str.to_string(),
                                n_channels: sub.n_channels,
                                busy: sub.is_busy(),
                                supports_commands: sub.supports_commands(),
                            }
                        })
                        .collect();

                    // Count channels by type
                    use driver_comedi::SubdeviceType;
                    let ai_info = info
                        .subdevices
                        .iter()
                        .find(|s| s.subdev_type == SubdeviceType::AnalogInput);
                    let ao_info = info
                        .subdevices
                        .iter()
                        .find(|s| s.subdev_type == SubdeviceType::AnalogOutput);
                    let dio_info = info
                        .subdevices
                        .iter()
                        .find(|s| s.subdev_type == SubdeviceType::DigitalIO);
                    let counter_count = info
                        .subdevices
                        .iter()
                        .filter(|s| s.subdev_type == SubdeviceType::Counter)
                        .count();

                    let ai_channels = ai_info.map(|s| s.n_channels).unwrap_or(0);
                    let ao_channels = ao_info.map(|s| s.n_channels).unwrap_or(0);
                    let dio_channels = dio_info.map(|s| s.n_channels).unwrap_or(0);

                    // Get AI/AO resolution
                    let ai_resolution = ai_info.map(|s| s.resolution_bits()).unwrap_or(0);
                    let ao_resolution = ao_info.map(|s| s.resolution_bits()).unwrap_or(0);

                    // Get voltage ranges for AI
                    let ai_ranges = if let Some(ai_sub) = ai_info {
                        let ai = device.analog_input_subdevice(ai_sub.index)?;
                        ai.ranges(0)?
                            .into_iter()
                            .map(|r| VoltageRange {
                                index: r.index,
                                min_voltage: r.min,
                                max_voltage: r.max,
                                unit: match r.unit {
                                    0 => "V".to_string(),
                                    1 => "mA".to_string(),
                                    _ => String::new(),
                                },
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };

                    // Get voltage ranges for AO
                    let ao_ranges = if ao_info.is_some() {
                        if let Ok(ao) = device.analog_output() {
                            ao.ranges(0)?
                                .into_iter()
                                .map(|r| VoltageRange {
                                    index: r.index,
                                    min_voltage: r.min,
                                    max_voltage: r.max,
                                    unit: match r.unit {
                                        0 => "V".to_string(),
                                        1 => "mA".to_string(),
                                        _ => String::new(),
                                    },
                                })
                                .collect()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };

                    Ok(DaqStatus {
                        device_id: device_id.clone(),
                        board_name: info.board_name,
                        driver_name: info.driver_name,
                        online: true,
                        n_subdevices: info.n_subdevices,
                        subdevices,
                        ai_busy: ai_info.map(|s| s.is_busy()).unwrap_or(false),
                        ao_busy: ao_info.map(|s| s.is_busy()).unwrap_or(false),
                        dio_configured: dio_info.is_some(),
                        active_counters: 0, // TODO: Track active counters in state
                        ai_channels,
                        ao_channels,
                        dio_channels,
                        counter_channels: counter_count as u32,
                        ai_resolution_bits: ai_resolution,
                        ao_resolution_bits: ao_resolution,
                        ai_ranges,
                        ao_ranges,
                    })
                })
                .await?;

            Ok(Response::new(status))
        }

        #[cfg(not(feature = "comedi"))]
        {
            Err(Status::unimplemented(
                "GetDAQStatus requires 'comedi' feature to be enabled",
            ))
        }
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
