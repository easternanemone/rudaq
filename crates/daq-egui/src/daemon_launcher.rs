//! Daemon process launcher and manager.
//!
//! This module provides functionality to auto-start and manage the rust-daq-daemon
//! process from the GUI application.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Daemon mode configuration determining how the GUI connects to a daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonMode {
    /// Auto-start a local daemon on the specified port (mock hardware by default)
    LocalAuto { port: u16 },
    /// Connect to a remote daemon at the specified URL (no auto-start)
    Remote { url: String },
    /// Auto-start a local daemon with lab hardware configuration
    LabHardware { port: u16 },
}

impl Default for DaemonMode {
    fn default() -> Self {
        Self::LocalAuto { port: 50051 }
    }
}

impl DaemonMode {
    /// Get the daemon URL for this mode
    pub fn daemon_url(&self) -> String {
        match self {
            Self::LocalAuto { port } | Self::LabHardware { port } => {
                format!("http://127.0.0.1:{}", port)
            }
            Self::Remote { url } => url.clone(),
        }
    }

    /// Check if this mode requires auto-starting a local daemon
    pub fn should_auto_start(&self) -> bool {
        matches!(self, Self::LocalAuto { .. } | Self::LabHardware { .. })
    }

    /// Get a human-readable label for this mode
    pub fn label(&self) -> &'static str {
        match self {
            Self::LocalAuto { .. } => "Local (Mock)",
            Self::Remote { .. } => "Remote",
            Self::LabHardware { .. } => "Lab Hardware",
        }
    }

    /// Get the port number if applicable
    pub fn port(&self) -> Option<u16> {
        match self {
            Self::LocalAuto { port } | Self::LabHardware { port } => Some(*port),
            Self::Remote { .. } => None,
        }
    }
}

/// Auto-connect lifecycle state machine
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AutoConnectState {
    /// Waiting for daemon process to start
    WaitingForDaemon { since: Instant },
    /// Ready to initiate connection
    ReadyToConnect,
    /// Auto-connect process is complete (now in manual mode)
    #[default]
    Complete,
    /// Auto-connect was skipped (remote mode or user intervention)
    Skipped,
}

/// Manages a local daemon process lifecycle
pub struct DaemonLauncher {
    /// Child process handle
    child: Option<Child>,
    /// Target port for the daemon
    port: u16,
    /// When the daemon was started
    started_at: Option<Instant>,
    /// Last error message encountered
    last_error: Option<String>,
}

impl DaemonLauncher {
    /// Create a new daemon launcher for the specified port
    pub fn new(port: u16) -> Self {
        Self {
            child: None,
            port,
            started_at: None,
            last_error: None,
        }
    }

