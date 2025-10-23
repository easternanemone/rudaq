//! Integration tests for the experiment sequencer (RunEngine and Plans).
//!
//! Tests the complete workflow of defining plans, executing them via RunEngine,
//! and validating checkpointing and state management.

use rust_daq::app_actor::DaqManagerActor;
use rust_daq::config::{ApplicationSettings, Settings, StorageSettings};
use rust_daq::data::registry::ProcessorRegistry;
use rust_daq::experiment::{
    Checkpoint, ExperimentState, GridScanPlan, LogLevel, Message, Plan, PlanStream,
    RunEngine, ScanPlan, TimeSeriesPlan,
};
use rust_daq::instrument::InstrumentRegistry;
use rust_daq::log_capture::LogBuffer;
use rust_daq::measurement::InstrumentMeasurement;
use rust_daq::messages::DaqCommand;
use rust_daq::modules::ModuleRegistry;
use anyhow::Result;
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

fn create_test_settings() -> Settings {
    Settings {
        log_level: "info".to_string(),
        application: ApplicationSettings {
            broadcast_channel_capacity: 64,
            command_channel_capacity: 16,
        },
        storage: StorageSettings {
            default_path: "./data".to_string(),
            default_format: "csv".to_string(),
        },
        instruments: HashMap::new(),
        processors: None,
    }
}

async fn setup_actor() -> mpsc::Sender<DaqCommand> {
    let settings = create_test_settings();
    let runtime = Arc::new(Runtime::new().expect("Failed to create runtime"));

    let actor = DaqManagerActor::<InstrumentMeasurement>::new(
        Arc::new(settings),
        Arc::new(InstrumentRegistry::new()),
        Arc::new(ProcessorRegistry::new()),
        Arc::new(ModuleRegistry::new()),
        LogBuffer::new(),
        runtime,
    )
    .expect("Failed to create actor");

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    tokio::spawn(actor.run(cmd_rx));

    cmd_tx
}

/// Simple test plan that emits a fixed sequence of messages.
struct SimplePlan {
    messages: Vec<Message>,
}

impl SimplePlan {
    fn new(messages: Vec<Message>) -> Self {
        Self { messages }
    }
}

impl Plan for SimplePlan {
    fn execute(&mut self) -> PlanStream<'_> {
        let messages = self.messages.clone();
        Box::pin(stream::iter(messages.into_iter().map(Ok)))
    }

    fn metadata(&self) -> (String, String) {
        (
            "SimplePlan".to_string(),
            format!("{} messages", self.messages.len()),
        )
    }
}

#[tokio::test]
async fn test_run_engine_creation() {
    let (tx, _rx) = mpsc::channel(32);
    let engine = RunEngine::new(tx);
    let status = engine.status();
    assert_eq!(status.state, ExperimentState::Idle);
    assert_eq!(status.message_count, 0);
    assert!(status.run_id.is_none());
}

#[tokio::test]
async fn test_run_engine_with_auto_checkpoint() {
    let (tx, _rx) = mpsc::channel(32);
    let engine = RunEngine::new(tx).with_auto_checkpoint(10);
    let status = engine.status();
    assert_eq!(status.state, ExperimentState::Idle);
}

#[tokio::test]
async fn test_simple_plan_execution() {
    let cmd_tx = setup_actor().await;
    let mut engine = RunEngine::new(cmd_tx);

    let mut metadata = HashMap::new();
    metadata.insert("test".to_string(), "simple".to_string());

    let messages = vec![
        Message::BeginRun {
            metadata: metadata.clone(),
        },
        Message::Log {
            level: LogLevel::Info,
            message: "Step 1".to_string(),
        },
        Message::Log {
            level: LogLevel::Info,
            message: "Step 2".to_string(),
        },
        Message::EndRun,
    ];

    let plan = SimplePlan::new(messages);
    let result = engine.run(Box::new(plan)).await;

    assert!(result.is_ok(), "Plan execution should succeed");
    let status = engine.status();
    assert_eq!(status.state, ExperimentState::Complete);
    assert_eq!(status.message_count, 4);
}

