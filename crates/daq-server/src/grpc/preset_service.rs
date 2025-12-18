//! PresetService implementation for configuration save/load (bd-akcm)
//!
//! Provides gRPC endpoints for persisting and loading device configurations
//! and scan templates. Uses filesystem-based storage with integrity checking.
//!
//! Performance optimization (bd-l2bt): Uses manifest file for O(1) listing
//! instead of reading all preset files.

use crate::grpc::proto::{
    DeletePresetRequest, DeletePresetResponse, GetPresetRequest, ListPresetsRequest,
    ListPresetsResponse, LoadPresetRequest, LoadPresetResponse, Preset, PresetMetadata,
    SavePresetRequest, SavePresetResponse, preset_service_server::PresetService,
};
use daq_hardware::registry::DeviceRegistry;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

/// Manifest file name for fast preset listing
const MANIFEST_FILENAME: &str = "manifest.json";

/// Preset gRPC service implementation
///
/// Stores presets as JSON files in a configurable directory with:
/// - Content hash for integrity verification
/// - Last-N backup retention (configurable, default 3)
/// - Manifest file for fast listing
pub struct PresetServiceImpl {
    registry: Arc<RwLock<DeviceRegistry>>,
    storage_path: PathBuf,
    max_backups: usize,
}

impl std::fmt::Debug for PresetServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresetServiceImpl")
            .field("registry", &"<Arc<RwLock<DeviceRegistry>>>")
            .field("storage_path", &self.storage_path)
            .field("max_backups", &self.max_backups)
            .finish()
    }
}

impl PresetServiceImpl {
    /// Create a new PresetService with the given storage directory
    pub fn new(registry: Arc<RwLock<DeviceRegistry>>, storage_path: PathBuf) -> Self {
        // Ensure storage directory exists
        if let Err(e) = fs::create_dir_all(&storage_path) {
            tracing::warn!("Failed to create preset storage directory: {}", e);
        }
        Self {
            registry,
            storage_path,
            max_backups: 3,
        }
    }

    /// Create with custom backup retention
    pub fn with_max_backups(mut self, max_backups: usize) -> Self {
        self.max_backups = max_backups;
        self
    }

    /// Get path to a preset file
    fn preset_path(&self, preset_id: &str) -> PathBuf {
        self.storage_path.join(format!("{}.json", preset_id))
    }

    /// Get path to a preset backup file
    fn backup_path(&self, preset_id: &str, backup_num: usize) -> PathBuf {
        self.storage_path
            .join(format!("{}.backup{}.json", preset_id, backup_num))
    }

    /// Get path to manifest file
    fn manifest_path(&self) -> PathBuf {
        self.storage_path.join(MANIFEST_FILENAME)
    }

