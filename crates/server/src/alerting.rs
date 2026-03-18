//! Webhook alerting for device faults and plan aborts (bd-kctc).
//!
//! Subscribes to the device health broadcast channel and RunEngine document
//! stream, sending Slack/Discord-compatible webhook notifications when:
//!
//! - A device transitions to `Faulted` state
//! - A device exhausts its maximum restart attempts
//! - The RunEngine aborts a plan
//!
//! Rate limiting prevents alert storms during cascading failures.

use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use reqwest::Client;
use serde_json::json;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use common::experiment::document::Document;
use common::health::device_health::DeviceHealth;
use hardware::registry::DeviceHealthEvent;

use crate::config::AlertingConfig;

/// Sends webhook notifications for hardware and experiment events.
///
/// Holds a pre-configured HTTP client and per-key rate limiting state.
/// All sends are fire-and-forget via `tokio::spawn` — the caller is
/// never blocked on network I/O.
pub struct WebhookNotifier {
    client: Client,
    webhook_url: String,
    rate_limit: std::time::Duration,
    last_alert: DashMap<String, Instant>,
    config: AlertingConfig,
}

impl WebhookNotifier {
    /// Create a notifier if a webhook URL is configured.
    ///
    /// Returns `None` when `webhook_url` is `None` or empty, meaning
    /// alerting is disabled and no background task should be spawned.
    pub fn new(config: AlertingConfig) -> Option<Self> {
        let url = config.webhook_url.as_deref()?.trim();
        if url.is_empty() {
            return None;
        }

        let client = match Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    error = %e,
                    "Webhook alerting requested but HTTP client failed to initialize"
                );
                return None;
            }
        };

        Some(Self {
            client,
            webhook_url: url.to_string(),
            rate_limit: std::time::Duration::from_secs(config.rate_limit_secs),
            last_alert: DashMap::new(),
            config,
        })
    }

    /// Check rate limit for a key. Returns `true` if the alert should be sent.
    fn check_rate_limit(&self, key: &str) -> bool {
        let now = Instant::now();
        if self
            .last_alert
            .get(key)
            .is_some_and(|last| now.duration_since(*last) < self.rate_limit)
        {
            debug!(key, "Alert rate-limited, skipping");
            return false;
        }
        self.last_alert.insert(key.to_string(), now);
        true
    }

    /// Send a Slack/Discord-compatible webhook payload.
    ///
    /// This is fire-and-forget: errors are logged but never propagated.
    fn send(&self, key: String, text: String) {
        if !self.check_rate_limit(&key) {
            return;
        }

        let client = self.client.clone();
        let url = self.webhook_url.clone();
        tokio::spawn(async move {
            let payload = json!({ "text": text });
            match client.post(&url).json(&payload).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    // Drain the response body to allow HTTP connection reuse.
                    let _ = resp.bytes().await;
                    if status.is_success() {
                        debug!(key, "Webhook alert sent successfully");
                    } else {
                        warn!(key, %status, "Webhook alert returned non-success status");
                    }
                }
                Err(e) => {
                    warn!(key, error = %e, "Webhook alert failed");
                }
            }
        });
    }

    /// Notify about a device fault (transition to Faulted state).
    pub fn notify_device_fault(&self, event: &DeviceHealthEvent) {
        if !self.config.alert_on_fault {
            return;
        }
        let key = format!("fault:{}", event.device_id);
        let text = format!(
            "\u{1f6a8} *Device Fault* | `{}` faulted after {} consecutive failures: {}",
            event.device_id, event.consecutive_failures, event.reason
        );
        info!(
            device_id = %event.device_id,
            consecutive_failures = event.consecutive_failures,
            "Sending device fault alert"
        );
        self.send(key, text);
    }

    /// Notify that a device has exhausted restart attempts.
    pub fn notify_restart_limit(&self, event: &DeviceHealthEvent) {
        if !self.config.alert_on_restart_limit {
            return;
        }
        let key = format!("restart_limit:{}", event.device_id);
        let text = format!(
            "\u{274c} *Restart Limit Reached* | `{}` failed after {} restart attempts \u{2014} giving up. Last error: {}",
            event.device_id, event.restart_attempts, event.reason
        );
        info!(
            device_id = %event.device_id,
            restart_attempts = event.restart_attempts,
            "Sending restart limit alert"
        );
        self.send(key, text);
    }

    /// Notify that the RunEngine aborted a plan.
    pub fn notify_plan_abort(&self, run_uid: &str, reason: &str, num_events: u32) {
        if !self.config.alert_on_plan_abort {
            return;
        }
        let key = format!("abort:{run_uid}");
        let text = format!(
            "\u{26a0}\u{fe0f} *Plan Aborted* | run `{run_uid}` aborted after {num_events} events: {reason}"
        );
        info!(run_uid, reason, num_events, "Sending plan abort alert");
        self.send(key, text);
    }
}

