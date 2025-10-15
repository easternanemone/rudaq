//! A data processor that implements trigger/threshold functionality.
use crate::core::{DataPoint, DataProcessor};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Defines the trigger condition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TriggerMode {
    Edge {
        threshold: f64,
        rising: bool,
    },
    Level {
        threshold: f64,
        above: bool,
    },
    Window {
        low_threshold: f64,
        high_threshold: f64,
        inside: bool,
    },
}

/// Represents the state of the trigger processor.
#[derive(Clone, Debug, PartialEq)]
enum TriggerState {
    Armed,
    Triggered,
    Holdoff,
}

/// Statistics about the trigger.
#[derive(Clone, Debug)]
pub struct TriggerStats {
    pub count: u64,
    pub rate: f64, // Triggers per second
    pub last_trigger_time: Option<DateTime<Utc>>,
}

/// A trigger processor that captures data only when specified conditions are met.
pub struct Trigger {
    mode: TriggerMode,
    holdoff: Duration,
    pre_trigger_samples: usize,
    post_trigger_samples: usize,
    state: TriggerState,
    buffer: VecDeque<DataPoint>,
    stats: TriggerStats,
    last_value: f64,
    holdoff_until: Option<DateTime<Utc>>,
    samples_after_trigger: usize,
    first_trigger_time: Option<DateTime<Utc>>,
}

impl Trigger {
    pub fn new(
        mode: TriggerMode,
        holdoff: Duration,
        pre_trigger_samples: usize,
        post_trigger_samples: usize,
    ) -> Self {
        Self {
            mode,
            holdoff,
            pre_trigger_samples,
            post_trigger_samples,
            state: TriggerState::Armed,
            buffer: VecDeque::with_capacity(pre_trigger_samples),
            stats: TriggerStats {
                count: 0,
                rate: 0.0,
                last_trigger_time: None,
            },
            last_value: 0.0,
            holdoff_until: None,
            samples_after_trigger: 0,
            first_trigger_time: None,
        }
    }

    /// Checks if the trigger condition is met.
    fn check_trigger_condition(&self, dp: &DataPoint) -> bool {
        let last_value = self.last_value;

        match self.mode {
            TriggerMode::Edge { threshold, rising } => {
                if rising {
                    last_value <= threshold && dp.value > threshold
                } else {
                    last_value >= threshold && dp.value < threshold
                }
            }
            TriggerMode::Level { threshold, above } => {
                if above {
                    dp.value > threshold
                } else {
                    dp.value < threshold
                }
            }
            TriggerMode::Window {
                low_threshold,
                high_threshold,
                inside,
            } => {
                let is_inside = dp.value >= low_threshold && dp.value <= high_threshold;
                if inside {
                    is_inside
                } else {
                    !is_inside
                }
            }
        }
    }

    /// Updates trigger statistics.
    fn update_stats(&mut self, timestamp: DateTime<Utc>) {
        self.stats.count += 1;
        self.stats.last_trigger_time = Some(timestamp);

        if self.stats.count == 1 {
            self.first_trigger_time = Some(timestamp);
        } else if let Some(first_time) = self.first_trigger_time {
            let elapsed = timestamp
                .signed_duration_since(first_time)
                .to_std()
                .unwrap()
                .as_secs_f64();
            if elapsed > 0.0 {
                self.stats.rate = self.stats.count as f64 / elapsed;
            }
        }
    }
}

