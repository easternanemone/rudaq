//! Plugin metadata with StableAbi derives.

use abi_stable::std_types::{RString, RVec};
use abi_stable::StableAbi;

/// ABI-stable plugin metadata.
///
/// This struct contains all information needed to identify and validate a plugin
/// before loading its modules.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct PluginMetadata {
    /// Unique plugin identifier (e.g., "com.example.power-monitor")
    pub plugin_id: RString,

    /// Human-readable plugin name
    pub name: RString,

    /// Plugin version (semver format)
    pub version: RString,

    /// Plugin author
    pub author: RString,

    /// Plugin description
    pub description: RString,

    /// Minimum compatible rust-daq version
    pub min_daq_version: RString,

    /// List of module type IDs this plugin provides
    pub module_types: RVec<RString>,

    /// Plugin dependencies (other plugin IDs)
    pub dependencies: RVec<PluginDependency>,
}

impl PluginMetadata {
    /// Create new plugin metadata with required fields
    pub fn new(plugin_id: &str, name: &str, version: &str) -> Self {
        Self {
            plugin_id: RString::from(plugin_id),
            name: RString::from(name),
            version: RString::from(version),
            author: RString::new(),
            description: RString::new(),
            min_daq_version: RString::from("0.1.0"),
            module_types: RVec::new(),
            dependencies: RVec::new(),
        }
    }

    /// Builder method to set author
    pub fn with_author(mut self, author: &str) -> Self {
        self.author = RString::from(author);
        self
    }

    /// Builder method to set description
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = RString::from(description);
        self
    }

    /// Builder method to add a module type
    pub fn with_module_type(mut self, type_id: &str) -> Self {
        self.module_types.push(RString::from(type_id));
        self
    }

    /// Builder method to set minimum DAQ version
    pub fn with_min_daq_version(mut self, version: &str) -> Self {
        self.min_daq_version = RString::from(version);
        self
    }

    /// Builder method to add a dependency
    pub fn with_dependency(mut self, plugin_id: &str, version_req: &str) -> Self {
        self.dependencies.push(PluginDependency {
            plugin_id: RString::from(plugin_id),
            version_requirement: RString::from(version_req),
        });
        self
    }
}

/// A plugin dependency specification
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct PluginDependency {
    /// The plugin ID this depends on
    pub plugin_id: RString,

    /// Semver version requirement (e.g., "^1.0", ">=2.0.0")
    pub version_requirement: RString,
}

/// Version information for ABI compatibility checking
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
pub struct AbiVersion {
    /// Major version - breaking changes
    pub major: u32,
    /// Minor version - backwards-compatible additions
    pub minor: u32,
    /// Patch version - bug fixes
    pub patch: u32,
}

impl AbiVersion {
    /// Current ABI version
    pub const CURRENT: Self = Self {
        major: 0,
        minor: 1,
        patch: 0,
    };

    /// Check if this version is compatible with another
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        // Major version must match, minor must be >= required
        self.major == other.major && self.minor >= other.minor
    }
}

impl std::fmt::Display for AbiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