/// Run the alerting background task.
///
/// Subscribes to device health events and RunEngine documents, dispatching
/// webhook notifications based on the alerting configuration. Exits when
/// `cancel` is triggered.
pub async fn run_alerting_task(
    notifier: Arc<WebhookNotifier>,
    mut health_rx: broadcast::Receiver<DeviceHealthEvent>,
    mut doc_rx: broadcast::Receiver<Document>,
    cancel: CancellationToken,
) {
    info!("Webhook alerting task started");

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("Webhook alerting task shutting down");
                return;
            }
            result = health_rx.recv() => {
                match result {
                    Ok(event) => handle_health_event(&notifier, &event),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "Health event receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Health broadcast channel closed, alerting task exiting");
                        return;
                    }
                }
            }
            result = doc_rx.recv() => {
                match result {
                    Ok(Document::Stop(stop)) if stop.exit_status == "abort" => {
                        notifier.notify_plan_abort(
                            &stop.run_uid,
                            &stop.reason,
                            stop.num_events,
                        );
                    }
                    Ok(_) => {} // Ignore non-abort documents
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "Document receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Document broadcast channel closed, alerting task exiting");
                        return;
                    }
                }
            }
        }
    }
}

fn handle_health_event(notifier: &WebhookNotifier, event: &DeviceHealthEvent) {
    if event.new_state != DeviceHealth::Faulted {
        // Only alert on transitions TO Faulted; recovery events are informational
        return;
    }

    // Check if this is a "giving up" event (restart limit reached)
    if event.restart_attempts >= notifier.config.max_restart_attempts
        && notifier.config.max_restart_attempts > 0
    {
        notifier.notify_restart_limit(event);
    } else {
        notifier.notify_device_fault(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notifier_disabled_when_no_url() {
        let config = AlertingConfig::default();
        assert!(config.webhook_url.is_none());
        assert!(WebhookNotifier::new(config).is_none());
    }

    #[test]
    fn test_notifier_disabled_when_empty_url() {
        let config = AlertingConfig {
            webhook_url: Some(String::new()),
            ..Default::default()
        };
        assert!(WebhookNotifier::new(config).is_none());
    }

    #[test]
    fn test_notifier_disabled_when_whitespace_url() {
        let config = AlertingConfig {
            webhook_url: Some("   ".to_string()),
            ..Default::default()
        };
        assert!(WebhookNotifier::new(config).is_none());
    }

    #[test]
    fn test_notifier_created_with_valid_url() {
        let config = AlertingConfig {
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            ..Default::default()
        };
        assert!(WebhookNotifier::new(config).is_some());
    }

    #[test]
    fn test_rate_limiting() {
        let config = AlertingConfig {
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            rate_limit_secs: 60,
            ..Default::default()
        };
        let notifier = WebhookNotifier::new(config).unwrap();

        // First call should pass
        assert!(notifier.check_rate_limit("device:cam1"));
        // Second call within rate limit should be blocked
        assert!(!notifier.check_rate_limit("device:cam1"));
        // Different key should pass
        assert!(notifier.check_rate_limit("device:cam2"));
    }

    #[test]
    fn test_fault_alert_respects_config_toggle() {
        let config = AlertingConfig {
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            alert_on_fault: false,
            ..Default::default()
        };
        let notifier = WebhookNotifier::new(config).unwrap();

        let event = DeviceHealthEvent {
            device_id: "cam1".to_string(),
            old_state: DeviceHealth::Degraded,
            new_state: DeviceHealth::Faulted,
            reason: "connection timeout".to_string(),
            timestamp: Instant::now(),
            consecutive_failures: 3,
            restart_attempts: 0,
        };

        // Should not panic or send (alert_on_fault is false)
        notifier.notify_device_fault(&event);
        // Rate limit key should NOT be set since the alert was skipped
        assert!(!notifier.last_alert.contains_key("fault:cam1"));
    }

    #[tokio::test]
    async fn test_alerting_task_exits_on_cancel() {
        let config = AlertingConfig {
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            ..Default::default()
        };
        let notifier = Arc::new(WebhookNotifier::new(config).unwrap());
        let (health_tx, health_rx) = broadcast::channel::<DeviceHealthEvent>(16);
        let (doc_tx, doc_rx) = broadcast::channel::<Document>(16);
        let cancel = CancellationToken::new();

        let cancel_clone = cancel.clone();
        let task = tokio::spawn(async move {
            run_alerting_task(notifier, health_rx, doc_rx, cancel_clone).await;
        });

        // Keep senders alive until cancel
        let _health_tx = health_tx;
        let _doc_tx = doc_tx;

        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("alerting task should exit within 2s")
            .expect("alerting task should not panic");
    }

    #[tokio::test]
    async fn test_handle_health_event_restart_limit() {
        let config = AlertingConfig {
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            max_restart_attempts: 5,
            ..Default::default()
        };
        let notifier = WebhookNotifier::new(config).unwrap();

        let event = DeviceHealthEvent {
            device_id: "laser".to_string(),
            old_state: DeviceHealth::Recovering,
            new_state: DeviceHealth::Faulted,
            reason: "hardware disconnect".to_string(),
            timestamp: Instant::now(),
            consecutive_failures: 3,
            restart_attempts: 5,
        };

        handle_health_event(&notifier, &event);
        // Should have used the restart_limit key, not the fault key
        assert!(notifier.last_alert.contains_key("restart_limit:laser"));
        assert!(!notifier.last_alert.contains_key("fault:laser"));
    }

    #[tokio::test]
    async fn test_handle_health_event_regular_fault() {
        let config = AlertingConfig {
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            max_restart_attempts: 5,
            ..Default::default()
        };
        let notifier = WebhookNotifier::new(config).unwrap();

        let event = DeviceHealthEvent {
            device_id: "stage".to_string(),
            old_state: DeviceHealth::Degraded,
            new_state: DeviceHealth::Faulted,
            reason: "serial timeout".to_string(),
            timestamp: Instant::now(),
            consecutive_failures: 3,
            restart_attempts: 0,
        };

        handle_health_event(&notifier, &event);
        assert!(notifier.last_alert.contains_key("fault:stage"));
    }

    #[test]
    fn test_non_faulted_events_ignored() {
        let config = AlertingConfig {
            webhook_url: Some("https://hooks.slack.com/test".to_string()),
            ..Default::default()
        };
        let notifier = WebhookNotifier::new(config).unwrap();

        let event = DeviceHealthEvent {
            device_id: "cam1".to_string(),
            old_state: DeviceHealth::Faulted,
            new_state: DeviceHealth::Recovering,
            reason: "restart initiated".to_string(),
            timestamp: Instant::now(),
            consecutive_failures: 0,
            restart_attempts: 1,
        };

        handle_health_event(&notifier, &event);
        assert!(notifier.last_alert.is_empty());
    }
}
