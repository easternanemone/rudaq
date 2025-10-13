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
    fn check_trigger_condition(&mut self, dp: &DataPoint) -> bool {
        let last_value = self.last_value;
        self.last_value = dp.value;

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
            TriggerMode::Window { low_threshold, high_threshold, inside } => {
                let is_inside = dp.value >= low_threshold && dp.value <= high_threshold;
                if inside { is_inside } else { !is_inside }
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
            let elapsed = timestamp.signed_duration_since(first_time).to_std().unwrap().as_secs_f64();
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
                        trigger_dp.metadata = Some(serde_json::json!({"trigger": true}));
                        output.push(trigger_dp);

                        if self.post_trigger_samples == 0 {
                            self.state = TriggerState::Holdoff;
                        }
                    } else {
                        if self.pre_trigger_samples > 0 {
                            self.buffer.push_back(dp.clone());
                            if self.buffer.len() > self.pre_trigger_samples {
                                self.buffer.pop_front();
                            }
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
                    if let Some(holdoff_end) = self.holdoff_until {
                        if dp.timestamp >= holdoff_end {
                            self.state = TriggerState::Armed;
                            self.holdoff_until = None;
                        }
                    } else {
                        self.state = TriggerState::Armed;
                    }
                }
            }
        }
        output
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DataPoint;
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn create_datapoint(value: f64, timestamp_offset_ms: i64) -> DataPoint {
        DataPoint {
            value,
            timestamp: Utc::now() + Duration::milliseconds(timestamp_offset_ms),
            channel: "test".to_string(),
            unit: "V".to_string(),
            metadata: None,
        }
    }

    #[test]
    fn test_edge_trigger_with_post_trigger_samples() {
        let mode = TriggerMode::Edge { threshold: 5.0, rising: true };
        let mut trigger = Trigger::new(mode, Duration::zero(), 2, 3); // 2 pre, 3 post

        let data = vec![
            create_datapoint(1.0, 0),
            create_datapoint(2.0, 10),
            create_datapoint(4.0, 15),
            create_datapoint(6.0, 20), // Trigger
            create_datapoint(7.0, 30),
            create_datapoint(8.0, 40),
            create_datapoint(9.0, 50),
        ];

        let mut all_output = Vec::new();
        for dp in data {
            all_output.extend(trigger.process(&[dp]));
        }

        assert_eq!(all_output.len(), 6); // 2 pre + 1 trigger + 3 post
        assert_eq!(all_output[0].value, 2.0);
        assert_eq!(all_output[1].value, 4.0);
        assert_eq!(all_output[2].value, 6.0);
        assert!(all_output[2].metadata.is_some());
        assert_eq!(all_output[2].metadata.as_ref().unwrap().get("trigger"), Some(&serde_json::json!(true)));
        assert_eq!(all_output[3].value, 7.0);
        assert_eq!(all_output[4].value, 8.0);
        assert_eq!(all_output[5].value, 9.0);
    }
}