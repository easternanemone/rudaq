//! # driver-registry
//!
//! Concrete driver factory registration for rust-daq.
//!
//! This crate owns the mapping from feature flags to concrete driver factories.
//! It breaks the "god-crate" anti-pattern where `hardware` depended on every
//! concrete driver. After this refactoring:
//!
//! - **`hardware`** = traits + `DeviceRegistry` + abstract APIs (no concrete drivers)
//! - **`driver-registry`** = concrete factory wiring + feature-flag gating
//!
//! ## Quick Example
//!
//! ```rust,ignore
//! use driver_registry::{register_all_factories, create_registry_from_config};
//! use hardware::registry::{DeviceRegistry, HardwareConfig};
//!
//! // Option A: Full registry from config file
//! let registry = create_registry_from_file(Path::new("config/maitai.toml")).await?;
//!
//! // Option B: Manual wiring
//! let registry = DeviceRegistry::new();
//! register_all_factories(&registry, Some(Path::new("config/devices"))).await?;
//! ```

use common::driver::DriverFactory;
use common::error::DaqError;
use hardware::registry::{DeviceRegistry, HardwareConfig, register_mock_factories};

// ============================================================================
// Runtime SDK probing (feature: runtime_probe)
// ============================================================================

/// Results of runtime SDK availability probing via `dlopen`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSdks {
    /// PVCAM SDK (`libpvcam.so`) is loadable at runtime.
    pub pvcam: bool,
    /// Andor SDK3 (`libatcore.so`) is loadable at runtime.
    pub andor: bool,
    /// Comedi (`libcomedi.so`) is loadable at runtime.
    pub comedi: bool,
}

impl std::fmt::Display for DiscoveredSdks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut found = Vec::new();
        if self.pvcam {
            found.push("pvcam");
        }
        if self.andor {
            found.push("andor");
        }
        if self.comedi {
            found.push("comedi");
        }
        if found.is_empty() {
            write!(f, "DiscoveredSdks(none)")
        } else {
            write!(f, "DiscoveredSdks({})", found.join(", "))
        }
    }
}

/// Probe for SDK shared libraries at runtime using `dlopen`.
///
/// Attempts to load each SDK's shared library without resolving any symbols.
/// This is a lightweight check that doesn't execute any SDK code — it only
/// verifies that the `.so` file is present and loadable by the dynamic linker.
///
/// Returns [`DiscoveredSdks`] indicating which SDKs are available.
///
/// # Platform Note
///
/// On macOS (development machines), this always returns all-false since the
/// SDK `.so` files are Linux-only. Use the shell script `scripts/ops/detect-sdk.sh`
/// for cross-platform detection.
#[cfg(feature = "runtime_probe")]
pub fn probe_available() -> DiscoveredSdks {
    DiscoveredSdks {
        pvcam: try_load_library("libpvcam.so"),
        andor: try_load_library("libatcore.so"),
        comedi: try_load_library("libcomedi.so"),
    }
}

/// Fallback when `runtime_probe` feature is disabled: reports nothing discovered.
#[cfg(not(feature = "runtime_probe"))]
pub fn probe_available() -> DiscoveredSdks {
    DiscoveredSdks {
        pvcam: false,
        andor: false,
        comedi: false,
    }
}

/// Attempt to load a shared library by name. Returns true if the library
/// is loadable by the system's dynamic linker.
#[cfg(feature = "runtime_probe")]
fn try_load_library(name: &str) -> bool {
    // SAFETY: We only load the library to check existence — we never resolve
    // symbols or call into it. The library is dropped immediately after the check.
    #[expect(unsafe_code, reason = "required to probe for dynamic library existence via libloading")]
    unsafe { libloading::Library::new(name) }.is_ok()
}

