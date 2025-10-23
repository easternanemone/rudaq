use crate::core::{InstrumentCommand, ParameterValue};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::any::TypeId;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Capability for instruments that expose position control semantics.
#[async_trait]
pub trait PositionControl: Send + Sync {
    async fn move_absolute(&self, axis: u8, position: f64) -> Result<()>;
    async fn move_relative(&self, axis: u8, distance: f64) -> Result<()>;
    async fn stop(&self, axis: u8) -> Result<()>;
}

/// Capability for instruments that can report power measurements.
#[async_trait]
pub trait PowerMeasurement: Send + Sync {
    async fn start_sampling(&self) -> Result<()>;
    async fn stop_sampling(&self) -> Result<()>;
    async fn set_range(&self, range: f64) -> Result<()>;
}

/// Capability for frequency-domain analysis instruments.
#[async_trait]
pub trait SpectrumAnalyzer: Send + Sync {
    async fn capture(&self) -> Result<()>;
    async fn set_span(&self, span_hz: f64) -> Result<()>;
}

fn position_control_type_id() -> TypeId {
    TypeId::of::<dyn PositionControl>()
}

fn power_measurement_type_id() -> TypeId {
    TypeId::of::<dyn PowerMeasurement>()
}

fn spectrum_analyzer_type_id() -> TypeId {
    TypeId::of::<dyn SpectrumAnalyzer>()
}

#[derive(Clone)]
struct PositionControlProxy {
    instrument_id: String,
    command_tx: mpsc::Sender<InstrumentCommand>,
}

impl PositionControlProxy {
    fn new(instrument_id: String, command_tx: mpsc::Sender<InstrumentCommand>) -> Self {
        Self {
            instrument_id,
            command_tx,
        }
    }

    async fn send_operation(&self, operation: &str, parameters: Vec<ParameterValue>) -> Result<()> {
        let command = InstrumentCommand::Capability {
            capability: position_control_type_id(),
            operation: operation.to_string(),
            parameters,
        };

        self.command_tx.send(command).await.map_err(|err| {
            anyhow!(
                "PositionControlProxy failed to send command to '{}': {}",
                self.instrument_id,
                err
            )
        })
    }
}

#[async_trait]
impl PositionControl for PositionControlProxy {
    async fn move_absolute(&self, axis: u8, position: f64) -> Result<()> {
        self.send_operation(
            "move_absolute",
            vec![ParameterValue::from(axis), ParameterValue::from(position)],
        )
        .await
    }

    async fn move_relative(&self, axis: u8, distance: f64) -> Result<()> {
        self.send_operation(
            "move_relative",
            vec![ParameterValue::from(axis), ParameterValue::from(distance)],
        )
        .await
    }

    async fn stop(&self, axis: u8) -> Result<()> {
        self.send_operation("stop", vec![ParameterValue::from(axis)])
            .await
    }
}

#[derive(Clone)]
struct PowerMeasurementProxy {
    instrument_id: String,
    command_tx: mpsc::Sender<InstrumentCommand>,
}

impl PowerMeasurementProxy {
    fn new(instrument_id: String, command_tx: mpsc::Sender<InstrumentCommand>) -> Self {
        Self {
            instrument_id,
            command_tx,
        }
    }

    async fn send_operation(&self, operation: &str, parameters: Vec<ParameterValue>) -> Result<()> {
        let command = InstrumentCommand::Capability {
            capability: power_measurement_type_id(),
            operation: operation.to_string(),
            parameters,
        };

        self.command_tx.send(command).await.map_err(|err| {
            anyhow!(
                "PowerMeasurementProxy failed to send command to '{}': {}",
                self.instrument_id,
                err
            )
        })
    }
}

#[async_trait]
impl PowerMeasurement for PowerMeasurementProxy {
    async fn start_sampling(&self) -> Result<()> {
        self.send_operation("start_sampling", Vec::new()).await
    }

    async fn stop_sampling(&self) -> Result<()> {
        self.send_operation("stop_sampling", Vec::new()).await
    }

    async fn set_range(&self, range: f64) -> Result<()> {
        self.send_operation("set_range", vec![ParameterValue::from(range)])
            .await
    }
}

#[derive(Clone)]
struct SpectrumAnalyzerProxy {
    instrument_id: String,
    command_tx: mpsc::Sender<InstrumentCommand>,
}

