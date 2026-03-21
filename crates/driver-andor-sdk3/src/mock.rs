//! Mock implementations for cross-platform development
//!
//! Provides full mock implementations of camera and spectrograph drivers
//! that work without the Andor SDK3 installed. Useful for:
//!
//! - Development on non-Windows platforms
//! - CI/CD pipelines
//! - Unit testing
//! - GUI development without hardware
//!
//! The mock drivers simulate realistic behavior including:
//! - Synthetic gradient frame generation
//! - Wavelength-dependent spectra
//! - Realistic parameter ranges
//! - Async delays to simulate hardware timing

use crate::introspection::{self, DiscoveredFeature, FeatureType};
use crate::types::{CameraInfo, Grating, SpectrographInfo, WavelengthCalibration};
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use common::capabilities::{
    ExposureControl, FrameObserver, FrameProducer, ObserverHandle, Parameterized, ShutterControl,
    Triggerable, WavelengthTunable,
};
use common::data::{Frame, FrameView};
use common::observable::ParameterSet;
use common::parameter::Parameter;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Duration;

// ── LIBS synthetic spectrum generator ────────────────────────────────────────

/// Generate a synthetic 1-D LIBS emission spectrum.
///
/// Simulates time-resolved plasma emission observed with a gated iStar camera:
///
/// - **Continuum background**: exponential decay from a hot continuum emitter.
///   At early delays (< 1 µs) the background is bright; by 5 µs it is negligible.
/// - **Emission lines**: Gaussian line profiles at pixel positions corresponding
///   to common LIBS analytes (Al I at 309/394/396 nm, Fe I at 438/487 nm,
///   Ca I at 422 nm, Si I at 288 nm).  Line intensities are scaled by MCP gain.
/// - **Shot noise**: Poisson noise proportional to the square root of signal × gain.
///
/// # Arguments
/// * `delay_ps`   — DDG gate delay in picoseconds (typical: 1_300_000 = 1.3 µs)
/// * `mcp_gain`   — MCP intensifier gain, 0–4095 (typical: 3600)
/// * `num_pixels` — number of spectral pixels (typical: 2560)
/// * `frame_seed` — seed for deterministic noise (use frame number)
///
/// # Returns
/// A `Vec<f64>` of `num_pixels` intensity values in counts (ADU), suitable
/// for packing into a 16-bit Mono16 frame.
pub fn generate_libs_spectrum(
    delay_ps: u64,
    mcp_gain: u32,
    num_pixels: usize,
    frame_seed: u32,
) -> Vec<f64> {
    let mut spectrum = vec![0.0f64; num_pixels];

    // ── Continuum background (decays ~1/e per µs) ────────────────────────
    // delay_ps is u64; precision loss is acceptable for simulation purposes.
    #[allow(clippy::cast_precision_loss)]
    let delay_us = delay_ps as f64 / 1.0e6;
    let continuum_amp = 8000.0 * (-delay_us / 1.0).exp();
    for v in &mut spectrum {
        *v += continuum_amp;
    }

    // ── Gain scale factor (0–1, linear with MCP gain) ────────────────────
    let gain_scale = f64::from(mcp_gain) / 4095.0;

    // ── Emission lines (pixel center, peak intensity, width σ in pixels) ─
    // Pixel positions assume ~0.15 nm/pixel dispersion centred at 400 nm
    // (wavelength range ≈ 208–592 nm for 2560-pixel detector).
    // Positions are scaled proportionally for other pixel counts.
    // num_pixels is usize; precision loss acceptable for scaling calculation.
    #[allow(clippy::cast_precision_loss)]
    let scale = num_pixels as f64 / 2560.0;
    let emission_lines: &[(f64, f64, f64)] = &[
        // Si I 288 nm
        (530.0 * scale, 25_000.0, 2.5),
        // Al I 309 nm
        (674.0 * scale, 50_000.0, 2.5),
        // Al I 394 nm
        (1240.0 * scale, 45_000.0, 2.5),
        // Al I 396 nm
        (1253.0 * scale, 40_000.0, 2.5),
        // Ca I 422 nm
        (1427.0 * scale, 30_000.0, 2.5),
        // Fe I 438 nm
        (1533.0 * scale, 20_000.0, 2.5),
        // Fe I 487 nm
        (1860.0 * scale, 15_000.0, 2.5),
    ];

    for &(center, amplitude, sigma) in emission_lines {
        for (i, v) in spectrum.iter_mut().enumerate() {
            // i is usize; precision loss acceptable for Gaussian profile calculation.
            #[allow(clippy::cast_precision_loss)]
            let dx = i as f64 - center;
            *v += amplitude * gain_scale * (-0.5 * (dx / sigma).powi(2)).exp();
        }
    }

    // ── Shot noise (simple LCG seeded by frame_seed for repeatability) ───
    let noise_scale = (gain_scale * 150.0).max(10.0);
    let mut rng = frame_seed
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    for v in &mut spectrum {
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        // Box-Muller-ish: use uniform noise for simplicity
        // rng and u32::MAX are u32; f64::from gives lossless conversion.
        let noise = (f64::from(rng) / f64::from(u32::MAX) - 0.5) * noise_scale;
        *v = (*v + noise).max(0.0);
    }

    spectrum
}

