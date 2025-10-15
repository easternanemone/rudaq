//! Python bindings for the `rust_daq` instrumentation framework.
//!
//! The `_rust_daq` extension module is consumed through the high-level
//! `rust_daq` Python package. It exposes ergonomic wrappers around selected
//! instrument drivers and shared data structures from the Rust core so that
//! experiments can be scripted in Python while reusing the real-time runtime.
//!
//! # Quick Start (Python)
//! ```python
//! import rust_daq
//! from datetime import datetime, timezone
//!
//! laser = rust_daq.MaiTai(port="COM3")
//! laser.set_wavelength(800.0)
//!
//! meter = rust_daq.Newport1830C(
//!     resource_string="USB0::0x1234::0x5678::SN910::INSTR",
//! )
//! power = meter.read_power()
//!
//! point = rust_daq.DataPoint(
//!     timestamp=datetime.now(timezone.utc),
//!     channel="power_reading",
//!     value=power,
//!     unit="W",
//!     metadata={"instrument": "Newport1830C"},
//! )
//! print(point)
//! ```
//!
//! Expanded API and contributor documentation is available in
//! `python/docs/api_guide.md` and `python/docs/developer_guide.md`.

use pyo3::prelude::*;
use rust_daq::core::DataPoint as RustDataPoint;

// Re-export chrono and serde_json types for use in PyO3 structs.
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;

/// A Python-friendly representation of [`rust_daq::core::DataPoint`].
///
/// Each instance carries the timestamp, channel name, measurement value, and
/// engineering units for a single acquisition sample. Optional metadata is
/// stored as a JSON-like value to preserve arbitrary experiment context across
/// the FFI boundary.
///
/// # Examples
/// ```rust
/// use chrono::Utc;
/// use rust_daq::core::DataPoint as RustDataPoint;
/// use rust_daq_py::PyDataPoint;
/// use serde_json::json;
///
/// let rust_point = RustDataPoint {
///     timestamp: Utc::now(),
///     channel: "power".to_owned(),
///     value: 1.2,
///     unit: "W".to_owned(),
///     metadata: Some(json!({"instrument": "Newport1830C"})),
/// };
///
/// let py_point = PyDataPoint::from(rust_point);
/// assert_eq!(py_point.unit, "W");
/// assert!(py_point.metadata.is_some());
/// ```
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
    /// Creates a new `DataPoint` instance that mirrors the tuple returned by
    /// `rust_daq` Python APIs.
    ///
    /// # Arguments
    /// * `timestamp` - A timezone-aware UTC `datetime` from Python (converted to
    ///   [`chrono::DateTime<Utc>`]).
    /// * `channel` - Name of the originating channel.
    /// * `value` - The measured value.
    /// * `unit` - Engineering unit string, e.g. `"W"`.
    /// * `metadata` - Optional JSON-like structure captured as
    ///   [`serde_json::Value`].
    ///
    /// # Examples
    /// ```rust
    /// use chrono::Utc;
    /// use rust_daq_py::PyDataPoint;
    /// use serde_json::json;
    ///
    /// let dp = PyDataPoint::new(
    ///     Utc::now(),
    ///     "power".into(),
    ///     1.23,
    ///     "W".into(),
    ///     Some(json!({"status": "ok"})),
    /// );
    /// assert_eq!(dp.channel, "power");
    /// ```
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

    /// Returns a concise string representation used by Python's `repr()`.
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

/// Converts a core Rust [`DataPoint`](rust_daq::core::DataPoint) into its Python
/// wrapper counterpart.
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
pub struct MaiTai {
    port: String,
}

#[pymethods]
impl MaiTai {
    #[new]
    /// Creates a new mock MaiTai driver bound to the supplied serial `port`.
    ///
    /// # Examples
    /// ```rust
    /// use pyo3::prelude::*;
    /// use rust_daq_py::MaiTai;
    ///
    /// Python::with_gil(|py| -> PyResult<()> {
    ///     let laser = Py::new(py, MaiTai::new("COM3".into()))?;
    ///     laser.borrow(py).set_wavelength(800.0)?;
    ///     Ok(())
    /// })?;
    /// # Ok::<_, PyErr>(())
    /// ```
    fn new(port: String) -> Self {
        // In a real implementation, this would establish a connection.
        println!("[Rust] MaiTai: Initializing on port {}", port);
        Self { port }
    }