impl SpectrumAnalyzerProxy {
    fn new(instrument_id: String, command_tx: mpsc::Sender<InstrumentCommand>) -> Self {
        Self {
            instrument_id,
            command_tx,
        }
    }

    async fn send_operation(&self, operation: &str, parameters: Vec<ParameterValue>) -> Result<()> {
        let command = InstrumentCommand::Capability {
            capability: spectrum_analyzer_type_id(),
            operation: operation.to_string(),
            parameters,
        };

        self.command_tx.send(command).await.map_err(|err| {
            anyhow!(
                "SpectrumAnalyzerProxy failed to send command to '{}': {}",
                self.instrument_id,
                err
            )
        })
    }
}

#[async_trait]
impl SpectrumAnalyzer for SpectrumAnalyzerProxy {
    async fn capture(&self) -> Result<()> {
        self.send_operation("capture", Vec::new()).await
    }

    async fn set_span(&self, span_hz: f64) -> Result<()> {
        self.send_operation("set_span", vec![ParameterValue::from(span_hz)])
            .await
    }
}

/// Erased capability proxy handle used by the module assignment system.
#[derive(Clone)]
pub enum CapabilityProxyHandle {
    PositionControl(Arc<dyn PositionControl>),
    PowerMeasurement(Arc<dyn PowerMeasurement>),
    SpectrumAnalyzer(Arc<dyn SpectrumAnalyzer>),
}

impl CapabilityProxyHandle {
    pub fn capability_id(&self) -> TypeId {
        match self {
            CapabilityProxyHandle::PositionControl(_) => position_control_type_id(),
            CapabilityProxyHandle::PowerMeasurement(_) => power_measurement_type_id(),
            CapabilityProxyHandle::SpectrumAnalyzer(_) => spectrum_analyzer_type_id(),
        }
    }

    pub fn as_position_control(&self) -> Option<Arc<dyn PositionControl>> {
        match self {
            CapabilityProxyHandle::PositionControl(proxy) => Some(proxy.clone()),
            _ => None,
        }
    }

    pub fn as_power_measurement(&self) -> Option<Arc<dyn PowerMeasurement>> {
        match self {
            CapabilityProxyHandle::PowerMeasurement(proxy) => Some(proxy.clone()),
            _ => None,
        }
    }

    pub fn as_spectrum_analyzer(&self) -> Option<Arc<dyn SpectrumAnalyzer>> {
        match self {
            CapabilityProxyHandle::SpectrumAnalyzer(proxy) => Some(proxy.clone()),
            _ => None,
        }
    }
}

/// Builds a capability proxy for the provided capability identifier.
pub fn create_proxy(
    capability: TypeId,
    instrument_id: impl Into<String>,
    command_tx: mpsc::Sender<InstrumentCommand>,
) -> Result<CapabilityProxyHandle> {
    let instrument_id = instrument_id.into();

    if capability == position_control_type_id() {
        Ok(CapabilityProxyHandle::PositionControl(Arc::new(
            PositionControlProxy::new(instrument_id, command_tx),
        )))
    } else if capability == power_measurement_type_id() {
        Ok(CapabilityProxyHandle::PowerMeasurement(Arc::new(
            PowerMeasurementProxy::new(instrument_id, command_tx),
        )))
    } else if capability == spectrum_analyzer_type_id() {
        Ok(CapabilityProxyHandle::SpectrumAnalyzer(Arc::new(
            SpectrumAnalyzerProxy::new(instrument_id, command_tx),
        )))
    } else {
        Err(anyhow!("Unsupported capability TypeId: {:?}", capability))
    }
}

pub fn position_control_capability_id() -> TypeId {
    position_control_type_id()
}

pub fn power_measurement_capability_id() -> TypeId {
    power_measurement_type_id()
}

pub fn spectrum_analyzer_capability_id() -> TypeId {
    spectrum_analyzer_type_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn position_control_proxy_sends_capability_command() {
        let (tx, mut rx) = mpsc::channel(1);
        let handle = create_proxy(position_control_capability_id(), "stage", tx).unwrap();

        let proxy = handle
            .as_position_control()
            .expect("expected position control proxy");

        proxy.move_absolute(1, 12.5).await.unwrap();

        let command = rx.recv().await.expect("command expected");
        match command {
            InstrumentCommand::Capability {
                capability,
                operation,
                parameters,
            } => {
                assert_eq!(capability, position_control_capability_id());
                assert_eq!(operation, "move_absolute");
                assert_eq!(parameters.len(), 2);
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }
}
