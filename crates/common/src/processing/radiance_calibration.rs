//! Radiance calibration for LIBS spectra.
//!
//! Ports `radiance_calibration()` from the Python LIBS codebase.  Given a
//! measured LIBS spectrum, a white-light lamp reference spectrum, and a
//! per-grating system-response calibration file, this module computes the
//! correction factor and returns a radiometrically corrected spectrum.
//!
//! # Algorithm
//!
//! ```text
//! correction[λ] = lamp[λ] / cal[λ]          (element-wise)
//! calibrated[λ] = raw[λ] × correction[λ]
//! ```
//!
//! Both `lamp` and `cal` are linearly interpolated onto the raw spectrum's
//! wavelength axis before the division.
//!
//! # File Formats
//!
//! Calibration files are two-column plain text (wavelength  value).  Lines
//! that start with `#`, `%`, or a non-numeric character are treated as
//! comment / header lines and skipped.  Columns may be separated by spaces,
//! tabs, or commas.

use std::{collections::HashMap, fs, path::Path};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};

/// Optional metadata parsed from a calibration text file header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalibrationFileMetadata {
    /// RFC 3339 timestamp indicating when the calibration data was recorded.
    pub calibration_timestamp: Option<DateTime<Utc>>,
    /// Logical calibration family, used for freshness policies.
    pub device_type: Option<String>,
}

/// A 1-D spectrum: parallel arrays of wavelengths (nm) and intensities.
///
/// `wavelengths` must be sorted in ascending order.
#[derive(Debug, Clone)]
pub struct Spectrum {
    /// Wavelength axis in nanometres, sorted ascending.
    pub wavelengths: Vec<f64>,
    /// Corresponding intensity values (arbitrary units).
    pub values: Vec<f64>,
}

impl Spectrum {
    /// Construct a spectrum from parallel wavelength and value vectors.
    ///
    /// Returns an error if the vectors have different lengths or are empty.
    pub fn new(wavelengths: Vec<f64>, values: Vec<f64>) -> Result<Self> {
        if wavelengths.is_empty() {
            return Err(anyhow!("spectrum wavelength array is empty"));
        }
        if wavelengths.len() != values.len() {
            return Err(anyhow!(
                "wavelength ({}) and value ({}) arrays have different lengths",
                wavelengths.len(),
                values.len()
            ));
        }
        Ok(Self {
            wavelengths,
            values,
        })
    }

    /// Number of spectral channels.
    #[inline]
    pub fn len(&self) -> usize {
        self.wavelengths.len()
    }

    /// Returns `true` if the spectrum has no channels.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.wavelengths.is_empty()
    }

    /// Normalise the intensity values so the maximum equals 1.0.
    ///
    /// A no-op (returns a clone) if all values are zero.
    #[must_use]
    pub fn normalize(&self) -> Self {
        let max = self
            .values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        if max == 0.0 || !max.is_finite() {
            return self.clone();
        }
        Self {
            wavelengths: self.wavelengths.clone(),
            values: self.values.iter().map(|v| v / max).collect(),
        }
    }
}

/// Performs piecewise-linear interpolation of `(src_x, src_y)` evaluated at
/// each point in `target_x`.
///
/// Points in `target_x` that fall outside the range of `src_x` are clamped
/// to the nearest boundary value (flat extrapolation).
///
/// `src_x` must be sorted ascending and have the same length as `src_y`.
pub fn linear_interp(src_x: &[f64], src_y: &[f64], target_x: &[f64]) -> Vec<f64> {
    assert_eq!(
        src_x.len(),
        src_y.len(),
        "interp: src arrays must be equal length"
    );
    assert!(!src_x.is_empty(), "interp: src arrays must not be empty");

    target_x
        .iter()
        .map(|&tx| {
            // Clamp below lower bound
            if tx <= src_x[0] {
                return src_y[0];
            }
            // Clamp above upper bound
            let n = src_x.len();
            if tx >= src_x[n - 1] {
                return src_y[n - 1];
            }
            // Binary search for the interval [src_x[i], src_x[i+1]] containing tx
            let idx = src_x.partition_point(|&x| x <= tx).saturating_sub(1);
            let x0 = src_x[idx];
            let x1 = src_x[idx + 1];
            let y0 = src_y[idx];
            let y1 = src_y[idx + 1];
            let t = (tx - x0) / (x1 - x0);
            y0 + t * (y1 - y0)
        })
        .collect()
}

