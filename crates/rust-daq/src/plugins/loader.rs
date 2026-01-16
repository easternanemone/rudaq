//! Script plugin loader for discovering and loading script-based modules.
//!
//! The `ScriptPluginLoader` scans directories for script files and creates
//! `ScriptModule` instances from them. It supports Rhai (.rhai) scripts and
//! optionally Python (.py) scripts when the `python` feature is enabled.
//!
//! # Directory Structure
//!
//! The loader expects script modules to be organized as:
//!
//! ```text
//! scripts/
//! ├── power_logger.rhai      # Rhai script module
//! ├── data_processor.rhai    # Another Rhai module
//! └── analysis.py            # Python module (if python feature enabled)
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use rust_daq::plugins::ScriptPluginLoader;
//!
//! let mut loader = ScriptPluginLoader::new();
//! loader.add_search_path("./scripts");
//!
//! // Discover all script modules
//! let module_types = loader.discover().await?;
//!
//! // Create a module instance
//! let module = loader.create_module("power_logger").await?;
//! ```

use super::script_module::ScriptModule;
use crate::modules::Module;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Supported script languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptLanguage {
    /// Rhai embedded scripting language
    Rhai,
    /// Python via PyO3 (requires python feature)
    #[cfg(feature = "scripting_python")]
    Python,
}

impl ScriptLanguage {
    /// Get the file extension for this language.
    pub fn extension(&self) -> &'static str {
        match self {
            ScriptLanguage::Rhai => "rhai",
            #[cfg(feature = "scripting_python")]
            ScriptLanguage::Python => "py",
        }
    }

    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rhai" => Some(ScriptLanguage::Rhai),
            #[cfg(feature = "scripting_python")]
            "py" => Some(ScriptLanguage::Python),
            _ => None,
        }
    }
}

/// Metadata about a discovered script module.
#[derive(Debug, Clone)]
pub struct ScriptModuleInfo {
    /// Module type ID (from script's module_type_info())
    pub type_id: String,
    /// Display name
    pub display_name: String,
    /// Description
    pub description: String,
    /// Version
    pub version: String,
    /// Path to the script file
    pub script_path: PathBuf,
    /// Script language
    pub language: ScriptLanguage,
}

/// Loader for script-based modules.
///
/// Discovers script files in configured directories and provides
/// factory methods for creating module instances.
#[derive(Debug)]
pub struct ScriptPluginLoader {
    /// Directories to search for script files
    search_paths: Vec<PathBuf>,
    /// Discovered modules: type_id -> info
    modules: HashMap<String, ScriptModuleInfo>,
    /// Cached script sources: path -> source
    script_cache: HashMap<PathBuf, String>,
}

impl Default for ScriptPluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptPluginLoader {
    /// Create a new script plugin loader.
    pub fn new() -> Self {
        Self {
            search_paths: Vec::new(),
            modules: HashMap::new(),
            script_cache: HashMap::new(),
        }
    }

    /// Add a directory to search for script files.
    pub fn add_search_path<P: AsRef<Path>>(&mut self, path: P) {
        let path = path.as_ref().to_path_buf();
        if !self.search_paths.contains(&path) {
            self.search_paths.push(path);
        }
    }

