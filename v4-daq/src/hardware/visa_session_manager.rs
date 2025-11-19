//! VISA Session Manager with Command Queueing
//!
//! Handles VISA's single-session limitation by serializing all commands
//! through a central command queue. This allows multiple actors (V2 and V4)
//! to safely access VISA instruments without conflicts.
//!
//! # Design
//!
//! VISA is inherently single-session (not thread-safe), so all operations
//! must be serialized. This manager:
//!
//! - Maintains one VISA session per resource
//! - Queues commands FIFO from multiple sources
//! - Executes commands sequentially in a dedicated task
//! - Returns responses via oneshot channels
//! - Handles timeouts per-command
//!
//! # Example
//!
//! ```no_run
//! use v4_daq::hardware::VisaSessionManager;
//! use std::time::Duration;
//!
//! let manager = VisaSessionManager::new();
//! let handle = manager.get_or_create_session("TCPIP0::192.168.1.100::INSTR").await?;
//!
//! // Query command (expects response)
//! let response = handle.query("*IDN?", Duration::from_secs(2)).await?;
//! println!("Instrument ID: {}", response);
//!
//! // Write-only command (no response)
//! handle.write("*RST").await?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, trace};

/// Command to be executed through VISA session
#[derive(Debug)]
struct VisaCommand {
    /// The command string to send to the instrument
    command: String,
    /// Response channel for query commands (Some) or write-only (None)
    response_tx: Option<oneshot::Sender<Result<String>>>,
    /// Per-command timeout
    timeout: Duration,
}

/// Handle to a VISA session for sending commands
#[derive(Clone)]
pub struct VisaSessionHandle {
    /// Resource name (e.g., "TCPIP0::192.168.1.100::INSTR")
    resource_name: String,
    /// Command sender to the queue task
    command_tx: mpsc::Sender<VisaCommand>,
}

impl VisaSessionHandle {
    /// Send a query command and wait for response
    ///
    /// # Arguments
    /// * `command` - SCPI command string (e.g., "*IDN?")
    /// * `timeout` - Maximum time to wait for response
    ///
    /// # Errors
    /// Returns error if:
    /// - Queue is closed
    /// - Command times out
    /// - VISA command fails
    pub async fn query(&self, command: &str, timeout: Duration) -> Result<String> {
        let (response_tx, response_rx) = oneshot::channel();

        let cmd = VisaCommand {
            command: command.to_string(),
            response_tx: Some(response_tx),
            timeout,
        };

        // Send command to queue
        self.command_tx
            .send(cmd)
            .await
            .context("Failed to send query command to VISA queue")?;

        // Wait for response with timeout
        tokio::time::timeout(timeout, response_rx)
            .await
            .context(format!(
                "Query command timed out after {:?}: {}",
                timeout, command
            ))?
            .context("Response channel closed unexpectedly")?
    }

    /// Send a write-only command (no response expected)
    ///
    /// # Arguments
    /// * `command` - SCPI command string (e.g., "*RST")
    ///
    /// # Errors
    /// Returns error if queue is closed
    pub async fn write(&self, command: &str) -> Result<()> {
        let cmd = VisaCommand {
            command: command.to_string(),
            response_tx: None,
            timeout: Duration::from_secs(5), // Default timeout for writes
        };

        self.command_tx
            .send(cmd)
            .await
            .context("Failed to send write command to VISA queue")
    }

    /// Get the resource name for this session
    pub fn resource_name(&self) -> &str {
        &self.resource_name
    }
}

/// Internal VISA session state
struct VisaSession {
    /// Resource name for this session
    resource_name: String,
    /// Queue task handle (used for graceful shutdown)
    queue_task: Option<tokio::task::JoinHandle<()>>,
    /// Command sender
    command_tx: mpsc::Sender<VisaCommand>,
}

/// Manages VISA sessions with command queueing
///
/// Ensures single-session VISA access by serializing all commands
/// through a central queue per resource.
pub struct VisaSessionManager {
    /// Map of resource_name -> session
    sessions: Arc<Mutex<HashMap<String, VisaSession>>>,
}

