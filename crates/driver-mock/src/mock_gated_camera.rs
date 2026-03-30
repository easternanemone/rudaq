//! Mock Gated Camera (ICCD) for scripting and testing.
//!
//! Provides a minimal `GatedCamera` + `Triggerable` implementation
//! for use in Rhai scripts and tests without requiring the Andor SDK.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{FrameProducer, GatedCamera, TemperatureStatus, Triggerable};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

/// Mock gated ICCD camera with DDG and MCP gain support.
pub struct MockGatedCamera {
    gate_mode: Mutex<String>,
    trigger_mode: Mutex<String>,
    ddg_delay_ps: Mutex<u64>,
    ddg_width_ps: Mutex<u64>,
    mcp_gain: Mutex<u32>,
    temperature_c: Mutex<f64>,
    armed: AtomicBool,
    streaming: AtomicBool,
}

impl MockGatedCamera {
    /// Create a new mock gated camera with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            gate_mode: Mutex::new("CW On".to_owned()),
            trigger_mode: Mutex::new("Internal".to_owned()),
            ddg_delay_ps: Mutex::new(0),
            ddg_width_ps: Mutex::new(10_000_000),
            mcp_gain: Mutex::new(0),
            temperature_c: Mutex::new(-20.0),
            armed: AtomicBool::new(false),
            streaming: AtomicBool::new(false),
        }
    }
}

impl Default for MockGatedCamera {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FrameProducer for MockGatedCamera {
    async fn start_stream(&self) -> Result<()> {
        self.streaming.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        self.streaming.store(false, Ordering::SeqCst);
        self.armed.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        (2560, 2160)
    }
}

#[async_trait]
impl Triggerable for MockGatedCamera {
    async fn arm(&self) -> Result<()> {
        self.armed.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        if !self.armed.load(Ordering::Relaxed) {
            anyhow::bail!("Camera not armed");
        }
        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.armed.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl GatedCamera for MockGatedCamera {
    async fn set_gate_mode(&self, mode: &str) -> Result<()> {
        mode.clone_into(&mut *self.gate_mode.lock().await);
        Ok(())
    }

    async fn set_trigger_mode(&self, mode: &str) -> Result<()> {
        mode.clone_into(&mut *self.trigger_mode.lock().await);
        Ok(())
    }

    async fn set_ddg_timing(&self, delay_ps: u64, width_ps: u64) -> Result<()> {
        *self.ddg_delay_ps.lock().await = delay_ps;
        *self.ddg_width_ps.lock().await = width_ps;
        Ok(())
    }

    async fn set_mcp_gain(&self, gain: u32) -> Result<()> {
        if gain > 4095 {
            anyhow::bail!("MCP gain {gain} out of range 0-4095");
        }
        *self.mcp_gain.lock().await = gain;
        Ok(())
    }

    async fn set_intelligate(&self, _enabled: bool) -> Result<()> {
        Ok(())
    }

    async fn get_temperature_status(&self) -> Result<TemperatureStatus> {
        Ok(TemperatureStatus::Stabilized)
    }

    async fn get_temperature(&self) -> Result<f64> {
        Ok(*self.temperature_c.lock().await)
    }

    fn supports_ddg(&self) -> bool {
        true
    }

    fn supports_mcp_gain(&self) -> bool {
        true
    }
}
