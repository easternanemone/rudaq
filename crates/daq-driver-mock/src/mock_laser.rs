//! Mock tunable laser implementation (MaiTai-like).

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use daq_core::capabilities::{
    EmissionControl, Parameterized, Readable, ShutterControl, WavelengthTunable,
};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::sleep;

// =============================================================================
// MockLaserFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MockLaser driver
#[derive(Debug, Clone, Deserialize)]
pub struct MockLaserConfig {
    /// Initial wavelength in nm (default: 800.0)
    #[serde(default = "default_wavelength")]
    pub wavelength_nm: f64,

    /// Base power output in mW (default: 3000.0 = 3W)
    #[serde(default = "default_power")]
    pub base_power_mw: f64,

    /// Warmup duration in seconds (default: 30.0)
    #[serde(default = "default_warmup")]
    pub warmup_duration_secs: f64,
}

fn default_wavelength() -> f64 {
    800.0
}

fn default_power() -> f64 {
    3000.0 // 3W in mW
}

fn default_warmup() -> f64 {
    30.0
}

impl Default for MockLaserConfig {
    fn default() -> Self {
        Self {
            wavelength_nm: default_wavelength(),
            base_power_mw: default_power(),
            warmup_duration_secs: default_warmup(),
        }
    }
}

/// Factory for creating MockLaser instances.
pub struct MockLaserFactory;

/// Static capabilities for MockLaser
static MOCK_LASER_CAPABILITIES: &[Capability] = &[
    Capability::Readable,
    Capability::WavelengthTunable,
    Capability::ShutterControl,
    Capability::EmissionControl,
    Capability::Parameterized,
];

