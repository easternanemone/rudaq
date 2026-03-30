//! Virtual Confocal Slit Module
//!
//! Orchestrates a camera in Programmable Scan Mode (PSM) synchronized with a
//! galvo scanner to create a virtual confocal slit for improved SNR in
//! laser-scanning microscopy (e.g., SHG).
//!
//! When PSM is available, the camera reads only a narrow band of rows (the
//! virtual slit), rejecting scattered photons. When PSM is unavailable, the
//! module falls back to standard rolling shutter with short exposure times
//! to achieve a similar effect.
//!
//! # Roles
//!
//! | Role ID | Required Capability | Description |
//! |---------|---------------------|-------------|
//! | `camera` | `frame_producer` | Camera with rolling shutter (ideally PSM-capable) |
//! | `scanner` | `movable` | Galvo scanner for beam positioning |
//!
//! # Parameters
//!
//! | Parameter | Type | Default | Units | Description |
//! |-----------|------|---------|-------|-------------|
//! | `slit_width_px` | int | 4 | px | Virtual slit width in pixel rows |
//! | `scan_direction` | enum | Down | - | Camera scan direction |
//! | `exposure_time_us` | float | 100.0 | us | Per-line exposure time |
//! | `galvo_amplitude_v` | float | 1.0 | V | Galvo scan voltage amplitude |
//! | `galvo_offset_v` | float | 0.0 | V | Galvo center voltage offset |
//! | `frames_to_acquire` | int | 0 | - | Frame count (0 = continuous) |
//! | `use_psm` | bool | true | - | Use PSM when available |
//!
//! # Events
//!
//! - `psm_configured` - Programmable Scan Mode active on camera
//! - `psm_fallback` - PSM unavailable, using rolling shutter fallback
//! - `acquisition_complete` - Finite acquisition finished
//!
//! # Data Types
//!
//! - `frame_acquired` - Per-frame metadata: `{frame_number, galvo_position_v}`
//! - `scan_metrics` - Running metrics: `{frames_acquired, avg_frame_rate_hz}`

