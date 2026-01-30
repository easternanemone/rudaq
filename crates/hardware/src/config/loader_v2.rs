use super::schema_v2::InstrumentManifest;
use anyhow::Result;
use std::path::Path;
use tracing::{debug, info, warn};

const V2_SUFFIXES: [&str; 2] = [".scpi.toml", ".declarative_scpi.toml"];

#[derive(Debug, thiserror::Error)]
pub enum ConfigV2LoadError {
    #[error("Config file not found: {0}")]
    NotFound(String),
    #[error("Failed to read config file: {0}")]
    ReadError(String),
    #[error("Failed to parse v2 config: {0}")]
    ParseError(String),
    #[error("V2 schema_version must be 2 (found {found}) in {path}")]
    SchemaVersionMismatch { path: String, found: u32 },
    #[error("V2 config filename must end with one of: {expected}. Path: {path}")]
    FilenameMismatch { path: String, expected: String },
    #[error("V2 config validation failed:\n{0}")]
    ValidationError(String),
}

pub fn is_v2_manifest_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    V2_SUFFIXES.iter().any(|suffix| file_name.ends_with(suffix))
}

pub fn load_device_manifest_v2(path: &Path) -> Result<InstrumentManifest> {
    let manifest = load_device_manifest_v2_raw(path)?;
    if !is_v2_manifest_path(path) {
        return Err(ConfigV2LoadError::FilenameMismatch {
            path: path.display().to_string(),
            expected: V2_SUFFIXES.join(", "),
        }
        .into());
    }
    validate_manifest_v2(manifest, path)
}

pub fn load_device_manifest_v2_raw(path: &Path) -> Result<InstrumentManifest> {
    if !path.exists() {
        return Err(ConfigV2LoadError::NotFound(path.display().to_string()).into());
    }
    let content =
        std::fs::read_to_string(path).map_err(|e| ConfigV2LoadError::ReadError(e.to_string()))?;
    let manifest: InstrumentManifest =
        toml::from_str(&content).map_err(|e| ConfigV2LoadError::ParseError(e.to_string()))?;
    Ok(manifest)
}

pub fn load_all_device_manifests_v2(dir: &Path) -> Result<Vec<InstrumentManifest>> {
    if !dir.exists() {
        return Err(ConfigV2LoadError::NotFound(dir.display().to_string()).into());
    }
    if !dir.is_dir() {
        return Err(anyhow::anyhow!(
            "Path is not a directory: {}",
            dir.display()
        ));
    }

    debug!("Loading v2 device manifests from: {}", dir.display());
    let mut manifests = Vec::new();
    let entries =
        std::fs::read_dir(dir).map_err(|e| ConfigV2LoadError::ReadError(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| ConfigV2LoadError::ReadError(e.to_string()))?;
        let path = entry.path();
        if !is_v2_manifest_path(&path) {
            continue;
        }
        match load_device_manifest_v2_raw(&path)
            .and_then(|manifest| validate_manifest_v2(manifest, &path))
        {
            Ok(manifest) => manifests.push(manifest),
            Err(err) => warn!("Failed to load v2 manifest {}: {}", path.display(), err),
        }
    }

    info!(
        "Loaded {} v2 device manifests from {}",
        manifests.len(),
        dir.display()
    );
    Ok(manifests)
}

fn validate_manifest_v2(manifest: InstrumentManifest, path: &Path) -> Result<InstrumentManifest> {
    if manifest.schema_version != 2 {
        return Err(ConfigV2LoadError::SchemaVersionMismatch {
            path: path.display().to_string(),
            found: manifest.schema_version,
        }
        .into());
    }

    if let Err(errors) = manifest.validate_strict() {
        let error_messages: Vec<String> =
            errors.to_string().lines().map(|s| s.to_string()).collect();
        return Err(ConfigV2LoadError::ValidationError(error_messages.join("\n")).into());
    }

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v2_suffix_detection() {
        assert!(is_v2_manifest_path(Path::new("device.scpi.toml")));
        assert!(is_v2_manifest_path(Path::new(
            "device.declarative_scpi.toml"
        )));
        assert!(!is_v2_manifest_path(Path::new("device.toml")));
    }
}