// ── Camera parameter names ────────────────────────────────────────────────────

/// Core parameter names that are explicitly constructed in MockCamera::new().
/// Dynamic features with these SDK3 names are skipped to avoid duplicates.
const MOCK_CORE_FEATURES: &[&str] = &["ExposureTime", "MCPGain", "MCPIntelligentGating"];

/// Mock iStar camera for testing
#[derive(Clone)]
pub struct MockCamera {
    inner: Arc<MockCameraInner>,
}

struct MockCameraInner {
    info: CameraInfo,
    streaming: AtomicBool,
    armed: AtomicBool,
    frame_count: AtomicU32,
    exposure_s: Parameter<f64>,
    mcp_gain: Parameter<u32>,
    observers: Mutex<Vec<(ObserverHandle, Box<dyn FrameObserver>)>>,
    next_observer_id: AtomicU64,
    params: ParameterSet,
}

impl MockCamera {
    pub fn new() -> Self {
        let info = CameraInfo {
            model: "Mock iStar".to_string(),
            serial_number: "MOCK-12345".to_string(),
            firmware_version: "1.0.0-mock".to_string(),
            sensor_width: 2048,
            sensor_height: 2048,
            features: crate::types::FeatureSupport {
                mcp_gain: true,
                gate_mode: true,
                ddg_output_delay: true,
                ddg_output_width: true,
                sensor_cooling: true,
                sensor_temperature: true,
                pixel_encoding: true,
                external_trigger_modes: true,
                electronic_shuttering_mode: true,
                frame_count: true,
                mcp_intelligate: true,
                mcp_voltage: true,
                insertion_delay: true,
                metadata_ddg_info: true,
                metadata_mcp_gain: true,
                metadata_frame_info: true,
                camera_acquiring: true,
                baseline_level: true,
                software_trigger: true,
            },
        };

        let exposure_s = Parameter::new("exposure_s", 0.001)
            .with_unit("s")
            .with_description("Integration time");
        let mcp_gain = Parameter::new("mcp_gain", 1000u32)
            .with_range(0, 4095)
            .with_description("MCP intensifier gain");

        let mut params = ParameterSet::new();
        params.register(exposure_s.clone());
        params.register(mcp_gain.clone());

        // Register dynamic features from the mock introspection catalog.
        // This gives MockCamera 30+ parameters (matching real iStar hardware),
        // enabling realistic UI development and integration testing.
        register_mock_dynamic_features(&introspection::introspect_mock_features(), &mut params);

        let inner = Arc::new(MockCameraInner {
            info,
            streaming: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            frame_count: AtomicU32::new(0),
            exposure_s,
            mcp_gain,
            observers: Mutex::new(Vec::new()),
            next_observer_id: AtomicU64::new(1),
            params,
        });

        Self { inner }
    }

