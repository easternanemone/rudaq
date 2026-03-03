//! Runtime feature flag configuration.
//!
//! Loads feature toggles from a TOML configuration file (`config/feature_flags.toml`)
//! and provides typed access to known flags with fallback to a dynamic lookup.
//!
//! # Usage
//!
//! ```rust,ignore
//! let flags = FeatureFlags::load("config/feature_flags.toml")?;
//! if flags.optimized_streaming {
//!     // Enable all performance optimizations
//! }
//! if flags.is_enabled("experimental_streaming") {
//!     // Check any flag by name
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// Runtime feature flags loaded from `config/feature_flags.toml`.
///
/// Well-known flags are deserialized into typed fields so call-sites get
/// compile-time checking.  Unknown / ad-hoc flags are still accessible
/// through [`is_enabled`](Self::is_enabled).
#[derive(Debug, Clone, Deserialize)]
pub struct FeatureFlags {
    /// Default value for flags not explicitly defined in the TOML file.
    #[serde(default)]
    pub default_enabled: bool,

    /// The raw flag map.  Typed fields below are derived from this after
    /// deserialization via [`resolve`](Self::resolve).
    #[serde(default)]
    flags: HashMap<String, toml::Value>,

    // ── Performance flags ──────────────────────────────────────────────
    /// Master toggle for performance optimizations.  When **false**, the
    /// system falls back to conservative (non-optimized) code paths, which
    /// is useful for A/B testing and quick rollback.
    ///
    /// Defaults to `true`.
    #[serde(skip)]
    pub optimized_streaming: bool,

    /// Pre-allocate frame pool buffers at startup rather than on first use.
    #[serde(skip)]
    pub frame_pool_preallocation: bool,

    /// Use the async ring-buffer wrapper for storage writes.
    #[serde(skip)]
    pub async_ring_buffer: bool,

    // ── Experimental flags ─────────────────────────────────────────────
    /// Experimental streaming pipeline (disabled by default).
    #[serde(skip)]
    pub experimental_streaming: bool,

    /// Experimental frame compression (disabled by default).
    #[serde(skip)]
    pub experimental_frame_compression: bool,

    // ── Debug flags ────────────────────────────────────────────────────
    /// Emit per-frame timing diagnostics.
    #[serde(skip)]
    pub debug_frame_timing: bool,

    /// Log raw serial command bytes.
    #[serde(skip)]
    pub debug_serial_commands: bool,

    /// Verbose gRPC request/response logging.
    #[serde(skip)]
    pub verbose_grpc_logging: bool,

    // ── Hardware compatibility ──────────────────────────────────────────
    /// Use legacy PVCAM initialisation sequence.
    #[serde(skip)]
    pub legacy_pvcam_mode: bool,

    /// Strictly validate serial device responses.
    #[serde(skip)]
    pub strict_serial_validation: bool,

    // ── GUI flags ──────────────────────────────────────────────────────
    /// Default to dark mode in the web UI.
    #[serde(skip)]
    pub dark_mode_default: bool,

    /// Show developer debug panels in the UI.
    #[serde(skip)]
    pub show_debug_panels: bool,
}