fn parse_calibration_metadata_line(line: &str, metadata: &mut CalibrationFileMetadata) {
    let line = line.trim_start_matches(['#', '%', ';']).trim();
    let Some((raw_key, raw_value)) = line
        .split_once(':')
        .or_else(|| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
    else {
        return;
    };

    let key = raw_key.to_ascii_lowercase();
    match key.as_str() {
        "calibration_timestamp" | "timestamp" | "created_at" => {
            if let Ok(parsed) = DateTime::parse_from_rfc3339(raw_value) {
                metadata.calibration_timestamp = Some(parsed.with_timezone(&Utc));
            }
        }
        "device_type" => {
            if !raw_value.is_empty() {
                metadata.device_type = Some(raw_value.to_ascii_lowercase());
            }
        }
        _ => {}
    }
}

/// Loads a two-column calibration file and returns a [`Spectrum`] plus parsed metadata.
///
/// Accepts `.lmp`, `.ICD`, `.asc`, `.csv`, `.txt` and similar formats.
/// Rows whose first token cannot be parsed as `f64` are silently skipped
/// (handles comment and header lines).  Columns may be whitespace- or
/// comma-separated.
pub fn load_calibration_file_with_metadata(
    path: impl AsRef<Path>,
) -> Result<(Spectrum, CalibrationFileMetadata)> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading calibration file {}", path.display()))?;

    let mut wavelengths = Vec::new();
    let mut values = Vec::new();
    let mut metadata = CalibrationFileMetadata::default();

    for line in text.lines() {
        let line = line.trim();
        // Skip blank lines and comment lines
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with('%')
            || line.starts_with(';')
        {
            parse_calibration_metadata_line(line, &mut metadata);
            continue;
        }
        // Split on whitespace or comma
        let parts: Vec<&str> = line
            .splitn(3, |c: char| c == ',' || c.is_whitespace())
            .collect();
        if parts.len() < 2 {
            continue;
        }
        let Ok(wl) = parts[0].trim().parse::<f64>() else {
            continue;
        };
        let Ok(val) = parts[1].trim().parse::<f64>() else {
            continue;
        };
        wavelengths.push(wl);
        values.push(val);
    }

    if wavelengths.is_empty() {
        return Err(anyhow!(
            "calibration file {} contained no valid data rows",
            path.display()
        ));
    }

    // Sort by wavelength in case file is not ordered
    let mut pairs: Vec<(f64, f64)> = wavelengths.into_iter().zip(values).collect();
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (wavelengths, values): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();

    let spectrum = Spectrum::new(wavelengths, values)?;
    Ok((spectrum, metadata))
}

/// Loads a two-column calibration file and returns a [`Spectrum`].
pub fn load_calibration_file(path: impl AsRef<Path>) -> Result<Spectrum> {
    load_calibration_file_with_metadata(path).map(|(spectrum, _)| spectrum)
}

fn resolve_calibration_device_type(
    device_types: impl IntoIterator<Item = Option<String>>,
) -> Result<String> {
    let mut resolved: Option<String> = None;

    for device_type in device_types.into_iter().flatten() {
        let candidate = device_type.trim().to_ascii_lowercase();
        if candidate.is_empty() {
            continue;
        }

        match &resolved {
            Some(existing) if existing != &candidate => {
                return Err(anyhow!(
                    "calibration file device_type mismatch: '{}' vs '{}'",
                    existing,
                    candidate
                ));
            }
            Some(_) => {}
            None => resolved = Some(candidate),
        }
    }

    Ok(resolved.unwrap_or_else(|| "spectroscopy".to_string()))
}

/// Radiometric correction engine for LIBS spectra.
///
/// Holds a white-light lamp reference spectrum and one calibration spectrum
/// per grating index.  Calibration spectra are typically recorded with a
/// diffuse reflectance standard (e.g. Spectralon) and represent the
/// wavelength-dependent system response.
///
/// # Example
/// ```no_run
/// use common::processing::radiance_calibration::{RadianceCalibrator, Spectrum};
///
/// let cal = RadianceCalibrator::from_files(
///     "calibration/lamp_spec.lmp",
///     &[(1, "calibration/g1_cal.asc"), (2, "calibration/g2_cal.asc")],
/// ).unwrap();
///
/// let spectrum = Spectrum::new(vec![300.0, 400.0, 500.0], vec![100.0, 200.0, 150.0]).unwrap();
/// let corrected = cal.calibrate(&spectrum, 1, true).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct RadianceCalibrator {
    lamp_spectrum: Spectrum,
    /// Map from 1-based grating index → system-response calibration spectrum.
    grating_calibrations: HashMap<u8, Spectrum>,
    calibration_timestamp: Option<DateTime<Utc>>,
    device_type: String,
}