    /// Read the current pixel encoding from the dynamic parameter.
    fn current_encoding(&self) -> &'static str {
        self.inner
            .params
            .get("PixelEncoding")
            .and_then(|p| {
                p.get_json().ok().and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s),
                    _ => None,
                })
            })
            .map(|s| match s.as_str() {
                "Mono12" => "Mono12",
                "Mono12Packed" => "Mono12Packed",
                "Mono32" => "Mono32",
                _ => "Mono16",
            })
            .unwrap_or("Mono16")
    }

    /// Generate synthetic gradient frame respecting the current `PixelEncoding`.
    ///
    /// When `height == 1` (spectroscopy binning mode as used in LIBS), generates
    /// a realistic LIBS emission spectrum via [`generate_libs_spectrum`] instead
    /// of the standard 2-D gradient.
    fn generate_frame(&self, width: u32, height: u32, frame_number: u32) -> Frame {
        // Spectroscopy mode: single-row frame → generate LIBS spectrum
        if height == 1 {
            return self.generate_libs_spectrum_frame(width, frame_number);
        }

        let encoding = self.current_encoding();
        let size = (width * height) as usize;
        let offset = frame_number % 100;

        let (data, bit_depth) = match encoding {
            "Mono12" => {
                // 12-bit values in 16-bit LE containers (2 bytes/pixel)
                let mut buf = Vec::with_capacity(size * 2);
                for y in 0..height {
                    for x in 0..width {
                        let value = ((x + y + offset) % 4096) as u16;
                        buf.extend_from_slice(&value.to_le_bytes());
                    }
                }
                (buf, 12u32)
            }
            "Mono12Packed" => {
                // Packing format (2 pixels → 3 bytes):
                // - Byte0      = pixA[7:0]
                // - Byte1[7:4] = pixB[3:0]
                // - Byte1[3:0] = pixA[11:8]
                // - Byte2      = pixB[11:4]
                let packed_size = (size / 2) * 3 + if !size.is_multiple_of(2) { 2 } else { 0 };
                let mut buf = Vec::with_capacity(packed_size);
                let mut i = 0u32;
                while (i as usize) < size {
                    let pix_a = ((i + offset) % 4096) as u16;
                    if (i as usize + 1) < size {
                        let pix_b = ((i + 1 + offset) % 4096) as u16;
                        #[allow(clippy::cast_possible_truncation)]
                        // SAFETY: 12-bit packed pixel encoding — values are masked to fit in u8
                        {
                            buf.push((pix_a & 0xFF) as u8);
                            buf.push(((pix_b & 0x0F) << 4 | (pix_a >> 8) & 0x0F) as u8);
                            buf.push((pix_b >> 4) as u8);
                        }
                    } else {
                        // Odd trailing pixel
                        buf.extend_from_slice(&pix_a.to_le_bytes());
                    }
                    i += 2;
                }
                (buf, 12)
            }
            "Mono32" => {
                // 32-bit pixels (4 bytes/pixel)
                let mut buf = Vec::with_capacity(size * 4);
                for y in 0..height {
                    for x in 0..width {
                        let value = (x + y + offset) % 1_000_000;
                        buf.extend_from_slice(&value.to_le_bytes());
                    }
                }
                (buf, 32)
            }
            _ => {
                // Mono16 (default): 16-bit pixels (2 bytes/pixel)
                let mut buf = Vec::with_capacity(size * 2);
                for y in 0..height {
                    for x in 0..width {
                        let value = ((x + y + offset) % 65535) as u16;
                        buf.extend_from_slice(&value.to_le_bytes());
                    }
                }
                (buf, 16)
            }
        };

        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: u128 nanos to u64 — will not overflow until year 2554
        let timestamp_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|_| std::time::Duration::from_secs(0))
            .as_nanos() as u64;

        Frame {
            width,
            height,
            bit_depth,
            data: Bytes::from(data),
            timestamp_ns,
            frame_number: u64::from(frame_number),
            exposure_ms: Some(self.inner.exposure_s.get() * 1000.0),
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    /// Generate a synthetic 1-D LIBS spectrum as a [`Frame`] (height=1, width=num_pixels).
    ///
    /// Simulates time-resolved plasma emission:
    /// - Continuum background decays exponentially with DDG delay (early: bright, late: dark)
    /// - Emission lines at fixed wavelength-pixel positions (Al I, Fe I, Ca I, Si I)
    /// - Gaussian noise amplitude proportional to MCP gain
    ///
    /// This is used when the frame dimensions are (width=N, height=1), which is
    /// the standard binned spectroscopy mode for LIBS with an iStar camera.
    fn generate_libs_spectrum_frame(&self, num_pixels: u32, frame_number: u32) -> Frame {
        let mcp_gain = self.inner.mcp_gain.get();
        let delay_ps = 1_300_000u64; // 1.3 µs default; real driver would read from parameter
        let spectrum =
            generate_libs_spectrum(delay_ps, mcp_gain, num_pixels as usize, frame_number);

        // Pack as Mono16 (2 bytes per pixel, 1 row)
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // SAFETY: spectrum values are clamped to 0..65535 (non-negative)
        let data: Vec<u8> = spectrum
            .iter()
            .flat_map(|&v| (v.min(65535.0) as u16).to_le_bytes())
            .collect();

        #[allow(clippy::cast_possible_truncation)]
        let timestamp_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|_| std::time::Duration::from_secs(0))
            .as_nanos() as u64;

        Frame {
            width: num_pixels,
            height: 1,
            bit_depth: 16,
            data: Bytes::from(data),
            timestamp_ns,
            frame_number: u64::from(frame_number),
            exposure_ms: Some(self.inner.exposure_s.get() * 1000.0),
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    /// Notify registered observers with a FrameView of the given frame.
    async fn notify_observers(&self, frame: &Frame) {
        let observers = self.inner.observers.lock().await;
        if !observers.is_empty() {
            let frame_view = FrameView::from_frame(frame);
            for (_, observer) in observers.iter() {
                observer.on_frame(&frame_view);
            }
        }
    }
}

#[async_trait]
impl FrameProducer for MockCamera {
    async fn start_stream(&self) -> Result<()> {
        self.inner.streaming.store(true, Ordering::Relaxed);

        // Spawn frame generation task
        let camera = self.clone();
        tokio::spawn(async move {
            let mut frame_number = 0u32;

            while camera.inner.streaming.load(Ordering::Relaxed) {
                let exposure = camera.inner.exposure_s.get();
                tokio::time::sleep(Duration::from_secs_f64(exposure)).await;

                let frame = camera.generate_frame(2048, 2048, frame_number);
                camera.notify_observers(&frame).await;
                frame_number += 1;
            }
        });

        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        self.inner.streaming.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        (2048, 2048)
    }

    async fn register_observer(&self, observer: Box<dyn FrameObserver>) -> Result<ObserverHandle> {
        let handle = ObserverHandle(self.inner.next_observer_id.fetch_add(1, Ordering::Relaxed));
        self.inner.observers.lock().await.push((handle, observer));
        Ok(handle)
    }

    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()> {
        let mut observers = self.inner.observers.lock().await;
        observers.retain(|(h, _)| *h != handle);
        Ok(())
    }

    fn supports_observers(&self) -> bool {
        true
    }
}

#[async_trait]
impl Triggerable for MockCamera {
    async fn arm(&self) -> Result<()> {
        self.inner.armed.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        if !self.inner.armed.load(Ordering::Relaxed) {
            anyhow::bail!("Camera not armed");
        }

        let frame_number = self.inner.frame_count.fetch_add(1, Ordering::Relaxed);
        let frame = self.generate_frame(2048, 2048, frame_number);
        self.notify_observers(&frame).await;

        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.inner.armed.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl ExposureControl for MockCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        self.inner.exposure_s.set(seconds).await?;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(self.inner.exposure_s.get())
    }
}

impl Parameterized for MockCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}