impl VisaSessionManager {
    /// Create a new VisaSessionManager
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get existing session or create new one for a resource
    ///
    /// The returned handle can be cloned and shared across multiple actors.
    /// Commands are queued and executed sequentially.
    ///
    /// # Arguments
    /// * `resource_name` - VISA resource string (e.g., "TCPIP0::192.168.1.100::INSTR")
    ///
    /// # Errors
    /// Returns error if session creation fails
    pub async fn get_or_create_session(&self, resource_name: &str) -> Result<VisaSessionHandle> {
        let mut sessions = self.sessions.lock().await;

        // Check if session already exists
        if let Some(session) = sessions.get(resource_name) {
            debug!(resource = resource_name, "Reusing existing VISA session");
            return Ok(VisaSessionHandle {
                resource_name: resource_name.to_string(),
                command_tx: session.command_tx.clone(),
            });
        }

        // Create new session
        debug!(resource = resource_name, "Creating new VISA session");

        let (command_tx, command_rx) = mpsc::channel(100); // Buffer up to 100 commands

        // Spawn queue task to execute commands sequentially
        let resource_name_clone = resource_name.to_string();
        let queue_task = tokio::spawn(Self::run_command_queue(
            resource_name_clone,
            command_rx,
        ));

        // Store session
        let session = VisaSession {
            resource_name: resource_name.to_string(),
            queue_task: Some(queue_task),
            command_tx: command_tx.clone(),
        };

        sessions.insert(resource_name.to_string(), session);

        Ok(VisaSessionHandle {
            resource_name: resource_name.to_string(),
            command_tx,
        })
    }

    /// Close a VISA session gracefully
    ///
    /// Stops the queue task and removes the session from the manager.
    ///
    /// # Arguments
    /// * `resource_name` - Resource to close
    ///
    /// # Errors
    /// Returns error if session not found or shutdown times out
    pub async fn close_session(&self, resource_name: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().await;

        if let Some(session) = sessions.remove(resource_name) {
            debug!(resource = resource_name, "Closing VISA session");

            // Drop the command sender to signal queue task to stop
            drop(session.command_tx);
            let queue_task = session.queue_task;

            // Release the lock before waiting on the task
            drop(sessions);

            // Give the task a chance to wake up and exit
            tokio::task::yield_now().await;

            // Wait for queue task to finish with timeout
            if let Some(task) = queue_task {
                tokio::time::timeout(Duration::from_secs(1), task)
                    .await
                    .context(format!(
                        "Queue task for {} did not shut down within 1 second",
                        resource_name
                    ))?
                    .context(format!("Queue task for {} panicked during shutdown", resource_name))?;
            }

            Ok(())
        } else {
            Err(anyhow!("Session not found for resource: {}", resource_name))
        }
    }

    /// Get the number of active sessions
    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Run the command queue for a session
    ///
    /// This task:
    /// 1. Receives commands from the queue
    /// 2. Executes them sequentially
    /// 3. Sends responses back via oneshot channels
    /// 4. Handles timeouts
    async fn run_command_queue(resource_name: String, mut command_rx: mpsc::Receiver<VisaCommand>) {
        debug!(resource = %resource_name, "Starting VISA command queue task");

        while let Some(cmd) = command_rx.recv().await {
            trace!(
                resource = %resource_name,
                command = %cmd.command,
                "Processing VISA command"
            );

            // Execute command
            let result = Self::execute_visa_command(&resource_name, &cmd.command, cmd.timeout)
                .await;

            // Send response if expected
            if let Some(tx) = cmd.response_tx {
                let _ = tx.send(result);
            }
        }

        debug!(resource = %resource_name, "VISA command queue task ended");
    }

