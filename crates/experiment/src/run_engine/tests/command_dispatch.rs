//! Tests for `CommandDispatcher` — hardware command dispatching without a full RunEngine.
//!
//! The `CommandDispatcher` is deliberately structured so its hardware-interaction
//! logic can be exercised without constructing a full `RunEngine`.  These tests
//! use lightweight local mock drivers registered directly with a `DeviceRegistry`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use async_trait::async_trait;
use tokio::sync::mpsc;

use common::capabilities::{DeviceCategory, Movable, Readable, Settable, Triggerable};
use common::driver::{Capability as DeviceCapability, DeviceComponents, DriverFactory};
use common::device_id::DeviceId;
use hardware::registry::DeviceRegistry;
use serde_json::Value;

use super::super::command_dispatch::CommandDispatcher;
use crate::feedback::FeedbackEvent;
use crate::plans::{ComparisonOp, EvalCondition};

// =============================================================================
// Minimal mock drivers
// =============================================================================

/// A stage that records the last requested position.
struct RecordingStage {
    last_position: Arc<Mutex<f64>>,
}

#[async_trait]
impl Movable for RecordingStage {
    async fn move_abs(&self, position: f64) -> anyhow::Result<()> {
        *self.last_position.lock().unwrap() = position;
        Ok(())
    }

    async fn move_rel(&self, distance: f64) -> anyhow::Result<()> {
        let mut pos = self.last_position.lock().unwrap();
        *pos += distance;
        Ok(())
    }

    async fn position(&self) -> anyhow::Result<f64> {
        Ok(*self.last_position.lock().unwrap())
    }

    async fn wait_settled(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

struct RecordingStageFactory {
    last_position: Arc<Mutex<f64>>,
}

impl DriverFactory for RecordingStageFactory {
    fn driver_type(&self) -> &'static str {
        "recording_stage"
    }

    fn name(&self) -> &'static str {
        "Recording Stage"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::Movable]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let last_position = self.last_position.clone();
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Stage)
                .with_movable(Arc::new(RecordingStage { last_position })))
        })
    }
}

/// A readable device that always returns a fixed value.
struct FixedReadable(f64);

#[async_trait]
impl Readable for FixedReadable {
    async fn read(&self) -> anyhow::Result<f64> {
        Ok(self.0)
    }
}

struct FixedReadableFactory {
    value: f64,
}

impl DriverFactory for FixedReadableFactory {
    fn driver_type(&self) -> &'static str {
        "fixed_readable_cd"
    }

    fn name(&self) -> &'static str {
        "Fixed Readable (CD tests)"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::Readable]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let value = self.value;
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Detector)
                .with_readable(Arc::new(FixedReadable(value))))
        })
    }
}

/// A triggerable device that counts how many times it has been triggered.
struct CountingTrigger {
    count: Arc<AtomicU32>,
}

#[async_trait]
impl Triggerable for CountingTrigger {
    async fn arm(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn trigger(&self) -> anyhow::Result<()> {
        self.count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct CountingTriggerFactory {
    count: Arc<AtomicU32>,
}

impl DriverFactory for CountingTriggerFactory {
    fn driver_type(&self) -> &'static str {
        "counting_trigger"
    }

    fn name(&self) -> &'static str {
        "Counting Trigger"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::Triggerable]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let count = self.count.clone();
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Detector)
                .with_triggerable(Arc::new(CountingTrigger { count })))
        })
    }
}

/// A settable device that records the last set parameter name.
struct RecordingSettable {
    last_name: Arc<Mutex<String>>,
    triggered: Arc<AtomicBool>,
}

#[async_trait]
impl Settable for RecordingSettable {
    async fn set_value(&self, name: &str, _value: Value) -> anyhow::Result<()> {
        *self.last_name.lock().unwrap() = name.to_string();
        self.triggered.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn get_value(&self, _name: &str) -> anyhow::Result<Value> {
        Ok(Value::Null)
    }
}

struct RecordingSettableFactory {
    last_name: Arc<Mutex<String>>,
    triggered: Arc<AtomicBool>,
}

impl DriverFactory for RecordingSettableFactory {
    fn driver_type(&self) -> &'static str {
        "recording_settable"
    }

    fn name(&self) -> &'static str {
        "Recording Settable"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::Settable]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let last_name = self.last_name.clone();
        let triggered = self.triggered.clone();
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Detector)
                .with_settable(Arc::new(RecordingSettable {
                    last_name,
                    triggered,
                })))
        })
    }
}