impl DataProcessor for Trigger {
    fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> {
        let mut output = Vec::new();

        for dp in data {
            // First, check if we should transition out of Holdoff state
            if self.state == TriggerState::Holdoff {
                if let Some(holdoff_end) = self.holdoff_until {
                    if dp.timestamp >= holdoff_end {
                        self.state = TriggerState::Armed;
                        self.holdoff_until = None;
                    }
                } else {
                    self.state = TriggerState::Armed;
                }
            }

            // Now process based on the (potentially updated) state
            match self.state {
                TriggerState::Armed => {
                    if self.check_trigger_condition(dp) {
                        self.state = TriggerState::Triggered;
                        self.update_stats(dp.timestamp);

                        if !self.holdoff.is_zero() {
                            self.holdoff_until = Some(dp.timestamp + self.holdoff);
                        }

                        output.extend(self.buffer.iter().cloned());
                        self.buffer.clear();

                        let mut trigger_dp = dp.clone();
                        let mut meta = trigger_dp
                            .metadata
                            .take()
                            .unwrap_or_default()
                            .as_object()
                            .cloned()
                            .unwrap_or_default();
                        meta.insert("trigger".to_string(), serde_json::Value::Bool(true));
                        trigger_dp.metadata = Some(serde_json::Value::Object(meta));
                        output.push(trigger_dp);

                        if self.post_trigger_samples == 0 {
                            self.state = TriggerState::Holdoff;
                        }
                    } else if self.pre_trigger_samples > 0 {
                        self.buffer.push_back(dp.clone());
                        if self.buffer.len() > self.pre_trigger_samples {
                            self.buffer.pop_front();
                        }
                    }
                }
                TriggerState::Triggered => {
                    self.samples_after_trigger += 1;
                    output.push(dp.clone());
                    if self.samples_after_trigger >= self.post_trigger_samples {
                        self.state = TriggerState::Holdoff;
                        self.samples_after_trigger = 0;
                    }
                }
                TriggerState::Holdoff => {
                    // Do nothing - we already handled holdoff expiry above
                }
            }

            // Update last_value for every data point to support edge triggering
            self.last_value = dp.value;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DataPoint;
    use chrono::{Duration, Utc};

    fn create_datapoint(value: f64, timestamp_offset_ms: i64) -> DataPoint {
        DataPoint {
            value,
            timestamp: Utc::now() + Duration::milliseconds(timestamp_offset_ms),
            channel: "test".to_string(),
            unit: "V".to_string(),
            metadata: None,
        }
    }

    fn assert_is_trigger(dp: &DataPoint) {
        assert!(dp.metadata.is_some(), "DataPoint should have metadata");
        let meta = dp
            .metadata
            .as_ref()
            .expect("metadata is some")
            .as_object()
            .expect("metadata is an object");
        assert_eq!(
            meta.get("trigger"),
            Some(&serde_json::Value::Bool(true)),
            "trigger metadata key should be true"
        );
    }

    #[test]
    fn test_edge_trigger_with_post_trigger_samples() {
        let mode = TriggerMode::Edge {
            threshold: 5.0,
            rising: true,
        };
        let mut trigger = Trigger::new(mode, Duration::zero(), 2, 3); // 2 pre, 3 post

        let data = vec![
            create_datapoint(1.0, 0),   // Buffered
            create_datapoint(2.0, 10),  // Buffered
            create_datapoint(4.0, 15),  // Buffered, 1.0 is dropped
            create_datapoint(6.0, 20),  // Trigger
            create_datapoint(7.0, 30),  // Post-trigger
            create_datapoint(8.0, 40),  // Post-trigger
            create_datapoint(9.0, 50),  // Post-trigger
            create_datapoint(10.0, 60), // Not captured
        ];

        let output = trigger.process(&data);

        assert_eq!(output.len(), 6); // 2 pre + 1 trigger + 3 post
        assert_eq!(output[0].value, 2.0);
        assert_eq!(output[1].value, 4.0);
        assert_eq!(output[2].value, 6.0);
        assert_is_trigger(&output[2]);
        assert_eq!(output[3].value, 7.0);
        assert_eq!(output[4].value, 8.0);
        assert_eq!(output[5].value, 9.0);
    }

    #[test]
    fn test_level_trigger() {
        // Test `above: true`
        let mode = TriggerMode::Level {
            threshold: 5.0,
            above: true,
        };
        let mut trigger = Trigger::new(mode.clone(), Duration::zero(), 1, 1);
        let data = vec![
            create_datapoint(4.0, -10), // Pre-trigger
            create_datapoint(6.0, 0),   // Trigger
            create_datapoint(7.0, 10),  // Post-trigger
        ];
        let output = trigger.process(&data);
        assert_eq!(output.len(), 3, "Expected 1 pre, 1 trigger, 1 post");
        assert_eq!(output[1].value, 6.0);
        assert_is_trigger(&output[1]);

        // Test `above: false`
        let mode = TriggerMode::Level {
            threshold: 5.0,
            above: false,
        };
        let mut trigger = Trigger::new(mode.clone(), Duration::zero(), 1, 1);
        let data = vec![
            create_datapoint(6.0, -10), // Pre-trigger
            create_datapoint(4.0, 0),   // Trigger
            create_datapoint(3.0, 10),  // Post-trigger
        ];
        let output = trigger.process(&data);
        assert_eq!(output.len(), 3, "Expected 1 pre, 1 trigger, 1 post");
        assert_eq!(output[1].value, 4.0);
        assert_is_trigger(&output[1]);

        // Test edge case: value at threshold should not trigger
        let mode = TriggerMode::Level {
            threshold: 5.0,
            above: true,
        };
        let mut trigger = Trigger::new(mode, Duration::zero(), 1, 1);
        let data = vec![
            create_datapoint(4.0, -10),
            create_datapoint(5.0, 0), // Exactly on threshold
        ];
        let output = trigger.process(&data);
        assert_eq!(output.len(), 0, "Should not trigger on threshold edge");
    }

    #[test]
    fn test_window_trigger() {
        // Test `inside: true`
        let mode = TriggerMode::Window {
            low_threshold: 3.0,
            high_threshold: 7.0,
            inside: true,
        };
        let mut trigger = Trigger::new(mode.clone(), Duration::zero(), 1, 1);
        let data = vec![
            create_datapoint(2.0, -10), // Pre-trigger
            create_datapoint(5.0, 0),   // Trigger
            create_datapoint(8.0, 10),  // Post-trigger
        ];
        let output = trigger.process(&data);
        assert_eq!(output.len(), 3);
        assert_is_trigger(&output[1]);

        // Test `inside: false`
        let mode = TriggerMode::Window {
            low_threshold: 3.0,
            high_threshold: 7.0,
            inside: false,
        };
        let mut trigger = Trigger::new(mode.clone(), Duration::zero(), 1, 1);
        let data = vec![
            create_datapoint(5.0, -10), // Pre-trigger
            create_datapoint(8.0, 0),   // Trigger
            create_datapoint(2.0, 10),  // Post-trigger
        ];
        let output = trigger.process(&data);
        assert_eq!(output.len(), 3);
        assert_is_trigger(&output[1]);

        // Test edge cases: on boundaries should trigger
        let mode = TriggerMode::Window {
            low_threshold: 3.0,
            high_threshold: 7.0,
            inside: true,
        };
        let mut trigger = Trigger::new(mode, Duration::zero(), 0, 0);
        let data = vec![create_datapoint(3.0, 0), create_datapoint(7.0, 1)];
        let output = trigger.process(&data);
        assert_eq!(output.len(), 2, "Should trigger on both boundaries");
        assert_is_trigger(&output[0]);
        assert_is_trigger(&output[1]);
    }

    #[test]
    fn test_holdoff() {
        let mode = TriggerMode::Edge {
            threshold: 5.0,
            rising: true,
        };
        let holdoff_duration = Duration::milliseconds(50);
        let mut trigger = Trigger::new(mode, holdoff_duration, 0, 0);

        let data = vec![
            create_datapoint(4.0, -10), // Below threshold
            create_datapoint(6.0, 0),   // First trigger
            create_datapoint(4.0, 10),  // Reset for edge
            create_datapoint(6.0, 20),  // Should be ignored (in holdoff)
            create_datapoint(4.0, 40),  // Reset for edge (still in holdoff)
            create_datapoint(6.0, 60),  // Should trigger (holdoff elapsed)
        ];

        let output = trigger.process(&data);

        assert_eq!(output.len(), 2, "Expected two triggers");
        assert_eq!(output[0].value, 6.0);
        assert_eq!(
            output[0].timestamp.timestamp_millis(),
            data[1].timestamp.timestamp_millis()
        );
        assert_is_trigger(&output[0]);

        assert_eq!(output[1].value, 6.0);
        assert_eq!(
            output[1].timestamp.timestamp_millis(),
            data[5].timestamp.timestamp_millis()
        );
        assert_is_trigger(&output[1]);
    }
}