/// Register all available hardware driver factories.
///
/// This registers factories for all enabled hardware drivers:
/// - Mock drivers (always available)
/// - Andor iStar / Shamrock (when `andor` feature enabled)
/// - PVCAM cameras (when `pvcam` feature enabled)
/// - Comedi DAQ (when `comedi` feature enabled)
/// - Config-driven devices from TOML manifests (always available)
///
/// Legacy serial drivers (Thorlabs, Newport, Spectra-Physics, Red Pitaya) have been
/// removed. Serial/TCP/SCPI devices now use driver-universal TOML manifests.
///
/// # Example
///
/// ```rust,ignore
/// use driver_registry::register_all_factories;
/// use hardware::registry::DeviceRegistry;
/// use std::path::Path;
///
/// let registry = DeviceRegistry::new();
/// register_all_factories(&registry, Some(Path::new("config/devices"))).await?;
/// ```
#[expect(clippy::unused_async, reason = "async for API compatibility; concrete driver features add .await points")]
pub async fn register_all_factories(
    registry: &DeviceRegistry,
    config_dir: Option<&std::path::Path>,
) -> Result<(), DaqError> {
    // Probe for SDK availability at runtime (if runtime_probe feature enabled).
    // This lets us warn early when a driver feature is compiled in but the
    // shared library isn't actually present on this machine.
    // sdks is used in #[cfg(feature)] blocks below — unused when no SDK features enabled
    #[expect(unused_variables, reason = "sdks is used in #[cfg(feature)] blocks below; unused when no SDK features enabled")]
    let sdks = probe_available();

    // Register mock factories (always available)
    register_mock_factories(registry);

    // Register Andor SDK3 factories (iStar camera + Shamrock spectrograph)
    #[cfg(feature = "andor")]
    {
        use driver_andor_sdk3::{AndorCameraFactory, AndorSpectrographFactory};
        registry.register_factory(Box::new(AndorCameraFactory));
        registry.register_factory(Box::new(AndorSpectrographFactory));

        #[cfg(feature = "andor_hardware")]
        if !sdks.andor {
            tracing::warn!(
                "Andor SDK3 hardware feature enabled but libatcore.so not loadable at runtime"
            );
        }
    }

    // Register PVCAM factory
    #[cfg(feature = "pvcam")]
    {
        use driver_pvcam::PvcamFactory;
        registry.register_factory(Box::new(PvcamFactory));

        #[cfg(feature = "pvcam_hardware")]
        if !sdks.pvcam {
            tracing::warn!(
                "PVCAM hardware feature enabled but libpvcam.so not loadable at runtime"
            );
        }
    }

    // Register Comedi factories (NI DAQ, etc.)
    #[cfg(feature = "comedi")]
    {
        use driver_comedi::{
            ComediAnalogInputFactory, ComediAnalogOutputFactory, ComediCounterFactory,
            ComediDigitalIOFactory,
        };
        registry.register_factory(Box::new(ComediAnalogInputFactory));
        registry.register_factory(Box::new(ComediAnalogOutputFactory));
        registry.register_factory(Box::new(ComediDigitalIOFactory));
        registry.register_factory(Box::new(ComediCounterFactory));

        #[cfg(feature = "comedi_hardware")]
        if !sdks.comedi {
            tracing::warn!(
                "Comedi hardware feature enabled but libcomedi.so not loadable at runtime"
            );
        }
    }

    // Load and register config-driven factories from TOML files (schema_version=3)
    if let Some(dir) = config_dir
        && dir.exists()
    {
        match driver_universal::factory::load_all_factories(dir) {
            Ok(factories) => {
                for factory in factories {
                    let driver_type = factory.driver_type().to_string();
                    registry.register_factory(Box::new(factory));
                    tracing::debug!(driver_type = %driver_type, "Registered universal config factory");
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load config factories from {}: {}",
                    dir.display(),
                    e
                );
            }
        }
    }

    Ok(())
}

/// Load hardware config from a file and create a fully-populated `DeviceRegistry`.
///
/// This is the primary entry point for production registry creation. It:
/// 1. Parses the TOML hardware config
/// 2. Registers all available driver factories (mock + concrete + universal)
/// 3. Loads manifest-driver plugins from configured search paths
/// 4. Registers all configured devices
///
/// For test/mock usage, prefer [`create_canonical_mock_registry`].
pub async fn create_registry_from_config(
    config: &HardwareConfig,
    config_dir: Option<&std::path::Path>,
) -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();

    // Register all available driver factories BEFORE loading devices
    register_all_factories(&registry, config_dir).await?;

    // Load plugins and register devices from config
    hardware::registry::populate_registry_from_config(&registry, config).await?;

    Ok(registry)
}

/// Load hardware configuration from a file and create a DeviceRegistry.
///
/// Convenience wrapper around [`create_registry_from_config`] that handles
/// file parsing and device manifest directory resolution.
pub async fn create_registry_from_file(path: &std::path::Path) -> Result<DeviceRegistry, DaqError> {
    let config = HardwareConfig::from_file(path)?;
    // Default factory directory is alongside the hardware config (config/devices).
    let config_dir = path
        .parent()
        .map(|p| p.join("devices"))
        .filter(|p| p.exists());
    create_registry_from_config(&config, config_dir.as_deref()).await
}

