//! Example data processors.
use crate::core::{DataPoint, DataProcessor};
use std::collections::VecDeque;

/// A simple moving average filter.
pub struct MovingAverage {
    window_size: usize,
    buffer: VecDeque<f64>,
}

impl MovingAverage {
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            buffer: VecDeque::with_capacity(window_size),
        }
    }
}

impl DataProcessor for MovingAverage {
    fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> {
        let mut processed_data = Vec::new();
        for dp in data {
            self.buffer.push_back(dp.value);
            if self.buffer.len() > self.window_size {
                self.buffer.pop_front();
            }

            let sum: f64 = self.buffer.iter().sum();
            let avg = sum / self.buffer.len() as f64;

            processed_data.push(DataPoint {
                value: avg,
                metadata: dp.metadata.clone(),
                ..dp.clone()
            });
        }
        processed_data
    }
}