    /// Load manifest from disk (returns empty vec if not found or corrupted)
    fn load_manifest(&self) -> Vec<PresetMetadata> {
        let path = self.manifest_path();
        if !path.exists() {
            return Vec::new();
        }

        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str::<Vec<ManifestEntry>>(&content)
                .map(|entries| entries.into_iter().map(|e| e.to_proto()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// Save manifest to disk
    fn save_manifest(&self, presets: &[PresetMetadata]) -> Result<(), Status> {
        let entries: Vec<ManifestEntry> = presets.iter().map(ManifestEntry::from_proto).collect();
        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| Status::internal(format!("Failed to serialize manifest: {}", e)))?;

        fs::write(self.manifest_path(), json)
            .map_err(|e| Status::internal(format!("Failed to write manifest: {}", e)))?;

        Ok(())
    }

    /// Update manifest with a new or modified preset metadata
    fn update_manifest_entry(&self, meta: &PresetMetadata) -> Result<(), Status> {
        let mut presets = self.load_manifest();

        // Remove existing entry if present
        presets.retain(|p| p.preset_id != meta.preset_id);

        // Add new entry
        presets.push(meta.clone());

        // Sort by updated_at (newest first)
        presets.sort_by(|a, b| b.updated_at_ns.cmp(&a.updated_at_ns));

        self.save_manifest(&presets)
    }

    /// Remove preset from manifest
    fn remove_manifest_entry(&self, preset_id: &str) -> Result<(), Status> {
        let mut presets = self.load_manifest();
        presets.retain(|p| p.preset_id != preset_id);
        self.save_manifest(&presets)
    }

    /// Rebuild manifest by scanning all preset files (fallback for corrupted manifest)
    fn rebuild_manifest(&self) -> Result<Vec<PresetMetadata>, Status> {
        tracing::info!("Rebuilding preset manifest from disk...");
        let presets = self.scan_presets_from_disk()?;
        self.save_manifest(&presets)?;
        tracing::info!("Manifest rebuilt with {} presets", presets.len());
        Ok(presets)
    }

    /// Compute SHA-256 hash of content
    fn hash_content(content: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content);
        format!("{:x}", hasher.finalize())
    }

    /// Save preset to filesystem with backup rotation
    fn save_preset_to_disk(&self, preset: &Preset) -> Result<(), Status> {
        let preset_id = preset
            .meta
            .as_ref()
            .map(|m| m.preset_id.clone())
            .unwrap_or_default();

        if preset_id.is_empty() {
            return Err(Status::invalid_argument("preset_id is required"));
        }

        // Validate preset_id (alphanumeric, underscore, hyphen only)
        if !preset_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Status::invalid_argument(
                "preset_id must contain only alphanumeric characters, underscores, and hyphens",
            ));
        }

        let path = self.preset_path(&preset_id);

        // Rotate backups if file exists
        if path.exists() {
            self.rotate_backups(&preset_id)?;
        }

        // Serialize preset to JSON
        let json = serde_json::to_string_pretty(&PresetFile::from_proto(preset))
            .map_err(|e| Status::internal(format!("Failed to serialize preset: {}", e)))?;

        // Write with hash verification
        let hash = Self::hash_content(json.as_bytes());

        let mut file = fs::File::create(&path)
            .map_err(|e| Status::internal(format!("Failed to create preset file: {}", e)))?;

        file.write_all(json.as_bytes())
            .map_err(|e| Status::internal(format!("Failed to write preset: {}", e)))?;

        // Write hash file
        let hash_path = path.with_extension("json.sha256");
        fs::write(&hash_path, &hash)
            .map_err(|e| Status::internal(format!("Failed to write hash file: {}", e)))?;

        // Update manifest for O(1) listing
        if let Some(meta) = &preset.meta {
            self.update_manifest_entry(meta)?;
        }

