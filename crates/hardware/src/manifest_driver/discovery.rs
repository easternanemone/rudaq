//! Plugin discovery and registry system.
//!
//! This module provides directory scanning for plugin discovery and a
//! versioned registry for managing loaded plugins.
//!
//! # Architecture
//!
//! - `PluginScanner` - Scans directories for plugin.toml manifests
//! - `PluginRegistry` - Central registry with versioning and dependency resolution
//! - `PluginInfo` - Loaded plugin metadata with source tracking
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_hardware::plugin::discovery::{PluginScanner, PluginRegistry};
//!
//! let mut registry = PluginRegistry::new();
//!
//! // Add search paths (user overrides system)
//! registry.add_search_path("~/.config/rust-daq/plugins/");
//! registry.add_search_path("/usr/share/rust-daq/plugins/");
//!
//! // Scan and discover plugins
//! let errors = registry.scan().await;
//! for err in &errors {
//!     eprintln!("Warning: {}", err);
//! }
//!
//! // List discovered plugins
//! for info in registry.list() {
//!     println!("{} v{} from {}", info.manifest.plugin.name,
//!              info.manifest.plugin.version, info.source_path.display());
//! }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::manifest::{ManifestError, PluginManifest, PluginType};

// =============================================================================
// Version Comparison
// =============================================================================

/// Simple semver version for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl Version {
    /// Parses a semver version string (e.g., "1.2.3", "1.2.3-beta.1").
    pub fn parse(s: &str) -> Option<Self> {
        let (version_part, prerelease) = if let Some(idx) = s.find('-') {
            (&s[..idx], Some(s[idx + 1..].to_string()))
        } else {
            (s, None)
        };

        let parts: Vec<&str> = version_part.split('.').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return None;
        }

        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);

        Some(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }
}

impl std::cmp::Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        match self.minor.cmp(&other.minor) {
            std::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        match self.patch.cmp(&other.patch) {
            std::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        // Prerelease versions are less than release versions
        match (&self.prerelease, &other.prerelease) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

impl std::cmp::PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(ref pre) = self.prerelease {
            write!(f, "-{}", pre)?;
        }
        Ok(())
    }
}

/// Version requirement for dependency resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionReq {
    pub comparator: Comparator,
    pub version: Version,
}

/// Comparison operator for version requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparator {
    /// Exact match (=1.2.3 or 1.2.3)
    Exact,
    /// Greater than (>1.2.3)
    Greater,
    /// Greater than or equal (>=1.2.3)
    GreaterEq,
    /// Less than (<1.2.3)
    Less,
    /// Less than or equal (<=1.2.3)
    LessEq,
    /// Compatible (^1.2.3) - same major, greater or equal minor/patch
    Caret,
    /// Approximately (~1.2.3) - same major and minor, any patch
    Tilde,
}

impl VersionReq {
    /// Parses a version requirement string (e.g., ">=1.0.0", "^1.2.3", "~1.2").
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();

        let (comparator, version_str) = if let Some(rest) = s.strip_prefix(">=") {
            (Comparator::GreaterEq, rest)
        } else if let Some(rest) = s.strip_prefix("<=") {
            (Comparator::LessEq, rest)
        } else if let Some(rest) = s.strip_prefix('>') {
            (Comparator::Greater, rest)
        } else if let Some(rest) = s.strip_prefix('<') {
            (Comparator::Less, rest)
        } else if let Some(rest) = s.strip_prefix('^') {
            (Comparator::Caret, rest)
        } else if let Some(rest) = s.strip_prefix('~') {
            (Comparator::Tilde, rest)
        } else if let Some(rest) = s.strip_prefix('=') {
            (Comparator::Exact, rest)
        } else {
            (Comparator::Exact, s)
        };

        let version = Version::parse(version_str.trim())?;
        Some(Self {
            comparator,
            version,
        })
    }

    /// Checks if a version satisfies this requirement.
    pub fn matches(&self, version: &Version) -> bool {
        match self.comparator {
            Comparator::Exact => version == &self.version,
            Comparator::Greater => version > &self.version,
            Comparator::GreaterEq => version >= &self.version,
            Comparator::Less => version < &self.version,
            Comparator::LessEq => version <= &self.version,
            Comparator::Caret => {
                // ^1.2.3 means >=1.2.3 and <2.0.0
                // ^0.2.3 means >=0.2.3 and <0.3.0
                // ^0.0.3 means >=0.0.3 and <0.0.4
                if version < &self.version {
                    return false;
                }
                if self.version.major == 0 {
                    if self.version.minor == 0 {
                        version.major == 0
                            && version.minor == 0
                            && version.patch == self.version.patch
                    } else {
                        version.major == 0 && version.minor == self.version.minor
                    }
                } else {
                    version.major == self.version.major
                }
            }
            Comparator::Tilde => {
                // ~1.2.3 means >=1.2.3 and <1.3.0
                version >= &self.version
                    && version.major == self.version.major
                    && version.minor == self.version.minor
            }
        }
    }
}

