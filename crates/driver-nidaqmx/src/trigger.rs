//! NI-DAQmx trigger implementation using PyO3
//!
//! This module ports the Python `digital` and `TOP` (Trigger On Position) classes
//! from LIBS/trigger.py to Rust using PyO3 bindings.

use crate::error::{NiDaqError, Result};
use async_trait::async_trait;
use common::capabilities::Triggerable;
use parking_lot::Mutex;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Trigger mode configuration
#[derive(Debug, Clone)]
pub enum TriggerMode {
    /// Simple digital pulse generation (software trigger only)
    Digital,

    /// Trigger on external digital edge (Trigger On Position mode)
    TriggerOnPosition {
        /// Trigger source (e.g., "/Dev1/PFI0")
        trigger_source: String,
        /// True for rising edge, false for falling edge
        rising_edge: bool,
        /// Allow retriggering
        retriggerable: bool,
    },
}

/// Internal state of the trigger task
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Idle,
    Configured,
    Armed,
    Running,
}

/// NI-DAQmx trigger driver using PyO3 bridge to Python nidaqmx package
pub struct NiDaqTrigger {
    /// Python Task object (PyObject reference)
    task: Arc<Mutex<Option<PyObject>>>,

    /// Trigger mode
    mode: TriggerMode,

    /// Pulse high time (seconds)
    high_time: f64,

    /// Pulse low time (seconds)
    low_time: f64,

    /// Samples per channel
    samps_per_chan: u32,

    /// Device name (e.g., "Dev1")
    device_name: String,

    /// Counter channel (e.g., "ctr0")
    counter: String,

    /// Current state
    state: Arc<Mutex<TaskState>>,
}

impl NiDaqTrigger {
    /// Create a new NI-DAQmx trigger
    ///
    /// # Arguments
    ///
    /// * `mode` - Trigger mode (Digital or TriggerOnPosition)
    /// * `high_time` - Pulse high time in seconds
    /// * `low_time` - Pulse low time in seconds
    /// * `samps_per_chan` - Number of samples per channel
    ///
    /// # Returns
    ///
    /// Returns a new trigger instance. The Python interpreter is automatically
    /// initialized on first use.
    pub async fn new(
        mode: TriggerMode,
        high_time: f64,
        low_time: f64,
        samps_per_chan: u32,
    ) -> Result<Self> {
        Self::with_device(
            mode,
            high_time,
            low_time,
            samps_per_chan,
            "Dev1",
            "ctr0",
        )
        .await
    }

    /// Create a new NI-DAQmx trigger with custom device and counter
    ///
    /// # Arguments
    ///
    /// * `mode` - Trigger mode
    /// * `high_time` - Pulse high time in seconds
    /// * `low_time` - Pulse low time in seconds
    /// * `samps_per_chan` - Number of samples per channel
    /// * `device_name` - Device name (e.g., "Dev1")
    /// * `counter` - Counter channel (e.g., "ctr0")
    pub async fn with_device(
        mode: TriggerMode,
        high_time: f64,
        low_time: f64,
        samps_per_chan: u32,
        device_name: &str,
        counter: &str,
    ) -> Result<Self> {
        // Validate parameters
        if high_time <= 0.0 {
            return Err(NiDaqError::InvalidParameter(
                "high_time must be positive".to_string(),
            ));
        }
        if low_time <= 0.0 {
            return Err(NiDaqError::InvalidParameter(
                "low_time must be positive".to_string(),
            ));
        }
        if samps_per_chan == 0 {
            return Err(NiDaqError::InvalidParameter(
                "samps_per_chan must be positive".to_string(),
            ));
        }

        info!(
            "Creating NI-DAQmx trigger: mode={:?}, high_time={}, low_time={}, samps_per_chan={}",
            mode, high_time, low_time, samps_per_chan
        );

        Ok(Self {
            task: Arc::new(Mutex::new(None)),
            mode,
            high_time,
            low_time,
            samps_per_chan,
            device_name: device_name.to_string(),
            counter: counter.to_string(),
            state: Arc::new(Mutex::new(TaskState::Idle)),
        })
    }

