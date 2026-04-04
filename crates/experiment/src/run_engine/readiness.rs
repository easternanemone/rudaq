//! Calibration readiness checks.
//!
//! Types and methods for gating run starts on calibration freshness
//! and hardware configuration matching.
//!
//! [`ReadinessManager`] owns the calibration state and all check logic.
//! [`RunEngine`] delegates through thin wrappers that also supply the
//! `DeviceRegistry` reference.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use common::capabilities::Parameterized;
use common::driver::Capability;
use echelle::EchelleFrameCompatibility;
use hardware::registry::DeviceRegistry;
use tokio::sync::RwLock;
use tokio::time::Duration;

use super::RunEngine;
use crate::plans::Plan;

/// Active calibration freshness data for a device family.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationWavelengthCoverage {
    /// Minimum wavelength covered by the calibration for the grating.
    pub min_nm: f64,
    /// Maximum wavelength covered by the calibration for the grating.
    pub max_nm: f64,
}

impl CalibrationWavelengthCoverage {
    pub(crate) fn contains(&self, wavelength_nm: f64) -> bool {
        wavelength_nm >= self.min_nm && wavelength_nm <= self.max_nm
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationFreshness {
    /// Logical device type (for example `spectroscopy`).
    pub device_type: String,
    /// Specific device the calibration snapshot should apply to, if scoped.
    pub target_device_id: Option<String>,
    /// RFC 3339 timestamp of the loaded calibration set, if present.
    pub calibration_timestamp: Option<DateTime<Utc>>,
    /// Per-grating wavelength coverage for radiance calibrations, if known.
    pub grating_wavelength_coverage: HashMap<u8, CalibrationWavelengthCoverage>,
    /// Camera/frame compatibility expectations for echelle profiles, if known.
    pub echelle_frame_compatibility: Option<EchelleFrameCompatibility>,
}

impl CalibrationFreshness {
    pub fn new(device_type: impl Into<String>) -> Self {
        Self {
            device_type: device_type.into(),
            target_device_id: None,
            calibration_timestamp: None,
            grating_wavelength_coverage: HashMap::new(),
            echelle_frame_compatibility: None,
        }
    }

    pub fn with_target_device_id(mut self, target_device_id: impl Into<String>) -> Self {
        self.target_device_id = Some(target_device_id.into());
        self
    }

    pub fn with_timestamp(mut self, calibration_timestamp: DateTime<Utc>) -> Self {
        self.calibration_timestamp = Some(calibration_timestamp);
        self
    }

    pub fn with_grating_wavelength_coverage(
        mut self,
        grating_wavelength_coverage: HashMap<u8, CalibrationWavelengthCoverage>,
    ) -> Self {
        self.grating_wavelength_coverage = grating_wavelength_coverage;
        self
    }

    pub fn with_echelle_frame_compatibility(
        mut self,
        echelle_frame_compatibility: EchelleFrameCompatibility,
    ) -> Self {
        self.echelle_frame_compatibility = Some(echelle_frame_compatibility);
        self
    }

    pub(crate) fn merge_from(&mut self, incoming: Self) {
        if let Some(target_device_id) = incoming.target_device_id {
            self.target_device_id = Some(target_device_id);
        }
        if let Some(calibration_timestamp) = incoming.calibration_timestamp {
            self.calibration_timestamp = Some(calibration_timestamp);
        }
        if !incoming.grating_wavelength_coverage.is_empty() {
            self.grating_wavelength_coverage = incoming.grating_wavelength_coverage;
        }
        if let Some(echelle_frame_compatibility) = incoming.echelle_frame_compatibility {
            self.echelle_frame_compatibility = Some(echelle_frame_compatibility);
        }
    }

    pub(crate) fn clear_radiance_fields(&mut self) {
        self.calibration_timestamp = None;
        self.grating_wavelength_coverage.clear();
    }

    pub(crate) fn clear_echelle_fields(&mut self) {
        self.target_device_id = None;
        self.echelle_frame_compatibility = None;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.calibration_timestamp.is_none()
            && self.grating_wavelength_coverage.is_empty()
            && self.echelle_frame_compatibility.is_none()
    }
}

// ---------------------------------------------------------------------------
// ReadinessManager — composed component owning calibration state + checks
// ---------------------------------------------------------------------------

/// Composed component that owns calibration freshness state and all
/// readiness check logic.
///
/// Methods that need hardware state take `&DeviceRegistry` explicitly,
/// making the dependency surface visible and the logic unit-testable
/// without a full `RunEngine`.
pub(crate) struct ReadinessManager {
    /// Active calibration snapshots keyed by device_type (lowercase).
    pub(crate) active_calibrations: RwLock<HashMap<String, CalibrationFreshness>>,
    /// Maximum allowed calibration age per device type.
    pub(crate) calibration_max_ages: RwLock<HashMap<String, Duration>>,
}

impl ReadinessManager {
    pub(crate) fn new() -> Self {
        Self {
            active_calibrations: RwLock::new(HashMap::new()),
            calibration_max_ages: RwLock::new(HashMap::from([(
                "spectroscopy".to_string(),
                Duration::from_secs(24 * 60 * 60),
            )])),
        }
    }

    // ---- Calibration snapshot management ----

    /// Register the freshest known calibration for a logical device type.
    pub(crate) async fn register_calibration_snapshot(&self, snapshot: CalibrationFreshness) {
        let mut snapshot = snapshot;
        snapshot.device_type = snapshot.device_type.to_ascii_lowercase();
        let device_type = snapshot.device_type.clone();
        let mut active_calibrations = self.active_calibrations.write().await;
        if let Some(existing) = active_calibrations.get_mut(&device_type) {
            existing.merge_from(snapshot);
        } else {
            active_calibrations.insert(device_type, snapshot);
        }
    }

    /// Clear the active calibration snapshot for a device type.
    pub(crate) async fn clear_calibration_snapshot(&self, device_type: &str) {
        self.active_calibrations
            .write()
            .await
            .remove(&device_type.to_ascii_lowercase());
    }

    /// Clear only the radiance/timestamp portion of the calibration snapshot.
    pub(crate) async fn clear_radiance_calibration_snapshot(&self, device_type: &str) {
        let device_type = device_type.to_ascii_lowercase();
        let mut active_calibrations = self.active_calibrations.write().await;
        let should_remove = if let Some(snapshot) = active_calibrations.get_mut(&device_type) {
            snapshot.clear_radiance_fields();
            snapshot.is_empty()
        } else {
            false
        };
        if should_remove {
            active_calibrations.remove(&device_type);
        }
    }

    /// Clear only the echelle-compatibility portion of the calibration snapshot.
    pub(crate) async fn clear_echelle_calibration_snapshot(&self, device_type: &str) {
        let device_type = device_type.to_ascii_lowercase();
        let mut active_calibrations = self.active_calibrations.write().await;
        let should_remove = if let Some(snapshot) = active_calibrations.get_mut(&device_type) {
            snapshot.clear_echelle_fields();
            snapshot.is_empty()
        } else {
            false
        };
        if should_remove {
            active_calibrations.remove(&device_type);
        }
    }

    /// Override the maximum allowed calibration age for a device type.
    pub(crate) async fn set_calibration_max_age(&self, device_type: &str, max_age: Duration) {
        self.calibration_max_ages
            .write()
            .await
            .insert(device_type.to_ascii_lowercase(), max_age);
    }

    // ---- Readiness checks (take &DeviceRegistry explicitly) ----

    /// Determine which device types are relevant for readiness checks.
    pub(crate) fn device_types_for_devices(
        registry: &DeviceRegistry,
        device_ids: &[String],
    ) -> Vec<String> {
        let mut device_types = Vec::new();

        for device_id in device_ids {
            let Some(info) = registry.get_device_info(device_id) else {
                continue;
            };

            if info.capabilities.contains(&Capability::SpectrometerControl)
                || info.capabilities.contains(&Capability::GatedCamera)
            {
                let spectroscopy = "spectroscopy".to_string();
                if !device_types.contains(&spectroscopy) {
                    device_types.push(spectroscopy);
                }
            }
        }

        device_types
    }

    /// Check all readiness issues for the given device IDs.
    pub(crate) async fn readiness_issues_for_devices(
        &self,
        device_ids: &[String],
        registry: &DeviceRegistry,
    ) -> Vec<RunReadinessIssue> {
        let device_types = Self::device_types_for_devices(registry, device_ids);
        if device_types.is_empty() {
            return Vec::new();
        }

        // Clone calibration data into locals and drop the RwLock guards
        // *before* any .await. This avoids holding read guards across
        // hardware I/O in config_match_issues_for_devices.
        let calibrations_snapshot: Vec<(String, CalibrationFreshness, Option<Duration>)> = {
            let active_calibrations = self.active_calibrations.read().await;
            let max_ages = self.calibration_max_ages.read().await;
            device_types
                .into_iter()
                .filter_map(|dt| {
                    let snapshot = active_calibrations.get(&dt)?.clone();
                    let max_age = max_ages.get(&dt).copied();
                    Some((dt, snapshot, max_age))
                })
                .collect()
            // Guards dropped here
        };

        let now = Utc::now();
        let mut issues = Vec::new();

        for (device_type, snapshot, max_age) in calibrations_snapshot {
            if let (Some(calibration_timestamp), Some(max_age)) =
                (snapshot.calibration_timestamp, max_age)
            {
                let age = now
                    .signed_duration_since(calibration_timestamp)
                    .to_std()
                    .unwrap_or_default();

                if age > max_age {
                    issues.push(Self::stale_calibration_issue(
                        &device_type,
                        calibration_timestamp,
                        age,
                        max_age,
                    ));
                }
            }

            issues.extend(
                Self::config_match_issues_for_devices(
                    registry,
                    &device_type,
                    &snapshot,
                    device_ids,
                )
                .await,
            );
        }

        issues
    }

    fn stale_calibration_issue(
        device_type: &str,
        calibration_timestamp: DateTime<Utc>,
        age: Duration,
        max_age: Duration,
    ) -> RunReadinessIssue {
        let age_hours = age.as_secs_f64() / 3600.0;
        let max_age_hours = max_age.as_secs_f64() / 3600.0;
        RunReadinessIssue {
            code: "calibration_stale".to_string(),
            message: format!(
                "CalibrationStalenessGate: {device_type} calibration is {:.1}h old (threshold {:.1}h, timestamp {})",
                age_hours,
                max_age_hours,
                calibration_timestamp.to_rfc3339()
            ),
            blocking: true,
            device_type: Some(device_type.to_string()),
            device_id: None,
            age_hours: Some(age_hours),
            max_age_hours: Some(max_age_hours),
        }
    }

    async fn config_match_issues_for_devices(
        registry: &DeviceRegistry,
        device_type: &str,
        snapshot: &CalibrationFreshness,
        device_ids: &[String],
    ) -> Vec<RunReadinessIssue> {
        if device_type != "spectroscopy" {
            return Vec::new();
        }

        let mut issues = Vec::new();
        for device_id in device_ids {
            if snapshot
                .target_device_id
                .as_deref()
                .is_some_and(|target_device_id| target_device_id != device_id)
            {
                continue;
            }

            if !snapshot.grating_wavelength_coverage.is_empty() {
                issues.extend(
                    Self::spectrometer_config_issues(registry, device_id, device_type, snapshot)
                        .await,
                );
            }

            if let Some(expected) = snapshot.echelle_frame_compatibility.as_ref()
                && let Some(info) = registry.get_device_info(device_id)
                && info.capabilities.contains(&Capability::GatedCamera)
                && let Some(issue) =
                    Self::echelle_config_issue(registry, device_id, device_type, expected)
            {
                issues.push(issue);
            }
        }

        issues
    }

    async fn spectrometer_config_issues(
        registry: &DeviceRegistry,
        device_id: &str,
        device_type: &str,
        snapshot: &CalibrationFreshness,
    ) -> Vec<RunReadinessIssue> {
        let Some(spectrometer) = registry.get_spectrometer_control(device_id) else {
            return Vec::new();
        };

        let current_grating: u8 = match spectrometer.get_grating().await {
            Ok(grating) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // Grating indices are small positive integers (1-3 typical)
                let g = grating as u8;
                g
            }
            Err(err) => {
                return vec![Self::config_issue(
                    "calibration_config_unavailable",
                    format!(
                        "CalibrationConfigGate: unable to read grating from {device_id}: {err}"
                    ),
                    device_type,
                    device_id,
                )];
            }
        };

        let Some(coverage) = snapshot.grating_wavelength_coverage.get(&current_grating) else {
            let mut loaded_gratings: Vec<u8> = snapshot
                .grating_wavelength_coverage
                .keys()
                .copied()
                .collect();
            loaded_gratings.sort_unstable();
            return vec![Self::config_issue(
                "calibration_config_mismatch",
                format!(
                    "CalibrationConfigGate: {device_id} is on grating {current_grating}, \
                     but the loaded calibration only covers gratings {:?}",
                    loaded_gratings
                ),
                device_type,
                device_id,
            )];
        };

        let current_wavelength = match spectrometer.get_wavelength().await {
            Ok(wavelength) => wavelength,
            Err(err) => {
                return vec![Self::config_issue(
                    "calibration_config_unavailable",
                    format!(
                        "CalibrationConfigGate: unable to read wavelength from {device_id}: {err}"
                    ),
                    device_type,
                    device_id,
                )];
            }
        };

        if coverage.contains(current_wavelength) {
            Vec::new()
        } else {
            vec![Self::config_issue(
                "calibration_config_mismatch",
                format!(
                    "CalibrationConfigGate: {device_id} wavelength {:.2}nm on grating {} is \
                     outside the loaded calibration range [{:.2}, {:.2}]nm",
                    current_wavelength, current_grating, coverage.min_nm, coverage.max_nm
                ),
                device_type,
                device_id,
            )]
        }
    }

    fn echelle_config_issue(
        registry: &DeviceRegistry,
        device_id: &str,
        device_type: &str,
        expected: &EchelleFrameCompatibility,
    ) -> Option<RunReadinessIssue> {
        let parameterized = registry.get_parameterized(device_id)?;
        let Some(frame_width) =
            Self::read_u32_parameter(parameterized.as_ref(), &[("AOIWidth", 0)])
        else {
            return Some(Self::config_issue(
                "calibration_config_unavailable",
                format!(
                    "CalibrationConfigGate: unable to read AOIWidth from {device_id} for echelle profile validation"
                ),
                device_type,
                device_id,
            ));
        };
        let Some(frame_height) =
            Self::read_u32_parameter(parameterized.as_ref(), &[("AOIHeight", 0)])
        else {
            return Some(Self::config_issue(
                "calibration_config_unavailable",
                format!(
                    "CalibrationConfigGate: unable to read AOIHeight from {device_id} for echelle profile validation"
                ),
                device_type,
                device_id,
            ));
        };
        let Some(roi_x) = Self::read_u32_parameter(parameterized.as_ref(), &[("AOILeft", 1)])
        else {
            return Some(Self::config_issue(
                "calibration_config_unavailable",
                format!(
                    "CalibrationConfigGate: unable to read AOILeft from {device_id} for echelle profile validation"
                ),
                device_type,
                device_id,
            ));
        };
        let Some(roi_y) = Self::read_u32_parameter(parameterized.as_ref(), &[("AOITop", 1)]) else {
            return Some(Self::config_issue(
                "calibration_config_unavailable",
                format!(
                    "CalibrationConfigGate: unable to read AOITop from {device_id} for echelle profile validation"
                ),
                device_type,
                device_id,
            ));
        };
        let Some(binning_x) = Self::read_u32_parameter(parameterized.as_ref(), &[("AOIHBin", 0)])
        else {
            return Some(Self::config_issue(
                "calibration_config_unavailable",
                format!(
                    "CalibrationConfigGate: unable to read AOIHBin from {device_id} for echelle profile validation"
                ),
                device_type,
                device_id,
            ));
        };
        let Some(binning_y) = Self::read_u32_parameter(parameterized.as_ref(), &[("AOIVBin", 0)])
        else {
            return Some(Self::config_issue(
                "calibration_config_unavailable",
                format!(
                    "CalibrationConfigGate: unable to read AOIVBin from {device_id} for echelle profile validation"
                ),
                device_type,
                device_id,
            ));
        };
        let bit_depth = expected.bit_depth.and_then(|_| {
            Self::read_u32_parameter(parameterized.as_ref(), &[("BitDepth", 0), ("bit_depth", 0)])
        });

        let mut mismatches = Vec::new();
        if frame_width != expected.frame_width {
            mismatches.push(format!(
                "frame_width expected {} got {}",
                expected.frame_width, frame_width
            ));
        }
        if frame_height != expected.frame_height {
            mismatches.push(format!(
                "frame_height expected {} got {}",
                expected.frame_height, frame_height
            ));
        }
        if roi_x != expected.roi_x {
            mismatches.push(format!("roi_x expected {} got {}", expected.roi_x, roi_x));
        }
        if roi_y != expected.roi_y {
            mismatches.push(format!("roi_y expected {} got {}", expected.roi_y, roi_y));
        }
        if binning_x != expected.binning_x {
            mismatches.push(format!(
                "binning_x expected {} got {}",
                expected.binning_x, binning_x
            ));
        }
        if binning_y != expected.binning_y {
            mismatches.push(format!(
                "binning_y expected {} got {}",
                expected.binning_y, binning_y
            ));
        }
        if let Some(expected_bit_depth) = expected.bit_depth {
            match bit_depth {
                Some(actual_bit_depth) if actual_bit_depth == expected_bit_depth => {}
                Some(actual_bit_depth) => mismatches.push(format!(
                    "bit_depth expected {} got {}",
                    expected_bit_depth, actual_bit_depth
                )),
                None => mismatches.push(format!(
                    "bit_depth expected {} but device did not expose BitDepth",
                    expected_bit_depth
                )),
            }
        }

        if mismatches.is_empty() {
            None
        } else {
            Some(Self::config_issue(
                "calibration_config_mismatch",
                format!(
                    "CalibrationConfigGate: echelle profile does not match {device_id}: {}",
                    mismatches.join(", ")
                ),
                device_type,
                device_id,
            ))
        }
    }

    // ---- Pure helpers ----

    pub(crate) fn config_issue(
        code: &str,
        message: String,
        device_type: &str,
        device_id: &str,
    ) -> RunReadinessIssue {
        RunReadinessIssue {
            code: code.to_string(),
            message,
            blocking: true,
            device_type: Some(device_type.to_string()),
            device_id: Some(device_id.to_string()),
            age_hours: None,
            max_age_hours: None,
        }
    }

    pub(crate) fn read_u32_parameter(
        parameterized: &dyn Parameterized,
        candidates: &[(&str, u32)],
    ) -> Option<u32> {
        for (name, subtract) in candidates {
            let Some(parameter) = parameterized.parameters().get(name) else {
                continue;
            };
            let Ok(value) = parameter.get_json() else {
                continue;
            };
            let Some(raw) = Self::json_to_u32(&value) else {
                continue;
            };
            return Some(raw.saturating_sub(*subtract));
        }
        None
    }

    pub(crate) fn json_to_u32(value: &serde_json::Value) -> Option<u32> {
        value
            .as_u64()
            .and_then(|raw| u32::try_from(raw).ok())
            .or_else(|| value.as_i64().and_then(|raw| u32::try_from(raw).ok()))
    }
}

/// Readiness issue detected before a run starts.
#[derive(Debug, Clone, PartialEq)]
pub struct RunReadinessIssue {
    /// Logical code for clients/tests.
    pub code: String,
    /// User-facing explanation.
    pub message: String,
    /// Whether this issue blocks `start()`.
    pub blocking: bool,
    /// Affected device type, if known.
    pub device_type: Option<String>,
    /// Specific device id involved in the issue, if known.
    pub device_id: Option<String>,
    /// Current calibration age in hours, if known.
    pub age_hours: Option<f64>,
    /// Maximum allowed age in hours, if known.
    pub max_age_hours: Option<f64>,
}

// ---------------------------------------------------------------------------
// RunEngine thin delegation wrappers
// ---------------------------------------------------------------------------

impl RunEngine {
    /// Register the freshest known calibration for a logical device type.
    pub async fn register_calibration_snapshot(&self, snapshot: CalibrationFreshness) {
        self.readiness.register_calibration_snapshot(snapshot).await;
    }