impl std::fmt::Display for VersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let op = match self.comparator {
            Comparator::Exact => "=",
            Comparator::Greater => ">",
            Comparator::GreaterEq => ">=",
            Comparator::Less => "<",
            Comparator::LessEq => "<=",
            Comparator::Caret => "^",
            Comparator::Tilde => "~",
        };
        write!(f, "{}{}", op, self.version)
    }
}

// =============================================================================
// Plugin Info
// =============================================================================

/// Information about a discovered plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// The parsed manifest.
    pub manifest: PluginManifest,

    /// Path to the plugin directory (containing plugin.toml).
    pub source_path: PathBuf,

    /// Parsed version for comparison.
    pub version: Version,

    /// Priority level (lower = higher priority, 0 = user, higher = system).
    pub priority: u32,

    /// Whether the plugin is currently enabled.
    pub enabled: bool,
}

impl PluginInfo {
    /// Creates a new PluginInfo from a manifest and source path.
    pub fn new(manifest: PluginManifest, source_path: PathBuf, priority: u32) -> Option<Self> {
        let version = Version::parse(&manifest.plugin.version)?;
        Some(Self {
            manifest,
            source_path,
            version,
            priority,
            enabled: true,
        })
    }

    /// Returns the plugin name.
    pub fn name(&self) -> &str {
        &self.manifest.plugin.name
    }

    /// Returns the plugin type.
    pub fn plugin_type(&self) -> PluginType {
        self.manifest.plugin.plugin_type
    }

    /// Returns the path to the library file for native plugins.
    pub fn library_path(&self) -> Option<PathBuf> {
        self.manifest.library_path(&self.source_path)
    }
}

// =============================================================================
// Discovery Errors
// =============================================================================

/// Error that occurred during plugin discovery.
#[derive(Debug)]
pub struct DiscoveryError {
    /// Path where the error occurred.
    pub path: PathBuf,
    /// Error message.
    pub message: String,
    /// Underlying error if available.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for DiscoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl From<ManifestError> for DiscoveryError {
    fn from(err: ManifestError) -> Self {
        let path = match &err {
            ManifestError::Io { path, .. } => path.clone(),
            ManifestError::Parse { path, .. } => path.clone(),
            ManifestError::Validation { path, .. } => path.clone(),
        };
        Self {
            path,
            message: err.to_string(),
            source: Some(Box::new(err)),
        }
    }
}

// =============================================================================
// Plugin Scanner
// =============================================================================

/// Scans directories for plugin manifests.
pub struct PluginScanner {
    /// Directories to scan in order.
    search_paths: Vec<PathBuf>,
}

impl PluginScanner {
    /// Creates a new scanner with no search paths.
    pub fn new() -> Self {
        Self {
            search_paths: Vec::new(),
        }
    }