    /// Find the daemon binary path
    fn find_daemon_binary() -> Result<std::path::PathBuf, String> {
        // Try same directory as the GUI binary first
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let daemon_path = exe_dir.join("rust-daq-daemon");
                if daemon_path.exists() {
                    return Ok(daemon_path);
                }
            }
        }

        // Fall back to searching in PATH
        which::which("rust-daq-daemon")
            .map_err(|_| "rust-daq-daemon binary not found in PATH or alongside GUI".to_string())
    }

    /// Start the daemon process based on the specified mode
    pub fn start_with_mode(&mut self, mode: &DaemonMode) -> Result<(), String> {
        match mode {
            DaemonMode::LocalAuto { .. } => self.start(),
            DaemonMode::LabHardware { .. } => self.start_with_lab_hardware(),
            DaemonMode::Remote { .. } => Err(
                "Cannot start daemon in Remote mode (should connect to existing daemon)"
                    .to_string(),
            ),
        }
    }

    /// Start the daemon process with mock hardware (default)
    pub fn start(&mut self) -> Result<(), String> {
        if self.is_running() {
            tracing::debug!("Daemon already running on port {}", self.port);
            return Ok(());
        }

        let daemon_bin = Self::find_daemon_binary()?;

        tracing::info!(
            "Starting daemon: {} daemon --port {}",
            daemon_bin.display(),
            self.port
        );

        let child = Command::new(&daemon_bin)
            .arg("daemon")
            .arg("--port")
            .arg(self.port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start daemon: {}", e))?;

        self.child = Some(child);
        self.started_at = Some(Instant::now());
        self.last_error = None;

        tracing::info!("Started local daemon on port {}", self.port);
        Ok(())
    }

    /// Start the daemon with lab hardware configuration
    pub fn start_with_lab_hardware(&mut self) -> Result<(), String> {
        if self.is_running() {
            tracing::debug!(
                "Daemon already running on port {} (lab hardware mode)",
                self.port
            );
            return Ok(());
        }

        let daemon_bin = Self::find_daemon_binary()?;

        tracing::info!(
            "Starting daemon with lab hardware: {} daemon --port {} --lab-hardware",
            daemon_bin.display(),
            self.port
        );

        let child = Command::new(&daemon_bin)
            .arg("daemon")
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--lab-hardware")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start daemon with lab hardware: {}", e))?;

        self.child = Some(child);
        self.started_at = Some(Instant::now());
        self.last_error = None;

        tracing::info!(
            "Started local daemon on port {} with lab hardware configuration",
            self.port
        );
        Ok(())
    }

    /// Check if the daemon process is currently running
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(None) => true, // Still running
                Ok(Some(status)) => {
                    tracing::warn!("Daemon process exited with status: {}", status);
                    self.last_error = Some(format!("Daemon exited: {}", status));
                    self.child = None;
                    false
                }
                Err(e) => {
                    tracing::error!("Failed to check daemon status: {}", e);
                    self.last_error = Some(format!("Failed to check daemon: {}", e));
                    false
                }
            }
        } else {
            false
        }
    }

    /// Stop the daemon process gracefully
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            tracing::info!("Stopping local daemon");
            // Try graceful termination first
            let _ = child.kill();
            // Wait for process to exit
            match child.wait() {
                Ok(status) => {
                    tracing::info!("Daemon stopped with status: {}", status);
                }
                Err(e) => {
                    tracing::warn!("Error waiting for daemon to stop: {}", e);
                }
            }
            self.started_at = None;
        }
    }

    /// Get the last error message if any
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Get the uptime of the daemon process
    pub fn uptime(&self) -> Option<Duration> {
        self.started_at.map(|t| t.elapsed())
    }

    /// Get the port this launcher targets
    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for DaemonLauncher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_mode_default() {
        let mode = DaemonMode::default();
        assert_eq!(mode, DaemonMode::LocalAuto { port: 50051 });
        assert!(mode.should_auto_start());
        assert_eq!(mode.daemon_url(), "http://127.0.0.1:50051");
    }

    #[test]
    fn test_daemon_mode_remote() {
        let mode = DaemonMode::Remote {
            url: "http://example.com:8080".to_string(),
        };
        assert!(!mode.should_auto_start());
        assert_eq!(mode.daemon_url(), "http://example.com:8080");
        assert_eq!(mode.label(), "Remote");
    }

    #[test]
    fn test_daemon_mode_lab_hardware() {
        let mode = DaemonMode::LabHardware { port: 50052 };
        // Lab hardware mode should auto-start with --lab-hardware flag
        assert!(mode.should_auto_start());
        assert_eq!(mode.daemon_url(), "http://127.0.0.1:50052");
        assert_eq!(mode.label(), "Lab Hardware");
        assert_eq!(mode.port(), Some(50052));
    }

    #[test]
    fn test_daemon_launcher_new() {
        let launcher = DaemonLauncher::new(50051);
        assert_eq!(launcher.port(), 50051);
        assert!(launcher.last_error().is_none());
        assert!(launcher.uptime().is_none());
    }

    #[test]
    fn test_start_with_mode_rejects_remote() {
        let mut launcher = DaemonLauncher::new(50051);
        let remote_mode = DaemonMode::Remote {
            url: "http://example.com:50051".to_string(),
        };
        let result = launcher.start_with_mode(&remote_mode);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Cannot start daemon in Remote mode"));
    }
}
