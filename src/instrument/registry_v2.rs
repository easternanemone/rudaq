//! Instrument Registry V2 for Phase 2.1 Migration (bd-46c9)
//!
//! This module provides `InstrumentRegistryV2`, a concrete implementation
//! of an instrument factory registry that replaces the generic `InstrumentRegistry<M>`.
//! V2 instruments implement `daq_core::Instrument` directly without generic parameters.
//!
//! The registry uses a HashMap-based storage wrapped in Arc<Mutex<>> for thread-safe
//! concurrent access. Instrument factories are stored as boxed closures that take
//! an ID string and return a pinned boxed trait object implementing Instrument.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::pin::Pin;
use daq_core::Instrument;

/// Type alias for V2 instrument factories
type InstrumentFactoryV2 = Box<dyn Fn(&str) -> Pin<Box<dyn Instrument>> + Send + Sync>;

/// V2 Instrument Registry - concrete implementation without generics
pub struct InstrumentRegistryV2 {
    factories: Arc<Mutex<HashMap<String, InstrumentFactoryV2>>>,
}

impl InstrumentRegistryV2 {
    /// Create a new empty instrument registry
    pub fn new() -> Self {
        Self {
            factories: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register an instrument factory function for the given instrument type
    pub fn register<F>(&mut self, instrument_type: &str, factory: F)
    where
        F: Fn(&str) -> Pin<Box<dyn Instrument>> + Send + Sync + 'static,
    {
        let mut factories = self.factories.lock().unwrap();
        factories.insert(instrument_type.to_string(), Box::new(factory));
    }

    /// Create an instrument instance of the specified type with the given ID
    /// Returns None if the instrument type is not registered
    pub fn create(&self, instrument_type: &str, id: &str) -> Option<Pin<Box<dyn Instrument>>> {
        let factories = self.factories.lock().unwrap();
        factories.get(instrument_type).map(|factory| factory(id))
    }

    /// List all registered instrument types
    pub fn list(&self) -> Vec<String> {
        let factories = self.factories.lock().unwrap();
        factories.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use daq_core::Instrument;

    use daq_core::{InstrumentState, InstrumentCommand, Measurement};
    use tokio::sync::broadcast;
    use anyhow::Result;

    struct MockInstrument {
        id: String,
        state: InstrumentState,
        tx: broadcast::Sender<Arc<Measurement>>,
    }

    impl MockInstrument {
        fn new(id: String) -> Self {
            let (tx, _) = broadcast::channel(16);
            Self {
                id,
                state: InstrumentState::Disconnected,
                tx,
            }
        }
    }

    #[async_trait::async_trait]
    impl Instrument for MockInstrument {
        fn id(&self) -> &str {
            &self.id
        }

        fn instrument_type(&self) -> &str {
            "mock"
        }

        fn state(&self) -> InstrumentState {
            self.state
        }

        async fn initialize(&mut self) -> Result<()> {
            self.state = InstrumentState::Ready;
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<()> {
            self.state = InstrumentState::Disconnected;
            Ok(())
        }

        fn measurement_stream(&self) -> broadcast::Receiver<Arc<Measurement>> {
            self.tx.subscribe()
        }

        async fn handle_command(&mut self, _cmd: InstrumentCommand) -> Result<()> {
            Ok(())
        }

        async fn recover(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_registry_creation() {
        let registry = InstrumentRegistryV2::new();
        assert_eq!(registry.list().len(), 0);
    }

    #[test]
    fn test_factory_registration() {
        let mut registry = InstrumentRegistryV2::new();
        registry.register("test_instrument", |id: &str| {
            Pin::new(Box::new(MockInstrument::new(id.to_string())))
        });
        assert_eq!(registry.list(), vec!["test_instrument"]);
    }

    #[test]
    fn test_instrument_creation() {
        let mut registry = InstrumentRegistryV2::new();
        registry.register("mock", |id: &str| {
            Pin::new(Box::new(MockInstrument::new(id.to_string())))
        });

        let instrument = registry.create("mock", "instrument_1").unwrap();
        assert_eq!(instrument.id(), "instrument_1");
    }

    #[test]
    fn test_list_functionality() {
        let mut registry = InstrumentRegistryV2::new();
        registry.register("type_a", |id: &str| {
            Pin::new(Box::new(MockInstrument::new(id.to_string())))
        });
        registry.register("type_b", |id: &str| {
            Pin::new(Box::new(MockInstrument::new(id.to_string())))
        });

        let mut types = registry.list();
        types.sort();
        assert_eq!(types, vec!["type_a", "type_b"]);
    }

    #[test]
    fn test_unknown_instrument_type() {
        let registry = InstrumentRegistryV2::new();
        let result = registry.create("unknown", "test_id");
        assert!(result.is_none());
    }
}