impl FeatureFlags {
    /// Load flags from a TOML file and resolve typed fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("reading feature flags from {}", path.as_ref().display()))?;
        let mut flags: Self =
            toml::from_str(&content).with_context(|| "parsing feature_flags.toml")?;
        flags.resolve();
        Ok(flags)
    }

    /// Load from a TOML string (useful for tests).
    ///
    /// # Errors
    ///
    /// Returns an error if parsing fails.
    pub fn from_str(toml_str: &str) -> anyhow::Result<Self> {
        let mut flags: Self =
            toml::from_str(toml_str).with_context(|| "parsing feature flags TOML string")?;
        flags.resolve();
        Ok(flags)
    }

    /// Populate the typed fields from the raw `flags` map.
    ///
    /// Performance flags (`frame_pool_preallocation`, `async_ring_buffer`)
    /// are gated behind `optimized_streaming` -- when the master toggle is
    /// off they are forced to `false` regardless of their individual values
    /// in the configuration file.
    fn resolve(&mut self) {
        let default = self.default_enabled;

        // Master performance toggle
        self.optimized_streaming = self.flag_bool("optimized_streaming", default);

        // Performance flags -- gated by optimized_streaming
        self.frame_pool_preallocation =
            self.optimized_streaming && self.flag_bool("frame_pool_preallocation", default);
        self.async_ring_buffer =
            self.optimized_streaming && self.flag_bool("async_ring_buffer", default);

        // Experimental
        self.experimental_streaming = self.flag_bool("experimental_streaming", default);
        self.experimental_frame_compression =
            self.flag_bool("experimental_frame_compression", default);

        // Debug
        self.debug_frame_timing = self.flag_bool("debug_frame_timing", default);
        self.debug_serial_commands = self.flag_bool("debug_serial_commands", default);
        self.verbose_grpc_logging = self.flag_bool("verbose_grpc_logging", default);

        // Hardware
        self.legacy_pvcam_mode = self.flag_bool("legacy_pvcam_mode", default);
        self.strict_serial_validation = self.flag_bool("strict_serial_validation", default);

        // GUI
        self.dark_mode_default = self.flag_bool("dark_mode_default", default);
        self.show_debug_panels = self.flag_bool("show_debug_panels", default);
    }

    /// Look up a boolean flag by name, falling back to `default`.
    fn flag_bool(&self, name: &str, default: bool) -> bool {
        self.flags
            .get(name)
            .and_then(toml::Value::as_bool)
            .unwrap_or(default)
    }

    /// Check whether an arbitrary flag is enabled (by name).
    ///
    /// This is the dynamic counterpart of the typed fields and is handy
    /// for flags that are not (yet) promoted to struct fields.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.flag_bool(name, self.default_enabled)
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            default_enabled: false,
            flags: HashMap::new(),
            optimized_streaming: true,
            frame_pool_preallocation: true,
            async_ring_buffer: true,
            experimental_streaming: false,
            experimental_frame_compression: false,
            debug_frame_timing: false,
            debug_serial_commands: false,
            verbose_grpc_logging: false,
            legacy_pvcam_mode: false,
            strict_serial_validation: true,
            dark_mode_default: false,
            show_debug_panels: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_from_string() {
        let toml = r#"
default_enabled = false

[flags]
optimized_streaming = true
frame_pool_preallocation = true
async_ring_buffer = true
experimental_streaming = false
"#;
        let flags = FeatureFlags::from_str(toml).expect("parse failed");
        assert!(flags.optimized_streaming);
        assert!(flags.frame_pool_preallocation);
        assert!(flags.async_ring_buffer);
        assert!(!flags.experimental_streaming);
    }

    #[test]
    fn test_optimized_streaming_gates_performance_flags() {
        let toml = r#"
default_enabled = false

[flags]
optimized_streaming = false
frame_pool_preallocation = true
async_ring_buffer = true
"#;
        let flags = FeatureFlags::from_str(toml).expect("parse failed");
        assert!(!flags.optimized_streaming);
        // Even though individually enabled, the master toggle forces them off.
        assert!(!flags.frame_pool_preallocation);
        assert!(!flags.async_ring_buffer);
    }

    #[test]
    fn test_default_flags() {
        let flags = FeatureFlags::default();
        assert!(flags.optimized_streaming);
        assert!(flags.frame_pool_preallocation);
        assert!(flags.async_ring_buffer);
        assert!(!flags.experimental_streaming);
        assert!(!flags.debug_frame_timing);
    }

    #[test]
    fn test_is_enabled_dynamic() {
        let toml = r#"
default_enabled = false

[flags]
my_custom_flag = true
"#;
        let flags = FeatureFlags::from_str(toml).expect("parse failed");
        assert!(flags.is_enabled("my_custom_flag"));
        assert!(!flags.is_enabled("nonexistent_flag"));
    }

    #[test]
    fn test_default_enabled_fallback() {
        let toml = r#"
default_enabled = true

[flags]
"#;
        let flags = FeatureFlags::from_str(toml).expect("parse failed");
        // Flags not in the map should use default_enabled = true
        assert!(flags.is_enabled("anything_not_defined"));
        assert!(flags.optimized_streaming);
    }
}
