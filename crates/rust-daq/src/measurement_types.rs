//! Common measurement data types shared between scripting and gRPC modules.
//!
//! This module provides the `DataPoint` type that is used internally by both
//! the scripting engine (for hardware measurements) and the gRPC server
//! (for broadcasting and streaming data).

/// Internal data point representation for hardware measurements.
///
/// This type is used to efficiently broadcast measurements from hardware drivers
/// to multiple consumers including RingBuffer and gRPC clients. It's designed
/// to be serializable and lightweight for high-frequency data streaming.
///
/// # Fields
/// * `channel` - Logical channel name (e.g., "stage_position", "camera_frame")
/// * `value` - Measured numerical value
/// * `timestamp_ns` - Nanosecond timestamp (UNIX epoch)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DataPoint {
    /// Channel name (e.g., "stage_position", "camera_frame")
    pub channel: String,
    /// Measured value
    pub value: f64,
    /// Nanosecond timestamp (UNIX epoch)
    pub timestamp_ns: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_datapoint_creation() {
        let dp = DataPoint {
            channel: "test".to_string(),
            value: 42.0,
            timestamp_ns: 1000,
        };
        assert_eq!(dp.channel, "test");
        assert_eq!(dp.value, 42.0);
        assert_eq!(dp.timestamp_ns, 1000);
    }

    #[test]
    fn test_datapoint_clone() {
        let dp1 = DataPoint {
            channel: "test".to_string(),
            value: 42.0,
            timestamp_ns: 1000,
        };
        let dp2 = dp1.clone();
        assert_eq!(dp1.channel, dp2.channel);
        assert_eq!(dp1.value, dp2.value);
        assert_eq!(dp1.timestamp_ns, dp2.timestamp_ns);
    }

    #[test]
    fn test_datapoint_serialization() {
        let dp = DataPoint {
            channel: "test".to_string(),
            value: 42.0,
            timestamp_ns: 1000,
        };
        let json = serde_json::to_string(&dp).unwrap();
        let dp2: DataPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(dp.channel, dp2.channel);
        assert_eq!(dp.value, dp2.value);
        assert_eq!(dp.timestamp_ns, dp2.timestamp_ns);
    }
}