/// Create a canonical mock registry using universal-driver emulators.
///
/// This is the **recommended** way to create mock registries for tests and demos.
/// Universal-eligible instruments (stages, meters, lasers, rotators) use the
/// manifest emulator path, while cameras remain on the native `mock_camera` driver.
///
/// The minimal profile provides three devices:
/// - `stage` — `universal_newport_esp300` (mock transport, address "1")
/// - `power_meter` — `universal_newport_1830-c` (mock transport)
/// - `camera` — `mock_camera` (native exception, 640x480)
///
/// For the full maitai lab mock profile (10 devices), use
/// [`create_registry_from_config`] with `config/profiles/mock_maitai_lab.toml`
/// and pass `config/devices` as the manifest directory (since
/// [`create_registry_from_file`] resolves manifests relative to the config
/// file's parent, which doesn't work for profiles nested under `config/profiles/`).
///
/// # Arguments
/// * `workspace_root` — Path to the repository root (for resolving device manifests
///   in `config/devices/`). Use `env!("CARGO_MANIFEST_DIR")` and navigate upward.
///
/// # Errors
/// Returns `DaqError::Config` if `<workspace_root>/config/devices` does not exist.
///
/// Requires the `test-util` feature (M-TEST-UTIL, bd-qd4zn).
#[cfg(any(test, feature = "test-util"))]
pub async fn create_canonical_mock_registry(
    workspace_root: &std::path::Path,
) -> Result<DeviceRegistry, DaqError> {
    let config: HardwareConfig = toml::from_str(CANONICAL_MOCK_CONFIG)
        .map_err(|e| DaqError::Config(format!("Failed to parse canonical mock config: {e}")))?;
    let devices_dir = workspace_root.join("config/devices");
    if !devices_dir.exists() {
        return Err(DaqError::Config(format!(
            "Device manifest directory not found at '{}'. \
             This is required for universal_* device registration.",
            devices_dir.display()
        )));
    }
    create_registry_from_config(&config, Some(devices_dir.as_path())).await
}

/// Embedded TOML for the canonical 3-device mock registry.
///
/// Universal-eligible instruments use `driver-universal` with `mock = true`.
/// Camera uses native `mock_camera` (SDK-bound, not expressible as a manifest).
#[cfg(any(test, feature = "test-util"))]
const CANONICAL_MOCK_CONFIG: &str = r#"
[[devices]]
id = "stage"
name = "Stage (Universal Mock)"
[devices.driver]
type = "universal_newport_esp300"
mock = true
address = "1"

[[devices]]
id = "power_meter"
name = "Power Meter (Universal Mock)"
[devices.driver]
type = "universal_newport_1830-c"
mock = true

[[devices]]
id = "camera"
name = "Camera (Mock)"
[devices.driver]
type = "mock_camera"
width = 640
height = 480
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use common::driver::Capability;

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crate should be under crates/")
            .parent()
            .expect("workspace root should exist")
            .to_path_buf()
    }

    #[tokio::test]
    async fn canonical_mock_registry_has_three_devices() {
        let registry = create_canonical_mock_registry(&workspace_root())
            .await
            .expect("canonical mock registry should build");

        assert_eq!(registry.len(), 3);
        assert!(registry.contains("stage"));
        assert!(registry.contains("power_meter"));
        assert!(registry.contains("camera"));
    }

    #[tokio::test]
    async fn canonical_mock_registry_capabilities() {
        let registry = create_canonical_mock_registry(&workspace_root())
            .await
            .expect("canonical mock registry should build");

        let devices = registry.list_devices();

        // Stage: universal ESP300 — Movable + Settable + Parameterized + StateRefreshable
        let stage = devices.iter().find(|d| d.id == "stage").unwrap();
        assert!(
            stage.capabilities.contains(&Capability::Movable),
            "universal stage should be Movable"
        );
        assert_eq!(stage.driver_type, "universal_newport_esp300");

        // Power meter: universal 1830-C — Readable + Settable + Parameterized
        let meter = devices.iter().find(|d| d.id == "power_meter").unwrap();
        assert!(
            meter.capabilities.contains(&Capability::Readable),
            "universal power meter should be Readable"
        );
        assert_eq!(meter.driver_type, "universal_newport_1830-c");

        // Camera: native mock — FrameProducer + Triggerable + ExposureControl
        let camera = devices.iter().find(|d| d.id == "camera").unwrap();
        assert!(
            camera.capabilities.contains(&Capability::FrameProducer),
            "mock camera should be FrameProducer"
        );
        assert_eq!(camera.driver_type, "mock_camera");
    }

    #[tokio::test]
    async fn canonical_mock_registry_universal_devices_are_parameterized() {
        let registry = create_canonical_mock_registry(&workspace_root())
            .await
            .expect("canonical mock registry should build");

        let devices = registry.list_devices();
        let stage = devices.iter().find(|d| d.id == "stage").unwrap();
        let meter = devices.iter().find(|d| d.id == "power_meter").unwrap();

        assert!(
            stage.capabilities.contains(&Capability::Parameterized),
            "universal stage should expose Parameterized"
        );
        assert!(
            meter.capabilities.contains(&Capability::Parameterized),
            "universal power meter should expose Parameterized"
        );
    }
}
