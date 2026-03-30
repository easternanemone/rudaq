//! Mock Trigger-On-Position stage for scripting and testing.
//!
//! Provides a minimal `TriggerOnPosition` + `Movable` implementation
//! with a velocity setter, for use in Rhai LIBS scripts and tests
//! without requiring the Dover Motion SDK.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{Movable, TriggerOnPosition};
use tokio::sync::Mutex;

/// Mock stage with Trigger-On-Position and velocity control.
pub struct MockTopStage {
    name: String,
    position: Mutex<f64>,
    velocity: Mutex<f64>,
    top_enabled: Mutex<bool>,
}

impl MockTopStage {
    /// Create a new mock TOP stage.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            position: Mutex::new(0.0),
            velocity: Mutex::new(1.0),
            top_enabled: Mutex::new(false),
        }
    }

    /// Set motion velocity (mm/s).
    pub async fn set_velocity(&self, v: f64) -> Result<()> {
        *self.velocity.lock().await = v;
        tracing::debug!("[MockTopStage {}] velocity = {v} mm/s", self.name);
        Ok(())
    }
}

#[async_trait]
impl Movable for MockTopStage {
    async fn move_abs(&self, position: f64) -> Result<()> {
        *self.position.lock().await = position;
        Ok(())
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let mut pos = self.position.lock().await;
        *pos += distance;
        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        Ok(*self.position.lock().await)
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    async fn wait_settled(&self) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl TriggerOnPosition for MockTopStage {
    async fn enable_top(
        &self,
        _start: f64,
        _end: f64,
        _increment: f64,
        _bidirectional: bool,
        _pulse_width_ns: u64,
    ) -> Result<()> {
        *self.top_enabled.lock().await = true;
        Ok(())
    }

    async fn disable_top(&self) -> Result<()> {
        *self.top_enabled.lock().await = false;
        Ok(())
    }

    async fn is_top_enabled(&self) -> Result<bool> {
        Ok(*self.top_enabled.lock().await)
    }
}