    /// Configure the task (closes existing task and creates a new one)
    ///
    /// This implements the `configure()` method from the Python classes.
    async fn configure(&self) -> Result<()> {
        debug!("Configuring NI-DAQmx task");

        // Close existing task if any
        self.close_task().await?;

        // Create new task using Python
        tokio::task::spawn_blocking({
            let task = Arc::clone(&self.task);
            let mode = self.mode.clone();
            let high_time = self.high_time;
            let low_time = self.low_time;
            let samps_per_chan = self.samps_per_chan;
            let device_name = self.device_name.clone();
            let counter = self.counter.clone();
            let state = Arc::clone(&self.state);

            move || -> Result<()> {
                Python::with_gil(|py| {
                    // Import nidaqmx
                    let nidaqmx = py.import_bound("nidaqmx")
                        .map_err(|e| NiDaqError::Configuration(format!("Failed to import nidaqmx: {}", e)))?;

                    // Import constants
                    let constants = py.import_bound("nidaqmx.constants")
                        .map_err(|e| NiDaqError::Configuration(format!("Failed to import constants: {}", e)))?;

                    let acquisition_type = constants.getattr("AcquisitionType")
                        .map_err(|e| NiDaqError::Configuration(format!("Failed to get AcquisitionType: {}", e)))?;
                    let finite = acquisition_type.getattr("FINITE")
                        .map_err(|e| NiDaqError::Configuration(format!("Failed to get FINITE: {}", e)))?;

                    // Create task
                    let py_task = nidaqmx.call_method0("Task")
                        .map_err(|e| NiDaqError::Configuration(format!("Failed to create Task: {}", e)))?;

                    // Add counter output channel for pulse generation
                    let channel_name = format!("{}/{}", device_name, counter);
                    let co_channels = py_task.getattr("co_channels")?;
                    let kwargs = PyDict::new_bound(py);
                    kwargs.set_item("high_time", high_time)?;
                    kwargs.set_item("low_time", low_time)?;
                    co_channels.call_method(
                        "add_co_pulse_chan_time",
                        (channel_name,),
                        Some(&kwargs),
                    )?;

                    // Configure timing
                    py_task.getattr("timing")?
                        .call_method("cfg_implicit_timing", (), Some(&{
                            let dict = PyDict::new_bound(py);
                            dict.set_item("sample_mode", finite)?;
                            dict.set_item("samps_per_chan", samps_per_chan)?;
                            dict
                        }))?;

                    // Configure trigger if in TriggerOnPosition mode
                    if let TriggerMode::TriggerOnPosition {
                        trigger_source,
                        rising_edge,
                        retriggerable,
                    } = &mode {
                        let edge = constants.getattr("Edge")?;
                        let trigger_edge = if *rising_edge {
                            edge.getattr("RISING")?
                        } else {
                            edge.getattr("FALLING")?
                        };

                        // Configure digital edge start trigger
                        py_task.getattr("triggers")?
                            .getattr("start_trigger")?
                            .call_method("cfg_dig_edge_start_trig", (), Some(&{
                                let dict = PyDict::new_bound(py);
                                dict.set_item("trigger_source", trigger_source)?;
                                dict.set_item("trigger_edge", trigger_edge)?;
                                dict
                            }))?;

                        // Set retriggerable
                        py_task.getattr("triggers")?
                            .getattr("start_trigger")?
                            .setattr("retriggerable", *retriggerable)?;
                    }

                    // Wait until configured
                    py_task.call_method0("wait_until_done")?;

                    // Store task
                    *task.lock() = Some(py_task.into());
                    *state.lock() = TaskState::Configured;

                    debug!("NI-DAQmx task configured successfully");
                    Ok(())
                })
            }
        })
        .await
        .map_err(|e| NiDaqError::Configuration(format!("Task spawn error: {}", e)))??;

        Ok(())
    }