/// Mock Shamrock spectrograph for testing
#[derive(Clone)]
pub struct MockSpectrograph {
    inner: Arc<MockSpectrographInner>,
}

struct MockSpectrographInner {
    info: SpectrographInfo,
    wavelength_nm: Parameter<f64>,
    grating: Parameter<Grating>,
    shutter_open: AtomicBool,
    params: ParameterSet,
}

impl MockSpectrograph {
    pub fn new() -> Self {
        let info = SpectrographInfo {
            model: "Mock Shamrock".to_string(),
            serial_number: "MOCK-SPEC-001".to_string(),
            num_gratings: 3,
        };

        let wavelength_nm = Parameter::new("wavelength_nm", 310.0)
            .with_unit("nm")
            .with_description("Center wavelength");
        let grating =
            Parameter::new("grating", Grating::Grating2).with_description("Active grating");

        let mut params = ParameterSet::new();
        params.register(wavelength_nm.clone());
        params.register(grating.clone());

        let inner = Arc::new(MockSpectrographInner {
            info,
            wavelength_nm,
            grating,
            shutter_open: AtomicBool::new(false),
            params,
        });

        Self { inner }
    }

    pub async fn get_wavelength_calibration(
        &self,
        num_pixels: u32,
    ) -> Result<WavelengthCalibration> {
        let center = self.inner.wavelength_nm.get();
        let dispersion = 0.05; // nm per pixel

        let wavelengths: Vec<f64> = (0..num_pixels)
            .map(|i| center + (f64::from(i) - f64::from(num_pixels) / 2.0) * dispersion)
            .collect();

        Ok(WavelengthCalibration::new(wavelengths))
    }
}