#[tokio::test]
async fn test_time_series_plan_structure() {
    let plan = TimeSeriesPlan::new(
        "test_module".to_string(),
        Duration::from_secs(5),
        Duration::from_secs(1),
    );

    assert_eq!(plan.total_steps(), 5);

    let (name, desc) = plan.metadata();
    assert!(name.contains("Time Series"));
    assert!(desc.contains("5 samples"));
}

#[tokio::test]
async fn test_time_series_plan_messages() {
    let mut plan = TimeSeriesPlan::new(
        "test_module".to_string(),
        Duration::from_secs(2),
        Duration::from_secs(1),
    );

    let mut stream = plan.execute();
    let mut messages = Vec::new();

    while let Some(Ok(message)) = stream.next().await {
        messages.push(message);
    }

    // Should have: BeginRun, (Trigger, Read, Sleep) * 2, EndRun
    // Plus Log messages every 10 steps
    assert!(!messages.is_empty(), "Plan should emit messages");

    // Check for BeginRun
    assert!(
        matches!(messages.first(), Some(Message::BeginRun { .. })),
        "First message should be BeginRun"
    );

    // Check for EndRun
    assert!(
        matches!(messages.last(), Some(Message::EndRun)),
        "Last message should be EndRun"
    );

    // Count Trigger messages
    let trigger_count = messages
        .iter()
        .filter(|m| matches!(m, Message::Trigger { .. }))
        .count();
    assert_eq!(trigger_count, 2, "Should have 2 trigger messages");
}

#[tokio::test]
async fn test_scan_plan_structure() {
    let plan = ScanPlan::new(
        "laser".to_string(),
        "power".to_string(),
        0.0,
        100.0,
        11,
        "detector".to_string(),
    );

    assert_eq!(plan.num_points, 11);

    let (name, desc) = plan.metadata();
    assert!(name.contains("1D Scan"));
    assert!(desc.contains("11 points"));
}

#[tokio::test]
async fn test_scan_plan_messages() {
    let mut plan = ScanPlan::new(
        "laser".to_string(),
        "power".to_string(),
        0.0,
        10.0,
        3,
        "detector".to_string(),
    );

    let mut stream = plan.execute();
    let mut messages = Vec::new();

    while let Some(Ok(message)) = stream.next().await {
        messages.push(message);
    }

    // Check for Set messages
    let set_count = messages
        .iter()
        .filter(|m| matches!(m, Message::Set { .. }))
        .count();
    assert_eq!(set_count, 3, "Should have 3 Set messages (one per point)");

    // Validate Set values
    let set_values: Vec<String> = messages
        .iter()
        .filter_map(|m| {
            if let Message::Set { value, .. } = m {
                Some(value.clone())
            } else {
                None
            }
        })
        .collect();

    // First value should be ~0.0
    assert!(
        set_values[0].parse::<f64>().unwrap() < 1.0,
        "First value should be near 0"
    );
    // Last value should be ~10.0
    assert!(
        set_values[2].parse::<f64>().unwrap() > 9.0,
        "Last value should be near 10"
    );
}

#[tokio::test]
async fn test_grid_scan_plan_structure() {
    let plan = GridScanPlan::new(
        "stage".to_string(),
        "x".to_string(),
        0.0,
        10.0,
        3,
        "y".to_string(),
        0.0,
        5.0,
        2,
        "camera".to_string(),
    );

    assert_eq!(plan.total_points(), 6); // 3 × 2

    let (name, desc) = plan.metadata();
    assert!(name.contains("2D Grid Scan"));
    assert!(desc.contains("3 × 2"));
}