    /// Sets the wavelength of the laser in nanometres.
    ///
    /// # Arguments
    /// * `wavelength` - Target wavelength in nanometres.
    ///
    /// This mock implementation logs the request; real drivers would validate
    /// bounds and forward the command to the instrument. Errors are surfaced as
    /// [`PyErr`] values.
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
pub struct Newport1830C {
    resource_string: String,
}

#[pymethods]
impl Newport1830C {
    #[new]
    /// Instantiates a mock Newport 1830C driver for the provided VISA resource.
    ///
    /// # Examples
    /// ```rust
    /// use pyo3::prelude::*;
    /// use rust_daq_py::Newport1830C;
    ///
    /// Python::with_gil(|py| -> PyResult<()> {
    ///     let meter = Py::new(py, Newport1830C::new("USB0::0x1234::0x5678::SN910".into()))?;
    ///     let reading: f64 = meter.borrow(py).read_power()?;
    ///     assert!(reading > 0.0);
    ///     Ok(())
    /// })?;
    /// # Ok::<_, PyErr>(())
    /// ```
    fn new(resource_string: String) -> Self {
        println!(
            "[Rust] Newport1830C: Initializing with resource '{}'",
            resource_string
        );
        Self { resource_string }
    }

    /// Reads the instantaneous optical power in watts.
    ///
    /// In this stub the value is mocked and logged to stdout. Production
    /// drivers would perform the actual VISA transaction and convert the raw
    /// data into SI units.
    ///
    /// # Examples
    /// ```rust
    /// use pyo3::prelude::*;
    /// use rust_daq_py::Newport1830C;
    ///
    /// Python::with_gil(|py| -> PyResult<()> {
    ///     let meter = Py::new(py, Newport1830C::new("USB0::FAKE::SN1".into()))?;
    ///     let reading = meter.borrow(py).read_power()?;
    ///     assert!(reading > 0.0);
    ///     Ok(())
    /// })?;
    /// # Ok::<_, PyErr>(())
    /// ```
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

/// Registers the PyO3 module that backs the `rust_daq` Python package.
///
/// The interpreter calls this entry point during `import rust_daq`. Each
/// exposed `pyclass` is added to the module so it becomes available as a
/// top-level attribute on the Python side.
#[pymodule]
fn _rust_daq(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDataPoint>()?;
    m.add_class::<MaiTai>()?;
    m.add_class::<Newport1830C>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use pyo3::exceptions::PyTypeError;
    use pyo3::types::PyType;
    use pyo3::Python;
    use rust_daq::core::DataPoint as RustDataPoint;
    use serde_json::json;

    fn rust_datapoint_with_metadata() -> RustDataPoint {
        RustDataPoint {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap(),
            channel: "detector:signal".to_string(),
            value: 2.5,
            unit: "V".to_string(),
            metadata: Some(json!({
                "experiment": "integration",
                "laser": { "wavelength_nm": 795.0 }
            })),
        }
    }

    #[test]
    fn pydata_point_from_rust_preserves_all_fields() {
        let rust_dp = rust_datapoint_with_metadata();
        let py_dp = PyDataPoint::from(rust_dp.clone());

        assert_eq!(py_dp.timestamp, rust_dp.timestamp);
        assert_eq!(py_dp.channel, rust_dp.channel);
        assert_eq!(py_dp.value, rust_dp.value);
        assert_eq!(py_dp.unit, rust_dp.unit);
        assert_eq!(py_dp.metadata, rust_dp.metadata);
    }

    #[test]
    fn pydata_point_from_rust_handles_none_metadata() {
        let rust_dp = RustDataPoint {
            metadata: None,
            ..rust_datapoint_with_metadata()
        };

        let py_dp = PyDataPoint::from(rust_dp.clone());

        assert!(py_dp.metadata.is_none());
        assert_eq!(py_dp.timestamp, rust_dp.timestamp);
    }

    #[test]
    fn pydata_point_repr_includes_channel_and_unit() {
        let py_dp = PyDataPoint::from(rust_datapoint_with_metadata());
        let repr = py_dp.__repr__();

        assert!(repr.contains("detector:signal"));
        assert!(repr.contains("V"));
    }

    #[test]
    fn pydata_point_clone_retains_metadata() {
        let original = PyDataPoint::from(rust_datapoint_with_metadata());
        let cloned = original.clone();

        assert_eq!(original.metadata, cloned.metadata);
        assert_eq!(original.channel, cloned.channel);
    }

    #[test]
    fn pydata_point_constructor_rejects_non_json_metadata() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let dp_type: &PyType = py.get_type::<PyDataPoint>();
            let timestamp = Utc.with_ymd_and_hms(2024, 7, 1, 8, 30, 0).unwrap();
            let invalid_metadata = py.eval_bound("object()", None, None).unwrap();
            let result = dp_type.call1((
                timestamp,
                "detector:signal".to_string(),
                1.0_f64,
                "V".to_string(),
                invalid_metadata,
            ));

            let err = result.expect_err("non-JSON metadata should raise a TypeError");
            assert!(err.is_instance_of::<PyTypeError>(py));
        });
    }
}