impl DriverFactory for MockLaserFactory {
    fn driver_type(&self) -> &'static str {
        "mock_laser"
    }

    fn name(&self) -> &'static str {
        "Mock Tunable Laser"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MOCK_LASER_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: MockLaserConfig = config.clone().try_into()?;

        // Validate wavelength range (MaiTai: 690-1040nm)
        if !(690.0..=1040.0).contains(&cfg.wavelength_nm) {
            return Err(anyhow!(
                "Wavelength {} nm out of range (690-1040 nm)",
                cfg.wavelength_nm
            ));
        }

        // Validate power is positive
        if cfg.base_power_mw <= 0.0 {
            return Err(anyhow!("Base power must be positive"));
        }

        // Validate warmup duration is positive
        if cfg.warmup_duration_secs <= 0.0 {
            return Err(anyhow!("Warmup duration must be positive"));
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MockLaserConfig = config.try_into().unwrap_or_default();

            let laser = Arc::new(MockLaser::with_config(cfg));

            Ok(DeviceComponents {
                readable: Some(laser.clone()),
                wavelength_tunable: Some(laser.clone()),
                shutter_control: Some(laser.clone()),
                emission_control: Some(laser.clone()),
                parameterized: Some(laser),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// MockLaser - Simulated Tunable Laser
// =============================================================================

/// Mock tunable laser with realistic behavior.
///
/// Simulates a Ti:Sapphire laser (like MaiTai) with:
/// - Wavelength tuning (690-1040nm)
/// - Shutter control
/// - Emission control with safety interlock
/// - Warmup transients (power ramps over time)
/// - Mode-lock status
/// - Status byte generation
///
/// # Safety Interlocks
///
/// - Cannot enable emission with shutter open
/// - Shutter state is checked before enabling emission
///
/// # Warmup Behavior
///
/// Power ramps exponentially from 0 to full power over ~30 seconds after emission enable.
/// Mode-lock indicator transitions to locked after warmup completes.
pub struct MockLaser {
    /// Current wavelength setting (nm)
    wavelength_nm: RwLock<f64>,

    /// Wavelength range
    min_wavelength: f64,
    max_wavelength: f64,

    /// Shutter state (true = open)
    shutter_open: AtomicBool,

    /// Emission state (true = enabled)
    emission_enabled: AtomicBool,

    /// Warmup tracking
    warmup_start: RwLock<Option<Instant>>,
    warmup_duration: Duration,

    /// Base power output (mW)
    base_power_mw: f64,

    /// Mode-lock status
    mode_locked: AtomicBool,

    /// Status byte (8-bit status with fault codes)
    status_byte: AtomicU8,

    /// Parameter registry
    params: Arc<ParameterSet>,
}

impl MockLaser {
    /// Create a new MockLaser with default configuration.
    pub fn new() -> Self {
        Self::with_config(MockLaserConfig::default())
    }

    /// Create a new MockLaser with custom configuration.
    pub fn with_config(config: MockLaserConfig) -> Self {
        let mut params = ParameterSet::new();

        let wavelength_param = Parameter::new("wavelength_nm", config.wavelength_nm)
            .with_description("Tunable laser wavelength")
            .with_unit("nm")
            .with_range(690.0, 1040.0);

        params.register(wavelength_param);

        Self {
            wavelength_nm: RwLock::new(config.wavelength_nm),
            min_wavelength: 690.0,
            max_wavelength: 1040.0,
            shutter_open: AtomicBool::new(false),
            emission_enabled: AtomicBool::new(false),
            warmup_start: RwLock::new(None),
            warmup_duration: Duration::from_secs_f64(config.warmup_duration_secs),
            base_power_mw: config.base_power_mw,
            mode_locked: AtomicBool::new(false),
            status_byte: AtomicU8::new(0),
            params: Arc::new(params),
        }
    }

    /// Calculate current power based on warmup state.
    ///
    /// Power ramps exponentially from 0 to full power over warmup duration.
    async fn calculate_power(&self) -> f64 {
        if !self.emission_enabled.load(Ordering::Relaxed) {
            return 0.0;
        }

        let warmup_start = self.warmup_start.read().await;

        match *warmup_start {
            None => 0.0, // Emission just enabled but warmup not started
            Some(start) => {
                let elapsed = start.elapsed();
                if elapsed >= self.warmup_duration {
                    // Warmup complete - full power and mode-locked
                    self.mode_locked.store(true, Ordering::Relaxed);
                    self.base_power_mw
                } else {
                    // Warmup in progress - exponential ramp
                    self.mode_locked.store(false, Ordering::Relaxed);
                    let fraction = elapsed.as_secs_f64() / self.warmup_duration.as_secs_f64();
                    // Exponential curve: 1 - e^(-3*t) reaches ~95% at t=1
                    let power_fraction = 1.0 - (-3.0 * fraction).exp();
                    self.base_power_mw * power_fraction
                }
            }
        }
    }

    /// Update status byte based on current state.
    ///
    /// Bit 0: Emission on/off
    /// Bit 1: Mode-locked
    /// Bit 2: Shutter open
    /// Bits 3-7: Reserved for fault codes
    fn update_status_byte(&self) {
        let mut status = 0u8;

        if self.emission_enabled.load(Ordering::Relaxed) {
            status |= 0x01; // Bit 0: emission on
        }

        if self.mode_locked.load(Ordering::Relaxed) {
            status |= 0x02; // Bit 1: mode-locked
        }

        if self.shutter_open.load(Ordering::Relaxed) {
            status |= 0x04; // Bit 2: shutter open
        }

        self.status_byte.store(status, Ordering::Relaxed);
    }

    /// Get the current status byte.
    pub fn status_byte(&self) -> u8 {
        self.update_status_byte();
        self.status_byte.load(Ordering::Relaxed)
    }

    /// Check if the laser is mode-locked.
    pub fn is_mode_locked(&self) -> bool {
        self.mode_locked.load(Ordering::Relaxed)
    }
}

impl Default for MockLaser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parameterized for MockLaser {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Readable for MockLaser {
    async fn read(&self) -> Result<f64> {
        // Return current power in watts (convert from mW)
        let power_mw = self.calculate_power().await;
        Ok(power_mw / 1000.0) // Convert mW to W
    }
}

#[async_trait]
impl WavelengthTunable for MockLaser {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        // Validate range
        if !(self.min_wavelength..=self.max_wavelength).contains(&wavelength_nm) {
            return Err(anyhow!(
                "Wavelength {} nm out of range ({}-{} nm)",
                wavelength_nm,
                self.min_wavelength,
                self.max_wavelength
            ));
        }

        *self.wavelength_nm.write().await = wavelength_nm;

        // Simulate tuning delay
        sleep(Duration::from_millis(100)).await;

        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        Ok(*self.wavelength_nm.read().await)
    }

    fn wavelength_range(&self) -> (f64, f64) {
        (self.min_wavelength, self.max_wavelength)
    }
}

#[async_trait]
impl ShutterControl for MockLaser {
    async fn open_shutter(&self) -> Result<()> {
        self.shutter_open.store(true, Ordering::Relaxed);
        self.update_status_byte();

        // Simulate mechanical delay
        sleep(Duration::from_millis(50)).await;

        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        self.shutter_open.store(false, Ordering::Relaxed);
        self.update_status_byte();

        // Simulate mechanical delay
        sleep(Duration::from_millis(50)).await;

        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        Ok(self.shutter_open.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl EmissionControl for MockLaser {
    async fn enable_emission(&self) -> Result<()> {
        // Safety interlock: refuse to enable emission if shutter is open
        if self.shutter_open.load(Ordering::Relaxed) {
            return Err(anyhow!(
                "Safety interlock: Cannot enable emission with shutter open. Close shutter first."
            ));
        }

        self.emission_enabled.store(true, Ordering::Relaxed);

        // Start warmup timer
        *self.warmup_start.write().await = Some(Instant::now());

        self.update_status_byte();

        Ok(())
    }

    async fn disable_emission(&self) -> Result<()> {
        self.emission_enabled.store(false, Ordering::Relaxed);

        // Reset warmup timer
        *self.warmup_start.write().await = None;

        // Reset mode-lock
        self.mode_locked.store(false, Ordering::Relaxed);

        self.update_status_byte();

        Ok(())
    }

    async fn is_emission_enabled(&self) -> Result<bool> {
        Ok(self.emission_enabled.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_factory_driver_type() {
        let factory = MockLaserFactory;
        assert_eq!(factory.driver_type(), "mock_laser");
        assert_eq!(factory.name(), "Mock Tunable Laser");
    }

    #[tokio::test]
    async fn test_factory_capabilities() {
        let factory = MockLaserFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Readable));
        assert!(caps.contains(&Capability::WavelengthTunable));
        assert!(caps.contains(&Capability::ShutterControl));
        assert!(caps.contains(&Capability::EmissionControl));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = MockLaserFactory;

        // Valid config
        let valid = toml::toml! {
            wavelength_nm = 800.0
            base_power_mw = 3000.0
        };
        assert!(factory.validate(&toml::Value::Table(valid)).is_ok());

        // Invalid wavelength (too low)
        let invalid_low = toml::toml! {
            wavelength_nm = 600.0
        };
        assert!(factory.validate(&toml::Value::Table(invalid_low)).is_err());

        // Invalid wavelength (too high)
        let invalid_high = toml::toml! {
            wavelength_nm = 1100.0
        };
        assert!(factory.validate(&toml::Value::Table(invalid_high)).is_err());
    }

    #[tokio::test]
    async fn test_wavelength_range_validation() -> Result<()> {
        let laser = MockLaser::new();

        // Valid wavelengths
        laser.set_wavelength(690.0).await?;
        laser.set_wavelength(800.0).await?;
        laser.set_wavelength(1040.0).await?;

        // Invalid wavelengths
        assert!(laser.set_wavelength(689.0).await.is_err());
        assert!(laser.set_wavelength(1041.0).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_shutter_emission_interlock() -> Result<()> {
        let laser = MockLaser::new();

        // Can enable emission with shutter closed
        laser.close_shutter().await?;
        assert!(laser.enable_emission().await.is_ok());
        laser.disable_emission().await?;

        // Cannot enable emission with shutter open
        laser.open_shutter().await?;
        assert!(laser.enable_emission().await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_warmup_behavior() -> Result<()> {
        let config = MockLaserConfig {
            wavelength_nm: 800.0,
            base_power_mw: 3000.0,
            warmup_duration_secs: 0.1, // Short warmup for testing
        };
        let laser = MockLaser::with_config(config);

        // No power when emission disabled
        assert_eq!(laser.read().await?, 0.0);

        // Enable emission (with shutter closed)
        laser.close_shutter().await?;
        laser.enable_emission().await?;

        // Power should be low immediately after enable
        let initial_power = laser.read().await?;
        assert!(initial_power < 3.0); // Should be well below 3W

        // Wait for warmup to complete
        sleep(Duration::from_millis(150)).await;

        // Power should be at full after warmup
        let final_power = laser.read().await?;
        assert!((final_power - 3.0).abs() < 0.1); // Should be ~3W

        // Should be mode-locked after warmup
        assert!(laser.is_mode_locked());

        Ok(())
    }

    #[tokio::test]
    async fn test_status_byte() -> Result<()> {
        let laser = MockLaser::new();

        // Initial status: nothing enabled
        let status = laser.status_byte();
        assert_eq!(status & 0x01, 0); // Emission off
        assert_eq!(status & 0x02, 0); // Not mode-locked
        assert_eq!(status & 0x04, 0); // Shutter closed

        // Open shutter
        laser.open_shutter().await?;
        let status = laser.status_byte();
        assert_eq!(status & 0x04, 0x04); // Shutter open bit set

        // Close shutter and enable emission
        laser.close_shutter().await?;
        laser.enable_emission().await?;
        let status = laser.status_byte();
        assert_eq!(status & 0x01, 0x01); // Emission on bit set
        assert_eq!(status & 0x04, 0); // Shutter closed

        Ok(())
    }

    #[tokio::test]
    async fn test_power_returns_zero_when_disabled() -> Result<()> {
        let laser = MockLaser::new();

        // Power should be zero when emission is disabled
        assert_eq!(laser.read().await?, 0.0);

        Ok(())
    }
}