    /// Adds a search path to scan.
    pub fn add_search_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.search_paths.push(path.into());
    }

    /// Scans all search paths and returns discovered plugins with any errors.
    pub fn scan(&self) -> (Vec<PluginInfo>, Vec<DiscoveryError>) {
        let mut plugins = Vec::new();
        let mut errors = Vec::new();

        for (priority, path) in self.search_paths.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: number of search paths is small; fits in u32
            let (found, errs) = self.scan_directory(path, priority as u32);
            plugins.extend(found);
            errors.extend(errs);
        }

        (plugins, errors)
    }

    /// Scans a single directory for plugins.
    fn scan_directory(&self, path: &Path, priority: u32) -> (Vec<PluginInfo>, Vec<DiscoveryError>) {
        let mut plugins = Vec::new();
        let mut errors = Vec::new();

        if !path.exists() {
            tracing::debug!("Plugin search path does not exist: {}", path.display());
            return (plugins, errors);
        }

        if !path.is_dir() {
            errors.push(DiscoveryError {
                path: path.to_path_buf(),
                message: "Not a directory".to_string(),
                source: None,
            });
            return (plugins, errors);
        }

        // Scan for subdirectories containing plugin.toml
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push(DiscoveryError {
                    path: path.to_path_buf(),
                    message: format!("Failed to read directory: {}", e),
                    source: Some(Box::new(e)),
                });
                return (plugins, errors);
            }
        };

        for entry in entries.flatten() {
            let entry_path = entry.path();

            // Check for plugin.toml in subdirectory (e.g., plugins/my-plugin/plugin.toml)
            if entry_path.is_dir() {
                let manifest_path = entry_path.join("plugin.toml");
                if manifest_path.exists() {
                    match self.load_plugin(&manifest_path, &entry_path, priority) {
                        Ok(info) => plugins.push(info),
                        Err(e) => errors.push(e),
                    }
                }
            }
            // Also check for plugin.toml directly in the search path
            // (for single-file plugin directories)
            else if entry_path.is_file()
                && entry_path.file_name().is_some_and(|n| n == "plugin.toml")
            {
                match self.load_plugin(&entry_path, path, priority) {
                    Ok(info) => plugins.push(info),
                    Err(e) => errors.push(e),
                }
            }
        }

        (plugins, errors)
    }

    /// Loads a single plugin from its manifest file.
    fn load_plugin(
        &self,
        manifest_path: &Path,
        plugin_dir: &Path,
        priority: u32,
    ) -> Result<PluginInfo, DiscoveryError> {
        let manifest = PluginManifest::from_file(manifest_path)?;

        PluginInfo::new(manifest.clone(), plugin_dir.to_path_buf(), priority).ok_or_else(|| {
            DiscoveryError {
                path: manifest_path.to_path_buf(),
                message: format!("Invalid version format: {}", manifest.plugin.version),
                source: None,
            }
        })
    }
}

impl Default for PluginScanner {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Plugin Registry
// =============================================================================

/// Central registry for discovered plugins with versioning support.
///
/// The registry supports:
/// - Multiple versions of the same plugin
/// - Priority-based override (user plugins override system plugins)
/// - Version-based selection (newest compatible version)
/// - Dependency tracking
pub struct PluginRegistry {
    /// Scanner for discovering plugins.
    scanner: PluginScanner,

    /// Plugins indexed by name, with multiple versions possible.
    plugins: HashMap<String, Vec<PluginInfo>>,

    /// Module type IDs mapped to plugin names.
    module_index: HashMap<String, String>,
}

impl PluginRegistry {
    /// Creates a new, empty registry.
    pub fn new() -> Self {
        Self {
            scanner: PluginScanner::new(),
            plugins: HashMap::new(),
            module_index: HashMap::new(),
        }
    }

    /// Adds a search path for plugin discovery.
    ///
    /// Paths added first have higher priority (lower priority number).
    pub fn add_search_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.scanner.add_search_path(path);
    }

    /// Scans all search paths and loads discovered plugins.
    ///
    /// Returns errors for plugins that failed to load.
    pub fn scan(&mut self) -> Vec<DiscoveryError> {
        let (plugins, errors) = self.scanner.scan();

        for info in plugins {
            self.register_plugin(info);
        }

        errors
    }

    /// Registers a plugin in the registry.
    fn register_plugin(&mut self, info: PluginInfo) {
        let name = info.name().to_string();

        // Index module type_id if present
        if let Some(ref module) = info.manifest.module {
            self.module_index
                .insert(module.type_id.clone(), name.clone());
        }

        // Add to version list
        let versions = self.plugins.entry(name).or_default();

        // Check if this exact version already exists
        let existing_idx = versions.iter().position(|v| v.version == info.version);

        if let Some(idx) = existing_idx {
            // Only replace if new one has higher priority (lower number)
            if info.priority < versions[idx].priority {
                tracing::info!(
                    "Overriding plugin '{}' v{} from {} with version from {} (higher priority)",
                    info.name(),
                    info.version,
                    versions[idx].source_path.display(),
                    info.source_path.display()
                );
                versions[idx] = info;
            } else {
                tracing::debug!(
                    "Skipping plugin '{}' v{} from {} (already loaded from {} with equal/higher priority)",
                    info.name(),
                    info.version,
                    info.source_path.display(),
                    versions[idx].source_path.display()
                );
            }
        } else {
            tracing::info!(
                "Registered plugin '{}' v{} from {}",
                info.name(),
                info.version,
                info.source_path.display()
            );
            versions.push(info);
            // Keep versions sorted (newest first)
            versions.sort_by(|a, b| b.version.cmp(&a.version));
        }
    }

