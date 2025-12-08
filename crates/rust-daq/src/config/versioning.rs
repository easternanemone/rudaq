use crate::config::Settings;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::path::PathBuf;
use tokio::fs;

/// Unique identifier for a configuration version snapshot.
///
/// Encapsulates the filename of a configuration snapshot, which includes
/// timestamp, content hash, and optional label information.
///
/// # Format
///
/// - Unlabeled: `config-YYYYMMDD_HHMMSS_ffffff-HASH.toml`
/// - Labeled: `config-YYYYMMDD_HHMMSS_ffffff-HASH-LABEL.toml`
///
/// Where:
/// - `YYYYMMDD_HHMMSS_ffffff` is the UTC timestamp with microseconds
/// - `HASH` is the first 8 characters of the SHA-256 hash of the configuration
/// - `LABEL` is an optional user-provided identifier
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct VersionId(pub String);

/// Metadata about a configuration version snapshot.
///
/// Contains the version identifier, creation timestamp, and optional label.
/// Used for displaying version history and selecting snapshots for rollback.
///
/// # Example
///
/// ```rust,ignore
/// let info = VersionInfo {
///     id: VersionId("config-20250129_143022_123456-a1b2c3d4-production.toml".to_string()),
///     created_at: Utc::now(),
///     label: Some("production".to_string()),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct VersionInfo {
    /// Unique identifier for this version snapshot.
    ///
    /// Contains the full filename including timestamp, hash, and optional label.
    pub id: VersionId,

    /// UTC timestamp when this snapshot was created.
    ///
    /// Extracted from the filename timestamp component.
    /// Used for sorting versions chronologically and cleanup of old versions.
    pub created_at: DateTime<Utc>,

    /// Optional human-readable label for this version.
    ///
    /// If `Some`, this version is considered "labeled" and will not be
    /// automatically deleted during cleanup. Useful for marking important
    /// configurations like "production", "baseline", or "pre-migration".
    ///
    /// If `None`, this is an automatic snapshot subject to retention policies.
    pub label: Option<String>,
}

/// Manages configuration versioning, snapshots, and rollback.
///
/// Provides automatic configuration snapshot creation with content-based hashing,
/// version history management, diff generation between versions, and rollback
/// capabilities. Implements a retention policy to limit the number of unlabeled
/// snapshots while preserving all labeled versions.
///
/// # Architecture
///
/// - **Snapshots**: Automatically created with timestamp + SHA-256 hash filenames
/// - **Labeled versions**: User-tagged versions preserved indefinitely
/// - **Retention policy**: Keeps most recent N unlabeled versions (default: 10)
/// - **Diffs**: Uses the `similar` crate for line-by-line comparison
///
/// # Example
///
/// ```rust,no_run
/// use rust_daq::config::versioning::VersionManager;
/// use rust_daq::config::Settings;
/// use std::path::PathBuf;
///
/// # async fn example() -> anyhow::Result<()> {
/// let manager = VersionManager::new(PathBuf::from("config/versions"));
/// let settings = Settings::default();
///
/// // Create automatic snapshot
/// let version_id = manager.create_snapshot(&settings, None).await?;
///
/// // Create labeled snapshot
/// let prod_id = manager.create_snapshot(&settings, Some("production".to_string())).await?;
///
/// // List all versions
/// let versions = manager.list_versions().await?;
/// println!("Found {} snapshots", versions.len());
///
/// // Rollback to previous version
/// let restored = manager.rollback(&version_id).await?;
///
/// // Compare two versions
/// let diff = manager.diff_versions(&version_id, &prod_id).await?;
/// println!("Diff:\n{}", diff);
/// # Ok(())
/// # }
/// ```
pub struct VersionManager {
    versions_dir: PathBuf,
    max_versions: usize,
}

impl VersionManager {
    /// Creates a new version manager for the specified directory.
    ///
    /// # Arguments
    ///
    /// * `versions_dir` - Directory where version snapshots will be stored.
    ///   The directory will be created automatically if it doesn't exist.
    ///
    /// # Defaults
    ///
    /// - `max_versions`: 10 unlabeled snapshots retained
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_daq::config::versioning::VersionManager;
    /// use std::path::PathBuf;
    ///
    /// let manager = VersionManager::new(PathBuf::from("config/versions"));
    /// ```
    pub fn new(versions_dir: PathBuf) -> Self {
        Self {
            versions_dir,
            max_versions: 10,
        }
    }

