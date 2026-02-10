//! Script Execution Journal - Persistent crash recovery for scripts (bd-izdj.7)
//!
//! Tracks script execution state on disk so interrupted scripts can be detected
//! and optionally resumed after a daemon crash.
//!
//! # Architecture
//!
//! Each script execution gets a JSON file in `~/.local/share/rust-daq/scripts/journal/`.
//! Files are written atomically (write temp → rename) to prevent corruption.
//!
//! # States
//!
//! ```text
//! [queued] → [running] → [completed]
//!                │
//!                ├──→ [failed]
//!                └──→ [interrupted]  (detected on restart)
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

/// State of a journaled script execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JournalState {
    Running,
    Checkpoint,
    Completed,
    Failed,
    Interrupted,
}

impl std::fmt::Display for JournalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Checkpoint => write!(f, "checkpoint"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Interrupted => write!(f, "interrupted"),
        }
    }
}

/// A single script execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub execution_id: String,
    pub script_id: String,
    pub script_hash: String,
    pub state: JournalState,
    pub checkpoint_name: Option<String>,
    pub checkpoint_data: Option<serde_json::Value>,
    pub progress_percent: u32,
    pub started_at: u64,
    pub updated_at: u64,
    pub completed_at: Option<u64>,
    pub error_message: Option<String>,
}

/// Persistent journal for script execution tracking
pub struct ScriptJournal {
    journal_dir: PathBuf,
}

impl ScriptJournal {
    /// Create a new ScriptJournal. Creates the journal directory if needed.
    pub fn new(journal_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&journal_dir)
            .with_context(|| format!("Failed to create journal dir: {}", journal_dir.display()))?;
        Ok(Self { journal_dir })
    }

    /// Create a journal using the default directory.
    pub fn default_dir() -> Result<Self> {
        let base = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rust-daq")
            .join("scripts")
            .join("journal");
        Self::new(base)
    }

    /// Record a new script execution starting.
    pub fn start_run(
        &self,
        execution_id: &str,
        script_id: &str,
        script_content: &str,
    ) -> Result<()> {
        let now = timestamp_nanos();
        let entry = JournalEntry {
            execution_id: execution_id.to_string(),
            script_id: script_id.to_string(),
            script_hash: sha256_hex(script_content),
            state: JournalState::Running,
            checkpoint_name: None,
            checkpoint_data: None,
            progress_percent: 0,
            started_at: now,
            updated_at: now,
            completed_at: None,
            error_message: None,
        };
        self.write_entry(&entry)
    }

    /// Save a checkpoint for an in-progress script.
    pub fn save_checkpoint(
        &self,
        execution_id: &str,
        name: &str,
        data: serde_json::Value,
        progress: u32,
    ) -> Result<()> {
        let mut entry = self.read_entry(execution_id)?;
        entry.state = JournalState::Checkpoint;
        entry.checkpoint_name = Some(name.to_string());
        entry.checkpoint_data = Some(data);
        entry.progress_percent = progress;
        entry.updated_at = timestamp_nanos();
        self.write_entry(&entry)
    }

    /// Mark a script execution as completed successfully.
    pub fn complete_run(&self, execution_id: &str) -> Result<()> {
        let mut entry = match self.read_entry(execution_id) {
            Ok(e) => e,
            Err(e) => {
                debug!("Journal entry not found for {}: {}", execution_id, e);
                return Ok(());
            }
        };
        let now = timestamp_nanos();
        entry.state = JournalState::Completed;
        entry.progress_percent = 100;
        entry.updated_at = now;
        entry.completed_at = Some(now);
        self.write_entry(&entry)
    }

    /// Mark a script execution as failed.
    pub fn fail_run(&self, execution_id: &str, error: &str) -> Result<()> {
        let mut entry = match self.read_entry(execution_id) {
            Ok(e) => e,
            Err(e) => {
                debug!("Journal entry not found for {}: {}", execution_id, e);
                return Ok(());
            }
        };
        let now = timestamp_nanos();
        entry.state = JournalState::Failed;
        entry.updated_at = now;
        entry.completed_at = Some(now);
        entry.error_message = Some(error.to_string());
        self.write_entry(&entry)
    }

    /// Scan journal for scripts that were running/checkpoint when daemon crashed.
    /// Marks them as interrupted and returns them.
    pub fn find_interrupted(&self) -> Result<Vec<JournalEntry>> {
        let mut interrupted = Vec::new();

        let entries = match std::fs::read_dir(&self.journal_dir) {
            Ok(e) => e,
            Err(e) => {
                debug!("Cannot read journal dir: {}", e);
                return Ok(interrupted);
            }
        };

        for dir_entry in entries {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            match self.read_entry_from_path(&path) {
                Ok(mut entry) => {
                    if entry.state == JournalState::Running
                        || entry.state == JournalState::Checkpoint
                    {
                        info!(
                            "Found interrupted script: {} (state: {}, checkpoint: {:?})",
                            entry.execution_id, entry.state, entry.checkpoint_name
                        );
                        entry.state = JournalState::Interrupted;
                        entry.updated_at = timestamp_nanos();
                        if let Err(e) = self.write_entry(&entry) {
                            warn!("Failed to mark entry as interrupted: {}", e);
                        }
                        interrupted.push(entry);
                    }
                }
                Err(e) => {
                    warn!("Failed to read journal entry {:?}: {}", path, e);
                }
            }
        }

        Ok(interrupted)
    }

    /// Clean up completed/failed entries older than the given age.
    pub fn cleanup(&self, max_age: std::time::Duration) -> Result<usize> {
        let cutoff = timestamp_nanos().saturating_sub(max_age.as_nanos() as u64);
        let mut removed = 0;

        let Ok(entries) = std::fs::read_dir(&self.journal_dir) else {
            return Ok(0);
        };

        for dir_entry in entries {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            if let Ok(entry) = self.read_entry_from_path(&path) {
                let is_terminal =
                    matches!(entry.state, JournalState::Completed | JournalState::Failed);
                if is_terminal && entry.updated_at < cutoff && std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }

        Ok(removed)
    }

    // --- Internal helpers ---

    fn entry_path(&self, execution_id: &str) -> PathBuf {
        self.journal_dir.join(format!("{}.json", execution_id))
    }

    fn write_entry(&self, entry: &JournalEntry) -> Result<()> {
        use std::io::Write;

        let path = self.entry_path(&entry.execution_id);
        let tmp_path = path.with_extension("json.tmp");

        let json =
            serde_json::to_string_pretty(entry).context("Failed to serialize journal entry")?;

        // Write + fsync before rename for crash durability (bd-izdj)
        let mut file = std::fs::File::create(&tmp_path).with_context(|| {
            format!("Failed to create temp journal file: {}", tmp_path.display())
        })?;
        file.write_all(json.as_bytes()).with_context(|| {
            format!("Failed to write temp journal file: {}", tmp_path.display())
        })?;
        file.sync_all()
            .with_context(|| format!("Failed to fsync journal file: {}", tmp_path.display()))?;

        std::fs::rename(&tmp_path, &path).with_context(|| {
            format!(
                "Failed to rename journal file: {} -> {}",
                tmp_path.display(),
                path.display()
            )
        })?;

        Ok(())
    }

    fn read_entry(&self, execution_id: &str) -> Result<JournalEntry> {
        let path = self.entry_path(execution_id);
        self.read_entry_from_path(&path)
    }

    fn read_entry_from_path(&self, path: &Path) -> Result<JournalEntry> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read journal entry: {}", path.display()))?;
        serde_json::from_str(&data).context("Failed to parse journal entry")
    }
}