// =============================================================================
// Helper to build a feedback channel pair
// =============================================================================

fn make_feedback_channel() -> (mpsc::Sender<FeedbackEvent>, mpsc::Receiver<FeedbackEvent>) {
    mpsc::channel(32)
}

// =============================================================================
// execute_move tests
// =============================================================================

#[tokio::test]
async fn test_execute_move_succeeds() {
    let last_position = Arc::new(Mutex::new(0.0_f64));

    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(RecordingStageFactory {
        last_position: last_position.clone(),
    }));
    registry
        .register_from_toml(
            "stage",
            "Test Stage",
            "recording_stage",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();

    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    dispatcher.execute_move("stage", 42.0).await.unwrap();
    assert_eq!(*last_position.lock().unwrap(), 42.0);
}

#[tokio::test]
async fn test_execute_move_missing_device_returns_error() {
    let registry = Arc::new(DeviceRegistry::new());
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let err = dispatcher.execute_move("nonexistent", 10.0).await;
    assert!(err.is_err(), "should fail when device is missing");
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("nonexistent"),
        "error should mention the device id, got: {msg}"
    );
}

// =============================================================================
// execute_read tests
// =============================================================================

#[tokio::test]
async fn test_execute_read_returns_value() {
    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(FixedReadableFactory { value: 3.14 }));
    registry
        .register_from_toml(
            "detector",
            "Test Detector",
            "fixed_readable_cd",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();

    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let value = dispatcher.execute_read("detector").await.unwrap();
    assert!((value - 3.14).abs() < f64::EPSILON, "got {value}");
}

#[tokio::test]
async fn test_execute_read_missing_device_returns_error() {
    let registry = Arc::new(DeviceRegistry::new());
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let err = dispatcher.execute_read("no_such_device").await;
    assert!(err.is_err());
}

// =============================================================================
// execute_trigger tests
// =============================================================================

#[tokio::test]
async fn test_execute_trigger_increments_count() {
    let count = Arc::new(AtomicU32::new(0));

    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(CountingTriggerFactory {
        count: count.clone(),
    }));
    registry
        .register_from_toml(
            "camera",
            "Test Camera",
            "counting_trigger",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();

    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    dispatcher.execute_trigger("camera").await.unwrap();
    dispatcher.execute_trigger("camera").await.unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn test_execute_trigger_missing_device_returns_error() {
    let registry = Arc::new(DeviceRegistry::new());
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let err = dispatcher.execute_trigger("ghost").await;
    assert!(err.is_err());
}

// =============================================================================
// execute_set_parameter tests
// =============================================================================

#[tokio::test]
async fn test_execute_set_parameter_calls_settable() {
    let last_name = Arc::new(Mutex::new(String::new()));
    let triggered = Arc::new(AtomicBool::new(false));

    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(RecordingSettableFactory {
        last_name: last_name.clone(),
        triggered: triggered.clone(),
    }));
    registry
        .register_from_toml(
            "dev",
            "Settable Dev",
            "recording_settable",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();

    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    dispatcher
        .execute_set_parameter("dev", "gain", "10")
        .await
        .unwrap();

    assert!(
        triggered.load(Ordering::Relaxed),
        "set_value should have been called"
    );
    assert_eq!(&*last_name.lock().unwrap(), "gain");
}

#[tokio::test]
async fn test_execute_set_parameter_missing_device_returns_error() {
    let registry = Arc::new(DeviceRegistry::new());
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let err = dispatcher
        .execute_set_parameter("ghost", "gain", "10")
        .await;
    assert!(err.is_err());
}

// =============================================================================
// evaluate_condition tests
// =============================================================================

async fn make_readable_registry_for_cd(device_id: &str, value: f64) -> Arc<DeviceRegistry> {
    struct OneShotReadableFactory {
        value: f64,
    }

    struct OneShotReadable(f64);

    #[async_trait]
    impl Readable for OneShotReadable {
        async fn read(&self) -> anyhow::Result<f64> {
            Ok(self.0)
        }
    }

    impl DriverFactory for OneShotReadableFactory {
        fn driver_type(&self) -> &'static str {
            "oneshot_readable_cd"
        }

        fn name(&self) -> &'static str {
            "One-shot Readable"
        }

        fn capabilities(&self) -> &'static [DeviceCapability] {
            &[DeviceCapability::Readable]
        }

        fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
            Ok(())
        }

        fn build(
            &self,
            _config: toml::Value,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
            let v = self.value;
            Box::pin(async move {
                Ok(DeviceComponents::new()
                    .with_category(DeviceCategory::Detector)
                    .with_readable(Arc::new(OneShotReadable(v))))
            })
        }
    }

    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(OneShotReadableFactory { value }));
    registry
        .register_from_toml(
            device_id,
            "Test Readable",
            "oneshot_readable_cd",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();
    registry
}

