//! Driver Factory and Component Types
//!
//! This module provides the plugin API for dynamically registered drivers.
//! Drivers implement [`DriverFactory`] and are registered with the DeviceRegistry
//! at startup via explicit `registry.register_factory(factory)` calls.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         Composition Root (main.rs)              │
//! │  registry.register_factory(Ell14Factory::new());               │
//! │  registry.register_factory(Esp300Factory::new());              │
//! │  registry.register_factory(PvcamFactory::new());               │
//! └─────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        DeviceRegistry                           │
//! │  factories: HashMap<driver_type, Box<dyn DriverFactory>>       │
//! │  devices: HashMap<device_id, DeviceComponents>                 │
//! └─────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    DriverFactory::build()                       │
//! │  Parses TOML config, instantiates driver, returns capabilities │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example: Implementing a Driver Factory
//!
//! ```rust,ignore
//! use common::driver::{DriverFactory, DeviceComponents, Capability};
//! use futures::future::BoxFuture;
//! use std::sync::Arc;
//!
//! pub struct Ell14Factory;
//!
//! impl DriverFactory for Ell14Factory {
//!     fn driver_type(&self) -> &'static str { "ell14" }
//!     fn name(&self) -> &'static str { "Thorlabs ELL14 Rotation Mount" }
//!     fn capabilities(&self) -> &'static [Capability] { &[Capability::Movable] }
//!
//!     fn validate(&self, config: &toml::Value) -> anyhow::Result<()> {
//!         // Validate "port" and "address" fields exist
//!         let table = config.as_table().ok_or_else(|| anyhow::anyhow!("expected table"))?;
//!         if !table.contains_key("port") {
//!             anyhow::bail!("missing 'port' field");
//!         }
//!         Ok(())
//!     }
//!
//!     fn build(&self, config: toml::Value) -> BoxFuture<'static, anyhow::Result<DeviceComponents>> {
//!         Box::pin(async move {
//!             let port = config.get("port").and_then(|v| v.as_str()).unwrap();
//!             let address = config.get("address").and_then(|v| v.as_str()).unwrap();
//!
//!             let driver = Arc::new(Ell14Driver::new_async(port, address).await?);
//!
//!             Ok(DeviceComponents::new()
//!                 .with_movable(driver.clone())
//!                 .with_parameterized(driver))
//!         })
//!     }
//! }
//! ```

use crate::capabilities::{
    Commandable, CounterConfigurable, DeviceCategory, DeviceIntrospection, EmissionControl,
    ExposureControl, FrameProducer, GatedCamera, Movable, Parameterized, PulseGenerator,
    RangeIntrospectable, Readable, ReadableWithMetadata, Reconfigurable, SafetyInterlock, Settable,
    ShutterControl, SpectrometerControl, Stageable, StateRefreshable, TriggerOnPosition,
    Triggerable, WavelengthTunable,
};
use crate::data::Frame;
use crate::pipeline::MeasurementSource;
use anyhow::Result;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// Capability Enum (Runtime Introspection)
// =============================================================================

