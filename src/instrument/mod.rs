//! Instrument trait, registry, and implementations.
use crate::core::Instrument;
use std::collections::HashMap;

pub mod mock;
#[cfg(feature = "instrument_visa")]
pub mod visa;
pub mod scpi;

type InstrumentFactory = Box<dyn Fn(&str) -> Box<dyn Instrument> + Send + Sync>;

/// A registry for available instrument types.
pub struct InstrumentRegistry {
    factories: HashMap<String, InstrumentFactory>,
}

impl Default for InstrumentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InstrumentRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Registers a new instrument type.
    pub fn register<F>(&mut self, instrument_type: &str, factory: F)
    where
        F: Fn(&str) -> Box<dyn Instrument> + Send + Sync + 'static,
    {
        self.factories.insert(instrument_type.to_string(), Box::new(factory));
    }

    /// Creates an instance of an instrument by its ID.
    pub fn create(&self, instrument_type: &str, id: &str) -> Option<Box<dyn Instrument>> {
        self.factories.get(instrument_type).map(|factory| factory(id))
    }

    /// Lists the IDs of all registered instruments.
    pub fn list(&self) -> impl Iterator<Item = String> + '_ {
        self.factories.keys().cloned()
    }
}
