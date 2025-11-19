//! PowerMeter meta-instrument trait
//!
//! Hardware-agnostic interface for optical power measurement instruments.
//! Follows DynExp pattern for runtime polymorphism.

use anyhow::Result;
use arrow::array::{Float64Array, StringArray, TimestampNanosecondArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use once_cell::sync::Lazy;
use std::sync::Arc;

/// Units for power measurement
#[derive(Debug, Clone, Copy, PartialEq, Eq, kameo::Reply)]
pub enum PowerUnit {
    Watts,
    MilliWatts,
    MicroWatts,
    NanoWatts,
    Dbm,
}

/// Wavelength for power measurement
#[derive(Debug, Clone, Copy, kameo::Reply)]
pub struct Wavelength {
    pub nm: f64,
}

/// Power meter measurement result
#[derive(Debug, Clone, kameo::Reply)]
pub struct PowerMeasurement {
    pub timestamp_ns: i64,
    pub power: f64,
    pub unit: PowerUnit,
    pub wavelength: Option<Wavelength>,
}

/// Meta-instrument trait for power meters
///
/// Hardware-agnostic interface that any power meter actor must implement.
/// Enables runtime instrument assignment and polymorphic control.
#[async_trait::async_trait]
pub trait PowerMeter: Send + Sync {
    /// Read current power measurement
    async fn read_power(&self) -> Result<PowerMeasurement>;

    /// Set wavelength for measurement calibration
    async fn set_wavelength(&self, wavelength: Wavelength) -> Result<()>;

    /// Get current wavelength setting
    async fn get_wavelength(&self) -> Result<Wavelength>;

    /// Set power unit for measurements
    async fn set_unit(&self, unit: PowerUnit) -> Result<()>;

    /// Get current power unit
    async fn get_unit(&self) -> Result<PowerUnit>;

    /// Convert measurement to Arrow RecordBatch
    fn to_arrow(&self, measurements: &[PowerMeasurement]) -> Result<RecordBatch> {
        static SCHEMA: Lazy<Arc<Schema>> = Lazy::new(|| {
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                    false,
                ),
                Field::new("power", DataType::Float64, false),
                Field::new("unit", DataType::Utf8, false),
                Field::new("wavelength_nm", DataType::Float64, true),
            ]))
        });

        let timestamps: Vec<i64> = measurements.iter().map(|m| m.timestamp_ns).collect();
        let powers: Vec<f64> = measurements.iter().map(|m| m.power).collect();
        let units: StringArray = measurements
            .iter()
            .map(|m| Some(format!("{:?}", m.unit)))
            .collect();
        let wavelengths: Vec<Option<f64>> = measurements
            .iter()
            .map(|m| m.wavelength.map(|w| w.nm))
            .collect();

        let batch = RecordBatch::try_new(
            SCHEMA.clone(),
            vec![
                Arc::new(TimestampNanosecondArray::from(timestamps)),
                Arc::new(Float64Array::from(powers)),
                Arc::new(units),
                Arc::new(Float64Array::from(wavelengths)),
            ],
        )?;

        Ok(batch)
    }
}