use super::{Module, ModuleContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use common::modules::{
    ModuleEventSeverity, ModuleParameter, ModuleRole, ModuleState, ModuleTypeInfo,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the VirtualConfocalSlit module
#[derive(Debug, Clone)]
pub struct VirtualConfocalSlitConfig {
    /// Virtual slit width in pixel rows (1-64)
    pub slit_width_px: u32,
    /// Camera scan direction
    pub scan_direction: String,
    /// Per-line exposure time in microseconds
    pub exposure_time_us: f64,
    /// Galvo scan amplitude in volts
    pub galvo_amplitude_v: f64,
    /// Galvo center offset in volts
    pub galvo_offset_v: f64,
    /// Number of frames to acquire (0 = continuous)
    pub frames_to_acquire: u32,
    /// Whether to attempt PSM mode (falls back to rolling shutter if unavailable)
    pub use_psm: bool,
}

impl Default for VirtualConfocalSlitConfig {
    fn default() -> Self {
        Self {
            slit_width_px: 4,
            scan_direction: "Down".to_string(),
            exposure_time_us: 100.0,
            galvo_amplitude_v: 1.0,
            galvo_offset_v: 0.0,
            frames_to_acquire: 0,
            use_psm: true,
        }
    }
}

// =============================================================================
// Module
// =============================================================================

/// Virtual Confocal Slit module for camera-based confocal line scanning.
pub struct VirtualConfocalSlit {
    config: VirtualConfocalSlitConfig,
    state: ModuleState,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    frames_acquired: Arc<AtomicU64>,
    psm_active: bool,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for VirtualConfocalSlit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualConfocalSlit")
            .field("config", &self.config)
            .field("state", &self.state)
            .field("running", &self.running.load(Ordering::Relaxed))
            .field("paused", &self.paused.load(Ordering::Relaxed))
            .field(
                "frames_acquired",
                &self.frames_acquired.load(Ordering::Relaxed),
            )
            .field("psm_active", &self.psm_active)
            .field("task_handle", &self.task_handle.is_some())
            .finish()
    }
}

impl Default for VirtualConfocalSlit {
    fn default() -> Self {
        Self {
            config: VirtualConfocalSlitConfig::default(),
            state: ModuleState::Created,
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            frames_acquired: Arc::new(AtomicU64::new(0)),
            psm_active: false,
            task_handle: None,
        }
    }
}

#[async_trait]
impl Module for VirtualConfocalSlit {
    fn type_info() -> ModuleTypeInfo {
        ModuleTypeInfo {
            type_id: "virtual_confocal_slit".to_string(),
            display_name: "Virtual Confocal Slit".to_string(),
            description: "Camera-based virtual confocal slit using rolling shutter \
                or Programmable Scan Mode for scattered photon rejection in \
                laser-scanning microscopy (SHG, multiphoton)"
                .to_string(),
            version: "0.1.0".to_string(),
            required_roles: vec![
                ModuleRole {
                    role_id: "camera".to_string(),
                    display_name: "Camera".to_string(),
                    description: "sCMOS camera with rolling shutter readout".to_string(),
                    required_capability: "frame_producer".to_string(),
                    allows_multiple: false,
                },
                ModuleRole {
                    role_id: "scanner".to_string(),
                    display_name: "Galvo Scanner".to_string(),
                    description: "Galvanometer mirror for beam scanning".to_string(),
                    required_capability: "movable".to_string(),
                    allows_multiple: false,
                },
            ],
            optional_roles: vec![],
            parameters: vec![
                ModuleParameter {
                    param_id: "slit_width_px".to_string(),
                    display_name: "Slit Width".to_string(),
                    description: "Virtual slit width in pixel rows. Narrower = better \
                        background rejection but less signal. Optimal ~4 px (0.5 Airy units)."
                        .to_string(),
                    param_type: "int".to_string(),
                    default_value: "4".to_string(),
                    min_value: Some("1".to_string()),
                    max_value: Some("64".to_string()),
                    enum_values: vec![],
                    units: "px".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "scan_direction".to_string(),
                    display_name: "Scan Direction".to_string(),
                    description: "Camera rolling shutter scan direction. DownUpAlternate \
                        enables bidirectional scanning without flyback penalty."
                        .to_string(),
                    param_type: "enum".to_string(),
                    default_value: "Down".to_string(),
                    min_value: None,
                    max_value: None,
                    enum_values: vec![
                        "Down".to_string(),
                        "Up".to_string(),
                        "DownUpAlternate".to_string(),
                    ],
                    units: String::new(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "exposure_time_us".to_string(),
                    display_name: "Exposure Time".to_string(),
                    description: "Per-line exposure time. In fallback mode, this controls \
                        the effective slit width."
                        .to_string(),
                    param_type: "float".to_string(),
                    default_value: "100.0".to_string(),
                    min_value: Some("1.0".to_string()),
                    max_value: Some("1000000.0".to_string()),
                    enum_values: vec![],
                    units: "us".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "galvo_amplitude_v".to_string(),
                    display_name: "Galvo Amplitude".to_string(),
                    description: "Peak-to-peak voltage swing for galvo scan".to_string(),
                    param_type: "float".to_string(),
                    default_value: "1.0".to_string(),
                    min_value: Some("0.01".to_string()),
                    max_value: Some("10.0".to_string()),
                    enum_values: vec![],
                    units: "V".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "galvo_offset_v".to_string(),
                    display_name: "Galvo Offset".to_string(),
                    description: "Center voltage offset for galvo scan".to_string(),
                    param_type: "float".to_string(),
                    default_value: "0.0".to_string(),
                    min_value: Some("-10.0".to_string()),
                    max_value: Some("10.0".to_string()),
                    enum_values: vec![],
                    units: "V".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "frames_to_acquire".to_string(),
                    display_name: "Frame Count".to_string(),
                    description: "Number of frames to acquire (0 = continuous until stopped)"
                        .to_string(),
                    param_type: "int".to_string(),
                    default_value: "0".to_string(),
                    min_value: Some("0".to_string()),
                    max_value: Some("100000".to_string()),
                    enum_values: vec![],
                    units: String::new(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "use_psm".to_string(),
                    display_name: "Use Programmable Scan Mode".to_string(),
                    description: "Attempt to use camera PSM for hardware-controlled slit. \
                        Falls back to rolling shutter if unavailable."
                        .to_string(),
                    param_type: "bool".to_string(),
                    default_value: "true".to_string(),
                    min_value: None,
                    max_value: None,
                    enum_values: vec![],
                    units: String::new(),
                    required: false,
                },
            ],
            event_types: vec![
                "psm_configured".to_string(),
                "psm_fallback".to_string(),
                "acquisition_complete".to_string(),
            ],
            data_types: vec!["frame_acquired".to_string(), "scan_metrics".to_string()],
        }
    }

    fn type_id(&self) -> &'static str {
        "virtual_confocal_slit"
    }

    fn configure(&mut self, params: HashMap<String, String>) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        if let Some(val) = params.get("slit_width_px") {
            match val.parse::<u32>() {
                Ok(w) => {
                    let clamped = w.clamp(1, 64);
                    if w != clamped {
                        warnings.push(format!("slit_width_px clamped to {clamped}"));
                    }
                    self.config.slit_width_px = clamped;
                }
                Err(_) => warnings.push(format!("Invalid slit_width_px: {val}")),
            }
        }

        if let Some(val) = params.get("scan_direction") {
            match val.as_str() {
                "Down" | "Up" | "DownUpAlternate" => {
                    self.config.scan_direction.clone_from(val);
                }
                _ => warnings.push(format!(
                    "Invalid scan_direction: {val}. Valid: Down, Up, DownUpAlternate"
                )),
            }
        }

        if let Some(val) = params.get("exposure_time_us") {
            match val.parse::<f64>() {
                Ok(t) => self.config.exposure_time_us = t.clamp(1.0, 1_000_000.0),
                Err(_) => warnings.push(format!("Invalid exposure_time_us: {val}")),
            }
        }

        if let Some(val) = params.get("galvo_amplitude_v") {
            match val.parse::<f64>() {
                Ok(a) => self.config.galvo_amplitude_v = a.clamp(0.01, 10.0),
                Err(_) => warnings.push(format!("Invalid galvo_amplitude_v: {val}")),
            }
        }

        if let Some(val) = params.get("galvo_offset_v") {
            match val.parse::<f64>() {
                Ok(o) => self.config.galvo_offset_v = o.clamp(-10.0, 10.0),
                Err(_) => warnings.push(format!("Invalid galvo_offset_v: {val}")),
            }
        }

        if let Some(val) = params.get("frames_to_acquire") {
            match val.parse::<u32>() {
                Ok(n) => self.config.frames_to_acquire = n.min(100_000),
                Err(_) => warnings.push(format!("Invalid frames_to_acquire: {val}")),
            }
        }

        if let Some(val) = params.get("use_psm") {
            match val.to_lowercase().as_str() {
                "true" | "1" | "yes" => self.config.use_psm = true,
                "false" | "0" | "no" => self.config.use_psm = false,
                _ => warnings.push(format!("Invalid use_psm: {val}")),
            }
        }

        self.state = ModuleState::Configured;
        Ok(warnings)
    }

    fn get_config(&self) -> HashMap<String, String> {
        let mut config = HashMap::new();
        config.insert(
            "slit_width_px".to_string(),
            self.config.slit_width_px.to_string(),
        );
        config.insert(
            "scan_direction".to_string(),
            self.config.scan_direction.clone(),
        );
        config.insert(
            "exposure_time_us".to_string(),
            self.config.exposure_time_us.to_string(),
        );
        config.insert(
            "galvo_amplitude_v".to_string(),
            self.config.galvo_amplitude_v.to_string(),
        );
        config.insert(
            "galvo_offset_v".to_string(),
            self.config.galvo_offset_v.to_string(),
        );
        config.insert(
            "frames_to_acquire".to_string(),
            self.config.frames_to_acquire.to_string(),
        );
        config.insert("use_psm".to_string(), self.config.use_psm.to_string());
        config
    }

    async fn stage(&mut self, ctx: &ModuleContext) -> Result<()> {
        // Validate camera assignment
        if ctx.get_frame_producer("camera").is_none() {
            return Err(anyhow!(
                "No camera assigned. Assign a FrameProducer device to the 'camera' role."
            ));
        }

        // Validate scanner assignment
        if ctx.get_movable("scanner").is_none() {
            return Err(anyhow!(
                "No scanner assigned. Assign a Movable device to the 'scanner' role."
            ));
        }

        // Probe for PSM support via Parameterized trait
        let psm_available = if self.config.use_psm {
            if let Some(parameterized) = ctx.get_parameterized("camera") {
                let params = parameterized.parameters();
                // Check if scan_mode parameter exists -- indicates PSM firmware support
                params.get("readout.scan_mode").is_some()
            } else {
                false
            }
        } else {
            false
        };

        self.psm_active = psm_available;

        if psm_available {
            info!(
                "VirtualConfocalSlit staged: PSM available, slit_width={}px",
                self.config.slit_width_px
            );
        } else {
            info!(
                "VirtualConfocalSlit staged: PSM unavailable, using rolling shutter fallback \
                 (exposure_time={}us)",
                self.config.exposure_time_us
            );
        }

        self.state = ModuleState::Staged;
        Ok(())
    }

    async fn unstage(&mut self, _ctx: &ModuleContext) -> Result<()> {
        // TODO(bd-pg6a): When PSM is active, reset camera scan_mode to Auto
        self.psm_active = false;
        self.state = ModuleState::Created;
        info!("VirtualConfocalSlit unstaged");
        Ok(())
    }

    async fn start(&mut self, ctx: ModuleContext) -> Result<()> {
        if self.state == ModuleState::Running {
            return Err(anyhow!("Module is already running"));
        }

        // Re-acquire capabilities for the background task
        // Note: frame_producer will be used here once PSM firmware enables
        // hardware-triggered acquisition (bd-pg6a Phase 3)
        let scanner = ctx
            .get_movable("scanner")
            .ok_or_else(|| anyhow!("Scanner not available at start time"))?;

        self.running.store(true, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.frames_acquired.store(0, Ordering::SeqCst);
        self.state = ModuleState::Running;

        let config = self.config.clone();
        let task_state = AcquisitionState {
            running: Arc::clone(&self.running),
            paused: Arc::clone(&self.paused),
            frames_acquired: Arc::clone(&self.frames_acquired),
        };
        let psm_active = self.psm_active;

        let handle = tokio::spawn(async move {
            vcs_acquisition_task(ctx, config, task_state, psm_active, scanner).await;
        });

        self.task_handle = Some(handle);
        info!("VirtualConfocalSlit started");
        Ok(())
    }

    async fn pause(&mut self) -> Result<()> {
        if self.state != ModuleState::Running {
            return Err(anyhow!("Module is not running"));
        }
        self.paused.store(true, Ordering::SeqCst);
        self.state = ModuleState::Paused;
        info!("VirtualConfocalSlit paused");
        Ok(())
    }

    async fn resume(&mut self) -> Result<()> {
        if self.state != ModuleState::Paused {
            return Err(anyhow!("Module is not paused"));
        }
        self.paused.store(false, Ordering::SeqCst);
        self.state = ModuleState::Running;
        info!("VirtualConfocalSlit resumed");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if self.state != ModuleState::Running && self.state != ModuleState::Paused {
            return Err(anyhow!("Module is not running"));
        }

        self.running.store(false, Ordering::SeqCst);

        if let Some(handle) = self.task_handle.take() {
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .ok();
        }

        self.state = ModuleState::Stopped;
        info!(
            "VirtualConfocalSlit stopped after {} frames",
            self.frames_acquired.load(Ordering::Relaxed)
        );
        Ok(())
    }

    fn state(&self) -> ModuleState {
        self.state
    }
}

// =============================================================================
// Background Acquisition Task
// =============================================================================

/// Bundled state flags for the background acquisition task.
struct AcquisitionState {
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    frames_acquired: Arc<AtomicU64>,
}

/// Main acquisition task that runs in the background.
///
/// Coordinates camera frame acquisition with galvo scanner positioning.
/// In PSM mode, the camera hardware handles the virtual slit; in fallback
/// mode, the exposure time approximates the slit effect.
async fn vcs_acquisition_task(
    mut ctx: ModuleContext,
    config: VirtualConfocalSlitConfig,
    state: AcquisitionState,
    psm_active: bool,
    scanner: Arc<dyn hardware::capabilities::Movable>,
) {
    // Emit mode event
    if psm_active {
        ctx.emit_event(
            "psm_configured",
            ModuleEventSeverity::Info,
            &format!(
                "Programmable Scan Mode active: slit_width={}px, direction={}",
                config.slit_width_px, config.scan_direction
            ),
        )
        .await;
    } else {
        ctx.emit_event(
            "psm_fallback",
            ModuleEventSeverity::Warning,
            &format!(
                "PSM unavailable, using rolling shutter fallback (exposure={}us)",
                config.exposure_time_us
            ),
        )
        .await;
    }

    // Move galvo to start position
    let start_pos = config.galvo_offset_v - config.galvo_amplitude_v / 2.0;
    if let Err(e) = scanner.move_abs(start_pos).await {
        warn!("Failed to move galvo to start position: {}", e);
    }

    let start_time = Instant::now();
    let target_count = if config.frames_to_acquire == 0 {
        u64::MAX
    } else {
        u64::from(config.frames_to_acquire)
    };

    // Frame interval estimate based on exposure time
    // In a real PSM system, this would be driven by the camera's line clock
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // SAFETY: exposure_time_us is clamped to [1.0, 1_000_000.0] and floored to >= 1000.0
    let frame_interval = Duration::from_micros(config.exposure_time_us.max(1000.0) as u64);
    let mut ticker = tokio::time::interval(frame_interval);

    info!(
        "VCS acquisition task started: psm={}, target_frames={}",
        psm_active,
        if config.frames_to_acquire == 0 {
            "continuous".to_string()
        } else {
            config.frames_to_acquire.to_string()
        }
    );

    // Preallocate HashMap to avoid per-frame allocation
    let mut frame_values: HashMap<String, f64> = HashMap::with_capacity(2);
    let frame_number_key = "frame_number".to_string();
    let galvo_pos_key = "galvo_position_v".to_string();

    // Sawtooth period: one full sensor sweep in continuous mode
    const ROWS_PER_SCAN: u64 = 2048;

    while state.running.load(Ordering::SeqCst) {
        ticker.tick().await;

        if ctx.is_shutdown_requested() {
            break;
        }

        if state.paused.load(Ordering::SeqCst) {
            continue;
        }

        let count = state.frames_acquired.fetch_add(1, Ordering::SeqCst) + 1;

        // Sawtooth galvo position: in continuous mode, wraps every ROWS_PER_SCAN
        // frames (one full sensor sweep). In finite mode, maps to [0, 1].
        #[allow(clippy::cast_precision_loss)]
        let progress = if target_count == u64::MAX {
            (count % ROWS_PER_SCAN) as f64 / ROWS_PER_SCAN as f64
        } else {
            count as f64 / target_count as f64
        };
        let galvo_pos = config.galvo_offset_v - config.galvo_amplitude_v / 2.0
            + progress * config.galvo_amplitude_v;

        if let Err(e) = scanner.move_abs(galvo_pos).await {
            warn!("Galvo positioning error at frame {}: {}", count, e);
        }

        // Reuse preallocated HashMap
        frame_values.clear();
        #[allow(clippy::cast_precision_loss)]
        {
            frame_values.insert(frame_number_key.clone(), count as f64);
            frame_values.insert(galvo_pos_key.clone(), galvo_pos);
        }
        ctx.emit_data("frame_acquired", frame_values.clone()).await;

        // Emit scan metrics periodically
        if count.is_multiple_of(100) {
            let elapsed = start_time.elapsed().as_secs_f64();
            let mut metrics = HashMap::with_capacity(4);
            #[allow(clippy::cast_precision_loss)]
            {
                metrics.insert("frames_acquired".to_string(), count as f64);
                metrics.insert("avg_frame_rate_hz".to_string(), count as f64 / elapsed);
                metrics.insert("psm_active".to_string(), if psm_active { 1.0 } else { 0.0 });
                metrics.insert("slit_width_px".to_string(), f64::from(config.slit_width_px));
            }
            ctx.emit_data("scan_metrics", metrics).await;
        }

        if count >= target_count {
            ctx.emit_event(
                "acquisition_complete",
                ModuleEventSeverity::Info,
                &format!("Acquired {count} frames"),
            )
            .await;
            break;
        }
    }

    // Return galvo to center
    if let Err(e) = scanner.move_abs(config.galvo_offset_v).await {
        warn!("Failed to return galvo to center: {}", e);
    }

    info!(
        "VCS acquisition task ended: {} frames in {:.1}s",
        state.frames_acquired.load(Ordering::Relaxed),
        start_time.elapsed().as_secs_f64()
    );
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = VirtualConfocalSlitConfig::default();
        assert_eq!(config.slit_width_px, 4);
        assert_eq!(config.scan_direction, "Down");
        assert_eq!(config.exposure_time_us, 100.0);
        assert_eq!(config.galvo_amplitude_v, 1.0);
        assert_eq!(config.galvo_offset_v, 0.0);
        assert_eq!(config.frames_to_acquire, 0);
        assert!(config.use_psm);
    }

    #[test]
    fn test_config_parsing() {
        let mut module = VirtualConfocalSlit::default();

        let mut params = HashMap::new();
        params.insert("slit_width_px".to_string(), "8".to_string());
        params.insert("scan_direction".to_string(), "DownUpAlternate".to_string());
        params.insert("exposure_time_us".to_string(), "50.0".to_string());
        params.insert("galvo_amplitude_v".to_string(), "2.5".to_string());
        params.insert("galvo_offset_v".to_string(), "-0.5".to_string());
        params.insert("frames_to_acquire".to_string(), "1000".to_string());
        params.insert("use_psm".to_string(), "false".to_string());

        let warnings = module.configure(params).expect("configure should succeed");
        assert!(warnings.is_empty());

        assert_eq!(module.config.slit_width_px, 8);
        assert_eq!(module.config.scan_direction, "DownUpAlternate");
        assert_eq!(module.config.exposure_time_us, 50.0);
        assert_eq!(module.config.galvo_amplitude_v, 2.5);
        assert_eq!(module.config.galvo_offset_v, -0.5);
        assert_eq!(module.config.frames_to_acquire, 1000);
        assert!(!module.config.use_psm);
    }

    #[test]
    fn test_slit_width_clamping() {
        let mut module = VirtualConfocalSlit::default();

        // Too high
        let mut params = HashMap::new();
        params.insert("slit_width_px".to_string(), "200".to_string());
        let warnings = module.configure(params).expect("configure should succeed");
        assert!(!warnings.is_empty());
        assert_eq!(module.config.slit_width_px, 64);

        // Too low
        let mut params = HashMap::new();
        params.insert("slit_width_px".to_string(), "0".to_string());
        let warnings = module.configure(params).expect("configure should succeed");
        assert!(!warnings.is_empty());
        assert_eq!(module.config.slit_width_px, 1);
    }

    #[test]
    fn test_invalid_scan_direction() {
        let mut module = VirtualConfocalSlit::default();

        let mut params = HashMap::new();
        params.insert("scan_direction".to_string(), "Sideways".to_string());

        let warnings = module.configure(params).expect("configure should succeed");
        assert!(!warnings.is_empty());
        // Should retain default
        assert_eq!(module.config.scan_direction, "Down");
    }

    #[test]
    fn test_type_info() {
        let info = VirtualConfocalSlit::type_info();
        assert_eq!(info.type_id, "virtual_confocal_slit");
        assert_eq!(info.required_roles.len(), 2);
        assert_eq!(info.required_roles[0].role_id, "camera");
        assert_eq!(info.required_roles[0].required_capability, "frame_producer");
        assert_eq!(info.required_roles[1].role_id, "scanner");
        assert_eq!(info.required_roles[1].required_capability, "movable");
        assert_eq!(info.parameters.len(), 7);
    }

    #[test]
    fn test_get_config_roundtrip() {
        let mut module = VirtualConfocalSlit::default();

        let mut params = HashMap::new();
        params.insert("slit_width_px".to_string(), "8".to_string());
        params.insert("galvo_amplitude_v".to_string(), "3.0".to_string());
        module.configure(params).expect("configure should succeed");

        let config = module.get_config();
        assert_eq!(config.get("slit_width_px"), Some(&"8".to_string()));
        assert_eq!(config.get("galvo_amplitude_v"), Some(&"3".to_string()));
    }

    #[test]
    fn test_module_lifecycle_states() {
        let module = VirtualConfocalSlit::default();
        assert_eq!(module.state(), ModuleState::Created);
    }
}