    /// Returns a list of all registered plugin names.
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    /// Returns all versions of a plugin by name.
    pub fn get_versions(&self, name: &str) -> Option<&[PluginInfo]> {
        self.plugins.get(name).map(|v| v.as_slice())
    }

    /// Returns the latest version of a plugin by name.
    pub fn get_latest(&self, name: &str) -> Option<&PluginInfo> {
        self.plugins.get(name).and_then(|v| v.first())
    }

    /// Returns a specific version of a plugin.
    pub fn get_version(&self, name: &str, version: &Version) -> Option<&PluginInfo> {
        self.plugins
            .get(name)
            .and_then(|versions| versions.iter().find(|v| &v.version == version))
    }

    /// Finds a plugin version that satisfies a requirement.
    pub fn find_matching(&self, name: &str, req: &VersionReq) -> Option<&PluginInfo> {
        self.plugins
            .get(name)
            .and_then(|versions| versions.iter().find(|v| req.matches(&v.version)))
    }

    /// Returns the plugin that provides a given module type.
    pub fn get_by_module_type(&self, type_id: &str) -> Option<&PluginInfo> {
        self.module_index
            .get(type_id)
            .and_then(|name| self.get_latest(name))
    }

    /// Lists all registered plugins (latest version of each).
    pub fn list(&self) -> Vec<&PluginInfo> {
        self.plugins
            .values()
            .filter_map(|versions| versions.first())
            .collect()
    }

    /// Lists all plugin versions across all plugins.
    pub fn list_all_versions(&self) -> Vec<&PluginInfo> {
        self.plugins.values().flatten().collect()
    }

    /// Returns the total number of unique plugins.
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// Returns the total number of plugin versions.
    pub fn version_count(&self) -> usize {
        self.plugins.values().map(|v| v.len()).sum()
    }

    /// Clears all registered plugins.
    pub fn clear(&mut self) {
        self.plugins.clear();
        self.module_index.clear();
    }

    /// Reloads all plugins from configured search paths.
    pub fn reload(&mut self) -> Vec<DiscoveryError> {
        self.clear();
        self.scan()
    }

    /// Filters plugins by type.
    pub fn filter_by_type(&self, plugin_type: PluginType) -> Vec<&PluginInfo> {
        self.list()
            .into_iter()
            .filter(|p| p.plugin_type() == plugin_type)
            .collect()
    }

    /// Filters plugins by category.
    pub fn filter_by_category(&self, category: &str) -> Vec<&PluginInfo> {
        self.list()
            .into_iter()
            .filter(|p| p.manifest.plugin.categories.iter().any(|c| c == category))
            .collect()
    }