        tracing::info!("Saved preset '{}' (hash: {})", preset_id, &hash[..8]);
        Ok(())
    }

    /// Rotate backup files (keep max_backups)
    fn rotate_backups(&self, preset_id: &str) -> Result<(), Status> {
        // Remove oldest backup if at limit
        let oldest_backup = self.backup_path(preset_id, self.max_backups);
        if oldest_backup.exists() {
            let _ = fs::remove_file(&oldest_backup);
            let _ = fs::remove_file(oldest_backup.with_extension("json.sha256"));
        }

        // Shift existing backups
        for i in (1..self.max_backups).rev() {
            let from = self.backup_path(preset_id, i);
            let to = self.backup_path(preset_id, i + 1);
            if from.exists() {
                let _ = fs::rename(&from, &to);
                let from_hash = from.with_extension("json.sha256");
                let to_hash = to.with_extension("json.sha256");
                if from_hash.exists() {
                    let _ = fs::rename(&from_hash, &to_hash);
                }
            }
        }

        // Move current to backup 1
        let current = self.preset_path(preset_id);
        let backup1 = self.backup_path(preset_id, 1);
        if current.exists() {
            let _ = fs::rename(&current, &backup1);
            let current_hash = current.with_extension("json.sha256");
            let backup1_hash = backup1.with_extension("json.sha256");
            if current_hash.exists() {
                let _ = fs::rename(&current_hash, &backup1_hash);
            }
        }

        Ok(())
    }

    /// Load preset from filesystem with integrity check
    fn load_preset_from_disk(&self, preset_id: &str) -> Result<Preset, Status> {
        let path = self.preset_path(preset_id);

        if !path.exists() {
            return Err(Status::not_found(format!(
                "Preset '{}' not found",
                preset_id
            )));
        }

        let content = fs::read(&path)
            .map_err(|e| Status::internal(format!("Failed to read preset file: {}", e)))?;

        // Verify hash if available
        let hash_path = path.with_extension("json.sha256");
        if hash_path.exists() {
            let stored_hash = fs::read_to_string(&hash_path)
                .map_err(|e| Status::internal(format!("Failed to read hash file: {}", e)))?;

            let computed_hash = Self::hash_content(&content);
            if stored_hash.trim() != computed_hash {
                return Err(Status::data_loss(format!(
                    "Preset '{}' failed integrity check (corrupted)",
                    preset_id
                )));
            }
        }

        let preset_file: PresetFile = serde_json::from_slice(&content)
            .map_err(|e| Status::internal(format!("Failed to parse preset: {}", e)))?;

        Ok(preset_file.to_proto())
    }

    /// List all presets using the manifest for O(1) performance (bd-l2bt)
    ///
    /// Falls back to scanning all files if manifest is missing or corrupted.
    fn list_presets_from_disk(&self) -> Result<Vec<PresetMetadata>, Status> {
        // Try to load from manifest first (fast path)
        let manifest = self.load_manifest();
        if !manifest.is_empty() {
            return Ok(manifest);
        }

        // Manifest is empty or missing - rebuild from disk scan
        // This handles first-time use or corrupted manifest
        self.rebuild_manifest()
    }

    /// Scan all preset files from disk (slow O(n) operation)
    ///
    /// Used to rebuild the manifest. Reads and parses each preset file.
    fn scan_presets_from_disk(&self) -> Result<Vec<PresetMetadata>, Status> {
        let mut presets = Vec::new();

        let entries = fs::read_dir(&self.storage_path)
            .map_err(|e| Status::internal(format!("Failed to read preset directory: {}", e)))?;

        for entry in entries.flatten() {
            let path = entry.path();

            // Only process .json files (not backups, hash files, or manifest)
            if let Some(ext) = path.extension() {
                if ext == "json"
                    && !path.to_string_lossy().contains(".backup")
                    && path
                        .file_name()
                        .map(|n| n != MANIFEST_FILENAME)
                        .unwrap_or(true)
                {
                    if let Some(stem) = path.file_stem() {
                        let preset_id = stem.to_string_lossy().to_string();

                        // Try to load metadata without full preset
                        match self.load_preset_from_disk(&preset_id) {
                            Ok(preset) => {
                                if let Some(meta) = preset.meta {
                                    presets.push(meta);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Skipping corrupted preset '{}': {}",
                                    preset_id,
                                    e.message()
                                );
                            }
                        }
                    }
                }
            }
        }

        // Sort by updated_at (newest first)
        presets.sort_by(|a, b| b.updated_at_ns.cmp(&a.updated_at_ns));

        Ok(presets)
    }

    /// Delete preset from storage
    fn delete_preset_from_disk(&self, preset_id: &str) -> Result<(), Status> {
        let path = self.preset_path(preset_id);

        if !path.exists() {
            return Err(Status::not_found(format!(
                "Preset '{}' not found",
                preset_id
            )));
        }

        // Remove main file
        fs::remove_file(&path)
            .map_err(|e| Status::internal(format!("Failed to delete preset: {}", e)))?;

        // Remove hash file
        let hash_path = path.with_extension("json.sha256");
        let _ = fs::remove_file(hash_path);

        // Remove backups
        for i in 1..=self.max_backups {
            let backup = self.backup_path(preset_id, i);
            let _ = fs::remove_file(&backup);
            let _ = fs::remove_file(backup.with_extension("json.sha256"));
        }

        // Remove from manifest for O(1) listing
        self.remove_manifest_entry(preset_id)?;

        tracing::info!("Deleted preset '{}'", preset_id);
        Ok(())
    }

    /// Apply preset configurations to devices
    async fn apply_preset_to_devices(&self, preset: &Preset) -> Result<String, Status> {
        let registry = self.registry.read().await;
        let mut applied_count = 0;
        let mut errors = Vec::new();

        for (device_id, config_json) in &preset.device_configs_json {
            // Check if device exists
            let device_info = registry.get_device_info(device_id);
            if device_info.is_none() {
                errors.push(format!("Device '{}' not found", device_id));
                continue;
            }

            // Parse config
            let config: serde_json::Value = match serde_json::from_str(config_json) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("Invalid config for '{}': {}", device_id, e));
                    continue;
                }
            };

            // Apply position if present (for Movable devices)
            if let Some(pos) = config.get("position").and_then(|v| v.as_f64()) {
                if let Some(movable) = registry.get_movable(device_id) {
                    match movable.move_abs(pos).await {
                        Ok(_) => applied_count += 1,
                        Err(e) => errors.push(format!("Failed to move '{}': {}", device_id, e)),
                    }
                }
            }

            // Apply exposure if present (for cameras)
            if let Some(exp) = config.get("exposure_ms").and_then(|v| v.as_f64()) {
                if let Some(exposure_ctrl) = registry.get_exposure_control(device_id) {
                    match exposure_ctrl.set_exposure(exp).await {
                        Ok(_) => applied_count += 1,
                        Err(e) => {
                            errors.push(format!("Failed to set exposure '{}': {}", device_id, e))
                        }
                    }
                }
            }

            // 3. Generic parameters (Parameterized devices)
            if let Some(param_set) = registry.get_parameters(device_id) {
                if let Some(obj) = config.as_object() {
                    for (param_name, value) in obj {
                        // Skip hardcoded fields handled above
                        if param_name == "position" || param_name == "exposure_ms" {
                            continue;
                        }

                        if let Some(parameter) = param_set.get(param_name) {
                            match parameter.set_json(value.clone()) {
                                Ok(_) => applied_count += 1,
                                Err(e) => errors.push(format!(
                                    "Failed to set parameter '{}.{}': {}",
                                    device_id, param_name, e
                                )),
                            }
                        }
                    }
                }
            }
        }

        drop(registry);

        if errors.is_empty() {
            Ok(format!("Applied {} device configurations", applied_count))
        } else {
            Ok(format!(
                "Applied {} configurations with {} errors: {}",
                applied_count,
                errors.len(),
                errors.join("; ")
            ))
        }
    }
}