    /// Clear the active calibration snapshot for a device type.
    pub async fn clear_calibration_snapshot(&self, device_type: &str) {
        self.readiness.clear_calibration_snapshot(device_type).await;
    }

    /// Clear only the radiance/timestamp portion of the calibration snapshot.
    pub async fn clear_radiance_calibration_snapshot(&self, device_type: &str) {
        self.readiness
            .clear_radiance_calibration_snapshot(device_type)
            .await;
    }

    /// Clear only the echelle-compatibility portion of the calibration snapshot.
    pub async fn clear_echelle_calibration_snapshot(&self, device_type: &str) {
        self.readiness
            .clear_echelle_calibration_snapshot(device_type)
            .await;
    }

    /// Override the maximum allowed calibration age for a device type.
    pub async fn set_calibration_max_age(&self, device_type: &str, max_age: Duration) {
        self.readiness
            .set_calibration_max_age(device_type, max_age)
            .await;
    }

    /// Inspect readiness issues for the next queued plan without starting it.
    pub async fn next_plan_readiness_issues(&self) -> Vec<RunReadinessIssue> {
        let device_ids = self
            .task_queue
            .peek_first(|queued| {
                queued.map_or_else(Vec::new, |q| Self::collect_plan_devices(q.plan.as_ref()))
            })
            .await;
        self.readiness
            .readiness_issues_for_devices(&device_ids, &self.device_registry)
            .await
    }

    pub(crate) fn collect_plan_devices(plan: &dyn Plan) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        plan.movers()
            .into_iter()
            .chain(plan.detectors())
            .filter(|device_id| seen.insert(device_id.clone()))
            .collect()
    }
}