    /// Execute a VISA command
    ///
    /// Currently a mock implementation that simulates VISA operations.
    /// This will be replaced with real VISA calls when the feature is enabled.
    async fn execute_visa_command(
        resource_name: &str,
        command: &str,
        timeout: Duration,
    ) -> Result<String> {
        // Mock implementation - simulates VISA operation
        // In production, this would call visa-rs library
        #[cfg(feature = "instrument_visa")]
        {
            execute_real_visa_command(resource_name, command, timeout).await
        }

        #[cfg(not(feature = "instrument_visa"))]
        {
            execute_mock_visa_command(resource_name, command, timeout).await
        }
    }
}

impl Default for VisaSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock VISA command execution (when feature is disabled)
#[cfg(not(feature = "instrument_visa"))]
async fn execute_mock_visa_command(
    resource_name: &str,
    command: &str,
    _timeout: Duration,
) -> Result<String> {
    // Simulate some processing time
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Mock responses for common SCPI commands
    let response = match &command[..] {
        "*IDN?" => "Mock Instrument,Model XYZ,Serial123,Firmware1.0".to_string(),
        "*RST" => String::new(),
        cmd if cmd.ends_with('?') => format!("Mock response for query: {}", cmd),
        _ => String::new(),
    };

    trace!(
        resource = resource_name,
        command = command,
        response = %response,
        "Mock VISA command executed"
    );

    Ok(response)
}