#[tonic::async_trait]
impl PresetService for PresetServiceImpl {
    async fn list_presets(
        &self,
        _request: Request<ListPresetsRequest>,
    ) -> Result<Response<ListPresetsResponse>, Status> {
        let presets = self.list_presets_from_disk()?;
        Ok(Response::new(ListPresetsResponse { presets }))
    }

    async fn save_preset(
        &self,
        request: Request<SavePresetRequest>,
    ) -> Result<Response<SavePresetResponse>, Status> {
        let req = request.into_inner();
        let preset = req
            .preset
            .ok_or_else(|| Status::invalid_argument("preset is required"))?;

        let preset_id = preset
            .meta
            .as_ref()
            .map(|m| m.preset_id.clone())
            .unwrap_or_default();

        // Check if exists and overwrite flag
        let path = self.preset_path(&preset_id);
        if path.exists() && !req.overwrite {
            return Ok(Response::new(SavePresetResponse {
                saved: false,
                message: format!(
                    "Preset '{}' already exists. Set overwrite=true to replace.",
                    preset_id
                ),
            }));
        }

        // Update timestamps
        let mut preset = preset;
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        if let Some(ref mut meta) = preset.meta {
            if meta.created_at_ns == 0 {
                meta.created_at_ns = now_ns;
            }
            meta.updated_at_ns = now_ns;
        }

        self.save_preset_to_disk(&preset)?;

        Ok(Response::new(SavePresetResponse {
            saved: true,
            message: format!("Preset '{}' saved successfully", preset_id),
        }))
    }