    /// Searches plugins by keyword.
    pub fn search(&self, query: &str) -> Vec<&PluginInfo> {
        let query_lower = query.to_lowercase();
        self.list()
            .into_iter()
            .filter(|p| {
                p.name().to_lowercase().contains(&query_lower)
                    || p.manifest
                        .plugin
                        .description
                        .to_lowercase()
                        .contains(&query_lower)
                    || p.manifest
                        .plugin
                        .keywords
                        .iter()
                        .any(|k| k.to_lowercase().contains(&query_lower))
            })
            .collect()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Dependency Resolution
// =============================================================================

/// Result of dependency resolution.
#[derive(Debug)]
pub struct DependencyResolution {
    /// Ordered list of plugins to load (dependencies first).
    pub load_order: Vec<String>,
    /// Any unresolved dependencies.
    pub unresolved: Vec<UnresolvedDep>,
    /// Any version conflicts.
    pub conflicts: Vec<VersionConflict>,
}

/// An unresolved dependency.
#[derive(Debug)]
pub struct UnresolvedDep {
    /// Plugin requiring the dependency.
    pub requirer: String,
    /// Name of missing dependency.
    pub dependency: String,
    /// Version requirement.
    pub requirement: String,
}

/// A version conflict between dependencies.
#[derive(Debug)]
pub struct VersionConflict {
    /// Plugin name with conflict.
    pub plugin: String,
    /// First requirement.
    pub req1: (String, String),
    /// Second conflicting requirement.
    pub req2: (String, String),
}

impl PluginRegistry {
    /// Resolves dependencies for a set of plugins.
    ///
    /// Returns the load order and any resolution errors.
    pub fn resolve_dependencies(&self, plugin_names: &[&str]) -> DependencyResolution {
        let mut load_order = Vec::new();
        let mut unresolved = Vec::new();
        let mut conflicts = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut in_progress = std::collections::HashSet::new();

        for name in plugin_names {
            self.resolve_plugin(
                name,
                &mut load_order,
                &mut unresolved,
                &mut conflicts,
                &mut visited,
                &mut in_progress,
            );
        }

        DependencyResolution {
            load_order,
            unresolved,
            conflicts,
        }
    }

    fn resolve_plugin(
        &self,
        name: &str,
        load_order: &mut Vec<String>,
        unresolved: &mut Vec<UnresolvedDep>,
        _conflicts: &mut Vec<VersionConflict>,
        visited: &mut std::collections::HashSet<String>,
        in_progress: &mut std::collections::HashSet<String>,
    ) {
        if visited.contains(name) {
            return;
        }

        if in_progress.contains(name) {
            // Circular dependency - already being processed
            return;
        }

        let Some(info) = self.get_latest(name) else {
            // Plugin not found - this is handled by caller
            return;
        };

        in_progress.insert(name.to_string());

        // Resolve dependencies first
        for (dep_name, dep_req) in &info.manifest.dependencies {
            if let Some(req) = VersionReq::parse(dep_req) {
                if self.find_matching(dep_name, &req).is_some() {
                    self.resolve_plugin(
                        dep_name,
                        load_order,
                        unresolved,
                        _conflicts,
                        visited,
                        in_progress,
                    );
                } else {
                    unresolved.push(UnresolvedDep {
                        requirer: name.to_string(),
                        dependency: dep_name.clone(),
                        requirement: dep_req.clone(),
                    });
                }
            } else {
                unresolved.push(UnresolvedDep {
                    requirer: name.to_string(),
                    dependency: dep_name.clone(),
                    requirement: format!("Invalid requirement: {}", dep_req),
                });
            }
        }

        in_progress.remove(name);
        visited.insert(name.to_string());
        load_order.push(name.to_string());
    }

    /// Checks if all dependencies for a plugin are satisfied.
    pub fn check_dependencies(&self, name: &str) -> Result<(), Vec<UnresolvedDep>> {
        let resolution = self.resolve_dependencies(&[name]);
        if resolution.unresolved.is_empty() {
            Ok(())
        } else {
            Err(resolution.unresolved)
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.prerelease, None);

        let v = Version::parse("1.2.3-beta.1").unwrap();
        assert_eq!(v.prerelease, Some("beta.1".to_string()));

        let v = Version::parse("1.2").unwrap();
        assert_eq!(v.patch, 0);
    }

    #[test]
    fn test_version_ordering() {
        let v1 = Version::parse("1.0.0").unwrap();
        let v2 = Version::parse("1.0.1").unwrap();
        let v3 = Version::parse("1.1.0").unwrap();
        let v4 = Version::parse("2.0.0").unwrap();
        let v5 = Version::parse("1.0.0-alpha").unwrap();

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v3 < v4);
        assert!(v5 < v1); // prerelease < release
    }

    #[test]
    fn test_version_req_parse() {
        let req = VersionReq::parse(">=1.0.0").unwrap();
        assert_eq!(req.comparator, Comparator::GreaterEq);

        let req = VersionReq::parse("^1.2.3").unwrap();
        assert_eq!(req.comparator, Comparator::Caret);

        let req = VersionReq::parse("~1.2.3").unwrap();
        assert_eq!(req.comparator, Comparator::Tilde);

        let req = VersionReq::parse("1.0.0").unwrap();
        assert_eq!(req.comparator, Comparator::Exact);
    }

    #[test]
    fn test_version_req_matches() {
        let req = VersionReq::parse(">=1.0.0").unwrap();
        assert!(req.matches(&Version::parse("1.0.0").unwrap()));
        assert!(req.matches(&Version::parse("1.0.1").unwrap()));
        assert!(req.matches(&Version::parse("2.0.0").unwrap()));
        assert!(!req.matches(&Version::parse("0.9.9").unwrap()));

        let req = VersionReq::parse("^1.2.3").unwrap();
        assert!(req.matches(&Version::parse("1.2.3").unwrap()));
        assert!(req.matches(&Version::parse("1.3.0").unwrap()));
        assert!(req.matches(&Version::parse("1.9.9").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.0").unwrap()));
        assert!(!req.matches(&Version::parse("1.2.2").unwrap()));

        let req = VersionReq::parse("~1.2.3").unwrap();
        assert!(req.matches(&Version::parse("1.2.3").unwrap()));
        assert!(req.matches(&Version::parse("1.2.9").unwrap()));
        assert!(!req.matches(&Version::parse("1.3.0").unwrap()));
    }

    #[test]
    fn test_caret_zero_versions() {
        // ^0.2.3 means >=0.2.3 and <0.3.0
        let req = VersionReq::parse("^0.2.3").unwrap();
        assert!(req.matches(&Version::parse("0.2.3").unwrap()));
        assert!(req.matches(&Version::parse("0.2.9").unwrap()));
        assert!(!req.matches(&Version::parse("0.3.0").unwrap()));
        assert!(!req.matches(&Version::parse("1.0.0").unwrap()));

        // ^0.0.3 means >=0.0.3 and <0.0.4
        let req = VersionReq::parse("^0.0.3").unwrap();
        assert!(req.matches(&Version::parse("0.0.3").unwrap()));
        assert!(!req.matches(&Version::parse("0.0.4").unwrap()));
    }

    #[test]
    fn test_registry_basic() {
        let mut registry = PluginRegistry::new();

        // Create a test plugin info
        let manifest = PluginManifest {
            plugin: super::super::manifest::PluginConfig {
                name: "test-plugin".to_string(),
                version: "1.0.0".to_string(),
                description: "A test plugin".to_string(),
                author: None,
                license: None,
                repository: None,
                categories: vec!["testing".to_string()],
                keywords: vec!["test".to_string()],
                plugin_type: PluginType::Native,
                requires: Default::default(),
                entry: Default::default(),
                activation: Default::default(),
                hooks: Default::default(),
            },
            module: None,
            dependencies: HashMap::new(),
        };

        let info = PluginInfo::new(manifest, PathBuf::from("/test"), 0).unwrap();
        registry.register_plugin(info);

        assert_eq!(registry.plugin_count(), 1);
        assert!(registry.get_latest("test-plugin").is_some());
    }

    #[test]
    fn test_registry_multiple_versions() {
        let mut registry = PluginRegistry::new();

        for version in ["1.0.0", "1.1.0", "2.0.0"] {
            let manifest = PluginManifest {
                plugin: super::super::manifest::PluginConfig {
                    name: "versioned-plugin".to_string(),
                    version: version.to_string(),
                    description: String::new(),
                    author: None,
                    license: None,
                    repository: None,
                    categories: vec![],
                    keywords: vec![],
                    plugin_type: PluginType::Native,
                    requires: Default::default(),
                    entry: Default::default(),
                    activation: Default::default(),
                    hooks: Default::default(),
                },
                module: None,
                dependencies: HashMap::new(),
            };

            let info = PluginInfo::new(manifest, PathBuf::from("/test"), 0).unwrap();
            registry.register_plugin(info);
        }

        assert_eq!(registry.plugin_count(), 1);
        assert_eq!(registry.version_count(), 3);

        // Latest should be 2.0.0
        let latest = registry.get_latest("versioned-plugin").unwrap();
        assert_eq!(latest.version.to_string(), "2.0.0");

        // Can get specific version
        let v1 = registry
            .get_version("versioned-plugin", &Version::parse("1.1.0").unwrap())
            .unwrap();
        assert_eq!(v1.version.to_string(), "1.1.0");
    }

    #[test]
    fn test_registry_filter_by_type() {
        let mut registry = PluginRegistry::new();

        for (name, ptype) in [
            ("native-plugin", PluginType::Native),
            ("script-plugin", PluginType::Script),
        ] {
            let manifest = PluginManifest {
                plugin: super::super::manifest::PluginConfig {
                    name: name.to_string(),
                    version: "1.0.0".to_string(),
                    description: String::new(),
                    author: None,
                    license: None,
                    repository: None,
                    categories: vec![],
                    keywords: vec![],
                    plugin_type: ptype,
                    requires: Default::default(),
                    entry: Default::default(),
                    activation: Default::default(),
                    hooks: Default::default(),
                },
                module: None,
                dependencies: HashMap::new(),
            };

            let info = PluginInfo::new(manifest, PathBuf::from("/test"), 0).unwrap();
            registry.register_plugin(info);
        }

        let native = registry.filter_by_type(PluginType::Native);
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].name(), "native-plugin");

        let scripts = registry.filter_by_type(PluginType::Script);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name(), "script-plugin");
    }
}