#[tokio::test]
async fn test_grid_scan_plan_messages() {
    let mut plan = GridScanPlan::new(
        "stage".to_string(),
        "x".to_string(),
        0.0,
        10.0,
        2,
        "y".to_string(),
        0.0,
        5.0,
        2,
        "camera".to_string(),
    );

    let mut stream = plan.execute();
    let mut messages = Vec::new();

    while let Some(Ok(message)) = stream.next().await {
        messages.push(message);
    }

    // Check for Set messages (2 per point: x and y)
    let set_count = messages
        .iter()
        .filter(|m| matches!(m, Message::Set { .. }))
        .count();
    assert_eq!(set_count, 8, "Should have 8 Set messages (2 per point, 4 points)");

    // Check for Trigger messages
    let trigger_count = messages
        .iter()
        .filter(|m| matches!(m, Message::Trigger { .. }))
        .count();
    assert_eq!(trigger_count, 4, "Should have 4 Trigger messages");
}

#[tokio::test]
async fn test_checkpoint_save_load() {
    let dir = tempdir().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("experiment".to_string(), "test".to_string());

    let checkpoint = Checkpoint::new(
        "run_001".to_string(),
        ExperimentState::Paused,
        metadata.clone(),
        42,
    )
    .with_label("manual_pause".to_string());

    let path = dir.path().join("checkpoint.json");
    checkpoint.save(&path).unwrap();

    let loaded = Checkpoint::load(&path).unwrap();
    assert_eq!(loaded.run_id, "run_001");
    assert_eq!(loaded.state, ExperimentState::Paused);
    assert_eq!(loaded.message_count, 42);
    assert_eq!(loaded.label, Some("manual_pause".to_string()));
}

#[tokio::test]
async fn test_checkpoint_with_plan_state() {
    let plan_state = serde_json::json!({
        "type": "TimeSeries",
        "current_step": 5,
        "total_steps": 100,
    });

    let checkpoint = Checkpoint::new(
        "run_002".to_string(),
        ExperimentState::Running,
        HashMap::new(),
        10,
    )
    .with_plan_state(plan_state.clone());

    assert_eq!(checkpoint.plan_state, Some(plan_state));
}

#[tokio::test]
async fn test_experiment_state_transitions() {
    assert!(ExperimentState::Idle.can_begin());
    assert!(!ExperimentState::Running.can_begin());

    assert!(ExperimentState::Running.can_pause());
    assert!(!ExperimentState::Idle.can_pause());

    assert!(ExperimentState::Paused.can_resume());
    assert!(!ExperimentState::Running.can_resume());
}

#[tokio::test]
async fn test_plan_validation() {
    struct InvalidPlan;

    impl Plan for InvalidPlan {
        fn execute(&mut self) -> PlanStream<'_> {
            Box::pin(stream::empty())
        }

        fn validate(&self) -> Result<()> {
            anyhow::bail!("Invalid plan")
        }
    }

    let cmd_tx = setup_actor().await;
    let mut engine = RunEngine::new(cmd_tx);

    let plan = InvalidPlan;
    let result = engine.run(Box::new(plan)).await;

    assert!(result.is_err(), "Invalid plan should fail validation");
}

#[tokio::test]
async fn test_pause_resume_messages() {
    let cmd_tx = setup_actor().await;
    let mut engine = RunEngine::new(cmd_tx);

    let mut metadata = HashMap::new();
    metadata.insert("test".to_string(), "pause_resume".to_string());

    let messages = vec![
        Message::BeginRun {
            metadata: metadata.clone(),
        },
        Message::Log {
            level: LogLevel::Info,
            message: "Before pause".to_string(),
        },
        Message::Pause,
        // Note: In real usage, Pause would suspend execution until Resume is sent externally
        // This test just validates message processing
        Message::Resume,
        Message::Log {
            level: LogLevel::Info,
            message: "After resume".to_string(),
        },
        Message::EndRun,
    ];

    let plan = SimplePlan::new(messages);
    let result = engine.run(Box::new(plan)).await;

    // The engine will process all messages including Pause/Resume
    // In a real scenario, execution would suspend on Pause
    assert!(result.is_ok(), "Plan with pause/resume should execute");
}