impl RadianceCalibrator {
    /// Construct from pre-loaded [`Spectrum`] objects.
    pub fn new(lamp_spectrum: Spectrum, grating_calibrations: HashMap<u8, Spectrum>) -> Self {
        Self::with_metadata(
            lamp_spectrum,
            grating_calibrations,
            None,
            "spectroscopy".to_string(),
        )
    }

    /// Construct from pre-loaded [`Spectrum`] objects plus freshness metadata.
    pub fn with_metadata(
        lamp_spectrum: Spectrum,
        grating_calibrations: HashMap<u8, Spectrum>,
        calibration_timestamp: Option<DateTime<Utc>>,
        device_type: impl Into<String>,
    ) -> Self {
        Self {
            lamp_spectrum,
            grating_calibrations,
            calibration_timestamp,
            device_type: device_type.into(),
        }
    }

    /// Construct by loading calibration data from files.
    ///
    /// # Arguments
    /// * `lamp_file` — path to the white-light lamp spectrum file
    /// * `cal_files` — slice of `(grating_index, path)` pairs, one per grating
    pub fn from_files<L, P>(lamp_file: L, cal_files: &[(u8, P)]) -> Result<Self>
    where
        L: AsRef<Path>,
        P: AsRef<Path>,
    {
        let (lamp_spectrum, lamp_metadata) = load_calibration_file_with_metadata(lamp_file)?;
        let mut grating_calibrations = HashMap::new();
        let mut timestamps = Vec::new();
        let mut device_types = vec![lamp_metadata.device_type.clone()];
        if let Some(timestamp) = lamp_metadata.calibration_timestamp {
            timestamps.push(timestamp);
        }
        for (grating, path) in cal_files {
            let (spec, metadata) = load_calibration_file_with_metadata(path.as_ref())
                .with_context(|| format!("loading grating {grating} calibration"))?;
            grating_calibrations.insert(*grating, spec);
            device_types.push(metadata.device_type);
            if let Some(timestamp) = metadata.calibration_timestamp {
                timestamps.push(timestamp);
            }
        }

        let calibration_timestamp = timestamps.into_iter().min();
        let device_type = resolve_calibration_device_type(device_types)?;

        Ok(Self {
            lamp_spectrum,
            grating_calibrations,
            calibration_timestamp,
            device_type,
        })
    }

    /// Apply radiometric correction to `spectrum` acquired with `grating`.
    ///
    /// # Arguments
    /// * `spectrum` — raw LIBS spectrum to calibrate
    /// * `grating`  — 1-based grating index (must match a loaded calibration)
    /// * `normalize` — if `true`, scale the result so its maximum equals 1.0
    ///
    /// # Algorithm
    /// 1. Linearly interpolate lamp and grating-calibration spectra onto
    ///    `spectrum.wavelengths`.
    /// 2. Compute per-channel correction `= lamp / cal`.
    /// 3. Multiply `spectrum.values` element-wise by the correction.
    pub fn calibrate(&self, spectrum: &Spectrum, grating: u8, normalize: bool) -> Result<Spectrum> {
        let cal = self
            .grating_calibrations
            .get(&grating)
            .ok_or_else(|| anyhow!("no calibration loaded for grating {grating}"))?;

        // Interpolate lamp and calibration onto the raw spectrum's wavelength axis
        let lamp_interp = linear_interp(
            &self.lamp_spectrum.wavelengths,
            &self.lamp_spectrum.values,
            &spectrum.wavelengths,
        );
        let cal_interp = linear_interp(&cal.wavelengths, &cal.values, &spectrum.wavelengths);

        // Correction factor: lamp_response / system_response
        // Guard against division by zero with a small epsilon
        let correction: Vec<f64> = lamp_interp
            .iter()
            .zip(cal_interp.iter())
            .map(|(&l, &c)| if c.abs() > 1e-12 { l / c } else { 0.0 })
            .collect();

        // Apply correction element-wise
        let calibrated: Vec<f64> = spectrum
            .values
            .iter()
            .zip(correction.iter())
            .map(|(&s, &c)| s * c)
            .collect();

        let result = Spectrum {
            wavelengths: spectrum.wavelengths.clone(),
            values: calibrated,
        };

        if normalize {
            Ok(result.normalize())
        } else {
            Ok(result)
        }
    }

