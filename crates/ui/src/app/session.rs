//! Session management - crash detection, session files, data integrity helpers.
//!
//! All functions are desktop-only (`cfg(not(target_arch = "wasm32"))`).

#[cfg(not(target_arch = "wasm32"))]
/// Directory for session state files (bd-izdj.30)
pub(super) fn session_dir() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rust-daq")
}

#[cfg(not(target_arch = "wasm32"))]
/// Per-process session file path to avoid cross-process races (bd-izdj.30)
pub(super) fn session_file_path() -> std::path::PathBuf {
    session_dir().join(format!("gui_session_{}.json", std::process::id()))
}

#[cfg(not(target_arch = "wasm32"))]
/// Check if a PID is still alive
pub(super) fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks process existence without sending a signal
        // SAFETY: kill(pid, 0) is a standard POSIX existence check with no side effects
        #[allow(unsafe_code)]
        #[allow(clippy::cast_possible_wrap)]
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
        alive
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        // Conservative: assume alive on non-Unix platforms
        true
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Write session state to disk atomically (marks GUI as running)
pub(super) fn write_session_file(daemon_url: &str) {
    let path = session_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let session = serde_json::json!({
        "running": true,
        "daemon_url": daemon_url,
        "pid": std::process::id(),
        "started_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });
    // Atomic write: temp file + rename to prevent partial/corrupt reads
    let tmp_path = path.with_extension("json.tmp");
    if std::fs::write(&tmp_path, session.to_string()).is_err() {
        tracing::warn!("Failed to write session temp file: {}", tmp_path.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        tracing::warn!("Failed to rename session file: {}", e);
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Check if a previous session crashed by scanning for session files
/// with running=true whose PID is no longer alive.
pub(super) fn check_crashed_session() -> Option<String> {
    let dir = session_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!("Cannot read session dir: {}", e);
            return None;
        }
    };

    let my_pid = std::process::id();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("gui_session_")
            || !std::path::Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            continue;
        }

        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to read session file {}: {}", path.display(), e);
                continue;
            }
        };
        let session: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse session file {}: {}", path.display(), e);
                continue;
            }
        };

        let was_running = session
            .get("running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !was_running {
            continue;
        }

        #[allow(clippy::cast_possible_truncation)]
        let pid = session.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        // Skip our own session file
        if pid == my_pid {
            continue;
        }

        // If the PID is no longer alive, this was a crashed session
        if !is_pid_alive(pid) {
            // Clean up the stale session file
            let _ = std::fs::remove_file(&path);
            return session
                .get("daemon_url")
                .and_then(|u| u.as_str())
                .map(|s| s.to_string());
        }
    }

    None
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn data_integrity_status_message(
    event: &crate::gui_log_layer::GuiLogEvent,
) -> Option<(crate::widgets::StatusLevel, String)> {
    let is_statusworthy_event = matches!(
        event.target.as_str(),
        "data_integrity" | "resource_pressure"
    ) || event.message.contains("DataIntegrityFault")
        || event.message.contains("ResourcePressureEvent");
    if !is_statusworthy_event {
        return None;
    }

    let level = match event.level {
        crate::panels::LogLevel::Error => crate::widgets::StatusLevel::Error,
        crate::panels::LogLevel::Warn => crate::widgets::StatusLevel::Warning,
        _ => return None,
    };

    Some((level, event.message.clone()))
}

#[cfg(not(target_arch = "wasm32"))]
/// Remove only our own session file (marks clean shutdown)
pub(super) fn clear_session_file() {
    let path = session_file_path();
    if let Err(e) = std::fs::remove_file(&path) {
        // ENOENT is fine — file may not exist if write_session_file was never called
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("Failed to remove session file: {}", e);
        }
    }
}
