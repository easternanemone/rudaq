use crate::core::{DataProcessorAdapter, MeasurementProcessor};
use crate::data::fft::{FFTConfig, FFTProcessor};
use crate::data::iir_filter::{IirFilter, IirFilterConfig};
use std::collections::HashMap;
use toml::Value;

type ProcessorFactory =
    Box<dyn Fn(&Value) -> Result<Box<dyn MeasurementProcessor>, anyhow::Error> + Send + Sync>;

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

        // Register IIR Filter (wrapped in adapter for backward compatibility)
        factories.insert(
            "iir".to_string(),
            Box::new(|config| {
                let iir_config: IirFilterConfig = config.clone().try_into()?;
                let filter = IirFilter::new(iir_config).map_err(|e| anyhow::anyhow!(e))?;
                let adapted = DataProcessorAdapter::new(Box::new(filter));
                Ok(Box::new(adapted) as Box<dyn MeasurementProcessor>)
            }),
        );

        // Register FFT Processor (native MeasurementProcessor - no adapter needed)
        factories.insert(
            "fft".to_string(),
            Box::new(|config| {
                let fft_config: FFTConfig = config.clone().try_into()?;
                let processor = FFTProcessor::new(fft_config);
                Ok(Box::new(processor) as Box<dyn MeasurementProcessor>)
            }),
        );

        Self { factories }
    }

    pub fn create(
        &self,
        id: &str,
        config: &Value,
    ) -> Result<Box<dyn MeasurementProcessor>, anyhow::Error> {
        self.factories
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Processor '{}' not found", id))
            .and_then(|factory| factory(config))
    }
}
