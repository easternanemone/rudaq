//! Comedi/NI DAQ Bindings for Rhai Scripts
//!
//! This module provides Rhai-compatible handles for NI DAQ hardware:
//! - `AnalogInputHandle` - Analog input channel reading
//! - `AnalogOutputHandle` - DAC voltage output
//! - `DigitalIOHandle` - Digital I/O pin control
//! - `CounterHandle` - Counter/timer operations
//!
//! # Architecture
//!
//! Uses the same async→sync bridge pattern as other hardware bindings:
//! - Wraps async driver traits in handle types
//! - Uses `block_in_place` for safe blocking from Rhai
//! - Provides measurement broadcast for data logging
//!
//! # Example Script
//! ```rhai
//! // Read voltage from AI channel 0
//! let voltage = ai.read(0);
//! print("Voltage: " + voltage + " V");
//!
//! // Set DAC output
//! ao.write(0, 2.5);
//!
//! // Read digital input
//! let state = dio.read(0);
//!
//! // Read counter
//! let count = counter.read(0);
//! ```

use chrono::Utc;
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::rhai_error;
use daq_core::core::Measurement; // bd-q2kl.5

// Simpler helper for synchronous operations that may error
fn map_error<T, E: std::fmt::Display>(
    label: &str,
    result: Result<T, E>,
) -> Result<T, Box<EvalAltResult>> {
    result.map_err(|e| rhai_error(label, e))
}

// =============================================================================
// Trait Definitions for DAQ Hardware
// =============================================================================

/// Trait for analog input devices
pub trait AnalogInput: Send + Sync {
    /// Read voltage from a channel
    fn read_voltage(&self, channel: u32) -> Result<f64, String>;
    /// Read raw ADC value from a channel
    fn read_raw(&self, channel: u32) -> Result<u32, String>;
    /// Get number of channels
    fn channel_count(&self) -> u32;
    /// Get voltage range (min, max) for a channel
    fn voltage_range(&self, channel: u32) -> (f64, f64);
}

/// Trait for analog output devices
pub trait AnalogOutput: Send + Sync {
    /// Write voltage to a channel
    fn write_voltage(&self, channel: u32, voltage: f64) -> Result<(), String>;
    /// Write raw DAC value to a channel
    fn write_raw(&self, channel: u32, value: u32) -> Result<(), String>;
    /// Get number of channels
    fn channel_count(&self) -> u32;
    /// Get voltage range (min, max) for a channel
    fn voltage_range(&self, channel: u32) -> (f64, f64);
}

/// Trait for digital I/O devices
pub trait DigitalIO: Send + Sync {
    /// Read state of a pin (true = high, false = low)
    fn read_pin(&self, pin: u32) -> Result<bool, String>;
    /// Write state to a pin
    fn write_pin(&self, pin: u32, state: bool) -> Result<(), String>;
    /// Configure pin direction (true = output, false = input)
    fn set_direction(&self, pin: u32, output: bool) -> Result<(), String>;
    /// Get pin direction (true = output, false = input)
    fn get_direction(&self, pin: u32) -> Result<bool, String>;
    /// Get number of pins
    fn pin_count(&self) -> u32;
    /// Read all pins as a bitmask
    fn read_port(&self) -> Result<u32, String>;
    /// Write all pins from a bitmask
    fn write_port(&self, value: u32) -> Result<(), String>;
}

/// Trait for counter devices
pub trait Counter: Send + Sync {
    /// Read current counter value
    fn read_count(&self, counter: u32) -> Result<u64, String>;
    /// Reset counter to zero
    fn reset(&self, counter: u32) -> Result<(), String>;
    /// Get number of counters
    fn counter_count(&self) -> u32;
    /// Arm counter for triggered acquisition
    fn arm(&self, counter: u32) -> Result<(), String>;
    /// Disarm counter
    fn disarm(&self, counter: u32) -> Result<(), String>;
}

// =============================================================================
// Handle Types - Rhai-Compatible Wrappers
// =============================================================================

/// Handle to an analog input device for Rhai scripts
///
/// # Script Example
/// ```rhai
/// let voltage = ai.read(0);
/// let all_voltages = ai.read_all();
/// let range = ai.range(0);
/// ```
#[derive(Clone)]
pub struct AnalogInputHandle {
    /// Hardware driver implementing AnalogInput trait
    pub driver: Arc<dyn AnalogInput>,
    /// Device name for measurements
    pub device_name: String,
    /// Optional data sender for broadcasting measurements
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
}

