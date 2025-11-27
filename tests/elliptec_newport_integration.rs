
use async_trait::async_trait;
use rust_daq::hardware::capabilities::{Movable, Readable};
use std::sync::{Arc, Mutex};
use anyhow::Result;

// Mock Elliptec Rotator
struct MockElliptecRotator {
    position: Arc<Mutex<f64>>,
}

impl MockElliptecRotator {
    fn new() -> Self {
        Self {
            position: Arc::new(Mutex::new(0.0)),
        }
    }
}

#[async_trait]
impl Movable for MockElliptecRotator {
    async fn move_abs(&self, position_deg: f64) -> Result<()> {
        let mut pos = self.position.lock().unwrap();
        *pos = position_deg;
        Ok(())
    }

    async fn move_rel(&self, distance_deg: f64) -> Result<()> {
        let mut pos = self.position.lock().unwrap();
        *pos += distance_deg;
        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        let pos = self.position.lock().unwrap();
        Ok(*pos)
    }

    async fn wait_settled(&self) -> Result<()> {
        // In a mock, this can be an instant return
        Ok(())
    }
}

// Mock Newport Power Meter
struct MockNewportPowerMeter {
    rotator_position: Arc<Mutex<f64>>,
}

impl MockNewportPowerMeter {
    fn new(rotator_position: Arc<Mutex<f64>>) -> Self {
        Self { rotator_position }
    }
}

#[async_trait]
impl Readable for MockNewportPowerMeter {
    async fn read(&self) -> Result<f64> {
        let pos = self.rotator_position.lock().unwrap();
        // Simulate a power reading that depends on the angle (e.g., a cosine squared relationship for a polarizer)
        let power = (pos.to_radians().cos()).powi(2);
        Ok(power)
    }
}

#[tokio::test]
async fn test_rotate_and_measure() -> Result<()> {
    let rotator = MockElliptecRotator::new();
    let power_meter = MockNewportPowerMeter::new(rotator.position.clone());

    let mut measurements = Vec::new();

    for angle in 0..=90 {
        rotator.move_abs(angle as f64).await?;
        let power = power_meter.read().await?;
        measurements.push((angle, power));
    }

    assert_eq!(measurements.len(), 91);
    assert_eq!(measurements[0].0, 0);
    assert!((measurements[0].1 - 1.0).abs() < 1e-9); // Max power at 0 degrees
    assert_eq!(measurements[90].0, 90);
    assert!((measurements[90].1 - 0.0).abs() < 1e-9); // Min power at 90 degrees

    Ok(())
}

#[tokio::test]
async fn test_find_max_power_angle() -> Result<()> {
    let rotator = MockElliptecRotator::new();
    let power_meter = MockNewportPowerMeter::new(rotator.position.clone());

    let mut max_power = 0.0;
    let mut best_angle = 0.0;

    for angle in 0..=180 {
        rotator.move_abs(angle as f64).await?;
        let power = power_meter.read().await?;
        if power > max_power {
            max_power = power;
            best_angle = angle as f64;
        }
    }

    assert!((best_angle - 0.0).abs() < 1e-9 || (best_angle - 180.0).abs() < 1e-9);
    assert!((max_power - 1.0).abs() < 1e-9);

    Ok(())
}

#[tokio::test]
async fn test_automated_polarization_measurement() -> Result<()> {
    let rotator = MockElliptecRotator::new();
    let power_meter = MockNewportPowerMeter::new(rotator.position.clone());

    // Simulate a scenario where we want to measure the polarization extinction ratio
    rotator.move_abs(0.0).await?;
    let max_power = power_meter.read().await?;

    rotator.move_abs(90.0).await?;
    let min_power = power_meter.read().await?;

    // In our mock, min_power is very close to 0 (cos(90 deg)^2), so the extinction ratio is very large.
    // We'll just check that the values are what we expect.
    assert!((max_power - 1.0).abs() < 1e-9);
    assert!(min_power < 1e-15); // Very close to 0

    // If min_power is effectively 0, extinction ratio is very large (but not literally infinite due to floating point)
    // Skip the extinction ratio test since log10(0) behavior varies

    Ok(())
}
