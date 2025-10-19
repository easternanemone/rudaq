//! A mock instrument that generates synthetic data.
use crate::{
    config::Settings,
    core::{DataPoint, Instrument, InstrumentCommand},
    measurement::InstrumentMeasurement,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::info;
use std::sync::Arc;
use tokio::time::{interval, Duration};

pub struct MockInstrument {
    id: String,
    measurement: Option<InstrumentMeasurement>,
}

impl Default for MockInstrument {
    fn default() -> Self {
        Self::new()
    }
}

impl MockInstrument {
    pub fn new() -> Self {
        Self {
            id: String::new(),
            measurement: None,
        }
    }
}

#[async_trait]
impl Instrument for MockInstrument {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        "Mock Instrument".to_string()
    }

    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to Mock Instrument '{}'...", id);
        self.id = id.to_string();
        let capacity = settings.application.broadcast_channel_capacity;
        let measurement = InstrumentMeasurement::new(capacity, self.id.clone());
        self.measurement = Some(measurement.clone());

        let settings = settings.clone();
        let instrument_id = self.id.clone();

        // Spawn a task to generate data
        tokio::spawn(async move {
            let config = settings.instruments.get(&instrument_id).unwrap().clone();
            let sample_rate = config.get("sample_rate_hz").unwrap().as_float().unwrap();
            let num_samples = config.get("num_samples").unwrap().as_integer().unwrap() as usize;
            let mut interval = interval(Duration::from_secs_f64(1.0 / sample_rate));
            let mut phase: f64 = 0.0;

            for _ in 0..num_samples {
                interval.tick().await;
                let now = chrono::Utc::now();
                phase += 0.1;

                // Use a simple deterministic noise instead of thread_rng for Send compatibility
                let noise = (phase * 37.0).sin() * 0.05;

                let sine_dp = DataPoint {
                    timestamp: now,
                    instrument_id: instrument_id.clone(),
                    channel: "sine_wave".to_string(),
                    value: phase.sin() + noise,
                    unit: "V".to_string(),
                    metadata: None,
                };
                let cosine_dp = DataPoint {
                    timestamp: now,
                    instrument_id: instrument_id.clone(),
                    channel: "cosine_wave".to_string(),
                    value: phase.cos() + noise * 0.8,
                    unit: "V".to_string(),
                    metadata: None,
                };

                // Broadcast using the new pattern
                if measurement.broadcast(sine_dp).await.is_err() {
                    break;
                }
                if measurement.broadcast(cosine_dp).await.is_err() {
                    break;
                }
            }
            info!(
                "Mock instrument finished generating {} samples.",
                num_samples
            );
        });

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from Mock Instrument.");
        self.measurement = None;
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement.as_ref().unwrap()
    }
}