    async fn load_preset(
        &self,
        request: Request<LoadPresetRequest>,
    ) -> Result<Response<LoadPresetResponse>, Status> {
        let req = request.into_inner();
        let preset = self.load_preset_from_disk(&req.preset_id)?;

        // Apply configurations to devices
        let message = self.apply_preset_to_devices(&preset).await?;

        Ok(Response::new(LoadPresetResponse {
            applied: true,
            message,
        }))
    }

    async fn delete_preset(
        &self,
        request: Request<DeletePresetRequest>,
    ) -> Result<Response<DeletePresetResponse>, Status> {
        let req = request.into_inner();
        self.delete_preset_from_disk(&req.preset_id)?;

        Ok(Response::new(DeletePresetResponse {
            deleted: true,
            message: format!("Preset '{}' deleted", req.preset_id),
        }))
    }

    async fn get_preset(
        &self,
        request: Request<GetPresetRequest>,
    ) -> Result<Response<Preset>, Status> {
        let req = request.into_inner();
        let preset = self.load_preset_from_disk(&req.preset_id)?;
        Ok(Response::new(preset))
    }
}

/// Internal file format for preset storage
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PresetFile {
    schema_version: u32,
    preset_id: String,
    name: String,
    description: String,
    author: String,
    created_at_ns: u64,
    updated_at_ns: u64,
    device_configs: HashMap<String, serde_json::Value>,
    scan_template: Option<serde_json::Value>,
}

impl PresetFile {
    fn from_proto(preset: &Preset) -> Self {
        let meta = preset.meta.as_ref();
        Self {
            schema_version: meta.map(|m| m.schema_version).unwrap_or(1),
            preset_id: meta.map(|m| m.preset_id.clone()).unwrap_or_default(),
            name: meta.map(|m| m.name.clone()).unwrap_or_default(),
            description: meta.map(|m| m.description.clone()).unwrap_or_default(),
            author: meta.map(|m| m.author.clone()).unwrap_or_default(),
            created_at_ns: meta.map(|m| m.created_at_ns).unwrap_or(0),
            updated_at_ns: meta.map(|m| m.updated_at_ns).unwrap_or(0),
            device_configs: preset
                .device_configs_json
                .iter()
                .filter_map(|(k, v)| {
                    serde_json::from_str(v)
                        .ok()
                        .map(|parsed: serde_json::Value| (k.clone(), parsed))
                })
                .collect(),
            scan_template: if preset.scan_template_json.is_empty() {
                None
            } else {
                serde_json::from_str(&preset.scan_template_json).ok()
            },
        }
    }

    fn to_proto(&self) -> Preset {
        Preset {
            meta: Some(PresetMetadata {
                preset_id: self.preset_id.clone(),
                name: self.name.clone(),
                description: self.description.clone(),
                author: self.author.clone(),
                created_at_ns: self.created_at_ns,
                updated_at_ns: self.updated_at_ns,
                schema_version: self.schema_version,
            }),
            device_configs_json: self
                .device_configs
                .iter()
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect(),
            scan_template_json: self
                .scan_template
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        }
    }
}

/// Lightweight manifest entry for fast listing (stores metadata only)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ManifestEntry {
    preset_id: String,
    name: String,
    description: String,
    author: String,
    created_at_ns: u64,
    updated_at_ns: u64,
    schema_version: u32,
}

impl ManifestEntry {
    fn from_proto(meta: &PresetMetadata) -> Self {
        Self {
            preset_id: meta.preset_id.clone(),
            name: meta.name.clone(),
            description: meta.description.clone(),
            author: meta.author.clone(),
            created_at_ns: meta.created_at_ns,
            updated_at_ns: meta.updated_at_ns,
            schema_version: meta.schema_version,
        }
    }

