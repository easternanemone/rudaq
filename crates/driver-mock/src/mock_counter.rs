//! Mock counter/timer implementation.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{
    CounterConfig, CounterConfigurable, CounterEdge, CounterMode, Parameterized, Readable, Settable,
};
use common::driver::{Capability, DeviceComponents, DeviceMetadata, DriverFactory};
use common::observable::ParameterSet;
use common::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

// =============================================================================
// MockCounterFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MockCounter driver.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MockCounterConfig {
    /// Counter channel number (default: 0).
    #[serde(default)]
    pub channel: u32,
}

/// Factory for creating MockCounter instances.
pub struct MockCounterFactory;

static MOCK_COUNTER_CAPABILITIES: &[Capability] = &[
    Capability::Readable,
    Capability::Settable,
    Capability::CounterConfigurable,
    Capability::Parameterized,
];

impl DriverFactory for MockCounterFactory {
    fn driver_type(&self) -> &'static str {
        "mock_counter"
    }

    fn name(&self) -> &'static str {
        "Mock Counter/Timer"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MOCK_COUNTER_CAPABILITIES
    }

    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MockCounterConfig = config.try_into().unwrap_or_default();
            let counter = Arc::new(MockCounter::new(cfg.channel));

            Ok(DeviceComponents {
                readable: Some(counter.clone()),
                settable: Some(counter.clone()),
                counter_configurable: Some(counter.clone()),
                parameterized: Some(counter),
                metadata: DeviceMetadata {
                    panel_kind: Some(common::panel_kind::COMEDI.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// MockCounter
// =============================================================================

/// A simulated counter/timer device.
///
/// Stores a count value and counter configuration as `Parameter<T>` for
/// reactive state management and subscriber notifications.
pub struct MockCounter {
    channel: u32,
    value: Parameter<u32>,
    config: Parameter<CounterConfig>,
    params: ParameterSet,
}

impl MockCounter {
    /// Create a new mock counter for the given channel.
    pub fn new(channel: u32) -> Self {
        let mut params = ParameterSet::new();

        let value = Parameter::new("value", 0u32)
            .with_description("Counter value")
            .with_dtype("int");

        let config = Parameter::new(
            "config",
            CounterConfig {
                mode: CounterMode::EventCounting,
                edge: CounterEdge::Rising,
                gate_source: None,
                clock_source: None,
            },
        )
        .with_description("Counter configuration");

        params.register(value.clone());
        params.register(config.clone());

        Self {
            channel,
            value,
            config,
            params,
        }
    }

    /// Get the channel number.
    pub fn channel(&self) -> u32 {
        self.channel
    }
}

impl Parameterized for MockCounter {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Readable for MockCounter {
    async fn read(&self) -> Result<f64> {
        Ok(f64::from(self.value.get()))
    }
}

#[async_trait]
impl Settable for MockCounter {
    async fn set_value(&self, name: &str, value: Value) -> Result<()> {
        match name {
            "value" | "count" => {
                #[allow(clippy::cast_possible_truncation)]
                let v = value
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("count must be an unsigned integer"))?
                    as u32;
                self.value.set(v).await?;
                Ok(())
            }
            "reset" | "reset_all" => {
                self.value.set(0).await?;
                Ok(())
            }
            _ => anyhow::bail!("Unknown parameter: {name}"),
        }
    }

    async fn get_value(&self, name: &str) -> Result<Value> {
        match name {
            "value" | "count" => Ok(Value::from(self.value.get())),
            "maxdata" => Ok(Value::from(u64::from(u32::MAX))),
            "bit_width" => Ok(Value::from(32)),
            "n_channels" => Ok(Value::from(1)),
            _ => anyhow::bail!("Unknown parameter: {name}"),
        }
    }
}

#[async_trait]
impl CounterConfigurable for MockCounter {
    async fn configure_counter(&self, config: CounterConfig) -> Result<()> {
        self.config.set(config).await?;
        Ok(())
    }

    async fn get_counter_config(&self) -> Result<CounterConfig> {
        Ok(self.config.get())
    }
}

impl std::fmt::Debug for MockCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockCounter")
            .field("channel", &self.channel)
            .finish_non_exhaustive()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_counter_configurable_roundtrip() {
        let counter = MockCounter::new(0);

        // Default config
        let config = counter.get_counter_config().await.expect("get config");
        assert_eq!(config.mode, CounterMode::EventCounting);
        assert_eq!(config.edge, CounterEdge::Rising);
        assert!(config.gate_source.is_none());
        assert!(config.clock_source.is_none());

        // Configure with new settings
        let new_config = CounterConfig {
            mode: CounterMode::FrequencyMeasurement,
            edge: CounterEdge::Both,
            gate_source: Some(3),
            clock_source: Some(7),
        };
        counter
            .configure_counter(new_config.clone())
            .await
            .expect("configure");

        // Read back
        let read_back = counter.get_counter_config().await.expect("get config");
        assert_eq!(read_back.mode, CounterMode::FrequencyMeasurement);
        assert_eq!(read_back.edge, CounterEdge::Both);
        assert_eq!(read_back.gate_source, Some(3));
        assert_eq!(read_back.clock_source, Some(7));
    }

    #[tokio::test]
    async fn test_mock_counter_read_write() {
        let counter = MockCounter::new(0);

        // Initial value is 0
        let val = counter.read().await.expect("read");
        assert_eq!(val, 0.0);

        // Write a value
        counter
            .set_value("count", Value::from(42))
            .await
            .expect("set");
        let val = counter.read().await.expect("read");
        assert_eq!(val, 42.0);

        // Reset
        counter
            .set_value("reset", Value::Null)
            .await
            .expect("reset");
        let val = counter.read().await.expect("read");
        assert_eq!(val, 0.0);
    }

    #[tokio::test]
    async fn test_mock_counter_factory() {
        let factory = MockCounterFactory;
        assert_eq!(factory.driver_type(), "mock_counter");
        assert_eq!(factory.name(), "Mock Counter/Timer");

        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Readable));
        assert!(caps.contains(&Capability::Settable));
        assert!(caps.contains(&Capability::CounterConfigurable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_mock_counter_parameterized() {
        let counter = MockCounter::new(0);

        // Parameters should be accessible via Parameterized trait
        let params = counter.parameters();
        assert!(params.get("value").is_some());
        assert!(params.get("config").is_some());
    }
}