    /// Get all configured search paths.
    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search_paths
    }

    /// Discover all script modules in search paths.
    ///
    /// This scans all directories for script files, loads them to extract
    /// module metadata, and registers them in the loader.
    ///
    /// Returns a list of discovered module type IDs.
    pub async fn discover(&mut self) -> Result<Vec<String>> {
        let mut discovered = Vec::new();

        for search_path in self.search_paths.clone() {
            if !search_path.exists() {
                debug!("Script search path does not exist: {:?}", search_path);
                continue;
            }

            let entries = std::fs::read_dir(&search_path)
                .map_err(|e| anyhow!("Failed to read directory {:?}: {}", search_path, e))?;

            for entry in entries.flatten() {
                let path = entry.path();

                // Check if it's a script file
                if !path.is_file() {
                    continue;
                }

                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let Some(language) = ScriptLanguage::from_extension(ext) else {
                    continue;
                };

                // Try to load and extract metadata
                match self.load_script_metadata(&path, language).await {
                    Ok(info) => {
                        let type_id = info.type_id.clone();
                        info!("Discovered script module: {} ({:?})", type_id, path);
                        discovered.push(type_id.clone());
                        self.modules.insert(type_id, info);
                    }
                    Err(e) => {
                        warn!("Failed to load script {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(discovered)
    }

    /// Load metadata from a script file.
    async fn load_script_metadata(
        &mut self,
        path: &Path,
        language: ScriptLanguage,
    ) -> Result<ScriptModuleInfo> {
        // Read and cache the script source
        let source = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read {:?}: {}", path, e))?;

        self.script_cache.insert(path.to_path_buf(), source.clone());

        // Load the script to extract metadata
        match language {
            ScriptLanguage::Rhai => {
                let module = ScriptModule::from_source(source, path.to_path_buf()).await?;

                Ok(ScriptModuleInfo {
                    type_id: module.type_id().to_string(),
                    display_name: module.type_id().to_string(), // TODO: Get from type_info
                    description: String::new(),
                    version: "1.0.0".to_string(),
                    script_path: path.to_path_buf(),
                    language,
                })
            }
            #[cfg(feature = "scripting_python")]
            ScriptLanguage::Python => {
                // Python support would go here
                Err(anyhow!("Python script modules not yet implemented"))
            }
        }
    }

    /// List all discovered module types.
    pub fn list_module_types(&self) -> Vec<&ScriptModuleInfo> {
        self.modules.values().collect()
    }

    /// Get info for a specific module type.
    pub fn get_module_info(&self, type_id: &str) -> Option<&ScriptModuleInfo> {
        self.modules.get(type_id)
    }

    /// Check if a module type is available.
    pub fn has_module_type(&self, type_id: &str) -> bool {
        self.modules.contains_key(type_id)
    }

    /// Create a module instance by type ID.
    ///
    /// This loads the script and creates a new `ScriptModule` instance.
    pub async fn create_module(&self, type_id: &str) -> Result<ScriptModule> {
        let info = self
            .modules
            .get(type_id)
            .ok_or_else(|| anyhow!("Unknown script module type: {}", type_id))?;

        // Get cached source or reload
        let source = self
            .script_cache
            .get(&info.script_path)
            .cloned()
            .ok_or_else(|| anyhow!("Script source not cached for {}", type_id))?;

        match info.language {
            ScriptLanguage::Rhai => {
                ScriptModule::from_source(source, info.script_path.clone()).await
            }
            #[cfg(feature = "scripting_python")]
            ScriptLanguage::Python => Err(anyhow!("Python script modules not yet implemented")),
        }
    }

    /// Create a module directly from a file path.
    ///
    /// This bypasses the discovery mechanism and loads a module directly.
    pub async fn create_module_from_file<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<ScriptModule> {
        let path = path.as_ref();

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language = ScriptLanguage::from_extension(ext)
            .ok_or_else(|| anyhow!("Unsupported script extension: {}", ext))?;

        match language {
            ScriptLanguage::Rhai => ScriptModule::from_file(path).await,
            #[cfg(feature = "scripting_python")]
            ScriptLanguage::Python => Err(anyhow!("Python script modules not yet implemented")),
        }
    }

    /// Register a module from inline source code.
    ///
    /// Useful for testing or embedding scripts directly in code.
    pub async fn register_inline(
        &mut self,
        source: String,
        language: ScriptLanguage,
    ) -> Result<String> {
        let fake_path = PathBuf::from(format!(
            "__inline_{}.{}",
            uuid::Uuid::new_v4(),
            language.extension()
        ));

        match language {
            ScriptLanguage::Rhai => {
                let module = ScriptModule::from_source(source.clone(), fake_path.clone()).await?;
                let type_id = module.type_id().to_string();

                self.script_cache.insert(fake_path.clone(), source);
                self.modules.insert(
                    type_id.clone(),
                    ScriptModuleInfo {
                        type_id: type_id.clone(),
                        display_name: type_id.clone(),
                        description: String::new(),
                        version: "1.0.0".to_string(),
                        script_path: fake_path,
                        language,
                    },
                );

                Ok(type_id)
            }
            #[cfg(feature = "scripting_python")]
            ScriptLanguage::Python => Err(anyhow!("Python script modules not yet implemented")),
        }
    }

    /// Clear all discovered modules and caches.
    pub fn clear(&mut self) {
        self.modules.clear();
        self.script_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    const TEST_SCRIPT: &str = r#"
fn module_type_info() {
    #{
        type_id: "test_loader_module",
        display_name: "Test Loader Module",
        description: "A test module for the loader",
        version: "1.0.0"
    }
}

fn start(ctx) {
    print("Hello from test module!");
}
"#;

    #[tokio::test]
    async fn test_discover_scripts() {
        let temp_dir = TempDir::new().unwrap();
        let script_path = temp_dir.path().join("test_module.rhai");

        // Write test script
        let mut file = std::fs::File::create(&script_path).unwrap();
        file.write_all(TEST_SCRIPT.as_bytes()).unwrap();

        // Create loader and discover
        let mut loader = ScriptPluginLoader::new();
        loader.add_search_path(temp_dir.path());

        let discovered = loader.discover().await.unwrap();
        assert_eq!(discovered, vec!["test_loader_module"]);

        assert!(loader.has_module_type("test_loader_module"));
        assert!(!loader.has_module_type("nonexistent"));
    }

    #[tokio::test]
    async fn test_create_module() {
        let temp_dir = TempDir::new().unwrap();
        let script_path = temp_dir.path().join("test_module.rhai");

        let mut file = std::fs::File::create(&script_path).unwrap();
        file.write_all(TEST_SCRIPT.as_bytes()).unwrap();

        let mut loader = ScriptPluginLoader::new();
        loader.add_search_path(temp_dir.path());
        loader.discover().await.unwrap();

        let module = loader.create_module("test_loader_module").await.unwrap();
        assert_eq!(module.type_id(), "test_loader_module");
    }

    #[tokio::test]
    async fn test_register_inline() {
        let mut loader = ScriptPluginLoader::new();

        let type_id = loader
            .register_inline(TEST_SCRIPT.to_string(), ScriptLanguage::Rhai)
            .await
            .unwrap();

        assert_eq!(type_id, "test_loader_module");
        assert!(loader.has_module_type("test_loader_module"));
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(
            ScriptLanguage::from_extension("rhai"),
            Some(ScriptLanguage::Rhai)
        );
        assert_eq!(ScriptLanguage::from_extension("txt"), None);
    }
}