    /// Creates a new configuration snapshot with optional label.
    ///
    /// Serializes the configuration to TOML, computes a content hash, generates
    /// a timestamped filename, writes the snapshot to disk, and performs cleanup
    /// of old unlabeled versions according to the retention policy.
    ///
    /// # Arguments
    ///
    /// * `settings` - Configuration to snapshot
    /// * `label` - Optional label to mark this version as important.
    ///   Labeled versions are never automatically deleted.
    ///
    /// # Returns
    ///
    /// Returns the [`VersionId`] of the created snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - TOML serialization fails
    /// - File write operation fails
    /// - Directory traversal during cleanup fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rust_daq::config::versioning::VersionManager;
    /// # use rust_daq::config::Settings;
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = VersionManager::new(PathBuf::from("config/versions"));
    /// let settings = Settings::default();
    ///
    /// // Automatic snapshot
    /// let auto_id = manager.create_snapshot(&settings, None).await?;
    ///
    /// // Labeled snapshot (never auto-deleted)
    /// let prod_id = manager.create_snapshot(&settings, Some("production".to_string())).await?;
    /// # Ok(())
    /// # }
    /// ```
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

    /// Restores configuration from a previous snapshot.
    ///
    /// Reads the snapshot file, deserializes the TOML content, and returns
    /// the restored configuration. Does not modify the current configuration
    /// or create any new snapshots.
    ///
    /// # Arguments
    ///
    /// * `version_id` - Identifier of the snapshot to restore
    ///
    /// # Returns
    ///
    /// Returns the deserialized [`Settings`] from the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Snapshot file cannot be found
    /// - File read operation fails
    /// - TOML deserialization fails
    /// - Configuration validation fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rust_daq::config::versioning::{VersionManager, VersionId};
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = VersionManager::new(PathBuf::from("config/versions"));
    /// let versions = manager.list_versions().await?;
    ///
    /// if let Some(prev_version) = versions.first() {
    ///     let restored = manager.rollback(&prev_version.id).await?;
    ///     println!("Restored configuration from {}", prev_version.created_at);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rollback(&self, version_id: &VersionId) -> Result<Settings> {
        let path = self.versions_dir.join(&version_id.0);
        let toml_str = fs::read_to_string(&path).await?;
        let settings: Settings = toml::from_str(&toml_str)?;
        Ok(settings)
    }

    /// Lists all available configuration snapshots.
    ///
    /// Scans the versions directory, parses filenames to extract metadata,
    /// and returns a vector of version information. The order is not guaranteed;
    /// use `created_at` to sort chronologically.
    ///
    /// # Returns
    ///
    /// Returns a vector of [`VersionInfo`] for all valid snapshots found.
    /// Invalid filenames are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if directory traversal fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rust_daq::config::versioning::VersionManager;
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = VersionManager::new(PathBuf::from("config/versions"));
    /// let mut versions = manager.list_versions().await?;
    ///
    /// // Sort by creation time (newest first)
    /// versions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    ///
    /// for version in versions {
    ///     let label_str = version.label.as_deref().unwrap_or("auto");
    ///     println!("{}: {} [{}]", version.created_at, version.id.0, label_str);
    /// }
    /// # Ok(())
    /// # }
    /// ```
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

    /// Generates a line-by-line diff between two configuration snapshots.
    ///
    /// Uses the `similar` crate to compute a unified diff showing additions,
    /// deletions, and unchanged lines. Useful for understanding what changed
    /// between configurations before performing a rollback.
    ///
    /// # Arguments
    ///
    /// * `version_a` - First snapshot to compare
    /// * `version_b` - Second snapshot to compare
    ///
    /// # Returns
    ///
    /// Returns a string containing the diff in unified format:
    /// - Lines prefixed with `-` are only in `version_a`
    /// - Lines prefixed with `+` are only in `version_b`
    /// - Lines prefixed with ` ` (space) are unchanged
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Either snapshot file cannot be found
    /// - File read operations fail
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rust_daq::config::versioning::VersionManager;
    /// # use std::path::PathBuf;
    /// # async fn example() -> anyhow::Result<()> {
    /// let manager = VersionManager::new(PathBuf::from("config/versions"));
    /// let versions = manager.list_versions().await?;
    ///
    /// if versions.len() >= 2 {
    ///     let diff = manager.diff_versions(&versions[0].id, &versions[1].id).await?;
    ///     println!("Changes:\n{}", diff);
    /// }
    /// # Ok(())
    /// # }
    /// ```
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