/// Handle to an analog output device for Rhai scripts
///
/// # Script Example
/// ```rhai
/// ao.write(0, 2.5);
/// ao.zero_all();
/// ```
#[derive(Clone)]
pub struct AnalogOutputHandle {
    /// Hardware driver implementing AnalogOutput trait
    pub driver: Arc<dyn AnalogOutput>,
    /// Device name for measurements
    pub device_name: String,
    /// Optional data sender for broadcasting measurements
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
}

/// Handle to a digital I/O device for Rhai scripts
///
/// # Script Example
/// ```rhai
/// let state = dio.read(0);
/// dio.write(0, true);
/// dio.set_output(0);
/// dio.set_input(0);
/// let port = dio.read_port();
/// ```
#[derive(Clone)]
pub struct DigitalIOHandle {
    /// Hardware driver implementing DigitalIO trait
    pub driver: Arc<dyn DigitalIO>,
    /// Device name for measurements
    pub device_name: String,
    /// Optional data sender for broadcasting measurements
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
}

/// Handle to a counter device for Rhai scripts
///
/// # Script Example
/// ```rhai
/// let count = counter.read(0);
/// counter.reset(0);
/// counter.arm(0);
/// ```
#[derive(Clone)]
pub struct CounterHandle {
    /// Hardware driver implementing Counter trait
    pub driver: Arc<dyn Counter>,
    /// Device name for measurements
    pub device_name: String,
    /// Optional data sender for broadcasting measurements
    pub data_tx: Option<Arc<broadcast::Sender<Measurement>>>,
}

// =============================================================================
// Rhai Registration
// =============================================================================

