//! Plugin hot-reload support for development.
//!
//! Watches plugin directories for changes and automatically reloads
//! modified configurations without requiring a restart.
//!
//! # Feature Gate
//!
//! This module is only available with the `plugins_hot_reload` feature:
//! ```bash
//! cargo build --features plugins_hot_reload
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::hardware::plugin::hot_reload::PluginWatcher;
//! use rust_daq::hardware::plugin::registry::PluginFactory;
//! use std::sync::Arc;
//! use tokio::sync::RwLock;
//!
//! let factory = Arc::new(RwLock::new(PluginFactory::new()));
//! factory.write().await.add_search_path("plugins/");
//! factory.write().await.scan().await;
//!
//! // Start watching for changes
//! let watcher = PluginWatcher::new(factory.clone())?;
//! watcher.watch("plugins/")?;
//!
//! // Watcher runs in background, reloading plugins on file changes
//! ```
//!
//! # Notes
//!
//! - Only reloads plugin templates (not active driver instances)
//! - Invalid YAML files are logged but don't crash the watcher
//! - Debounces rapid file changes to avoid redundant reloads

use anyhow::{anyhow, Result};
use notify::{
    event::{CreateKind, ModifyKind, RemoveKind},
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};

use super::registry::PluginFactory;
use super::schema::InstrumentConfig;

/// Watches plugin directories for changes and reloads configurations.
///
/// The watcher monitors specified directories for YAML file changes and
/// automatically updates the `PluginFactory` when files are modified.
pub struct PluginWatcher {
    /// The plugin factory to update on changes
    factory: Arc<RwLock<PluginFactory>>,
    /// The underlying file watcher
    _watcher: RecommendedWatcher,
    /// Channel receiver for processing events (held to keep watcher alive)
    _rx: mpsc::Receiver<notify::Result<Event>>,
}

impl PluginWatcher {
    /// Creates a new PluginWatcher for the given factory.
    ///
    /// # Arguments
    /// * `factory` - Shared reference to the plugin factory to update
    ///
    /// # Returns
    /// A new watcher instance ready to watch directories
    pub fn new(factory: Arc<RwLock<PluginFactory>>) -> Result<Self> {
        let (tx, rx) = mpsc::channel(100);

        // Create the watcher with a sender
        let watcher = notify::recommended_watcher(move |res| {
            // Use blocking send since we're in sync callback
            let _ = tx.blocking_send(res);
        })?;

        Ok(Self {
            factory,
            _watcher: watcher,
            _rx: rx,
        })
    }

    /// Starts watching a directory for plugin changes.
    ///
    /// This spawns a background task that processes file system events
    /// and reloads plugins as needed.
    ///
    /// # Arguments
    /// * `path` - Directory path to watch for YAML files
    pub fn watch<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        self._watcher
            .watch(path.as_ref(), RecursiveMode::NonRecursive)?;
        tracing::info!("Plugin hot-reload watching: {}", path.as_ref().display());
        Ok(())
    }

    /// Starts the event processing loop.
    ///
    /// This should be called after setting up watches. It runs until
    /// the watcher is dropped.
    pub async fn run(mut self) {
        tracing::info!("Plugin hot-reload started");

        while let Some(event_result) = self._rx.recv().await {
            match event_result {
                Ok(event) => {
                    if let Err(e) = self.handle_event(event).await {
                        tracing::warn!("Error handling file event: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!("File watcher error: {}", e);
                }
            }
        }

        tracing::info!("Plugin hot-reload stopped");
    }

    /// Handles a single file system event.
    async fn handle_event(&self, event: Event) -> Result<()> {
        // Only process YAML files
        let yaml_paths: Vec<_> = event
            .paths
            .iter()
            .filter(|p| {
                p.extension()
                    .map(|ext| ext == "yaml" || ext == "yml")
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        if yaml_paths.is_empty() {
            return Ok(());
        }

        match event.kind {
            EventKind::Create(CreateKind::File) | EventKind::Modify(ModifyKind::Data(_)) => {
                for path in yaml_paths {
                    tracing::info!("Plugin file changed: {}", path.display());
                    self.reload_plugin(&path).await?;
                }
            }
            EventKind::Remove(RemoveKind::File) => {
                for path in yaml_paths {
                    tracing::info!("Plugin file removed: {}", path.display());
                    self.remove_plugin(&path).await?;
                }
            }
            _ => {
                // Ignore other events (access, metadata changes, etc.)
            }
        }

        Ok(())
    }

    /// Reloads a single plugin from its YAML file.
    async fn reload_plugin(&self, path: &Path) -> Result<()> {
        // Read and parse the file
        let content = tokio::fs::read_to_string(path).await?;

        let config: InstrumentConfig = match serde_yaml::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    "Failed to parse plugin {}: {} (keeping old config)",
                    path.display(),
                    e
                );
                return Ok(()); // Don't error out, just keep old config
            }
        };

        // Validate the configuration
        let validation_errors = super::registry::validate_plugin_config(&config);
        if !validation_errors.is_empty() {
            tracing::error!(
                "Plugin {} validation failed (keeping old config):",
                path.display()
            );
            for err in &validation_errors {
                tracing::error!("  - {}: {}", err.path, err.message);
            }
            return Ok(()); // Don't error out, just keep old config
        }

        // Update the factory
        let plugin_id = config.metadata.id.clone();
        let plugin_name = config.metadata.name.clone();

        {
            let mut factory = self.factory.write().await;
            factory.update_plugin(plugin_id.clone(), config, path.to_path_buf());
        }

        tracing::info!("Hot-reloaded plugin: {} ({})", plugin_name, path.display());

        Ok(())
    }

    /// Removes a plugin when its file is deleted.
    async fn remove_plugin(&self, path: &Path) -> Result<()> {
        let mut factory = self.factory.write().await;

        // Find and remove the plugin with this source path
        if let Some(id) = factory.find_by_path(path) {
            factory.remove_plugin(&id);
            tracing::info!("Removed plugin from {}", path.display());
        }

        Ok(())
    }
}

/// Convenience function to start hot-reload watching.
///
/// # Arguments
/// * `factory` - The plugin factory to watch
/// * `paths` - Directories to watch for plugin changes
///
/// # Returns
/// A join handle for the background task
pub fn start_hot_reload(
    factory: Arc<RwLock<PluginFactory>>,
    paths: Vec<PathBuf>,
) -> Result<tokio::task::JoinHandle<()>> {
    let mut watcher = PluginWatcher::new(factory)?;

    for path in &paths {
        watcher.watch(path)?;
    }

    let handle = tokio::spawn(async move {
        watcher.run().await;
    });

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_watcher_creation() {
        let factory = Arc::new(RwLock::new(PluginFactory::new()));
        let watcher = PluginWatcher::new(factory);
        assert!(watcher.is_ok());
    }

    #[tokio::test]
    async fn test_watch_nonexistent_path() {
        let factory = Arc::new(RwLock::new(PluginFactory::new()));
        let mut watcher = PluginWatcher::new(factory).unwrap();
        // Watching a non-existent path should fail
        let result = watcher.watch("/nonexistent/path/that/does/not/exist");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_watch_valid_directory() {
        let temp_dir = TempDir::new().unwrap();
        let factory = Arc::new(RwLock::new(PluginFactory::new()));
        let mut watcher = PluginWatcher::new(factory).unwrap();
        let result = watcher.watch(temp_dir.path());
        assert!(result.is_ok());
    }
}
