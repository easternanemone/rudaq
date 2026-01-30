//! Timing configuration for realistic mode.
//!
//! Defines hardware-like delays to simulate real device behavior in integration tests.

/// Timing configuration for realistic mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingConfig {
    /// Frame readout time in milliseconds (camera)
    pub frame_readout_ms: u64,
    /// Settling time in milliseconds (stage, rotator)
    pub settling_time_ms: u64,
    /// Communication delay in milliseconds (serial, network)
    pub communication_delay_ms: u64,
}

impl TimingConfig {
    /// Create timing config for camera (30 fps = 33ms per frame)
    pub fn camera() -> Self {
        Self {
            frame_readout_ms: 33,
            settling_time_ms: 0,
            communication_delay_ms: 2,
        }
    }

    /// Create timing config for motion stage
    pub fn stage() -> Self {
        Self {
            frame_readout_ms: 0,
            settling_time_ms: 50,
            communication_delay_ms: 5,
        }
    }

    /// Create timing config for power meter
    pub fn power_meter() -> Self {
        Self {
            frame_readout_ms: 0,
            settling_time_ms: 10,
            communication_delay_ms: 2,
        }
    }

    /// Create timing config for laser
    pub fn laser() -> Self {
        Self {
            frame_readout_ms: 0,
            settling_time_ms: 100, // Wavelength tuning
            communication_delay_ms: 5,
        }
    }

    /// Create timing config for rotator
    pub fn rotator() -> Self {
        Self {
            frame_readout_ms: 0,
            settling_time_ms: 30,
            communication_delay_ms: 3,
        }
    }

    /// Create timing config for DAQ output
    pub fn daq_output() -> Self {
        Self {
            frame_readout_ms: 0,
            settling_time_ms: 1,
            communication_delay_ms: 1,
        }
    }
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            frame_readout_ms: 0,
            settling_time_ms: 0,
            communication_delay_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_timing() {
        let config = TimingConfig::default();
        assert_eq!(config.frame_readout_ms, 0);
        assert_eq!(config.settling_time_ms, 0);
        assert_eq!(config.communication_delay_ms, 0);
    }

    #[test]
    fn test_camera_timing() {
        let config = TimingConfig::camera();
        assert_eq!(config.frame_readout_ms, 33);
        assert_eq!(config.settling_time_ms, 0);
        assert_eq!(config.communication_delay_ms, 2);
    }

    #[test]
    fn test_stage_timing() {
        let config = TimingConfig::stage();
        assert_eq!(config.frame_readout_ms, 0);
        assert_eq!(config.settling_time_ms, 50);
        assert_eq!(config.communication_delay_ms, 5);
    }

    #[test]
    fn test_power_meter_timing() {
        let config = TimingConfig::power_meter();
        assert_eq!(config.settling_time_ms, 10);
    }

    #[test]
    fn test_laser_timing() {
        let config = TimingConfig::laser();
        assert_eq!(config.settling_time_ms, 100);
    }

    #[test]
    fn test_rotator_timing() {
        let config = TimingConfig::rotator();
        assert_eq!(config.settling_time_ms, 30);
    }

    #[test]
    fn test_daq_output_timing() {
        let config = TimingConfig::daq_output();
        assert_eq!(config.settling_time_ms, 1);
    }
}