/// Register all NI DAQ bindings with the Rhai engine
///
/// # Registered Types
/// - `AnalogInput` - AI channel reading
/// - `AnalogOutput` - DAC output
/// - `DigitalIO` - DIO pin control
/// - `Counter` - Counter/timer operations
pub fn register_comedi_hardware(engine: &mut Engine) {
    // Register custom types
    engine.register_type_with_name::<AnalogInputHandle>("AnalogInput");
    engine.register_type_with_name::<AnalogOutputHandle>("AnalogOutput");
    engine.register_type_with_name::<DigitalIOHandle>("DigitalIO");
    engine.register_type_with_name::<CounterHandle>("Counter");

    // =========================================================================
    // Analog Input Methods
    // =========================================================================

    // ai.read(channel) -> voltage
    engine.register_fn(
        "read",
        |ai: &mut AnalogInputHandle, channel: i64| -> Result<f64, Box<EvalAltResult>> {
            let voltage = map_error("AI read", ai.driver.read_voltage(channel as u32))?;

            // Broadcast measurement if sender available
            if let Some(ref tx) = ai.data_tx {
                let measurement = Measurement::Scalar {
                    name: format!("{}_ai{}", ai.device_name, channel),
                    value: voltage,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                };
                let _ = tx.send(measurement);
            }

            Ok(voltage)
        },
    );

    // ai.read_raw(channel) -> raw_value
    engine.register_fn(
        "read_raw",
        |ai: &mut AnalogInputHandle, channel: i64| -> Result<i64, Box<EvalAltResult>> {
            let raw = map_error("AI read_raw", ai.driver.read_raw(channel as u32))?;
            Ok(raw as i64)
        },
    );

    // ai.read_all() -> [voltage, voltage, ...]
    engine.register_fn(
        "read_all",
        |ai: &mut AnalogInputHandle| -> Result<Array, Box<EvalAltResult>> {
            let n = ai.driver.channel_count();
            let mut voltages = Array::new();

            for ch in 0..n {
                let v = map_error("AI read_all", ai.driver.read_voltage(ch))?;
                voltages.push(Dynamic::from(v));
            }

            Ok(voltages)
        },
    );

    // ai.channels() -> count
    engine.register_fn("channels", |ai: &mut AnalogInputHandle| -> i64 {
        ai.driver.channel_count() as i64
    });

    // ai.range(channel) -> [min, max]
    engine.register_fn(
        "range",
        |ai: &mut AnalogInputHandle, channel: i64| -> Array {
            let (min, max) = ai.driver.voltage_range(channel as u32);
            vec![Dynamic::from(min), Dynamic::from(max)]
        },
    );

    // =========================================================================
    // Analog Output Methods
    // =========================================================================

    // ao.write(channel, voltage)
    engine.register_fn(
        "write",
        |ao: &mut AnalogOutputHandle,
         channel: i64,
         voltage: f64|
         -> Result<(), Box<EvalAltResult>> {
            map_error("AO write", ao.driver.write_voltage(channel as u32, voltage))?;

            // Broadcast measurement
            if let Some(ref tx) = ao.data_tx {
                let measurement = Measurement::Scalar {
                    name: format!("{}_ao{}", ao.device_name, channel),
                    value: voltage,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                };
                let _ = tx.send(measurement);
            }

            Ok(())
        },
    );

    // ao.write_raw(channel, value)
    engine.register_fn(
        "write_raw",
        |ao: &mut AnalogOutputHandle, channel: i64, value: i64| -> Result<(), Box<EvalAltResult>> {
            map_error(
                "AO write_raw",
                ao.driver.write_raw(channel as u32, value as u32),
            )
        },
    );

    // ao.zero(channel) - set channel to 0V
    engine.register_fn(
        "zero",
        |ao: &mut AnalogOutputHandle, channel: i64| -> Result<(), Box<EvalAltResult>> {
            map_error("AO zero", ao.driver.write_voltage(channel as u32, 0.0))
        },
    );

    // ao.zero_all() - set all channels to 0V
    engine.register_fn(
        "zero_all",
        |ao: &mut AnalogOutputHandle| -> Result<(), Box<EvalAltResult>> {
            let n = ao.driver.channel_count();
            for ch in 0..n {
                map_error("AO zero_all", ao.driver.write_voltage(ch, 0.0))?;
            }
            Ok(())
        },
    );

    // ao.channels() -> count
    engine.register_fn("channels", |ao: &mut AnalogOutputHandle| -> i64 {
        ao.driver.channel_count() as i64
    });

    // ao.range(channel) -> [min, max]
    engine.register_fn(
        "range",
        |ao: &mut AnalogOutputHandle, channel: i64| -> Array {
            let (min, max) = ao.driver.voltage_range(channel as u32);
            vec![Dynamic::from(min), Dynamic::from(max)]
        },
    );

    // =========================================================================
    // Digital I/O Methods
    // =========================================================================

    // dio.read(pin) -> bool
    engine.register_fn(
        "read",
        |dio: &mut DigitalIOHandle, pin: i64| -> Result<bool, Box<EvalAltResult>> {
            map_error("DIO read", dio.driver.read_pin(pin as u32))
        },
    );

    // dio.write(pin, state)
    engine.register_fn(
        "write",
        |dio: &mut DigitalIOHandle, pin: i64, state: bool| -> Result<(), Box<EvalAltResult>> {
            map_error("DIO write", dio.driver.write_pin(pin as u32, state))?;

            // Broadcast measurement
            if let Some(ref tx) = dio.data_tx {
                let measurement = Measurement::Scalar {
                    name: format!("{}_dio{}", dio.device_name, pin),
                    value: if state { 1.0 } else { 0.0 },
                    unit: "".to_string(),
                    timestamp: Utc::now(),
                };
                let _ = tx.send(measurement);
            }

            Ok(())
        },
    );

    // dio.set_output(pin) - configure as output
    engine.register_fn(
        "set_output",
        |dio: &mut DigitalIOHandle, pin: i64| -> Result<(), Box<EvalAltResult>> {
            map_error("DIO set_output", dio.driver.set_direction(pin as u32, true))
        },
    );

    // dio.set_input(pin) - configure as input
    engine.register_fn(
        "set_input",
        |dio: &mut DigitalIOHandle, pin: i64| -> Result<(), Box<EvalAltResult>> {
            map_error("DIO set_input", dio.driver.set_direction(pin as u32, false))
        },
    );

    // dio.is_output(pin) -> bool
    engine.register_fn(
        "is_output",
        |dio: &mut DigitalIOHandle, pin: i64| -> Result<bool, Box<EvalAltResult>> {
            map_error("DIO is_output", dio.driver.get_direction(pin as u32))
        },
    );

    // dio.pins() -> count
    engine.register_fn("pins", |dio: &mut DigitalIOHandle| -> i64 {
        dio.driver.pin_count() as i64
    });

    // dio.read_port() -> bitmask
    engine.register_fn(
        "read_port",
        |dio: &mut DigitalIOHandle| -> Result<i64, Box<EvalAltResult>> {
            let port = map_error("DIO read_port", dio.driver.read_port())?;
            Ok(port as i64)
        },
    );

    // dio.write_port(value) - write bitmask
    engine.register_fn(
        "write_port",
        |dio: &mut DigitalIOHandle, value: i64| -> Result<(), Box<EvalAltResult>> {
            map_error("DIO write_port", dio.driver.write_port(value as u32))
        },
    );

    // =========================================================================
    // Counter Methods
    // =========================================================================

    // counter.read(index) -> count
    engine.register_fn(
        "read",
        |ctr: &mut CounterHandle, counter: i64| -> Result<i64, Box<EvalAltResult>> {
            let count = map_error("Counter read", ctr.driver.read_count(counter as u32))?;

            // Broadcast measurement
            if let Some(ref tx) = ctr.data_tx {
                let measurement = Measurement::Scalar {
                    name: format!("{}_ctr{}", ctr.device_name, counter),
                    value: count as f64,
                    unit: "counts".to_string(),
                    timestamp: Utc::now(),
                };
                let _ = tx.send(measurement);
            }

            Ok(count as i64)
        },
    );

    // counter.reset(index)
    engine.register_fn(
        "reset",
        |ctr: &mut CounterHandle, counter: i64| -> Result<(), Box<EvalAltResult>> {
            map_error("Counter reset", ctr.driver.reset(counter as u32))
        },
    );

    // counter.arm(index)
    engine.register_fn(
        "arm",
        |ctr: &mut CounterHandle, counter: i64| -> Result<(), Box<EvalAltResult>> {
            map_error("Counter arm", ctr.driver.arm(counter as u32))
        },
    );

    // counter.disarm(index)
    engine.register_fn(
        "disarm",
        |ctr: &mut CounterHandle, counter: i64| -> Result<(), Box<EvalAltResult>> {
            map_error("Counter disarm", ctr.driver.disarm(counter as u32))
        },
    );

    // counter.count() -> number of counters
    engine.register_fn("count", |ctr: &mut CounterHandle| -> i64 {
        ctr.driver.counter_count() as i64
    });

    // counter.reset_all()
    engine.register_fn(
        "reset_all",
        |ctr: &mut CounterHandle| -> Result<(), Box<EvalAltResult>> {
            let n = ctr.driver.counter_count();
            for i in 0..n {
                map_error("Counter reset_all", ctr.driver.reset(i))?;
            }
            Ok(())
        },
    );
}

