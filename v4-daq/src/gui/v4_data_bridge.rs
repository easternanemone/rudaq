//! V4 Data Bridge for GUI
//!
//! Bridges V4 Arrow-based data from the DataPublisher actor to the egui GUI.
//! Implements the DataConsumer trait to receive RecordBatch updates and maintains
//! thread-safe ringbuffers for GUI consumption.

use crate::actors::data_publisher::DataConsumer;
use anyhow::Result;
use arrow::array::{Float64Array, StringArray, TimestampNanosecondArray};
use arrow::record_batch::RecordBatch;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// GUI-friendly measurement representation
#[derive(Debug, Clone)]
pub struct GuiMeasurement {
    /// Timestamp in nanoseconds since epoch
    pub timestamp_ns: i64,
    /// Measured power value
    pub power: f64,
    /// Power unit (e.g., "MilliWatts", "Watts", "Dbm")
    pub unit: String,
    /// Wavelength in nanometers (optional)
    pub wavelength_nm: Option<f64>,
}

/// V4 Data Bridge implementation
///
/// Consumes Arrow RecordBatch updates from DataPublisher and maintains
/// thread-safe ringbuffers of recent measurements for each instrument.
///
/// # Thread Safety
/// - Uses Arc<Mutex<>> for shared ownership and synchronization
/// - GUI render thread reads from ringbuffers without blocking the actor
/// - Safe for concurrent access from multiple threads
#[derive(Clone)]
pub struct V4DataBridge {
    /// Map of instrument_id -> ringbuffer of recent measurements
    /// Ringbuffer size: 1000 measurements per instrument
    measurements: Arc<Mutex<std::collections::HashMap<String, VecDeque<GuiMeasurement>>>>,
    /// Ringbuffer capacity per instrument
    capacity: usize,
}

impl V4DataBridge {
    /// Create a new V4DataBridge
    ///
    /// # Arguments
    /// * `capacity` - Maximum number of measurements to keep per instrument (default: 1000)
    pub fn new(capacity: usize) -> Self {
        Self {
            measurements: Arc::new(Mutex::new(std::collections::HashMap::new())),
            capacity,
        }
    }

    /// Get the default bridge with standard capacity
    pub fn default_capacity() -> Self {
        Self::new(1000)
    }

    /// Get latest measurements for an instrument
    ///
    /// Returns a clone of the current ringbuffer for the specified instrument.
    /// If no data exists, returns an empty VecDeque.
    pub fn get_measurements(&self, instrument_id: &str) -> VecDeque<GuiMeasurement> {
        self.measurements
            .lock()
            .unwrap()
            .get(instrument_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the latest measurement for an instrument
    ///
    /// Returns the most recent measurement if available.
    pub fn get_latest(&self, instrument_id: &str) -> Option<GuiMeasurement> {
        self.measurements
            .lock()
            .unwrap()
            .get(instrument_id)
            .and_then(|buf| buf.back().cloned())
    }

    /// Get statistics for measurements
    ///
    /// Returns (min, max, mean) power values if measurements exist.
    pub fn get_statistics(&self, instrument_id: &str) -> Option<(f64, f64, f64)> {
        let measurements = self.get_measurements(instrument_id);

        if measurements.is_empty() {
            return None;
        }

        let powers: Vec<f64> = measurements.iter().map(|m| m.power).collect();
        let min = powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mean = powers.iter().sum::<f64>() / powers.len() as f64;

        Some((min, max, mean))
    }

    /// Clear measurements for an instrument
    pub fn clear(&self, instrument_id: &str) {
        self.measurements.lock().unwrap().remove(instrument_id);
    }

    /// Clear all measurements
    pub fn clear_all(&self) {
        self.measurements.lock().unwrap().clear();
    }

    /// Get list of instruments with data
    pub fn instruments(&self) -> Vec<String> {
        self.measurements.lock().unwrap().keys().cloned().collect()
    }

    /// Convert Arrow RecordBatch to GUI measurements
    ///
    /// Expects standard power meter Arrow schema with columns:
    /// - timestamp: Timestamp(Nanosecond)
    /// - power: Float64
    /// - unit: Utf8
    /// - wavelength_nm: Float64 (nullable)
    fn batch_to_measurements(&self, batch: &RecordBatch) -> Result<Vec<GuiMeasurement>> {
        // Extract columns from batch
        let timestamps = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .ok_or_else(|| anyhow::anyhow!("Invalid timestamp column"))?;

        let powers = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| anyhow::anyhow!("Invalid power column"))?;

        let units = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("Invalid unit column"))?;

        let wavelengths = batch
            .column(3)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| anyhow::anyhow!("Invalid wavelength column"))?;

        // Convert to GuiMeasurement vec
        let mut measurements = Vec::with_capacity(batch.num_rows());
        for i in 0..batch.num_rows() {
            measurements.push(GuiMeasurement {
                timestamp_ns: timestamps.value(i),
                power: powers.value(i),
                unit: units.value(i).to_string(),
                wavelength_nm: wavelengths.is_valid(i).then(|| wavelengths.value(i)),
            });
        }

        Ok(measurements)
    }
}