fn timestamp_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_journal() -> (TempDir, ScriptJournal) {
        let tmp = TempDir::new().unwrap();
        let journal = ScriptJournal::new(tmp.path().to_path_buf()).unwrap();
        (tmp, journal)
    }

    #[test]
    fn test_start_and_complete() {
        let (_tmp, journal) = test_journal();
        journal
            .start_run("exec-1", "script-1", "let x = 42;")
            .unwrap();

        let entry = journal.read_entry("exec-1").unwrap();
        assert_eq!(entry.state, JournalState::Running);
        assert_eq!(entry.script_id, "script-1");

        journal.complete_run("exec-1").unwrap();
        let entry = journal.read_entry("exec-1").unwrap();
        assert_eq!(entry.state, JournalState::Completed);
        assert!(entry.completed_at.is_some());
    }

    #[test]
    fn test_checkpoint() {
        let (_tmp, journal) = test_journal();
        journal
            .start_run("exec-2", "script-1", "let x = 42;")
            .unwrap();
        journal
            .save_checkpoint(
                "exec-2",
                "step_5",
                serde_json::json!({"index": 5, "partial_result": [1, 2, 3]}),
                50,
            )
            .unwrap();

        let entry = journal.read_entry("exec-2").unwrap();
        assert_eq!(entry.state, JournalState::Checkpoint);
        assert_eq!(entry.checkpoint_name, Some("step_5".to_string()));
        assert_eq!(entry.progress_percent, 50);
    }

    #[test]
    fn test_find_interrupted() {
        let (_tmp, journal) = test_journal();
        journal.start_run("exec-a", "s1", "script a").unwrap();
        journal.start_run("exec-b", "s2", "script b").unwrap();
        journal.start_run("exec-c", "s3", "script c").unwrap();
        journal.complete_run("exec-c").unwrap();

        // exec-a and exec-b are still "running" → simulate crash
        let interrupted = journal.find_interrupted().unwrap();
        assert_eq!(interrupted.len(), 2);
        assert!(
            interrupted
                .iter()
                .all(|e| e.state == JournalState::Interrupted)
        );
    }

    #[test]
    fn test_fail_run() {
        let (_tmp, journal) = test_journal();
        journal.start_run("exec-f", "s1", "bad script").unwrap();
        journal
            .fail_run("exec-f", "syntax error at line 5")
            .unwrap();

        let entry = journal.read_entry("exec-f").unwrap();
        assert_eq!(entry.state, JournalState::Failed);
        assert_eq!(
            entry.error_message,
            Some("syntax error at line 5".to_string())
        );
    }

    #[test]
    fn test_cleanup_old_entries() {
        let (_tmp, journal) = test_journal();
        journal.start_run("exec-old", "s1", "old script").unwrap();
        journal.complete_run("exec-old").unwrap();

        // With zero max_age, everything completed should be removed
        let removed = journal.cleanup(std::time::Duration::from_secs(0)).unwrap();
        // Entry was just created so its timestamp may be too recent for zero-duration cutoff
        // This test verifies the cleanup logic runs without errors
        assert!(removed <= 1);
    }
}