// =============================================================================
// Mock Implementations for Testing
// =============================================================================

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    /// Mock analog input for testing
    pub struct MockAnalogInput {
        channels: u32,
        values: Vec<AtomicU32>,
    }

    impl MockAnalogInput {
        pub fn new(channels: u32) -> Self {
            Self {
                channels,
                values: (0..channels).map(|_| AtomicU32::new(0)).collect(),
            }
        }

        pub fn set_value(&self, channel: u32, raw: u32) {
            if let Some(v) = self.values.get(channel as usize) {
                v.store(raw, Ordering::SeqCst);
            }
        }
    }

    impl AnalogInput for MockAnalogInput {
        fn read_voltage(&self, channel: u32) -> Result<f64, String> {
            let raw = self.read_raw(channel)?;
            // Assume 16-bit, -10V to +10V range
            Ok((raw as f64 / 32768.0) * 10.0 - 10.0)
        }

        fn read_raw(&self, channel: u32) -> Result<u32, String> {
            self.values
                .get(channel as usize)
                .map(|v| v.load(Ordering::SeqCst))
                .ok_or_else(|| format!("Channel {} out of range", channel))
        }

        fn channel_count(&self) -> u32 {
            self.channels
        }

        fn voltage_range(&self, _channel: u32) -> (f64, f64) {
            (-10.0, 10.0)
        }
    }

    /// Mock analog output for testing
    pub struct MockAnalogOutput {
        channels: u32,
        values: Vec<AtomicU32>,
    }

    impl MockAnalogOutput {
        pub fn new(channels: u32) -> Self {
            Self {
                channels,
                values: (0..channels).map(|_| AtomicU32::new(0)).collect(),
            }
        }

        pub fn get_raw(&self, channel: u32) -> Option<u32> {
            self.values
                .get(channel as usize)
                .map(|v| v.load(Ordering::SeqCst))
        }
    }

    impl AnalogOutput for MockAnalogOutput {
        fn write_voltage(&self, channel: u32, voltage: f64) -> Result<(), String> {
            // Convert -10V..+10V to 0..65535
            let raw = (((voltage + 10.0) / 20.0) * 65535.0).clamp(0.0, 65535.0) as u32;
            self.write_raw(channel, raw)
        }

        fn write_raw(&self, channel: u32, value: u32) -> Result<(), String> {
            self.values
                .get(channel as usize)
                .map(|v| v.store(value, Ordering::SeqCst))
                .ok_or_else(|| format!("Channel {} out of range", channel))
        }

        fn channel_count(&self) -> u32 {
            self.channels
        }

        fn voltage_range(&self, _channel: u32) -> (f64, f64) {
            (-10.0, 10.0)
        }
    }

    /// Mock digital I/O for testing
    pub struct MockDigitalIO {
        pins: u32,
        state: AtomicU32,
        direction: AtomicU32, // 1 = output, 0 = input
    }

    impl MockDigitalIO {
        pub fn new(pins: u32) -> Self {
            Self {
                pins,
                state: AtomicU32::new(0),
                direction: AtomicU32::new(0),
            }
        }
    }

    impl DigitalIO for MockDigitalIO {
        fn read_pin(&self, pin: u32) -> Result<bool, String> {
            if pin >= self.pins {
                return Err(format!("Pin {} out of range", pin));
            }
            let state = self.state.load(Ordering::SeqCst);
            Ok((state >> pin) & 1 != 0)
        }

        fn write_pin(&self, pin: u32, state: bool) -> Result<(), String> {
            if pin >= self.pins {
                return Err(format!("Pin {} out of range", pin));
            }
            let current = self.state.load(Ordering::SeqCst);
            let new = if state {
                current | (1 << pin)
            } else {
                current & !(1 << pin)
            };
            self.state.store(new, Ordering::SeqCst);
            Ok(())
        }

        fn set_direction(&self, pin: u32, output: bool) -> Result<(), String> {
            if pin >= self.pins {
                return Err(format!("Pin {} out of range", pin));
            }
            let current = self.direction.load(Ordering::SeqCst);
            let new = if output {
                current | (1 << pin)
            } else {
                current & !(1 << pin)
            };
            self.direction.store(new, Ordering::SeqCst);
            Ok(())
        }

        fn get_direction(&self, pin: u32) -> Result<bool, String> {
            if pin >= self.pins {
                return Err(format!("Pin {} out of range", pin));
            }
            let dir = self.direction.load(Ordering::SeqCst);
            Ok((dir >> pin) & 1 != 0)
        }

        fn pin_count(&self) -> u32 {
            self.pins
        }

        fn read_port(&self) -> Result<u32, String> {
            Ok(self.state.load(Ordering::SeqCst))
        }

        fn write_port(&self, value: u32) -> Result<(), String> {
            self.state.store(value, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Mock counter for testing
    pub struct MockCounter {
        counters: u32,
        counts: Vec<AtomicU64>,
    }

    impl MockCounter {
        pub fn new(counters: u32) -> Self {
            Self {
                counters,
                counts: (0..counters).map(|_| AtomicU64::new(0)).collect(),
            }
        }

        pub fn increment(&self, counter: u32, amount: u64) {
            if let Some(c) = self.counts.get(counter as usize) {
                c.fetch_add(amount, Ordering::SeqCst);
            }
        }
    }

    impl Counter for MockCounter {
        fn read_count(&self, counter: u32) -> Result<u64, String> {
            self.counts
                .get(counter as usize)
                .map(|c| c.load(Ordering::SeqCst))
                .ok_or_else(|| format!("Counter {} out of range", counter))
        }

        fn reset(&self, counter: u32) -> Result<(), String> {
            self.counts
                .get(counter as usize)
                .map(|c| c.store(0, Ordering::SeqCst))
                .ok_or_else(|| format!("Counter {} out of range", counter))
        }

        fn counter_count(&self) -> u32 {
            self.counters
        }

        fn arm(&self, counter: u32) -> Result<(), String> {
            if counter >= self.counters {
                return Err(format!("Counter {} out of range", counter));
            }
            Ok(()) // Mock: no-op
        }

        fn disarm(&self, counter: u32) -> Result<(), String> {
            if counter >= self.counters {
                return Err(format!("Counter {} out of range", counter));
            }
            Ok(()) // Mock: no-op
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::*;
    use super::*;

    #[test]
    fn test_mock_analog_input() {
        let ai = MockAnalogInput::new(4);
        ai.set_value(0, 32768); // Should be ~0V

        let voltage = ai.read_voltage(0).unwrap();
        assert!((voltage - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_mock_digital_io() {
        let dio = MockDigitalIO::new(8);

        dio.write_pin(0, true).unwrap();
        assert!(dio.read_pin(0).unwrap());

        dio.write_pin(0, false).unwrap();
        assert!(!dio.read_pin(0).unwrap());
    }

    #[test]
    fn test_mock_counter() {
        let counter = MockCounter::new(3);

        counter.increment(0, 100);
        assert_eq!(counter.read_count(0).unwrap(), 100);

        counter.reset(0).unwrap();
        assert_eq!(counter.read_count(0).unwrap(), 0);
    }
}