#[async_trait::async_trait]
impl DataConsumer for V4DataBridge {
    /// Handle incoming Arrow RecordBatch from DataPublisher
    ///
    /// Converts the batch to GUI-friendly format and updates the ringbuffer
    /// for the specified instrument.
    async fn handle_batch(&self, batch: RecordBatch, instrument_id: String) -> Result<()> {
        // Convert Arrow batch to GUI measurements
        let measurements = self.batch_to_measurements(&batch)?;

        // Update ringbuffer for this instrument
        let mut map = self.measurements.lock().unwrap();
        let ringbuf = map
            .entry(instrument_id.clone())
            .or_insert_with(|| VecDeque::with_capacity(self.capacity));

        // Add new measurements, removing old ones if at capacity
        for measurement in measurements {
            if ringbuf.len() >= self.capacity {
                ringbuf.pop_front();
            }
            ringbuf.push_back(measurement);
        }

        tracing::trace!(
            "V4DataBridge: Updated {} with {} measurements (total: {})",
            instrument_id,
            batch.num_rows(),
            ringbuf.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, StringArray, TimestampNanosecondArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    fn create_test_batch(num_rows: usize) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "timestamp",
                DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                false,
            ),
            Field::new("power", DataType::Float64, false),
            Field::new("unit", DataType::Utf8, false),
            Field::new("wavelength_nm", DataType::Float64, true),
        ]));

        let timestamps: Vec<i64> = (0..num_rows).map(|i| i as i64 * 1_000_000_000).collect();
        let powers: Vec<f64> = (0..num_rows).map(|i| 1.0 + i as f64 * 0.1).collect();
        let units: Vec<Option<&str>> = vec!["MilliWatts"; num_rows]
            .iter()
            .map(|&s| Some(s))
            .collect();
        let wavelengths: Vec<Option<f64>> = (0..num_rows).map(|_| Some(1550.0)).collect();

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampNanosecondArray::from(timestamps)),
                Arc::new(Float64Array::from(powers)),
                Arc::new(StringArray::from(units)),
                Arc::new(Float64Array::from(wavelengths)),
            ],
        )
        .expect("Failed to create test batch")
    }

    #[tokio::test]
    async fn test_bridge_creation() {
        let bridge = V4DataBridge::new(100);
        assert_eq!(bridge.instruments().len(), 0);
    }

    #[tokio::test]
    async fn test_batch_handling() {
        let bridge = V4DataBridge::new(100);
        let batch = create_test_batch(5);

        bridge
            .handle_batch(batch, "test_instrument".to_string())
            .await
            .expect("Failed to handle batch");

        let measurements = bridge.get_measurements("test_instrument");
        assert_eq!(measurements.len(), 5);

        // Check first measurement
        let first = &measurements[0];
        assert_eq!(first.timestamp_ns, 0);
        assert_eq!(first.power, 1.0);
        assert_eq!(first.unit, "MilliWatts");
        assert_eq!(first.wavelength_nm, Some(1550.0));
    }

    #[tokio::test]
    async fn test_ringbuffer_capacity() {
        let bridge = V4DataBridge::new(10);

        // Add batches that exceed capacity
        for batch_idx in 0..3 {
            let batch = create_test_batch(5);
            bridge
                .handle_batch(batch, "test_instrument".to_string())
                .await
                .expect("Failed to handle batch");
        }

        let measurements = bridge.get_measurements("test_instrument");
        assert_eq!(measurements.len(), 10); // Should not exceed capacity
    }

    #[tokio::test]
    async fn test_get_latest() {
        let bridge = V4DataBridge::new(100);
        let batch = create_test_batch(3);

        bridge
            .handle_batch(batch, "test_instrument".to_string())
            .await
            .expect("Failed to handle batch");

        let latest = bridge.get_latest("test_instrument");
        assert!(latest.is_some());
        let m = latest.unwrap();
        assert_eq!(m.power, 1.2); // Last value should be 1.0 + 2 * 0.1
    }

    #[tokio::test]
    async fn test_statistics() {
        let bridge = V4DataBridge::new(100);
        let batch = create_test_batch(5);

        bridge
            .handle_batch(batch, "test_instrument".to_string())
            .await
            .expect("Failed to handle batch");

        let stats = bridge.get_statistics("test_instrument");
        assert!(stats.is_some());
        let (min, max, _mean) = stats.unwrap();
        assert!(min >= 1.0);
        assert!(max <= 1.4);
    }

    #[tokio::test]
    async fn test_multiple_instruments() {
        let bridge = V4DataBridge::new(100);

        for inst_id in &["inst1", "inst2", "inst3"] {
            let batch = create_test_batch(2);
            bridge
                .handle_batch(batch, inst_id.to_string())
                .await
                .expect("Failed to handle batch");
        }

        assert_eq!(bridge.instruments().len(), 3);
        assert_eq!(bridge.get_measurements("inst1").len(), 2);
        assert_eq!(bridge.get_measurements("inst2").len(), 2);
        assert_eq!(bridge.get_measurements("inst3").len(), 2);
    }

    #[tokio::test]
    async fn test_clear() {
        let bridge = V4DataBridge::new(100);
        let batch = create_test_batch(5);

        bridge
            .handle_batch(batch, "test_instrument".to_string())
            .await
            .expect("Failed to handle batch");

        assert_eq!(bridge.get_measurements("test_instrument").len(), 5);

        bridge.clear("test_instrument");
        assert_eq!(bridge.get_measurements("test_instrument").len(), 0);
    }

    #[tokio::test]
    async fn test_clear_all() {
        let bridge = V4DataBridge::new(100);

        for inst_id in &["inst1", "inst2"] {
            let batch = create_test_batch(3);
            bridge
                .handle_batch(batch, inst_id.to_string())
                .await
                .expect("Failed to handle batch");
        }

        assert_eq!(bridge.instruments().len(), 2);
        bridge.clear_all();
        assert_eq!(bridge.instruments().len(), 0);
    }
}
