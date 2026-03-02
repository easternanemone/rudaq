#![allow(dead_code)]
//! Prototype sidecar runner for future Python echelle extraction integration.
//!
//! This module intentionally keeps the contract simple: JSON request/response
//! over stdio, per-request process spawn (which also provides trivial restart
//! semantics), timeout handling, and structured stderr capture.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(super) struct EchelleSidecarRunner {
    program: String,
    args: Vec<String>,
    timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum SidecarLogEvent {
    Structured(Value),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SidecarInvocationResult {
    pub(super) response: Value,
    pub(super) stderr_events: Vec<SidecarLogEvent>,
    pub(super) elapsed: Duration,
}

#[derive(Debug)]
pub(super) enum SidecarRunnerError {
    Spawn {
        program: String,
        source: std::io::Error,
    },
    MissingPipes,
    StdinWrite(std::io::Error),
    StdoutRead(std::io::Error),
    EmptyResponse,
    InvalidJson(serde_json::Error),
    Timeout {
        timeout_ms: u64,
    },
    Kill(std::io::Error),
}

impl std::fmt::Display for SidecarRunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { program, source } => {
                write!(f, "failed to spawn sidecar {program}: {source}")
            }
            Self::MissingPipes => write!(f, "failed to access sidecar stdin/stdout"),
            Self::StdinWrite(e) => write!(f, "failed to write request to sidecar stdin: {e}"),
            Self::StdoutRead(e) => write!(f, "failed to read sidecar stdout: {e}"),
            Self::EmptyResponse => write!(f, "sidecar did not produce a response line on stdout"),
            Self::InvalidJson(e) => write!(f, "sidecar response was not valid JSON: {e}"),
            Self::Timeout { timeout_ms } => {
                write!(f, "sidecar request timed out after {timeout_ms} ms")
            }
            Self::Kill(e) => write!(f, "failed to terminate timed-out sidecar: {e}"),
        }
    }
}

impl std::error::Error for SidecarRunnerError {}

impl EchelleSidecarRunner {
    pub(super) fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            timeout: Duration::from_secs(2),
        }
    }

    pub(super) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(super) fn health_check(&self) -> Result<SidecarInvocationResult, SidecarRunnerError> {
        self.request_json(&serde_json::json!({
            "request_id": "health-check",
            "op": "health"
        }))
    }

    pub(super) fn request_json(
        &self,
        request: &Value,
    ) -> Result<SidecarInvocationResult, SidecarRunnerError> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let start = Instant::now();
        let mut child = cmd.spawn().map_err(|source| SidecarRunnerError::Spawn {
            program: self.program.clone(),
            source,
        })?;

        let mut stdin = child.stdin.take().ok_or(SidecarRunnerError::MissingPipes)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(SidecarRunnerError::MissingPipes)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(SidecarRunnerError::MissingPipes)?;

        let stderr_handle = std::thread::spawn(move || read_stderr_events(stderr));

        let request_line =
            serde_json::to_string(request).map_err(SidecarRunnerError::InvalidJson)?;
        stdin
            .write_all(request_line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .map_err(SidecarRunnerError::StdinWrite)?;
        drop(stdin);

        let stdout_handle = std::thread::spawn(move || -> Result<String, std::io::Error> {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                return Ok(String::new());
            }
            Ok(line)
        });

        // Prototype runner uses per-request spawn; timeout + kill gives restart semantics by respawn.
        loop {
            if start.elapsed() > self.timeout {
                child.kill().map_err(SidecarRunnerError::Kill)?;
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(SidecarRunnerError::Timeout {
                    timeout_ms: self.timeout.as_millis() as u64,
                });
            }

            if child
                .try_wait()
                .map_err(SidecarRunnerError::StdoutRead)?
                .is_some()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let stdout_line = stdout_handle
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stdout reader thread panicked")))
            .map_err(SidecarRunnerError::StdoutRead)?;
        let stderr_events = stderr_handle.join().unwrap_or_default();

        if stdout_line.trim().is_empty() {
            return Err(SidecarRunnerError::EmptyResponse);
        }
        let response = serde_json::from_str::<Value>(stdout_line.trim())
            .map_err(SidecarRunnerError::InvalidJson)?;

        Ok(SidecarInvocationResult {
            response,
            stderr_events,
            elapsed: start.elapsed(),
        })
    }
}

fn read_stderr_events(stderr: impl std::io::Read) -> Vec<SidecarLogEvent> {
    let mut events = Vec::new();
    let reader = BufReader::new(stderr);
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => events.push(SidecarLogEvent::Structured(v)),
            Err(_) => events.push(SidecarLogEvent::Text(line)),
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_json_response_and_structured_stderr() {
        let runner = EchelleSidecarRunner::new(
            "/bin/sh",
            [
                "-c",
                r#"read line; echo '{"ok":true,"echo":'"$line"'}'; echo '{"level":"info","msg":"sidecar started"}' 1>&2"#,
            ],
        )
        .with_timeout(Duration::from_secs(1));

        let result = runner
            .request_json(&serde_json::json!({"request_id":"1","op":"health"}))
            .unwrap();
        assert_eq!(result.response["ok"], true);
        assert_eq!(result.response["echo"]["op"], "health");
        assert!(!result.stderr_events.is_empty());
        assert!(matches!(
            result.stderr_events[0],
            SidecarLogEvent::Structured(_)
        ));
    }

    #[test]
    fn times_out_and_kills_hanging_sidecar() {
        let runner = EchelleSidecarRunner::new("/bin/sh", ["-c", "sleep 2"])
            .with_timeout(Duration::from_millis(100));
        let err = runner.health_check().unwrap_err();
        assert!(matches!(err, SidecarRunnerError::Timeout { .. }));
    }
}
