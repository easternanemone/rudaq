//! Instrument trait, registry, and implementations.
use crate::core::Instrument;
use crate::measurement::{Measure, InstrumentMeasurement};
use std::collections::HashMap;

pub mod config;
pub mod mock;
pub mod scpi;
#[cfg(feature = "instrument_visa")]
pub mod visa;

// Temporary serial helper for V1 instruments during migration
#[cfg(feature = "instrument_serial")]
pub mod serial_helper;

#[cfg(feature = "instrument_serial")]
pub mod newport_1830c;

#[cfg(feature = "instrument_serial")]
pub mod maitai;

#[cfg(feature = "instrument_serial")]
pub mod elliptec;

#[cfg(feature = "instrument_serial")]
pub mod esp300;

pub mod pvcam;

type InstrumentFactory<M> = Box<dyn Fn(&str) -> Box<dyn Instrument<Measure = M>> + Send + Sync>;

/// A registry for available instrument types.
pub struct InstrumentRegistry<M: Measure> {
    factories: HashMap<String, InstrumentFactory<M>>,
}

impl<M: Measure> Default for InstrumentRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Measure> InstrumentRegistry<M> {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Registers a new instrument type.
    pub fn register<F>(&mut self, instrument_type: &str, factory: F)
    where
        F: Fn(&str) -> Box<dyn Instrument<Measure = M>> + Send + Sync + 'static,
    {
        self.factories
            .insert(instrument_type.to_string(), Box::new(factory));
    }

    /// Creates an instance of an instrument by its ID.
    pub fn create(&self, instrument_type: &str, id: &str) -> Option<Box<dyn Instrument<Measure = M>>> {
        self.factories
            .get(instrument_type)
            .map(|factory| factory(id))
    }

    /// Lists the IDs of all registered instruments.
    pub fn list(&self) -> impl Iterator<Item = String> + '_ {
        self.factories.keys().cloned()
    }
}
