//! Photometrics PVCAM camera driver V1 (PrimeBSI)
//!
//! This module provides a V1 `Instrument` implementation for Photometrics cameras
//! using the PVCAM SDK. This V1 implementation broadcasts frame statistics (mean, min, max)
//! as scalar DataPoints.
//!
//! **Note**: Image viewing is not supported in V1 instruments. For full image data
//! support, use the V2 PVCAM implementation (`pvcam_v2` type) which natively broadcasts
//! `Measurement::Image` data. V2 integration is planned for Phase 3 (bd-51).
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.prime_bsi]
//! type = "pvcam"
//! camera_name = "PrimeBSI"
//! exposure_ms = 100.0
//! roi = [0, 0, 2048, 2048]  # [x, y, width, height]
//! binning = [1, 1]  # [x_bin, y_bin]
//! polling_rate_hz = 10.0
//! ```
//!
//! Note: This driver requires the PVCAM SDK to be installed and linked.

use crate::{
    config::Settings,
    core::{DataPoint, Instrument, InstrumentCommand},
    measurement::InstrumentMeasurement,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;

/// PVCAM camera instrument implementation (V1)
///
/// This V1 implementation broadcasts frame statistics only. For image data support,
/// use the V2 PVCAM implementation.
#[derive(Clone)]
pub struct PVCAMCamera {
    id: String,
    camera_name: String,
    exposure_ms: f64,
    measurement: Option<InstrumentMeasurement>,
}

impl PVCAMCamera {
    /// Creates a new PVCAM camera instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            camera_name: "PrimeBSI".to_string(),
            exposure_ms: 100.0,
            measurement: None,
        }
    }

    // Note: Actual PVCAM SDK calls would go here
    // This is a placeholder implementation that generates synthetic data
    fn simulate_frame_data(&self) -> Vec<u16> {
        // Generate a 512x512 synthetic image
        let width = 512;
        let height = 512;
        let mut frame = vec![0u16; width * height];

        // Simple pattern for testing
        for y in 0..height {
            for x in 0..width {
                let value = ((x + y) % 256) as u16 * 256;
                frame[y * width + x] = value;
            }
        }

        frame
    }

    fn calculate_frame_stats(&self, frame: &[u16]) -> (f64, f64, f64) {
        if frame.is_empty() {
            return (0.0, 0.0, 0.0);
        }

        let sum: u64 = frame.iter().map(|&v| v as u64).sum();
        let mean = sum as f64 / frame.len() as f64;

        let min = *frame.iter().min().unwrap_or(&0) as f64;
        let max = *frame.iter().max().unwrap_or(&0) as f64;

        (mean, min, max)
    }
}

#[async_trait]
impl Instrument for PVCAMCamera {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        self.id.clone()
    }

    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to PVCAM camera: {}", id);
        self.id = id.to_string();

        let instrument_config = settings
            .instruments
            .get(id)
            .ok_or_else(|| anyhow!("Configuration for '{}' not found", id))?;

        self.camera_name = instrument_config
            .get("camera_name")
            .and_then(|v| v.as_str())
            .unwrap_or("PrimeBSI")
            .to_string();

        self.exposure_ms = instrument_config
            .get("exposure_ms")
            .and_then(|v| v.as_float())
            .unwrap_or(100.0);

        info!(
            "Camera: {}, Exposure: {} ms",
            self.camera_name, self.exposure_ms
        );

        // TODO: Initialize PVCAM SDK
        // pl_pvcam_init()
        // pl_cam_open()
        // Configure ROI, binning, exposure time, etc.

        warn!("PVCAM SDK integration not yet implemented - using simulated data");

        // Create broadcast channel with configured capacity
        let capacity = settings.application.broadcast_channel_capacity;
        let measurement = InstrumentMeasurement::new(capacity, self.id.clone());
        self.measurement = Some(measurement.clone());

        // Spawn acquisition task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(10.0);

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate));

            let mut frame_count = 0u64;

            loop {
                interval.tick().await;

                let timestamp = chrono::Utc::now();

                // TODO: Acquire actual frame from PVCAM
                // pl_exp_start_seq()
                // pl_exp_check_status()
                // pl_exp_get_latest_frame()

                // For now, simulate frame acquisition
                let frame_data = instrument.simulate_frame_data();
                let (mean, min, max) = instrument.calculate_frame_stats(&frame_data);

                frame_count += 1;

                // Send frame statistics as data points
                // Use instrument ID as channel name for default plot compatibility
                let dp_mean = DataPoint {
                    timestamp,
                    instrument_id: instrument.id.clone(),
                    channel: instrument.id.clone(), // Use ID as channel for GUI plot
                    value: mean,
                    unit: "counts".to_string(),
                    metadata: Some(serde_json::json!({
                        "frame": frame_count,
                        "stat_type": "mean"
                    })),
                };

                let dp_min = DataPoint {
                    timestamp,
                    instrument_id: instrument.id.clone(),
                    channel: "min_intensity".to_string(),
                    value: min,
                    unit: "counts".to_string(),
                    metadata: Some(serde_json::json!({"frame": frame_count})),
                };

                let dp_max = DataPoint {
                    timestamp,
                    instrument_id: instrument.id.clone(),
                    channel: "max_intensity".to_string(),
                    value: max,
                    unit: "counts".to_string(),
                    metadata: Some(serde_json::json!({"frame": frame_count})),
                };

                if measurement.broadcast(dp_mean).await.is_err()
                    || measurement.broadcast(dp_min).await.is_err()
                    || measurement.broadcast(dp_max).await.is_err()
                {
                    warn!("No active receivers for PVCAM camera data");
                    break;
                }
            }
        });

        info!("PVCAM camera '{}' connected successfully", self.id);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from PVCAM camera: {}", self.id);

        // TODO: Cleanup PVCAM SDK
        // pl_cam_close()
        // pl_pvcam_uninit()

        self.measurement = None;
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement.as_ref().unwrap()
    }

    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => {
                match key.as_str() {
                    "exposure_ms" => {
                        self.exposure_ms = value.as_f64().unwrap_or(self.exposure_ms);
                        info!("PVCAM exposure set to {} ms", self.exposure_ms);
                        // TODO: Apply to camera hardware
                    }
                    "gain" => {
                        // TODO: Set camera gain
                        info!("PVCAM gain set to {}", value);
                    }
                    "binning" => {
                        // TODO: Set camera binning
                        info!("PVCAM binning set to {}", value);
                    }
                    _ => {
                        warn!("Unknown parameter '{}' for PVCAM", key);
                    }
                }
            }
            InstrumentCommand::Execute(cmd, _) => {
                match cmd.as_str() {
                    "start_acquisition" => {
                        info!("PVCAM starting continuous acquisition");
                        // TODO: Start continuous acquisition
                    }
                    "stop_acquisition" => {
                        info!("PVCAM stopping acquisition");
                        // TODO: Stop acquisition
                    }
                    "snap" => {
                        info!("PVCAM snap single frame");
                        // TODO: Acquire single frame
                    }
                    _ => {
                        warn!("Unknown command '{}' for PVCAM", cmd);
                    }
                }
            }
            _ => {
                warn!("Unsupported command type for PVCAM");
            }
        }
        Ok(())
    }
}