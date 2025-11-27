use crate::config::Settings;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[allow(dead_code)]
pub struct VersionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct VersionInfo {
    pub id: VersionId,
    pub created_at: DateTime<Utc>,
    pub label: Option<String>,
}

pub struct VersionManager {
    versions_dir: PathBuf,
    max_versions: usize,
}

impl VersionManager {
    pub fn new(versions_dir: PathBuf) -> Self {
        Self {
            versions_dir,
            max_versions: 10,
        }
    }

    pub async fn create_snapshot(
        &self,
        settings: &Settings,
        label: Option<String>,
    ) -> Result<VersionId> {
        // 1. Compute timestamp and hash
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%f");
        let hash = self.compute_hash(settings);

        // 2. Generate filename
        let filename = if let Some(label) = &label {
            format!("config-{}-{}-{}.toml", timestamp, hash, label)
        } else {
            format!("config-{}-{}.toml", timestamp, hash)
        };

        // 3. Serialize to TOML
        let toml_str = toml::to_string_pretty(settings)?;

        // 4. Write to file
        let path = self.versions_dir.join(&filename);
        fs::write(&path, toml_str).await?;

        // 5. Cleanup old versions
        let mut versions = self.list_versions().await?;
        self.cleanup_old_versions(&mut versions).await?;

        Ok(VersionId(filename))
    }

    fn compute_hash(&self, settings: &Settings) -> String {
        let mut hasher = Sha256::new();
        let toml_str = toml::to_string(settings).unwrap();
        hasher.update(toml_str.as_bytes());
        format!("{:x}", hasher.finalize())[..8].to_string()
    }

    async fn cleanup_old_versions(&self, versions: &mut [VersionInfo]) -> Result<()> {
        versions.sort_by_key(|v| v.created_at);

        let unlabeled_versions: Vec<_> = versions.iter().filter(|v| v.label.is_none()).collect();

        if unlabeled_versions.len() > self.max_versions {
            let versions_to_delete = unlabeled_versions.len() - self.max_versions;
            for version in unlabeled_versions.iter().take(versions_to_delete) {
                let path = self.versions_dir.join(&version.id.0);
                fs::remove_file(path).await?;
            }
        }

        Ok(())
    }

    pub async fn rollback(&self, version_id: &VersionId) -> Result<Settings> {
        let path = self.versions_dir.join(&version_id.0);
        let toml_str = fs::read_to_string(&path).await?;
        let settings: Settings = toml::from_str(&toml_str)?;
        Ok(settings)
    }

    pub async fn list_versions(&self) -> Result<Vec<VersionInfo>> {
        let mut versions = Vec::new();
        let mut read_dir = fs::read_dir(&self.versions_dir).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                let filename = path.file_name().unwrap().to_string_lossy().to_string();
                if let Some(info) = self.parse_version_info(&filename).await {
                    versions.push(info);
                }
            }
        }

        Ok(versions)
    }

    async fn parse_version_info(&self, filename: &str) -> Option<VersionInfo> {
        // Expected format: config-YYYYMMDD_HHMMSS-hash.toml
        // or: config-YYYYMMDD_HHMMSS-hash-label.toml
        let name_without_ext = filename.strip_suffix(".toml")?;
        let parts: Vec<&str> = name_without_ext.splitn(4, '-').collect();

        if parts.len() < 3 || parts[0] != "config" {
            return None;
        }

        let created_at_str = parts[1];
        let created_at = chrono::NaiveDateTime::parse_from_str(created_at_str, "%Y%m%d_%H%M%S_%f")
            .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
            .ok()?;

        let label = if parts.len() > 3 {
            Some(parts[3].to_string())
        } else {
            None
        };

        Some(VersionInfo {
            id: VersionId(filename.to_string()),
            created_at,
            label,
        })
    }

    pub async fn diff_versions(
        &self,
        version_a: &VersionId,
        version_b: &VersionId,
    ) -> Result<String> {
        let path_a = self.versions_dir.join(&version_a.0);
        let path_b = self.versions_dir.join(&version_b.0);

        let content_a = fs::read_to_string(&path_a).await?;
        let content_b = fs::read_to_string(&path_b).await?;

        let diff = TextDiff::from_lines(&content_a, &content_b);
        let mut diff_text = String::new();
        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            diff_text.push_str(&format!("{}{}", sign, change));
        }

        Ok(diff_text)
    }
}