/// Runtime capability flags for device introspection.
///
/// This enum is used for querying what capabilities a device supports
/// without needing to check each trait individually. It mirrors the
/// capability traits but as an enum for easy matching and listing.
///
/// # Example
///
/// ```rust,ignore
/// use common::driver::Capability;
///
/// let caps = device.capabilities();
/// if caps.contains(&Capability::Movable) {
///     println!("Device supports motion control");
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Can move to positions (stages, rotation mounts)
    /// Corresponds to [`crate::capabilities::Movable`]
    Movable,

    /// Can read scalar values (power meters, temperature sensors)
    /// Corresponds to [`crate::capabilities::Readable`]
    Readable,

    /// Can be armed and triggered (cameras, pulse generators)
    /// Corresponds to [`crate::capabilities::Triggerable`]
    Triggerable,

    /// Produces image frames (cameras)
    /// Corresponds to [`crate::capabilities::FrameProducer`]
    FrameProducer,

    /// Has exposure/integration time control
    /// Corresponds to [`crate::capabilities::ExposureControl`]
    ExposureControl,

    /// Has settable parameters (QCodes/ScopeFoundry pattern)
    /// Corresponds to [`crate::capabilities::Settable`]
    Settable,

    /// Has shutter control (lasers)
    /// Corresponds to [`crate::capabilities::ShutterControl`]
    ShutterControl,

    /// Has wavelength tuning (tunable lasers)
    /// Corresponds to [`crate::capabilities::WavelengthTunable`]
    WavelengthTunable,

    /// Has emission on/off control (lasers)
    /// Corresponds to [`crate::capabilities::EmissionControl`]
    EmissionControl,

    /// Can execute structured JSON commands
    /// Corresponds to [`crate::capabilities::Commandable`]
    Commandable,

    /// Can be staged/unstaged for acquisition sequences (Bluesky pattern)
    /// Corresponds to [`crate::capabilities::Stageable`]
    Stageable,

    /// Has observable parameters with subscriptions
    /// Corresponds to [`crate::capabilities::Parameterized`]
    Parameterized,

    /// Gated camera with DDG and MCP control (ICCD)
    /// Corresponds to [`crate::capabilities::GatedCamera`]
    GatedCamera,

    /// Spectrometer with grating and wavelength control
    /// Corresponds to [`crate::capabilities::SpectrometerControl`]
    SpectrometerControl,

    /// Motion stage with position-based triggering
    /// Corresponds to [`crate::capabilities::TriggerOnPosition`]
    TriggerOnPosition,

    /// Pulse generator with configurable timing
    /// Corresponds to [`crate::capabilities::PulseGenerator`]
    PulseGenerator,

    /// Safety interlock monitoring
    /// Corresponds to [`crate::capabilities::SafetyInterlock`]
    SafetyInterlock,

    /// Runtime reconfiguration without full restart
    /// Corresponds to [`crate::capabilities::Reconfigurable`]
    Reconfigurable,

    /// Post-reconnection state refresh (bd-47p2)
    /// Corresponds to [`crate::capabilities::StateRefreshable`]
    StateRefreshable,

    /// Configurable counter/timer (Comedi INSN_CONFIG, bd-f3pq)
    /// Corresponds to [`crate::capabilities::CounterConfigurable`]
    CounterConfigurable,

    /// Can report available analog voltage/current ranges (bd-3bjp)
    /// Corresponds to [`crate::capabilities::RangeIntrospectable`]
    RangeIntrospectable,

    /// Can report board/subdevice metadata (bd-sa9p)
    /// Corresponds to [`crate::capabilities::DeviceIntrospection`]
    DeviceIntrospection,

    /// Reads with raw value, voltage, and timestamp metadata (bd-09ls)
    /// Corresponds to [`crate::capabilities::ReadableWithMetadata`]
    ReadableWithMetadata,
}

impl Capability {
    /// Human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            Self::Movable => "Movable",
            Self::Readable => "Readable",
            Self::Triggerable => "Triggerable",
            Self::FrameProducer => "Frame Producer",
            Self::ExposureControl => "Exposure Control",
            Self::Settable => "Settable",
            Self::ShutterControl => "Shutter Control",
            Self::WavelengthTunable => "Wavelength Tunable",
            Self::EmissionControl => "Emission Control",
            Self::Commandable => "Commandable",
            Self::Stageable => "Stageable",
            Self::Parameterized => "Parameterized",
            Self::GatedCamera => "Gated Camera",
            Self::SpectrometerControl => "Spectrometer Control",
            Self::TriggerOnPosition => "Trigger On Position",
            Self::PulseGenerator => "Pulse Generator",
            Self::SafetyInterlock => "Safety Interlock",
            Self::Reconfigurable => "Reconfigurable",
            Self::StateRefreshable => "State Refreshable",
            Self::CounterConfigurable => "Counter Configurable",
            Self::RangeIntrospectable => "Range Introspectable",
            Self::DeviceIntrospection => "Device Introspection",
            Self::ReadableWithMetadata => "Readable With Metadata",
        }
    }

    /// Snake_case string for proto/API transport (bd-4myc.3)
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Movable => "movable",
            Self::Readable => "readable",
            Self::Triggerable => "triggerable",
            Self::FrameProducer => "frame_producer",
            Self::ExposureControl => "exposure_controllable",
            Self::Settable => "settable",
            Self::ShutterControl => "shutter_controllable",
            Self::WavelengthTunable => "wavelength_tunable",
            Self::EmissionControl => "emission_controllable",
            Self::Commandable => "commandable",
            Self::Stageable => "stageable",
            Self::Parameterized => "parameterized",
            Self::GatedCamera => "gated_camera",
            Self::SpectrometerControl => "spectrometer_control",
            Self::TriggerOnPosition => "trigger_on_position",
            Self::PulseGenerator => "pulse_generator",
            Self::SafetyInterlock => "safety_interlock",
            Self::Reconfigurable => "reconfigurable",
            Self::StateRefreshable => "state_refreshable",
            Self::CounterConfigurable => "counter_configurable",
            Self::RangeIntrospectable => "range_introspectable",
            Self::DeviceIntrospection => "device_introspection",
            Self::ReadableWithMetadata => "readable_with_metadata",
        }
    }
}