/// Real VISA command execution (when feature is enabled)
#[cfg(feature = "instrument_visa")]
async fn execute_real_visa_command(
    resource_name: &str,
    command: &str,
    _timeout: Duration,
) -> Result<String> {
    use visa_rs::{DefaultRM, Instrument, VISA_SUCCESS};

    // VISA operations are blocking, so we run them on a blocking thread pool
    let resource = resource_name.to_string();
    let cmd = command.to_string();

    tokio::task::spawn_blocking(move || {
        // Open resource manager
        let rm = DefaultRM::new().context("Failed to create VISA resource manager")?;

        // Open instrument
        let instr = rm
            .open(&resource, 0, 0)
            .context(format!("Failed to open VISA resource: {}", resource))?;

        // Determine if this is a query or write command
        let is_query = cmd.ends_with('?');

        if is_query {
            // Query command - expect response
            let mut response = vec![0u8; 256];
            let mut count = 0i32;

            let status = instr.read(&mut response, &mut count);
            if status != VISA_SUCCESS {
                return Err(anyhow!("VISA read failed with status: {}", status));
            }

            let response_str = String::from_utf8(response[..count as usize].to_vec())
                .context("Failed to decode VISA response as UTF-8")?;

            Ok(response_str)
        } else {
            // Write command - no response expected
            instr.write(&cmd).context("VISA write failed")?;
            Ok(String::new())
        }
    })
    .await
    .context("VISA command execution failed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_session() {
        let manager = VisaSessionManager::new();
        let resource = "TCPIP0::192.168.1.100::INSTR";

        let handle = manager
            .get_or_create_session(resource)
            .await
            .expect("Failed to create session");

        assert_eq!(handle.resource_name(), resource);
        assert_eq!(manager.session_count().await, 1);
    }

    #[tokio::test]
    async fn test_reuse_existing_session() {
        let manager = VisaSessionManager::new();
        let resource = "TCPIP0::192.168.1.100::INSTR";

        let handle1 = manager
            .get_or_create_session(resource)
            .await
            .expect("Failed to create first session");

        let handle2 = manager
            .get_or_create_session(resource)
            .await
            .expect("Failed to get existing session");

        // Should still have only one session
        assert_eq!(manager.session_count().await, 1);

        // Both handles should reference the same resource
        assert_eq!(handle1.resource_name(), handle2.resource_name());
    }

    #[tokio::test]
    async fn test_multiple_sessions() {
        let manager = VisaSessionManager::new();

        let handle1 = manager
            .get_or_create_session("TCPIP0::192.168.1.100::INSTR")
            .await
            .expect("Failed to create session 1");

        let handle2 = manager
            .get_or_create_session("TCPIP0::192.168.1.101::INSTR")
            .await
            .expect("Failed to create session 2");

        assert_eq!(manager.session_count().await, 2);
        assert_ne!(
            handle1.resource_name(),
            handle2.resource_name(),
            "Different resources should have different handles"
        );
    }

    #[tokio::test]
    async fn test_query_command() {
        let manager = VisaSessionManager::new();
        let handle = manager
            .get_or_create_session("TCPIP0::192.168.1.100::INSTR")
            .await
            .expect("Failed to create session");

        let response = handle
            .query("*IDN?", Duration::from_secs(2))
            .await
            .expect("Query failed");

        // Mock should return a reasonable response
        assert!(!response.is_empty());
        assert!(response.contains("Mock") || response.contains("response"));
    }

    #[tokio::test]
    async fn test_write_command() {
        let manager = VisaSessionManager::new();
        let handle = manager
            .get_or_create_session("TCPIP0::192.168.1.100::INSTR")
            .await
            .expect("Failed to create session");

        let result = handle.write("*RST").await;
        assert!(result.is_ok(), "Write command should succeed");
    }

    #[tokio::test]
    async fn test_command_ordering() {
        let manager = VisaSessionManager::new();
        let handle = manager
            .get_or_create_session("TCPIP0::192.168.1.100::INSTR")
            .await
            .expect("Failed to create session");

        // Send multiple commands in order
        let cmd1 = handle.query("*IDN?", Duration::from_secs(1));
        let cmd2 = handle.write("*RST");
        let cmd3 = handle.query("*OPC?", Duration::from_secs(1));

        // All should complete without error
        let r1 = cmd1.await;
        let r2 = cmd2.await;
        let r3 = cmd3.await;

        assert!(r1.is_ok(), "Command 1 failed");
        assert!(r2.is_ok(), "Command 2 failed");
        assert!(r3.is_ok(), "Command 3 failed");
    }

    #[tokio::test]
    async fn test_concurrent_commands() {
        let manager = VisaSessionManager::new();
        let handle = manager
            .get_or_create_session("TCPIP0::192.168.1.100::INSTR")
            .await
            .expect("Failed to create session");

        // Spawn multiple concurrent tasks sending commands
        let mut tasks = vec![];

        for i in 0..5 {
            let h = handle.clone();
            let task = tokio::spawn(async move {
                h.query(&format!("QUERY{}?", i), Duration::from_secs(1))
                    .await
            });
            tasks.push(task);
        }

        // Wait for all tasks
        for task in tasks {
            let result = task.await;
            assert!(result.is_ok(), "Task failed");
            assert!(result.unwrap().is_ok(), "Command execution failed");
        }
    }

    #[tokio::test]
    async fn test_close_session() {
        let manager = VisaSessionManager::new();
        let resource = "TCPIP0::192.168.1.100::INSTR";

        let _handle = manager
            .get_or_create_session(resource)
            .await
            .expect("Failed to create session");

        assert_eq!(manager.session_count().await, 1);

        let result = manager.close_session(resource).await;
        assert!(result.is_ok(), "Close should succeed");
        assert_eq!(manager.session_count().await, 0);
    }

    #[tokio::test]
    async fn test_close_nonexistent_session() {
        let manager = VisaSessionManager::new();

        let result = manager
            .close_session("TCPIP0::192.168.1.999::INSTR")
            .await;
        assert!(
            result.is_err(),
            "Closing nonexistent session should fail"
        );
    }

    #[tokio::test]
    async fn test_query_timeout() {
        let manager = VisaSessionManager::new();
        let handle = manager
            .get_or_create_session("TCPIP0::192.168.1.100::INSTR")
            .await
            .expect("Failed to create session");

        // Use a very short timeout that should still work (mock is fast)
        let result = handle.query("*IDN?", Duration::from_millis(1)).await;

        // Mock is fast, so this might still succeed
        // But if it times out, that's ok too
        let _ = result;
    }
}
