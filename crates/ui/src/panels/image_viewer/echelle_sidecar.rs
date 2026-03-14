#![allow(dead_code)]
//! Production sidecar runner for Python echelle extraction integration.
//!
//! Per-request process spawn with retry, circuit breaking, and proper timeout
//! handling. The per-request model is intentional: spawn overhead (~5ms) is
//! dwarfed by extraction time, and process isolation gives clean failure
//! boundaries.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Configuration for sidecar retry and circuit breaker behavior.
#[derive(Debug, Clone)]
pub(super) struct SidecarConfig {
    /// Maximum retry attempts per request (0 = no retries).
    pub(super) max_retries: u32,
    /// Initial backoff delay between retries.
    pub(super) initial_backoff: Duration,
    /// Maximum backoff delay (caps exponential growth).
    pub(super) max_backoff: Duration,
    /// Consecutive failures before the circuit breaker opens.
    pub(super) circuit_breaker_threshold: u32,
    /// Cooldown period while circuit is open before allowing a probe request.
    pub(super) circuit_breaker_cooldown: Duration,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub(super) struct EchelleSidecarRunner {
    program: String,
    args: Vec<String>,
    timeout: Duration,
    config: SidecarConfig,
    /// Consecutive failure count for circuit breaker.
    consecutive_failures: AtomicU32,
    /// Timestamp (as epoch millis) when circuit breaker opened. 0 = closed.
    circuit_opened_at_ms: AtomicU32,
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
    /// Circuit breaker is open — too many consecutive failures.
    CircuitOpen {
        consecutive_failures: u32,
        cooldown_remaining: Duration,
    },
    /// All retry attempts exhausted.
    RetriesExhausted {
        attempts: u32,
        last_error: Box<SidecarRunnerError>,
    },
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
            Self::CircuitOpen {
                consecutive_failures,
                cooldown_remaining,
            } => {
                write!(
                    f,
                    "sidecar circuit breaker open after {consecutive_failures} consecutive failures \
                     (cooldown: {cooldown_remaining:?} remaining)"
                )
            }
            Self::RetriesExhausted {
                attempts,
                last_error,
            } => {
                write!(
                    f,
                    "sidecar request failed after {attempts} attempts: {last_error}"
                )
            }
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
            config: SidecarConfig::default(),
            consecutive_failures: AtomicU32::new(0),
            circuit_opened_at_ms: AtomicU32::new(0),
        }
    }

    pub(super) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(super) fn with_config(mut self, config: SidecarConfig) -> Self {
        self.config = config;
        self
    }

    /// Run a health check. Bypasses circuit breaker (used to probe recovery).
    pub(super) fn health_check(&self) -> Result<SidecarInvocationResult, SidecarRunnerError> {
        self.request_json_once(&serde_json::json!({
            "request_id": "health-check",
            "op": "health"
        }))
    }

    /// Validate the sidecar is functional. Runs a health check and verifies
    /// the response contains `{"ok": true}`.
    pub(super) fn validate(&self) -> Result<(), SidecarRunnerError> {
        let result = self.health_check()?;
        if result.response.get("ok") == Some(&Value::Bool(true)) {
            Ok(())
        } else {
            Err(SidecarRunnerError::EmptyResponse)
        }
    }

    /// Send a JSON request with retry and circuit breaker protection.
    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn request_json(
        &self,
        request: &Value,
    ) -> Result<SidecarInvocationResult, SidecarRunnerError> {
        self.check_circuit_breaker()?;

        let max_attempts = self.config.max_retries + 1;
        let mut last_error = None;
        let mut backoff = self.config.initial_backoff;

        for attempt in 0..max_attempts {
            if attempt > 0 {
                eprintln!(
                    "[sidecar] retry {attempt}/{} after {backoff:?}",
                    self.config.max_retries
                );
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(self.config.max_backoff);
            }

            match self.request_json_once(request) {
                Ok(result) => {
                    self.record_success();
                    return Ok(result);
                }
                Err(e) => {
                    self.record_failure();
                    last_error = Some(e);
                }
            }
        }

        Err(SidecarRunnerError::RetriesExhausted {
            attempts: max_attempts,
            last_error: Box::new(last_error.expect("at least one attempt was made")),
        })
    }

    /// Send a single JSON request without retry (the low-level path).
    #[allow(clippy::cast_possible_truncation)]
    fn request_json_once(
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

        // Poll for child exit with timeout. Child stays in this thread so we
        // can kill it on timeout. 50ms granularity is fine for processes that
        // typically run 10-500ms.
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
            std::thread::sleep(Duration::from_millis(50));
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

    /// Check if the circuit breaker allows a request through.
    #[allow(clippy::cast_possible_truncation)]
    fn check_circuit_breaker(&self) -> Result<(), SidecarRunnerError> {
        let failures = self.consecutive_failures.load(Ordering::Relaxed);
        if failures < self.config.circuit_breaker_threshold {
            return Ok(());
        }

        let opened_at = self.circuit_opened_at_ms.load(Ordering::Relaxed);
        if opened_at == 0 {
            return Ok(());
        }

        let now_ms = epoch_millis_u32();
        let elapsed = Duration::from_millis(u64::from(now_ms.saturating_sub(opened_at)));

        if elapsed >= self.config.circuit_breaker_cooldown {
            // Half-open: allow one probe request through, reset will happen on success.
            return Ok(());
        }

        Err(SidecarRunnerError::CircuitOpen {
            consecutive_failures: failures,
            cooldown_remaining: self.config.circuit_breaker_cooldown.saturating_sub(elapsed),
        })
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.circuit_opened_at_ms.store(0, Ordering::Relaxed);
    }

    #[allow(clippy::cast_possible_truncation)]
    fn record_failure(&self) {
        let prev = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        if prev + 1 >= self.config.circuit_breaker_threshold {
            let opened = self.circuit_opened_at_ms.load(Ordering::Relaxed);
            if opened == 0 {
                self.circuit_opened_at_ms
                    .store(epoch_millis_u32(), Ordering::Relaxed);
                eprintln!(
                    "[sidecar] circuit breaker OPEN after {} consecutive failures",
                    prev + 1
                );
            }
        }
    }

    /// Current consecutive failure count (for observability).
    pub(super) fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures.load(Ordering::Relaxed)
    }
}