#[tokio::test]
async fn test_evaluate_condition_threshold_above_true() {
    let registry = make_readable_registry_for_cd("pm", 10.0).await;
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let condition = EvalCondition::Threshold {
        device_id: DeviceId::from("pm"),
        field: "value".to_string(),
        threshold: 5.0,
        above: true,
    };

    assert!(dispatcher.evaluate_condition(&condition).await);
}

#[tokio::test]
async fn test_evaluate_condition_threshold_above_false() {
    let registry = make_readable_registry_for_cd("pm", 2.0).await;
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let condition = EvalCondition::Threshold {
        device_id: DeviceId::from("pm"),
        field: "value".to_string(),
        threshold: 5.0,
        above: true,
    };

    assert!(!dispatcher.evaluate_condition(&condition).await);
}

#[tokio::test]
async fn test_evaluate_condition_threshold_below_true() {
    let registry = make_readable_registry_for_cd("pm", 1.0).await;
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let condition = EvalCondition::Threshold {
        device_id: DeviceId::from("pm"),
        field: "value".to_string(),
        threshold: 5.0,
        above: false,
    };

    assert!(dispatcher.evaluate_condition(&condition).await);
}

#[tokio::test]
async fn test_evaluate_condition_missing_device_returns_false() {
    let registry = Arc::new(DeviceRegistry::new());
    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let condition = EvalCondition::Threshold {
        device_id: DeviceId::from("nonexistent"),
        field: "value".to_string(),
        threshold: 5.0,
        above: true,
    };

    assert!(!dispatcher.evaluate_condition(&condition).await);
}

#[tokio::test]
async fn test_evaluate_condition_comparison_equal() {
    struct ReadableAFactory;
    struct ReadableA;

    #[async_trait]
    impl Readable for ReadableA {
        async fn read(&self) -> anyhow::Result<f64> {
            Ok(7.0)
        }
    }

    impl DriverFactory for ReadableAFactory {
        fn driver_type(&self) -> &'static str {
            "cmp_readable_a"
        }
        fn name(&self) -> &'static str {
            "Cmp A"
        }
        fn capabilities(&self) -> &'static [DeviceCapability] {
            &[DeviceCapability::Readable]
        }
        fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
            Ok(())
        }
        fn build(
            &self,
            _config: toml::Value,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
            Box::pin(async move {
                Ok(DeviceComponents::new()
                    .with_category(DeviceCategory::Detector)
                    .with_readable(Arc::new(ReadableA)))
            })
        }
    }

    struct ReadableBFactory;
    struct ReadableB;

    #[async_trait]
    impl Readable for ReadableB {
        async fn read(&self) -> anyhow::Result<f64> {
            Ok(7.0)
        }
    }

    impl DriverFactory for ReadableBFactory {
        fn driver_type(&self) -> &'static str {
            "cmp_readable_b"
        }
        fn name(&self) -> &'static str {
            "Cmp B"
        }
        fn capabilities(&self) -> &'static [DeviceCapability] {
            &[DeviceCapability::Readable]
        }
        fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
            Ok(())
        }
        fn build(
            &self,
            _config: toml::Value,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
            Box::pin(async move {
                Ok(DeviceComponents::new()
                    .with_category(DeviceCategory::Detector)
                    .with_readable(Arc::new(ReadableB)))
            })
        }
    }

    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(ReadableAFactory));
    registry.register_factory(Box::new(ReadableBFactory));
    registry
        .register_from_toml(
            "dev_a",
            "Device A",
            "cmp_readable_a",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();
    registry
        .register_from_toml(
            "dev_b",
            "Device B",
            "cmp_readable_b",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();

    let (feedback_tx, _feedback_rx) = make_feedback_channel();
    let dispatcher = CommandDispatcher {
        registry: &registry,
        feedback_tx: &feedback_tx,
    };

    let condition = EvalCondition::Comparison {
        left_device_id: DeviceId::from("dev_a"),
        left_field: "value".to_string(),
        right_device_id: DeviceId::from("dev_b"),
        right_field: "value".to_string(),
        operator: ComparisonOp::Eq,
    };

    assert!(dispatcher.evaluate_condition(&condition).await);
}
