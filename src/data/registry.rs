use crate::core::DataProcessor;
use crate::data::fft::FFTProcessor;
use crate::data::iir_filter::{IirFilter, IirFilterConfig};
use std::collections::HashMap;
use toml::Value;

type ProcessorFactory = Box<dyn Fn(&Value) -> Result<Box<dyn DataProcessor>, anyhow::Error> + Send + Sync>;

pub struct ProcessorRegistry {
    factories: HashMap<String, ProcessorFactory>,
}

impl Default for ProcessorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessorRegistry {
    pub fn new() -> Self {
        let mut factories: HashMap<String, ProcessorFactory> = HashMap::new();

        // Register IIR Filter
        factories.insert(
            "iir".to_string(),
            Box::new(|config| {
                let iir_config: IirFilterConfig = config.clone().try_into()?;
                let filter = IirFilter::new(iir_config).map_err(|e| anyhow::anyhow!(e))?;
                Ok(Box::new(filter))
            }),
        );

        // Register FFT Processor
        factories.insert(
            "fft".to_string(),
            Box::new(|config| {
                let window_size = config.get("window_size").and_then(Value::as_integer).unwrap_or(1024) as usize;
                let overlap = config.get("overlap").and_then(Value::as_integer).unwrap_or(512) as usize;
                let sampling_rate = config.get("sampling_rate").and_then(Value::as_float).unwrap_or(1024.0);
                let processor = FFTProcessor::new(window_size, overlap, sampling_rate);
                Ok(Box::new(processor))
            }),
        );

        Self { factories }
    }

    pub fn create(&self, id: &str, config: &Value) -> Result<Box<dyn DataProcessor>, anyhow::Error> {
        self.factories
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Processor '{}' not found", id))
            .and_then(|factory| factory(config))
    }
}