// =============================================================================
// Device Components (Capability Bag)
// =============================================================================

/// Container for capability trait objects returned by drivers.
///
/// When a driver is instantiated, it returns a `DeviceComponents` struct
/// containing all the capabilities it implements. The registry then uses
/// these to populate its internal maps for capability-based lookups.
///
/// # Builder Pattern
///
/// Use the builder methods to construct a `DeviceComponents`:
///
/// ```rust,ignore
/// let driver = Arc::new(MyDriver::new().await?);
///
/// let components = DeviceComponents::new()
///     .with_movable(driver.clone())
///     .with_readable(driver.clone())
///     .with_parameterized(driver);
/// ```
///
/// # Why Not a Single `Arc<dyn Driver>`?
///
/// By storing each capability separately, we:
/// 1. Avoid runtime downcasting (no `Any` bounds)
/// 2. Enable compile-time type safety for capability access
/// 3. Allow drivers to implement only the capabilities they need
/// 4. Support drivers that use different objects for different capabilities
#[derive(Default)]
pub struct DeviceComponents {
    /// Device category for UI grouping
    pub category: Option<DeviceCategory>,

    /// Movable implementation (motion control)
    pub movable: Option<Arc<dyn Movable>>,

    /// Readable implementation (scalar measurements)
    pub readable: Option<Arc<dyn Readable>>,

    /// Triggerable implementation (arm/trigger/disarm)
    pub triggerable: Option<Arc<dyn Triggerable>>,

    /// FrameProducer implementation (camera streaming)
    pub frame_producer: Option<Arc<dyn FrameProducer>>,

    /// MeasurementSource for frames (Bluesky-style)
    pub source_frame:
        Option<Arc<dyn MeasurementSource<Output = Arc<Frame>, Error = anyhow::Error>>>,

    /// ExposureControl implementation (exposure time)
    pub exposure_control: Option<Arc<dyn ExposureControl>>,

    /// Settable implementation (observable parameters)
    pub settable: Option<Arc<dyn Settable>>,

    /// Stageable implementation (Bluesky lifecycle)
    pub stageable: Option<Arc<dyn Stageable>>,

    /// Commandable implementation (structured commands)
    pub commandable: Option<Arc<dyn Commandable>>,

    /// Parameterized implementation (parameter registry)
    pub parameterized: Option<Arc<dyn Parameterized>>,

    /// ShutterControl implementation (laser shutter)
    pub shutter_control: Option<Arc<dyn ShutterControl>>,

    /// EmissionControl implementation (laser on/off)
    pub emission_control: Option<Arc<dyn EmissionControl>>,

    /// WavelengthTunable implementation (tunable wavelength)
    pub wavelength_tunable: Option<Arc<dyn WavelengthTunable>>,

    /// GatedCamera implementation (ICCD with DDG and MCP)
    pub gated_camera: Option<Arc<dyn GatedCamera>>,

    /// SpectrometerControl implementation (grating and wavelength control)
    pub spectrometer_control: Option<Arc<dyn SpectrometerControl>>,

    /// TriggerOnPosition implementation (position-based triggering)
    pub trigger_on_position: Option<Arc<dyn TriggerOnPosition>>,

    /// PulseGenerator implementation (pulse train generation)
    pub pulse_generator: Option<Arc<dyn PulseGenerator>>,

    /// SafetyInterlock implementation (interlock monitoring)
    pub safety_interlock: Option<Arc<dyn SafetyInterlock>>,

    /// Reconfigurable implementation (runtime config changes)
    pub reconfigurable: Option<Arc<dyn Reconfigurable>>,

    /// StateRefreshable implementation (post-reconnection state refresh, bd-47p2)
    pub state_refreshable: Option<Arc<dyn StateRefreshable>>,

    /// CounterConfigurable implementation (DAQ counter/timer config, bd-f3pq)
    pub counter_configurable: Option<Arc<dyn CounterConfigurable>>,