    /// Returns the lamp spectrum.
    pub fn lamp_spectrum(&self) -> &Spectrum {
        &self.lamp_spectrum
    }

    /// Returns the calibration spectrum for `grating`, if loaded.
    pub fn grating_calibration(&self, grating: u8) -> Option<&Spectrum> {
        self.grating_calibrations.get(&grating)
    }

    /// Returns the calibration timestamp, if present in the source files.
    pub fn calibration_timestamp(&self) -> Option<DateTime<Utc>> {
        self.calibration_timestamp
    }

    /// Returns the logical device type for freshness gating.
    pub fn device_type(&self) -> &str {
        &self.device_type
    }

    /// Returns the elapsed time since calibration, if timestamp metadata exists.
    pub fn calibration_age(&self, now: DateTime<Utc>) -> Option<chrono::Duration> {
        self.calibration_timestamp
            .as_ref()
            .map(|timestamp| now.signed_duration_since(timestamp.to_owned()))
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_spectrum(n: usize, value: f64) -> Spectrum {
        #[allow(clippy::cast_precision_loss)] // test: small n, sub-nm precision acceptable
        let wl: Vec<f64> = (0..n).map(|i| 300.0 + i as f64).collect();
        let vals = vec![value; n];
        Spectrum::new(wl, vals).unwrap()
    }

    #[test]
    fn test_linear_interp_exact_points() {
        let src_x = vec![0.0, 1.0, 2.0, 3.0];
        let src_y = vec![0.0, 10.0, 20.0, 30.0];
        let result = linear_interp(&src_x, &src_y, &src_x);
        for (got, exp) in result.iter().zip(src_y.iter()) {
            assert!((got - exp).abs() < 1e-10, "expected {exp}, got {got}");
        }
    }

    #[test]
    fn test_linear_interp_midpoints() {
        let src_x = vec![0.0, 2.0, 4.0];
        let src_y = vec![0.0, 10.0, 20.0];
        let target = vec![1.0, 3.0];
        let result = linear_interp(&src_x, &src_y, &target);
        assert!((result[0] - 5.0).abs() < 1e-10);
        assert!((result[1] - 15.0).abs() < 1e-10);
    }

    #[test]
    fn test_linear_interp_clamps_out_of_range() {
        let src_x = vec![1.0, 2.0, 3.0];
        let src_y = vec![10.0, 20.0, 30.0];
        let result = linear_interp(&src_x, &src_y, &[0.0, 4.0]);
        assert_eq!(result[0], 10.0, "below lower bound should clamp");
        assert_eq!(result[1], 30.0, "above upper bound should clamp");
    }

    #[test]
    fn test_calibrate_flat_unity() {
        // lamp == cal → correction == 1 everywhere → output == input
        let lamp = flat_spectrum(100, 1000.0);
        let cal = flat_spectrum(100, 1000.0);
        let mut cals = HashMap::new();
        cals.insert(1u8, cal);
        let calibrator = RadianceCalibrator::new(lamp, cals);

        let raw = flat_spectrum(100, 500.0);
        let result = calibrator.calibrate(&raw, 1, false).unwrap();

        for (got, exp) in result.values.iter().zip(raw.values.iter()) {
            assert!(
                (got - exp).abs() < 1e-6,
                "flat unity: expected {exp}, got {got}"
            );
        }
    }

    #[test]
    fn test_calibrate_normalize() {
        let lamp = flat_spectrum(50, 2000.0);
        let cal = flat_spectrum(50, 1000.0);
        let mut cals = HashMap::new();
        cals.insert(2u8, cal);
        let calibrator = RadianceCalibrator::new(lamp, cals);

        let raw = flat_spectrum(50, 500.0);
        let result = calibrator.calibrate(&raw, 2, true).unwrap();

        let max = result
            .values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (max - 1.0).abs() < 1e-10,
            "normalized max should be 1.0, got {max}"
        );
    }

    #[test]
    fn test_calibrate_unknown_grating_errors() {
        let lamp = flat_spectrum(10, 100.0);
        let calibrator = RadianceCalibrator::new(lamp, HashMap::new());
        let raw = flat_spectrum(10, 1.0);
        assert!(
            calibrator.calibrate(&raw, 99, false).is_err(),
            "should error on unknown grating"
        );
    }