    fn to_proto(&self) -> PresetMetadata {
        PresetMetadata {
            preset_id: self.preset_id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            author: self.author.clone(),
            created_at_ns: self.created_at_ns,
            updated_at_ns: self.updated_at_ns,
            schema_version: self.schema_version,
        }
    }
}

/// Default storage path for presets
pub fn default_preset_storage_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rust-daq")
        .join("presets")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_preset(id: &str) -> Preset {
        Preset {
            meta: Some(PresetMetadata {
                preset_id: id.to_string(),
                name: format!("Test Preset {}", id),
                description: "A test preset".to_string(),
                author: "test".to_string(),
                created_at_ns: 0,
                updated_at_ns: 0,
                schema_version: 1,
            }),
            device_configs_json: {
                let mut map = HashMap::new();
                map.insert("stage1".to_string(), r#"{"position": 10.5}"#.to_string());
                map
            },
            scan_template_json: String::new(),
        }
    }

    #[tokio::test]
    async fn test_save_and_load_preset() {
        let temp_dir = TempDir::new().unwrap();
        let registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let service = PresetServiceImpl::new(registry, temp_dir.path().to_path_buf());

        let preset = create_test_preset("test1");

        // Save
        service.save_preset_to_disk(&preset).unwrap();

        // Load
        let loaded = service.load_preset_from_disk("test1").unwrap();
        assert_eq!(
            loaded.meta.as_ref().unwrap().preset_id,
            preset.meta.as_ref().unwrap().preset_id
        );
        assert_eq!(
            loaded.meta.as_ref().unwrap().name,
            preset.meta.as_ref().unwrap().name
        );
    }

    #[tokio::test]
    async fn test_list_presets() {
        let temp_dir = TempDir::new().unwrap();
        let registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let service = PresetServiceImpl::new(registry, temp_dir.path().to_path_buf());

        // Save multiple presets
        service
            .save_preset_to_disk(&create_test_preset("preset_a"))
            .unwrap();
        service
            .save_preset_to_disk(&create_test_preset("preset_b"))
            .unwrap();

        let list = service.list_presets_from_disk().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_preset() {
        let temp_dir = TempDir::new().unwrap();
        let registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let service = PresetServiceImpl::new(registry, temp_dir.path().to_path_buf());

        service
            .save_preset_to_disk(&create_test_preset("to_delete"))
            .unwrap();
        assert!(service.load_preset_from_disk("to_delete").is_ok());

        service.delete_preset_from_disk("to_delete").unwrap();
        assert!(service.load_preset_from_disk("to_delete").is_err());
    }

    #[tokio::test]
    async fn test_backup_rotation() {
        let temp_dir = TempDir::new().unwrap();
        let registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let service =
            PresetServiceImpl::new(registry, temp_dir.path().to_path_buf()).with_max_backups(2);

        // Save same preset multiple times
        let mut preset = create_test_preset("rotate_test");
        service.save_preset_to_disk(&preset).unwrap();

        preset.meta.as_mut().unwrap().description = "v2".to_string();
        service.save_preset_to_disk(&preset).unwrap();

        preset.meta.as_mut().unwrap().description = "v3".to_string();
        service.save_preset_to_disk(&preset).unwrap();

        // Check backups exist
        assert!(service.backup_path("rotate_test", 1).exists());
        assert!(service.backup_path("rotate_test", 2).exists());
        // Backup 3 should not exist (max_backups=2)
        assert!(!service.backup_path("rotate_test", 3).exists());
    }

    #[tokio::test]
    async fn test_integrity_check() {
        let temp_dir = TempDir::new().unwrap();
        let registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let service = PresetServiceImpl::new(registry, temp_dir.path().to_path_buf());

        service
            .save_preset_to_disk(&create_test_preset("integrity"))
            .unwrap();

        // Corrupt the file
        let path = service.preset_path("integrity");
        fs::write(&path, "corrupted content").unwrap();

        // Load should fail integrity check
        let result = service.load_preset_from_disk("integrity");
        assert!(result.is_err());
        assert!(result.unwrap_err().message().contains("integrity"));
    }
}
