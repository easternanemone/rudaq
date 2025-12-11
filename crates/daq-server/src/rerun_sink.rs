//! Rerun Visualization Sink
//!
//! Pushes measurements to Rerun.io for visualization.

use anyhow::Result;
use async_trait::async_trait;
use daq_core::core::{Measurement, PixelBuffer};
use daq_core::pipeline::MeasurementSink;
use rerun::{RecordingStream, RecordingStreamBuilder};
use rerun::archetypes::{Scalars, Tensor};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub struct RerunSink {
    rec: RecordingStream,
}

impl RerunSink {
    /// Create a new Rerun sink that spawns a viewer or connects to a remote one.
    pub fn new(application_id: &str) -> Result<Self> {
        let rec = RecordingStreamBuilder::new(application_id)
            .spawn() // Spawns a viewer process or connects to one
            ?;
        Ok(Self { rec })
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
        
        rec.set_time_nanos("stable_time", ts.timestamp_nanos_opt().unwrap_or(0));

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
