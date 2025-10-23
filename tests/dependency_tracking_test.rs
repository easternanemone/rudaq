use anyhow::Result;
use rust_daq::{
    app_actor::DaqManagerActor,
    config::dependencies::DependencyGraph,
    instrument::InstrumentRegistry,
    measurement::InstrumentMeasurement,
    modules::{
        Module, ModuleCapabilityRequirement, ModuleConfig, ModuleInstrumentAssignment,
        ModuleRegistry, ModuleStatus,
    },
};
use std::{any::TypeId, collections::HashMap, sync::Arc};
use tokio::runtime::Runtime;

// A mock capability for testing
fn mock_capability_id() -> TypeId {
    TypeId::of::<u32>()
}

// A mock module for testing dependency tracking
struct MockModule {}

impl Module for MockModule {
    fn name(&self) -> &str {
        "mock_module"
    }

    fn init(&mut self, _config: ModuleConfig) -> Result<()> {
        Ok(())
    }

    fn status(&self) -> ModuleStatus {
        ModuleStatus::Initialized
    }

    fn required_capabilities(&self) -> Vec<ModuleCapabilityRequirement> {
        vec![ModuleCapabilityRequirement::new(
            "mock_role",
            mock_capability_id(),
        )]
    }

    fn assign_instrument(&mut self, _assignment: ModuleInstrumentAssignment) -> Result<()> {
        Ok(())
    }
}

fn create_test_actor(runtime: Arc<Runtime>) -> DaqManagerActor<InstrumentMeasurement> {
    let settings = Arc::new(rust_daq::config::Settings {
        log_level: "info".to_string(),
        application: rust_daq::config::ApplicationSettings {
            broadcast_channel_capacity: 64,
            command_channel_capacity: 16,
        },
        storage: rust_daq::config::StorageSettings {
            default_path: "./data".to_string(),
            default_format: "csv".to_string(),
        },
        instruments: HashMap::new(),
        processors: None,
    });

    let instrument_registry = Arc::new(InstrumentRegistry::new());
    let processor_registry = Arc::new(rust_daq::data::registry::ProcessorRegistry::new());
    let mut module_registry = ModuleRegistry::new();
    module_registry.register("mock_module", |_name| Box::new(MockModule {}));

    DaqManagerActor::<InstrumentMeasurement>::new(
        settings,
        instrument_registry,
        processor_registry,
        Arc::new(module_registry),
        rust_daq::log_capture::LogBuffer::new(),
        runtime,
    )
    .unwrap()
}

#[tokio::test]
async fn test_assignment_tracking() -> Result<()> {
    let mut dependency_graph = DependencyGraph::new();
    dependency_graph.add_assignment("module1", "role1", "instrument1");
    let dependents = dependency_graph.get_dependents("instrument1");
    assert_eq!(dependents.len(), 1);
    assert_eq!(dependents[0], ("module1".to_string(), "role1".to_string()));
    Ok(())
}

#[tokio::test]
async fn test_removal_blocked_when_in_use() -> Result<()> {
    let mut dependency_graph = DependencyGraph::new();
    dependency_graph.add_assignment("module1", "role1", "instrument1");
    let result = dependency_graph.can_remove("instrument1");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), vec!["module1".to_string()]);
    Ok(())
}

#[test]
fn test_removal_logic_in_actor() -> Result<()> {
    let runtime = Arc::new(Runtime::new()?);
    let mut actor = create_test_actor(runtime.clone());

    runtime.block_on(async {
        // Spawn module and instrument
        actor
            .spawn_module("mm1", "mock_module", ModuleConfig::new())
            .unwrap();

        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let handle = rust_daq::core::InstrumentHandle {
            task: tokio::spawn(async { Ok(()) }),
            command_tx: tx,
            capabilities: vec![mock_capability_id()],
        };
        actor.instruments.insert("instrument1".to_string(), handle);

        // Assign instrument to module
        actor
            .assign_instrument_to_module("mm1", "mock_role", "instrument1")
            .await
            .unwrap();

        // Verify assignment was tracked
        assert_eq!(
            actor
                .dependency_graph
                .get_dependents("instrument1")
                .len(),
            1
        );

        // Test that removal is blocked
        let result = actor
            .remove_instrument_dynamic("instrument1", false)
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("in use by modules"));

        // Test that force removal works
        let result = actor
            .remove_instrument_dynamic("instrument1", true)
            .await;
        assert!(result.is_ok());

        // Verify that the dependency graph is cleaned up
        assert!(actor
            .dependency_graph
            .get_dependents("instrument1")
            .is_empty());
    });
    Ok(())
}

#[tokio::test]
async fn test_graph_cleanup_on_removal() -> Result<()> {
    let mut dependency_graph = DependencyGraph::new();
    dependency_graph.add_assignment("module1", "role1", "instrument1");
    dependency_graph.remove_all("instrument1");
    let dependents = dependency_graph.get_dependents("instrument1");
    assert!(dependents.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_concurrent_access() -> Result<()> {
    let dependency_graph = Arc::new(tokio::sync::Mutex::new(DependencyGraph::new()));
    let mut handles = vec![];

    for i in 0..10 {
        let graph_clone = dependency_graph.clone();
        handles.push(tokio::spawn(async move {
            let mut graph = graph_clone.lock().await;
            graph.add_assignment(&format!("module{}", i), "role", "instrument");
        }));
    }

    for handle in handles {
        handle.await?;
    }

    let graph = dependency_graph.lock().await;
    assert_eq!(graph.get_dependents("instrument").len(), 10);
    Ok(())
}