    #[test]
    fn test_calibrate_division_by_zero_guarded() {
        // calibration values of zero should produce 0 output (not NaN/inf)
        let lamp = flat_spectrum(10, 500.0);
        let zero_cal = Spectrum::new(
            (0..10).map(|i| 300.0 + f64::from(i)).collect(),
            vec![0.0; 10],
        )
        .unwrap();
        let mut cals = HashMap::new();
        cals.insert(1u8, zero_cal);
        let calibrator = RadianceCalibrator::new(lamp, cals);
        let raw = flat_spectrum(10, 1.0);
        let result = calibrator.calibrate(&raw, 1, false).unwrap();
        for v in &result.values {
            assert!(
                v.is_finite(),
                "division-by-zero should yield 0, not NaN/inf"
            );
        }
    }

    #[test]
    fn test_spectrum_normalize() {
        let spec = Spectrum::new(vec![300.0, 400.0, 500.0], vec![2.0, 4.0, 8.0]).unwrap();
        let norm = spec.normalize();
        assert!((norm.values[2] - 1.0).abs() < 1e-10);
        assert!((norm.values[0] - 0.25).abs() < 1e-10);
    }

    #[test]
    fn test_load_calibration_file_roundtrip() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new()?;
        writeln!(tmp, "# header")?;
        writeln!(tmp, "300.0\t100.0")?;
        writeln!(tmp, "400.0\t200.0")?;
        writeln!(tmp, "500.0\t150.0")?;
        let spec = load_calibration_file(tmp.path())?;
        assert_eq!(spec.len(), 3);
        assert!((spec.wavelengths[1] - 400.0).abs() < 1e-10);
        assert!((spec.values[1] - 200.0).abs() < 1e-10);
        Ok(())
    }

    #[test]
    fn test_load_calibration_file_parses_metadata(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;

        let mut tmp = tempfile::NamedTempFile::new()?;
        writeln!(tmp, "# calibration_timestamp: 2026-03-10T12:30:00Z")?;
        writeln!(tmp, "# device_type: spectroscopy")?;
        writeln!(tmp, "300.0\t100.0")?;
        writeln!(tmp, "400.0\t200.0")?;

        let (_spec, metadata) = load_calibration_file_with_metadata(tmp.path())?;
        assert_eq!(
            metadata.calibration_timestamp,
            Some(DateTime::parse_from_rfc3339("2026-03-10T12:30:00Z")?.with_timezone(&Utc))
        );
        assert_eq!(metadata.device_type.as_deref(), Some("spectroscopy"));
        Ok(())
    }

    #[test]
    fn test_radiance_calibrator_from_files_uses_oldest_timestamp(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;

        let write_file = |content: &str| -> std::io::Result<tempfile::NamedTempFile> {
            let mut file = tempfile::NamedTempFile::new()?;
            write!(file, "{content}")?;
            Ok(file)
        };

        let lamp = write_file(
            "# calibration_timestamp: 2026-03-11T12:00:00Z\n# device_type: spectroscopy\n300 1\n400 1\n",
        )?;
        let g1 = write_file(
            "# calibration_timestamp: 2026-03-10T09:00:00Z\n# device_type: spectroscopy\n300 1\n400 1\n",
        )?;
        let g2 = write_file(
            "# calibration_timestamp: 2026-03-12T08:00:00Z\n# device_type: spectroscopy\n300 1\n400 1\n",
        )?;

        let calibrator =
            RadianceCalibrator::from_files(lamp.path(), &[(1u8, g1.path()), (2u8, g2.path())])?;

        assert_eq!(calibrator.device_type(), "spectroscopy");
        assert_eq!(
            calibrator.calibration_timestamp(),
            Some(DateTime::parse_from_rfc3339("2026-03-10T09:00:00Z")?.with_timezone(&Utc))
        );
        Ok(())
    }

    #[test]
    fn test_radiance_calibrator_from_files_rejects_device_type_mismatch() {
        use std::io::Write;

        let write_file = |content: &str| -> std::io::Result<tempfile::NamedTempFile> {
            let mut file = tempfile::NamedTempFile::new()?;
            write!(file, "{content}")?;
            Ok(file)
        };

        let lamp = write_file("# device_type: spectroscopy\n300 1\n400 1\n").unwrap();
        let g1 = write_file("# device_type: imaging\n300 1\n400 1\n").unwrap();

        let err = RadianceCalibrator::from_files(lamp.path(), &[(1u8, g1.path())]).unwrap_err();
        assert!(
            err.to_string().contains("device_type mismatch"),
            "unexpected error: {err}"
        );
    }
}
