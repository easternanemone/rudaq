//! Python bindings for the rust_daq project.

use pyo3::prelude::*;
use rust_daq::core::DataPoint as RustDataPoint;

// Re-export chrono and serde_json types for use in PyO3 structs.
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;

/// A single data point captured from an instrument.
///
/// This class is a Python representation of the core Rust `DataPoint` struct.
/// It supports direct attribute access and conversion from the Rust type.
#[pyclass(name = "DataPoint", module = "rust_daq._rust_daq")]
#[derive(Clone)]
pub struct PyDataPoint {
    #[pyo3(get, set)]
    pub timestamp: DateTime<Utc>,
    #[pyo3(get, set)]
    pub channel: String,
    #[pyo3(get, set)]
    pub value: f64,
    #[pyo3(get, set)]
    pub unit: String,
    /// Optional metadata for this data point, represented as a JSON-like object.
    #[pyo3(get, set)]
    pub metadata: Option<JsonValue>,
}

#[pymethods]
impl PyDataPoint {
    #[new]
    #[pyo3(signature = (timestamp, channel, value, unit, metadata=None))]
    fn new(
        timestamp: DateTime<Utc>,
        channel: String,
        value: f64,
        unit: String,
        metadata: Option<JsonValue>,
    ) -> Self {
        Self {
            timestamp,
            channel,
            value,
            unit,
            metadata,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "DataPoint(timestamp={}, channel='{}', value={}, unit='{}')",
            self.timestamp.to_rfc3339(),
            self.channel,
            self.value,
            self.unit
        )
    }
}

/// Conversion from the core Rust DataPoint to the Python-exposed PyDataPoint.
impl From<RustDataPoint> for PyDataPoint {
    fn from(dp: RustDataPoint) -> Self {
        Self {
            timestamp: dp.timestamp,
            channel: dp.channel,
            value: dp.value,
            unit: dp.unit,
            metadata: dp.metadata,
        }
    }
}

/// Mock implementation of a MaiTai laser for Python bindings.
#[pyclass(name = "MaiTai", module = "rust_daq._rust_daq")]
struct MaiTai {
    port: String,
}

#[pymethods]
impl MaiTai {
    #[new]
    fn new(port: String) -> Self {
        // In a real implementation, this would establish a connection.
        println!("[Rust] MaiTai: Initializing on port {}", port);
        Self { port }
    }

    /// Sets the wavelength of the laser.
    fn set_wavelength(&self, wavelength: f64) -> PyResult<()> {
        println!(
            "[Rust] MaiTai on port {}: Setting wavelength to {} nm",
            self.port, wavelength
        );
        // In a real implementation, this would send a command to the device.
        Ok(())
    }
}

/// Mock implementation of a Newport 1830C power meter.
#[pyclass(name = "Newport1830C", module = "rust_daq._rust_daq")]
struct Newport1830C {
    resource_string: String,
}

#[pymethods]
impl Newport1830C {
    #[new]
    fn new(resource_string: String) -> Self {
        println!(
            "[Rust] Newport1830C: Initializing with resource '{}'",
            resource_string
        );
        Self { resource_string }
    }

    /// Reads the power from the meter.
    fn read_power(&self) -> PyResult<f64> {
        println!(
            "[Rust] Newport1830C at {}: Reading power",
            self.resource_string
        );
        // Return a mock value for demonstration.
        let mock_power = 1.23e-3; // 1.23 mW
        Ok(mock_power)
    }
}

/// The main Python module definition.
///
/// This function is called by the Python interpreter when the module is imported.
/// It registers all the `pyclass`es that should be available in Python.
#[pymodule]
fn _rust_daq(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDataPoint>()?;
    m.add_class::<MaiTai>()?;
    m.add_class::<Newport1830C>()?;
    Ok(())
}
