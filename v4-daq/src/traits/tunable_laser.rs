//! TunableLaser meta-instrument trait
//!
//! Hardware-agnostic interface for tunable laser instruments.
//! Follows DynExp pattern for runtime polymorphism.

use anyhow::Result;
use arrow::array::{Float64Array, Int64Array, StringArray, TimestampNanosecondArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use once_cell::sync::Lazy;
use std::sync::Arc;

/// Wavelength in nanometers
#[derive(Debug, Clone, Copy, kameo::Reply)]
pub struct Wavelength {
    pub nm: f64,
}

/// Shutter state
#[derive(Debug, Clone, Copy, PartialEq, Eq, kameo::Reply)]
pub enum ShutterState {
    Open,
    Closed,
}

/// Laser power measurement
#[derive(Debug, Clone, kameo::Reply)]
pub struct LaserMeasurement {
    pub timestamp_ns: i64,
    pub wavelength: Wavelength,
    pub power_watts: f64,
    pub shutter: ShutterState,
}

/// Meta-instrument trait for tunable lasers
///
/// Hardware-agnostic interface that any tunable laser actor must implement.
/// Enables runtime instrument assignment and polymorphic control.
#[async_trait::async_trait]
pub trait TunableLaser: Send + Sync {
    /// Set wavelength in nanometers
    async fn set_wavelength(&self, wavelength: Wavelength) -> Result<()>;

    /// Get current wavelength setting
    async fn get_wavelength(&self) -> Result<Wavelength>;

    /// Read current laser power in watts
    async fn read_power(&self) -> Result<f64>;

    /// Open laser shutter
    async fn open_shutter(&self) -> Result<()>;

    /// Close laser shutter
    async fn close_shutter(&self) -> Result<()>;

    /// Get shutter state
    async fn get_shutter_state(&self) -> Result<ShutterState>;

    /// Take a complete measurement snapshot
    async fn measure(&self) -> Result<LaserMeasurement>;

    /// Convert measurements to Arrow RecordBatch
    fn to_arrow(&self, measurements: &[LaserMeasurement]) -> Result<RecordBatch> {
        static SCHEMA: Lazy<Arc<Schema>> = Lazy::new(|| {
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                    false,
                ),
                Field::new("wavelength_nm", DataType::Float64, false),
                Field::new("power_watts", DataType::Float64, false),
                Field::new("shutter_state", DataType::Utf8, false),
                Field::new("shutter_open", DataType::Int64, false),
            ]))
        });

        let timestamps: Vec<i64> = measurements.iter().map(|m| m.timestamp_ns).collect();
        let wavelengths: Vec<f64> = measurements.iter().map(|m| m.wavelength.nm).collect();
        let powers: Vec<f64> = measurements.iter().map(|m| m.power_watts).collect();
        let shutter_states: StringArray = measurements
            .iter()
            .map(|m| {
                Some(match m.shutter {
                    ShutterState::Open => "open",
                    ShutterState::Closed => "closed",
                })
            })
            .collect();
        let shutter_open: Vec<i64> = measurements
            .iter()
            .map(|m| match m.shutter {
                ShutterState::Open => 1,
                ShutterState::Closed => 0,
            })
            .collect();

        let batch = RecordBatch::try_new(
            SCHEMA.clone(),
            vec![
                Arc::new(TimestampNanosecondArray::from(timestamps)),
                Arc::new(Float64Array::from(wavelengths)),
                Arc::new(Float64Array::from(powers)),
                Arc::new(shutter_states),
                Arc::new(Int64Array::from(shutter_open)),
            ],
        )?;

        Ok(batch)
    }
}