/// Truncated epoch milliseconds fitting in u32 (~49 days of range, wraps).
/// Sufficient for relative duration comparisons within a single process lifetime.
#[allow(clippy::cast_possible_truncation)]
fn epoch_millis_u32() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u32
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
            .expect("request should succeed");
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
        let runner = EchelleSidecarRunner::new("/bin/sh", ["-c", "sleep 10"])
            .with_timeout(Duration::from_millis(100))
            .with_config(SidecarConfig {
                max_retries: 0,
                ..SidecarConfig::default()
            });
        let err = runner.health_check().unwrap_err();
        assert!(matches!(err, SidecarRunnerError::Timeout { .. }));
    }

    #[test]
    fn retries_on_transient_failure() {
        // First invocation fails (bad program), but we test the retry machinery
        // by using a program that always fails — we expect RetriesExhausted.
        let runner = EchelleSidecarRunner::new("/bin/sh", ["-c", "exit 1"])
            .with_timeout(Duration::from_secs(1))
            .with_config(SidecarConfig {
                max_retries: 2,
                initial_backoff: Duration::from_millis(10),
                max_backoff: Duration::from_millis(50),
                ..SidecarConfig::default()
            });

        let err = runner
            .request_json(&serde_json::json!({"op":"test"}))
            .unwrap_err();
        assert!(
            matches!(
                err,
                SidecarRunnerError::RetriesExhausted { attempts: 3, .. }
            ),
            "expected RetriesExhausted with 3 attempts, got: {err}"
        );
    }

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let runner = EchelleSidecarRunner::new("/bin/sh", ["-c", "exit 1"])
            .with_timeout(Duration::from_secs(1))
            .with_config(SidecarConfig {
                max_retries: 0,
                circuit_breaker_threshold: 3,
                circuit_breaker_cooldown: Duration::from_secs(60),
                ..SidecarConfig::default()
            });

        // Burn through 3 failures to trip the circuit breaker.
        for _ in 0..3 {
            let _ = runner.request_json(&serde_json::json!({"op":"test"}));
        }

        assert!(runner.consecutive_failures() >= 3);

        // Next request should be rejected by the circuit breaker.
        let err = runner
            .request_json(&serde_json::json!({"op":"test"}))
            .unwrap_err();
        assert!(
            matches!(err, SidecarRunnerError::CircuitOpen { .. }),
            "expected CircuitOpen, got: {err}"
        );
    }

    #[test]
    fn validate_succeeds_on_healthy_sidecar() {
        let runner =
            EchelleSidecarRunner::new("/bin/sh", ["-c", r#"read line; echo '{"ok":true}'"#])
                .with_timeout(Duration::from_secs(1));

        runner.validate().expect("validation should pass");
    }

    #[test]
    fn success_resets_circuit_breaker() {
        let runner = EchelleSidecarRunner::new(
            "/bin/sh",
            ["-c", r#"read line; echo '{"ok":true,"echo":'"$line"'}'"#],
        )
        .with_timeout(Duration::from_secs(1))
        .with_config(SidecarConfig {
            max_retries: 0,
            circuit_breaker_threshold: 10,
            ..SidecarConfig::default()
        });

        // Manually bump the failure counter.
        runner.consecutive_failures.store(5, Ordering::Relaxed);
        assert_eq!(runner.consecutive_failures(), 5);

        // A successful request should reset it.
        runner
            .request_json(&serde_json::json!({"op":"health"}))
            .expect("request should succeed");
        assert_eq!(runner.consecutive_failures(), 0);
    }
}