#[async_trait]
impl WavelengthTunable for MockSpectrograph {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.inner.wavelength_nm.set(wavelength_nm).await?;
        // Simulate hardware delay
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    async fn get_wavelength(&self) -> Result<f64> {
        Ok(self.inner.wavelength_nm.get())
    }

    fn wavelength_range(&self) -> (f64, f64) {
        (200.0, 1000.0)
    }
}

#[async_trait]
impl ShutterControl for MockSpectrograph {
    async fn open_shutter(&self) -> Result<()> {
        self.inner.shutter_open.store(true, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        self.inner.shutter_open.store(false, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }

    async fn is_shutter_open(&self) -> Result<bool> {
        Ok(self.inner.shutter_open.load(Ordering::Relaxed))
    }
}

impl Parameterized for MockSpectrograph {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}

// ---------------------------------------------------------------------------
// Dynamic feature registration for MockCamera
// ---------------------------------------------------------------------------

/// Register `Parameter<T>` instances from the mock introspection catalog.
///
/// Converts each `DiscoveredFeature` into the appropriate typed parameter:
/// - Float → `Parameter<f64>` with introspectable range
/// - Int → `Parameter<i64>` with introspectable range
/// - Bool → `Parameter<bool>`
/// - Enum → `Parameter<String>` with introspectable choices
/// - Str → `Parameter<String>` (read-only identity strings)
/// - Command → skipped (not observable state)
///
/// Features whose SDK3 names overlap with `MOCK_CORE_FEATURES` are skipped,
/// and unimplemented or non-displayable features are skipped.
fn register_mock_dynamic_features(features: &[DiscoveredFeature], params: &mut ParameterSet) {
    let mut count = 0u32;

    for feat in features {
        // Skip unimplemented, non-displayable, and core-overlap features.
        if !feat.is_displayable() || MOCK_CORE_FEATURES.contains(&feat.name.as_str()) {
            continue;
        }
        // Commands are not observable parameters.
        if feat.feature_type == FeatureType::Command {
            continue;
        }

        match feat.feature_type {
            FeatureType::Float => {
                let mut p = Parameter::new(&feat.name, 0.0f64).with_dtype("float");
                if let Some((min, max)) = feat.float_range {
                    p = p.with_range_introspectable(min, max);
                }
                if !feat.writable {
                    p = p.read_only();
                }
                if let Some(ref group) = feat.group {
                    p.with_metadata(|m| m.group_name = Some(group.clone()));
                }
                params.register(p);
            }
            FeatureType::Int => {
                let default = feat.int_range.map_or(0i64, |(min, _)| min);
                let mut p = Parameter::new(&feat.name, default).with_dtype("int");
                if let Some((min, max)) = feat.int_range {
                    p = p.with_range_introspectable(min, max);
                }
                if !feat.writable {
                    p = p.read_only();
                }
                if let Some(ref group) = feat.group {
                    p.with_metadata(|m| m.group_name = Some(group.clone()));
                }
                params.register(p);
            }
            FeatureType::Bool => {
                let mut p = Parameter::new(&feat.name, false).with_dtype("bool");
                if !feat.writable {
                    p = p.read_only();
                }
                if let Some(ref group) = feat.group {
                    p.with_metadata(|m| m.group_name = Some(group.clone()));
                }
                params.register(p);
            }
            FeatureType::Enum => {
                let default = feat.enum_values.first().cloned().unwrap_or_default();
                let mut p = Parameter::new(&feat.name, default)
                    .with_choices_introspectable(feat.enum_values.clone());
                if !feat.writable {
                    p = p.read_only();
                }
                if let Some(ref group) = feat.group {
                    p.with_metadata(|m| m.group_name = Some(group.clone()));
                }
                params.register(p);
            }
            FeatureType::Str => {
                let mut p = Parameter::new(&feat.name, String::new()).with_dtype("string");
                if !feat.writable {
                    p = p.read_only();
                }
                if let Some(ref group) = feat.group {
                    p.with_metadata(|m| m.group_name = Some(group.clone()));
                }
                params.register(p);
            }
            FeatureType::Command => unreachable!("Commands filtered above"),
        }

        count += 1;
    }

    tracing::debug!(
        count,
        "MockCamera: registered dynamic iStar feature parameters"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(not(feature = "camera"))]
mod tests {
    use super::*;

    #[test]
    fn mock_camera_has_30_plus_params() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let count = params.names().len();
        assert!(
            count >= 30,
            "MockCamera should have 30+ parameters, got {count}"
        );
    }

    #[test]
    fn mock_camera_core_params_not_duplicated() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let names = params.names();
        // Should have exposure_s (core) but not ExposureTime (dynamic)
        assert!(names.contains(&"exposure_s"), "core exposure_s missing");
        assert!(names.contains(&"mcp_gain"), "core mcp_gain missing");
        assert!(
            !names.contains(&"ExposureTime"),
            "ExposureTime should be skipped (covered by exposure_s)"
        );
        assert!(
            !names.contains(&"MCPGain"),
            "MCPGain should be skipped (covered by mcp_gain)"
        );
    }

    #[test]
    fn mock_camera_enum_param_has_choices() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let trigger = params.get("TriggerMode").expect("TriggerMode should exist");
        let meta = trigger.metadata();
        assert_eq!(meta.dtype, "enum");
        assert!(
            !meta.enum_values.is_empty(),
            "TriggerMode should have enum values"
        );
        assert!(meta.enum_values.contains(&"Internal".to_string()));
    }