    /// RangeIntrospectable implementation (analog range queries, bd-3bjp)
    pub range_introspectable: Option<Arc<dyn RangeIntrospectable>>,

    /// DeviceIntrospection implementation (board/subdevice metadata, bd-sa9p)
    pub device_introspection: Option<Arc<dyn DeviceIntrospection>>,

    /// ReadableWithMetadata implementation (structured analog reads, bd-09ls)
    pub readable_with_metadata: Option<Arc<dyn ReadableWithMetadata>>,

    /// Optional lifecycle hooks for device registration/shutdown
    pub lifecycle: Option<Arc<dyn DeviceLifecycle>>,

    /// Capability-specific metadata (units, ranges, etc.)
    pub metadata: DeviceMetadata,
}

impl DeviceComponents {
    /// Create a new empty DeviceComponents
    pub fn new() -> Self {
        Self::default()
    }

    /// Get list of capabilities this device supports
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut caps = Vec::new();

        if self.movable.is_some() {
            caps.push(Capability::Movable);
        }
        if self.readable.is_some() {
            caps.push(Capability::Readable);
        }
        if self.triggerable.is_some() {
            caps.push(Capability::Triggerable);
        }
        if self.frame_producer.is_some() {
            caps.push(Capability::FrameProducer);
        }
        if self.exposure_control.is_some() {
            caps.push(Capability::ExposureControl);
        }
        if self.settable.is_some() {
            caps.push(Capability::Settable);
        }
        if self.shutter_control.is_some() {
            caps.push(Capability::ShutterControl);
        }
        if self.wavelength_tunable.is_some() {
            caps.push(Capability::WavelengthTunable);
        }
        if self.emission_control.is_some() {
            caps.push(Capability::EmissionControl);
        }
        if self.commandable.is_some() {
            caps.push(Capability::Commandable);
        }
        if self.stageable.is_some() {
            caps.push(Capability::Stageable);
        }
        if self.parameterized.is_some() {
            caps.push(Capability::Parameterized);
        }
        if self.gated_camera.is_some() {
            caps.push(Capability::GatedCamera);
        }
        if self.spectrometer_control.is_some() {
            caps.push(Capability::SpectrometerControl);
        }
        if self.trigger_on_position.is_some() {
            caps.push(Capability::TriggerOnPosition);
        }
        if self.pulse_generator.is_some() {
            caps.push(Capability::PulseGenerator);
        }
        if self.safety_interlock.is_some() {
            caps.push(Capability::SafetyInterlock);
        }
        if self.state_refreshable.is_some() {
            caps.push(Capability::StateRefreshable);
        }
        if self.counter_configurable.is_some() {
            caps.push(Capability::CounterConfigurable);
        }
        if self.range_introspectable.is_some() {
            caps.push(Capability::RangeIntrospectable);
        }
        if self.device_introspection.is_some() {
            caps.push(Capability::DeviceIntrospection);
        }
        if self.readable_with_metadata.is_some() {
            caps.push(Capability::ReadableWithMetadata);
        }

