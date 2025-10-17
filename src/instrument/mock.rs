//! A mock instrument that generates synthetic data.
use crate::{
    config::Settings,
    core::{DataPoint, Instrument},
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::info;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};

pub struct MockInstrument {
    id: String,
    sender: Option<broadcast::Sender<DataPoint>>,
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
            sender: None,
        }
    }
}

#[async_trait]
impl Instrument for MockInstrument {
    fn name(&self) -> String {
        "Mock Instrument".to_string()
    }

    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to Mock Instrument '{}'...", id);
        self.id = id.to_string();
        let (sender, _) = broadcast::channel(1024);
        self.sender = Some(sender.clone());

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

                // Ignore errors if no receivers are active
                if sender.send(sine_dp).is_err() || sender.send(cosine_dp).is_err() {
                    // Stop if the receiver has been dropped
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
        self.sender = None;
        Ok(())
    }

    async fn data_stream(&mut self) -> Result<broadcast::Receiver<DataPoint>> {
        self.sender
            .as_ref()
            .map(|s| s.subscribe())
            .ok_or_else(|| anyhow!("Not connected to mock instrument"))
    }
}
