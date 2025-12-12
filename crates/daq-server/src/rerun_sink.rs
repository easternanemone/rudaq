//! Rerun Visualization Sink
//!
//! Pushes measurements to Rerun.io for visualization.
//!
//! ## Blueprint Support
//!
//! Since the Rust Blueprint API is not yet available (see rerun-io/rerun#5521),
//! blueprints must be created using Python and loaded via `load_blueprint()`.
//!
//! Generate blueprints with:
//! ```bash
//! cd crates/daq-server/blueprints
//! pip install rerun-sdk
//! python generate_blueprints.py
//! ```
//!
//! Then load in Rust:
//! ```rust,ignore
//! let sink = RerunSink::new()?;
//! sink.load_blueprint("crates/daq-server/blueprints/daq_default.rbl")?;
//! ```

use anyhow::Result;
use async_trait::async_trait;
use daq_core::core::{Measurement, PixelBuffer};
use daq_core::pipeline::MeasurementSink;
use rerun::{RecordingStream, RecordingStreamBuilder};
use rerun::archetypes::{Scalars, Tensor};
use std::path::Path;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Default application ID - must match the Python blueprint generator
pub const APP_ID: &str = "rust-daq";

pub struct RerunSink {
    rec: RecordingStream,
}

impl RerunSink {
    /// Create a new Rerun sink that spawns a viewer or connects to a remote one.
    pub fn new() -> Result<Self> {
        Self::with_app_id(APP_ID)
    }

    /// Create a new Rerun sink with a custom application ID.
    ///
    /// Note: If using pre-generated blueprints, the app ID must match.
    pub fn with_app_id(application_id: &str) -> Result<Self> {
        let rec = RecordingStreamBuilder::new(application_id)
            .spawn() // Spawns a viewer process or connects to one
            ?;
        Ok(Self { rec })
    }

    /// Load a blueprint from an .rbl file.
    ///
    /// The blueprint's application ID must match the recording's application ID.
    /// Generate blueprints using `crates/daq-server/blueprints/generate_blueprints.py`.
    ///
    /// # Example
    /// ```rust,ignore
    /// let sink = RerunSink::new()?;
    /// sink.load_blueprint("crates/daq-server/blueprints/daq_default.rbl")?;
    /// ```
    pub fn load_blueprint(&self, path: impl AsRef<Path>) -> Result<()> {
        self.rec.log_file_from_path(
            path,
            None,   // No entity path prefix
            true,   // Static (blueprint doesn't change over time)
        )?;
        Ok(())
    }

    /// Load a blueprint only if the file exists.
    ///
    /// Returns `Ok(true)` if the blueprint was loaded, `Ok(false)` if the path
    /// does not exist, and `Err` if loading failed.
    pub fn load_blueprint_if_exists(&self, path: impl AsRef<Path>) -> Result<bool> {
        let path_ref = path.as_ref();
        if !path_ref.exists() {
            return Ok(false);
        }
        self.load_blueprint(path_ref)?;
        Ok(true)
    }

    /// Subscribe to a broadcast channel and log all received measurements.
    pub fn monitor_broadcast(&self, mut rx: tokio::sync::broadcast::Receiver<Measurement>) {
        let rec = self.rec.clone();
        tokio::spawn(async move {
            while let Ok(meas) = rx.recv().await {
                Self::log_measurement(&rec, meas);
            }
        });
    }

    fn log_measurement(rec: &RecordingStream, meas: Measurement) {
        let name = match &meas {
            Measurement::Scalar { name, .. } => name,
            Measurement::Vector { name, .. } => name,
            Measurement::Image { name, .. } => name,
            Measurement::Spectrum { name, .. } => name,
        };
        
        let entity_path = format!("device/{}", name);
        
        // Extract timestamp
        let ts = match &meas {
            Measurement::Scalar { timestamp, .. } => timestamp,
            Measurement::Vector { timestamp, .. } => timestamp,
            Measurement::Image { timestamp, .. } => timestamp,
            Measurement::Spectrum { timestamp, .. } => timestamp,
        };
        
        rec.set_time(
            "stable_time",
            rerun::TimeCell::from_timestamp_nanos_since_epoch(ts.timestamp_nanos_opt().unwrap_or(0)),
        );

        match meas {
            Measurement::Scalar { value, .. } => {
                let _ = rec.log(entity_path, &Scalars::new([value]));
            }
            Measurement::Image { width, height, buffer, .. } => {
                let shape = vec![height as u64, width as u64];
                match buffer {
                    PixelBuffer::U8(data) => {
                        let tensor_data = rerun::TensorData::new(shape, rerun::TensorBuffer::U8(data.into()));
                        let _ = rec.log(entity_path, &Tensor::new(tensor_data));
                    }
                    PixelBuffer::U16(data) => {
                        let tensor_data = rerun::TensorData::new(shape, rerun::TensorBuffer::U16(data.into()));
                        let _ = rec.log(entity_path, &Tensor::new(tensor_data));
                    }
                    _ => {} 
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl MeasurementSink for RerunSink {
    type Input = Measurement;
    type Error = anyhow::Error;

    fn register_input(
        &mut self,
        mut rx: mpsc::Receiver<Self::Input>,
    ) -> Result<JoinHandle<()>, Self::Error> {
        let rec = self.rec.clone();

        Ok(tokio::spawn(async move {
            while let Some(meas) = rx.recv().await {
                Self::log_measurement(&rec, meas);
            }
        }))
    }
}