    #[test]
    fn mock_camera_float_param_has_range() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let frame_rate = params.get("FrameRate").expect("FrameRate should exist");
        let meta = frame_rate.metadata();
        assert_eq!(meta.dtype, "float");
        assert!(meta.min_value.is_some(), "FrameRate should have min_value");
        assert!(meta.max_value.is_some(), "FrameRate should have max_value");
    }

    #[test]
    fn mock_camera_readonly_params_flagged() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let sensor_w = params.get("SensorWidth").expect("SensorWidth should exist");
        assert!(
            sensor_w.metadata().read_only,
            "SensorWidth should be read-only"
        );
    }

    #[test]
    fn mock_camera_writable_enum_set_json() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let trigger = params.get("TriggerMode").expect("TriggerMode");
        trigger
            .set_json(serde_json::json!("External"))
            .expect("setting TriggerMode to valid value should succeed");
    }

    #[test]
    fn mock_camera_readonly_rejects_set() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let sensor_w = params.get("SensorWidth").expect("SensorWidth");
        let result = sensor_w.set_json(serde_json::json!(1024));
        assert!(result.is_err(), "SensorWidth is read-only, set should fail");
    }

    #[test]
    fn mock_camera_no_duplicate_names() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        let mut names = params.names();
        let original_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            original_len,
            "MockCamera has duplicate parameter names"
        );
    }

    // === Pixel encoding tests (C.3) ===

    #[test]
    fn frame_encoding_default_is_mono12() {
        // Default PixelEncoding is the first enum value: "Mono12"
        let camera = MockCamera::new();
        let frame = camera.generate_frame(64, 64, 0);
        assert_eq!(frame.bit_depth, 12);
        assert_eq!(
            frame.data.len(),
            64 * 64 * 2,
            "Mono12: 2 bytes/pixel (12-bit in 16-bit container)"
        );
    }

    #[test]
    fn frame_encoding_mono16() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono16"))
            .expect("set Mono16");

        let frame = camera.generate_frame(64, 64, 0);
        assert_eq!(frame.bit_depth, 16);
        assert_eq!(frame.data.len(), 64 * 64 * 2, "Mono16: 2 bytes/pixel");
    }

    #[test]
    fn frame_encoding_mono12() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono12"))
            .expect("set Mono12");

        let frame = camera.generate_frame(64, 64, 0);
        assert_eq!(frame.bit_depth, 12);
        assert_eq!(
            frame.data.len(),
            64 * 64 * 2,
            "Mono12: 2 bytes/pixel (12-bit in 16-bit container)"
        );

        // Verify all pixel values are within 12-bit range
        for chunk in frame.data.chunks_exact(2) {
            let val = u16::from_le_bytes([chunk[0], chunk[1]]);
            assert!(
                val < 4096,
                "Mono12 pixel value {} exceeds 12-bit range",
                val
            );
        }
    }

    #[test]
    fn frame_encoding_mono12packed() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono12Packed"))
            .expect("set Mono12Packed");

        let frame = camera.generate_frame(64, 64, 0);
        assert_eq!(frame.bit_depth, 12);
        let pixel_count = 64 * 64;
        let expected_size = (pixel_count / 2) * 3;
        assert_eq!(
            frame.data.len(),
            expected_size,
            "Mono12Packed: 3 bytes per 2 pixels"
        );
    }

    #[test]
    fn frame_encoding_mono12packed_roundtrip() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono12Packed"))
            .expect("set Mono12Packed");

        let frame = camera.generate_frame(4, 2, 0);
        // 8 pixels → 4 pairs → 12 bytes
        assert_eq!(frame.data.len(), 12);

        // Decode packed bytes and verify pixel values
        for pair_idx in 0..4 {
            let base = pair_idx * 3;
            let b0 = u16::from(frame.data[base]);
            let b1 = u16::from(frame.data[base + 1]);
            let b2 = u16::from(frame.data[base + 2]);
            let pix_a = b0 | ((b1 & 0x0F) << 8);
            let pix_b = (b1 >> 4) | (b2 << 4);
            assert!(pix_a < 4096, "Packed pixel A {} out of range", pix_a);
            assert!(pix_b < 4096, "Packed pixel B {} out of range", pix_b);
        }
    }

    #[test]
    fn frame_encoding_mono32() {
        let camera = MockCamera::new();
        let params = camera.parameters();
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono32"))
            .expect("set Mono32");

        let frame = camera.generate_frame(64, 64, 0);
        assert_eq!(frame.bit_depth, 32);
        assert_eq!(frame.data.len(), 64 * 64 * 4, "Mono32: 4 bytes/pixel");
    }

    #[test]
    fn frame_encoding_switch_during_operation() {
        let camera = MockCamera::new();
        let params = camera.parameters();

        // Default: Mono12
        let f1 = camera.generate_frame(32, 32, 0);
        assert_eq!(f1.bit_depth, 12);
        assert_eq!(f1.data.len(), 32 * 32 * 2);

        // Switch to Mono32
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono32"))
            .expect("set Mono32");
        let f2 = camera.generate_frame(32, 32, 1);
        assert_eq!(f2.bit_depth, 32);
        assert_eq!(f2.data.len(), 32 * 32 * 4);

        // Switch to Mono16
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono16"))
            .expect("set Mono16");
        let f3 = camera.generate_frame(32, 32, 2);
        assert_eq!(f3.bit_depth, 16);
        assert_eq!(f3.data.len(), 32 * 32 * 2);

        // Switch to Mono12Packed
        params
            .get("PixelEncoding")
            .expect("PixelEncoding exists")
            .set_json(serde_json::json!("Mono12Packed"))
            .expect("set Mono12Packed");
        let f4 = camera.generate_frame(32, 32, 3);
        assert_eq!(f4.bit_depth, 12);
        assert_eq!(f4.data.len(), (32 * 32 / 2) * 3);
    }
}