    /// Execute a single task cycle (start → wait → stop)
    ///
    /// This implements the `single_task()` method from the Python classes.
    async fn single_task(&self) -> Result<()> {
        debug!("Executing single task cycle");

        tokio::task::spawn_blocking({
            let task = Arc::clone(&self.task);
            let state = Arc::clone(&self.state);

            move || -> Result<()> {
                let task_guard = task.lock();
                let py_task = task_guard.as_ref()
                    .ok_or(NiDaqError::Configuration("Task not configured".to_string()))?;

                Python::with_gil(|py| {
                    let py_task = py_task.bind(py);

                    // Start task
                    *state.lock() = TaskState::Running;
                    py_task.call_method0("start")?;
                    debug!("Task started");

                    // Wait until done
                    py_task.call_method0("wait_until_done")?;
                    debug!("Task completed");

                    // Stop task
                    py_task.call_method0("stop")?;
                    *state.lock() = TaskState::Configured;
                    debug!("Task stopped");

                    Ok(())
                })
            }
        })
        .await
        .map_err(|e| NiDaqError::Hardware(format!("Task spawn error: {}", e)))??;

        Ok(())
    }

    /// Close the Python task
    async fn close_task(&self) -> Result<()> {
        let mut task_guard = self.task.lock();

        if let Some(py_task) = task_guard.take() {
            debug!("Closing NI-DAQmx task");

            tokio::task::spawn_blocking(move || {
                Python::with_gil(|py| {
                    let py_task = py_task.bind(py);
                    if let Err(e) = py_task.call_method0("close") {
                        warn!("Error closing task: {}", e);
                    }
                });
            })
            .await
            .ok();

            *self.state.lock() = TaskState::Idle;
        }

        Ok(())
    }

    /// Get current state
    pub fn get_state(&self) -> TaskState {
        *self.state.lock()
    }
}

#[async_trait]
impl Triggerable for NiDaqTrigger {
    async fn arm(&self) -> anyhow::Result<()> {
        let current_state = self.get_state();

        match current_state {
            TaskState::Running => {
                return Err(NiDaqError::AlreadyRunning.into());
            }
            TaskState::Idle | TaskState::Configured => {
                // Configure (or reconfigure) the task
                self.configure().await?;
                *self.state.lock() = TaskState::Armed;
                info!("NI-DAQmx trigger armed");
                Ok(())
            }
            TaskState::Armed => {
                debug!("Trigger already armed");
                Ok(())
            }
        }
    }

    async fn trigger(&self) -> anyhow::Result<()> {
        let current_state = self.get_state();

        if current_state != TaskState::Armed {
            return Err(NiDaqError::NotArmed.into());
        }

        // Execute single task cycle
        self.single_task().await?;

        info!("NI-DAQmx trigger executed");
        Ok(())
    }

    async fn is_armed(&self) -> bool {
        self.get_state() == TaskState::Armed
    }

    async fn disarm(&self) -> anyhow::Result<()> {
        self.close_task().await?;
        info!("NI-DAQmx trigger disarmed");
        Ok(())
    }
}

impl Drop for NiDaqTrigger {
    fn drop(&mut self) {
        // Best-effort cleanup (synchronous)
        if let Some(py_task) = self.task.lock().take() {
            Python::with_gil(|py| {
                let py_task = py_task.bind(py);
                let _ = py_task.call_method0("close");
            });
        }
    }
}

// Safety: NiDaqTrigger is Send because it uses Arc<Mutex<>> for shared state
// and PyObject is Send when protected by a mutex.
unsafe impl Send for NiDaqTrigger {}

// Safety: NiDaqTrigger is Sync because all interior mutability is protected
// by Mutex, making it safe to share references across threads.
unsafe impl Sync for NiDaqTrigger {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_trigger() {
        let trigger = NiDaqTrigger::new(
            TriggerMode::Digital,
            0.1,
            0.001,
            1,
        ).await;

        assert!(trigger.is_ok());
    }

    #[tokio::test]
    async fn test_invalid_parameters() {
        // Negative high_time
        let result = NiDaqTrigger::new(
            TriggerMode::Digital,
            -0.1,
            0.001,
            1,
        ).await;
        assert!(result.is_err());

        // Zero samps_per_chan
        let result = NiDaqTrigger::new(
            TriggerMode::Digital,
            0.1,
            0.001,
            0,
        ).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let trigger = NiDaqTrigger::new(
            TriggerMode::Digital,
            0.1,
            0.001,
            1,
        ).await.expect("Failed to create trigger");

        assert_eq!(trigger.get_state(), TaskState::Idle);
        assert!(!trigger.is_armed().await);
    }
}