        caps
    }

    // Builder methods

    /// Set device category
    pub fn with_category(mut self, category: DeviceCategory) -> Self {
        self.category = Some(category);
        self.metadata.category = Some(category);
        self
    }

    /// Set Movable implementation
    pub fn with_movable(mut self, m: Arc<dyn Movable>) -> Self {
        self.movable = Some(m);
        self
    }

    /// Set Readable implementation
    pub fn with_readable(mut self, r: Arc<dyn Readable>) -> Self {
        self.readable = Some(r);
        self
    }

    /// Set Triggerable implementation
    pub fn with_triggerable(mut self, t: Arc<dyn Triggerable>) -> Self {
        self.triggerable = Some(t);
        self
    }

    /// Set FrameProducer implementation
    pub fn with_frame_producer(mut self, f: Arc<dyn FrameProducer>) -> Self {
        self.frame_producer = Some(f);
        self
    }

    /// Set MeasurementSource<Frame> implementation
    pub fn with_source_frame(
        mut self,
        s: Arc<dyn MeasurementSource<Output = Arc<Frame>, Error = anyhow::Error>>,
    ) -> Self {
        self.source_frame = Some(s);
        self
    }

    /// Set ExposureControl implementation
    pub fn with_exposure_control(mut self, e: Arc<dyn ExposureControl>) -> Self {
        self.exposure_control = Some(e);
        self
    }

    /// Set Settable implementation
    pub fn with_settable(mut self, s: Arc<dyn Settable>) -> Self {
        self.settable = Some(s);
        self
    }

    /// Set Stageable implementation
    pub fn with_stageable(mut self, s: Arc<dyn Stageable>) -> Self {
        self.stageable = Some(s);
        self
    }

    /// Set Commandable implementation
    pub fn with_commandable(mut self, c: Arc<dyn Commandable>) -> Self {
        self.commandable = Some(c);
        self
    }

    /// Set Parameterized implementation
    pub fn with_parameterized(mut self, p: Arc<dyn Parameterized>) -> Self {
        self.parameterized = Some(p);
        self
    }

    /// Set ShutterControl implementation
    pub fn with_shutter_control(mut self, s: Arc<dyn ShutterControl>) -> Self {
        self.shutter_control = Some(s);
        self
    }

    /// Set EmissionControl implementation
    pub fn with_emission_control(mut self, e: Arc<dyn EmissionControl>) -> Self {
        self.emission_control = Some(e);
        self
    }

    /// Set WavelengthTunable implementation
    pub fn with_wavelength_tunable(mut self, w: Arc<dyn WavelengthTunable>) -> Self {
        self.wavelength_tunable = Some(w);
        self
    }

    /// Set GatedCamera implementation
    pub fn with_gated_camera(mut self, g: Arc<dyn GatedCamera>) -> Self {
        self.gated_camera = Some(g);
        self
    }

    /// Set SpectrometerControl implementation
    pub fn with_spectrometer_control(mut self, s: Arc<dyn SpectrometerControl>) -> Self {
        self.spectrometer_control = Some(s);
        self
    }

    /// Set TriggerOnPosition implementation
    pub fn with_trigger_on_position(mut self, t: Arc<dyn TriggerOnPosition>) -> Self {
        self.trigger_on_position = Some(t);
        self
    }

    /// Set PulseGenerator implementation
    pub fn with_pulse_generator(mut self, p: Arc<dyn PulseGenerator>) -> Self {
        self.pulse_generator = Some(p);
        self
    }

    /// Set SafetyInterlock implementation
    pub fn with_safety_interlock(mut self, s: Arc<dyn SafetyInterlock>) -> Self {
        self.safety_interlock = Some(s);
        self
    }

    /// Set Reconfigurable implementation
    pub fn with_reconfigurable(mut self, r: Arc<dyn Reconfigurable>) -> Self {
        self.reconfigurable = Some(r);
        self
    }

    /// Set StateRefreshable implementation (bd-47p2)
    pub fn with_state_refreshable(mut self, s: Arc<dyn StateRefreshable>) -> Self {
        self.state_refreshable = Some(s);
        self
    }

    /// Set CounterConfigurable implementation (bd-f3pq)
    pub fn with_counter_configurable(mut self, c: Arc<dyn CounterConfigurable>) -> Self {
        self.counter_configurable = Some(c);
        self
    }

    /// Set RangeIntrospectable implementation (bd-3bjp)
    pub fn with_range_introspectable(mut self, r: Arc<dyn RangeIntrospectable>) -> Self {
        self.range_introspectable = Some(r);
        self
    }

    /// Set DeviceIntrospection implementation (bd-sa9p)
    pub fn with_device_introspection(mut self, d: Arc<dyn DeviceIntrospection>) -> Self {
        self.device_introspection = Some(d);
        self
    }

    /// Set ReadableWithMetadata implementation (bd-09ls)
    pub fn with_readable_with_metadata(mut self, r: Arc<dyn ReadableWithMetadata>) -> Self {
        self.readable_with_metadata = Some(r);
        self
    }

    /// Set device lifecycle hooks
    pub fn with_lifecycle(mut self, lifecycle: Arc<dyn DeviceLifecycle>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// Set device metadata
    pub fn with_metadata(mut self, metadata: DeviceMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

// =============================================================================
// Device Metadata
// =============================================================================

/// Capability-specific metadata for a device.
///
/// This struct holds additional information about device capabilities
/// that isn't captured in the trait objects themselves (units, ranges, etc.).
#[derive(Debug, Clone, Default)]
pub struct DeviceMetadata {
    /// Device category for UI grouping
    pub category: Option<DeviceCategory>,

    /// For Movable devices: position units (e.g., "mm", "degrees")
    pub position_units: Option<String>,

    /// For Movable devices: minimum position
    pub min_position: Option<f64>,

    /// For Movable devices: maximum position
    pub max_position: Option<f64>,

    /// For Readable devices: measurement units (e.g., "W", "V")
    pub measurement_units: Option<String>,

    /// For FrameProducer devices: frame width in pixels
    pub frame_width: Option<u32>,

    /// For FrameProducer devices: frame height in pixels
    pub frame_height: Option<u32>,

    /// For FrameProducer devices: bits per pixel (e.g., 8, 12, 16)
    pub bits_per_pixel: Option<u32>,

    /// For ExposureControl devices: minimum exposure in milliseconds
    pub min_exposure_ms: Option<f64>,

    /// For ExposureControl devices: maximum exposure in milliseconds
    pub max_exposure_ms: Option<f64>,

    /// For WavelengthTunable devices: minimum wavelength in nm
    pub min_wavelength_nm: Option<f64>,

    /// For WavelengthTunable devices: maximum wavelength in nm
    pub max_wavelength_nm: Option<f64>,

    /// Available command names for command-capable devices.
    ///
    /// For universal TOML drivers, this is populated from manifest commands.
    pub available_commands: Vec<String>,

    /// Optional serialized UI schema/configuration for metadata-driven control panels.
    ///
    /// For universal TOML drivers this is sourced from the `[ui]` manifest section.
    pub ui_schema_json: Option<String>,

    /// Explicit panel routing hint for the UI.
    ///
    /// When set, the UI uses this field to route the device to a specific control
    /// panel without any heuristic guessing. Use the constants from [`common::panel_kind`].
    /// When absent, the UI falls back to capability-based dispatch.
    pub panel_kind: Option<String>,

    /// Manifest-derived parameter/feature metadata for DB persistence.
    ///
    /// For universal TOML drivers, extracted from `[parameters]` section.
    /// Each entry describes a device parameter with type, range, unit, etc.
    /// The reconciler uses this as a fallback when `Parameterized` is not
    /// available (i.e., for manifest-driven devices).
    pub manifest_features: Vec<ManifestFeatureMeta>,
}

/// Static feature metadata derived from a device manifest.
///
/// Crate-agnostic equivalent of `driver_universal::ManifestParameterMeta`,
/// carried through `DeviceMetadata` for reconciler → DB persistence.
#[derive(Debug, Clone, Default)]
pub struct ManifestFeatureMeta {
    /// Feature name (e.g., "position_deg", "move_absolute").
    pub name: String,
    /// Data type: "float", "int", "bool", "string", "enum", "command".
    pub feature_type: String,
    /// Whether this feature can be read.
    pub readable: bool,
    /// Whether this feature can be written.
    pub writable: bool,
    /// Minimum value for numeric features.
    pub min_value: Option<f64>,
    /// Maximum value for numeric features.
    pub max_value: Option<f64>,
    /// Physical unit (e.g., "degrees", "nm").
    pub unit: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
}

// =============================================================================
// Device Lifecycle Hooks
// =============================================================================

/// Optional lifecycle hooks for devices that need explicit startup/shutdown.
///
/// Implementations should be idempotent and safe to call during partial failures.
pub trait DeviceLifecycle: Send + Sync + 'static {
    /// Called after a device is successfully instantiated but before it is exposed.
    fn on_register(&self) -> BoxFuture<'static, Result<()>> {
        Box::pin(async { Ok(()) })
    }

    /// Called when a device is unregistered or during shutdown cleanup.
    fn on_unregister(&self) -> BoxFuture<'static, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

// =============================================================================
// Driver Factory Trait
// =============================================================================

/// Trait for driver factories that create device instances.
///
/// Each driver crate implements this trait to register itself with the
/// DeviceRegistry. The factory is responsible for:
///
/// 1. Declaring what driver type it handles (matching TOML `type` field)
/// 2. Validating configuration before instantiation
/// 3. Asynchronously creating the driver and returning capabilities
///
/// # Lifetime
///
/// Factories are registered once at startup and live for the program's lifetime.
/// They must be `Send + Sync + 'static` because they may be called from any task.
///
/// # Thread Safety
///
/// The `build()` method takes `&self` and returns a `BoxFuture<'static, ...>`.
/// This means:
/// - The factory must not hold mutable state across builds
/// - If shared state is needed (e.g., shared serial ports), use internal synchronization
///
/// # Error Handling
///
/// Both `validate()` and `build()` return `Result`. Validation errors should be
/// descriptive and actionable. Build errors may include hardware connection failures.
/// Current API version for the `DriverFactory` trait.
///
/// Bump this when making breaking changes to `DriverFactory`. Factories
/// built against an older API version will log a warning at registration.
pub const DRIVER_FACTORY_API_VERSION: u32 = 1;

pub trait DriverFactory: Send + Sync + 'static {
    /// Returns the plugin API version this factory was built against.
    ///
    /// Defaults to the current version. Override only if your factory was
    /// compiled against a specific older version and you want to signal that.
    fn api_version(&self) -> u32 {
        DRIVER_FACTORY_API_VERSION
    }

    /// Driver type name used in TOML config `type` field.
    ///
    /// This must match exactly what users write in their config:
    /// ```toml
    /// [devices.driver]
    /// type = "ell14"  # matches driver_type() returning "ell14"
    /// ```
    fn driver_type(&self) -> &'static str;

    /// Human-readable name for documentation and error messages.
    ///
    /// Example: "Thorlabs ELL14 Rotation Mount"
    fn name(&self) -> &'static str;

    /// List of capabilities this driver type provides.
    ///
    /// Used for introspection and documentation. The actual capabilities
    /// are determined by what's returned in `DeviceComponents::build()`.
    fn capabilities(&self) -> &'static [Capability] {
        &[]
    }

    /// List available command names exposed by this driver type.
    ///
    /// Factories that do not support command introspection can return an empty
    /// list. This is used for metadata publication (DB/UI), not execution.
    fn available_commands(&self) -> Vec<String> {
        Vec::new()
    }

    /// Validate configuration without instantiating.
    ///
    /// Called before `build()` to provide early error feedback.
    /// Should check that all required fields exist and have valid types.
    ///
    /// # Arguments
    ///
    /// * `config` - TOML value containing driver configuration (the `[devices.driver]` section)
    ///
    /// # Returns
    ///
    /// - `Ok(())` if configuration is valid
    /// - `Err` with descriptive message if validation fails
    fn validate(&self, config: &toml::Value) -> Result<()>;

    /// Async instantiation of the driver.
    ///
    /// This method is called after validation passes. It should:
    /// 1. Parse the configuration
    /// 2. Open connections to hardware (serial ports, USB, etc.)
    /// 3. Optionally validate device identity (query version strings)
    /// 4. Return DeviceComponents with all implemented capabilities
    ///
    /// # Blocking I/O
    ///
    /// If your implementation performs synchronous / blocking I/O (e.g.,
    /// opening serial ports via `serialport::new(...)`, USB initialization,
    /// or SCPI socket connects), wrap those calls in
    /// [`tokio::task::spawn_blocking`] to avoid stalling the async runtime.
    /// The reconciler already spawns `build()` on a separate task as a safety
    /// net, but factory implementations should not rely on that.
    ///
    /// # Arguments
    ///
    /// * `config` - TOML value containing driver configuration
    ///
    /// # Returns
    ///
    /// - `Ok(DeviceComponents)` with populated capability trait objects
    /// - `Err` if driver fails to initialize (port not found, wrong device, etc.)
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_name() {
        assert_eq!(Capability::Movable.name(), "Movable");
        assert_eq!(Capability::FrameProducer.name(), "Frame Producer");
    }

    #[test]
    fn test_device_components_builder() {
        let components = DeviceComponents::new()
            .with_category(DeviceCategory::Stage)
            .with_metadata(DeviceMetadata {
                position_units: Some("mm".to_string()),
                min_position: Some(-100.0),
                max_position: Some(100.0),
                ..Default::default()
            });

        assert_eq!(components.category, Some(DeviceCategory::Stage));
        assert_eq!(components.metadata.position_units, Some("mm".to_string()));
    }

    #[test]
    fn test_device_components_capabilities() {
        // Empty components should have no capabilities
        let empty = DeviceComponents::new();
        assert!(empty.capabilities().is_empty());
    }

    #[test]
    fn test_capability_serde() {
        // Test serialization
        let cap = Capability::Movable;
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, "\"movable\"");

        // Test deserialization
        let cap: Capability = serde_json::from_str("\"frame_producer\"").unwrap();
        assert_eq!(cap, Capability::FrameProducer);
    }
}
