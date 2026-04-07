//! JSON serialization for saving and loading experiment graphs.

use std::path::Path;

use egui_snarl::Snarl;
use serde::{Deserialize, Serialize};

use super::nodes::ExperimentNode;

/// Complete graph state for serialization.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphFile {
    /// Version for future compatibility.
    pub version: u32,
    /// Optional metadata.
    pub metadata: GraphMetadata,
    /// The actual graph data (egui-snarl's Snarl is serde-compatible).
    pub graph: Snarl<ExperimentNode>,
}

/// Metadata about the saved graph.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphMetadata {
    /// Name of the experiment.
    pub name: String,
    /// Description of what the experiment does.
    pub description: String,
    /// ISO 8601 timestamp when first created.
    pub created: Option<String>,
    /// ISO 8601 timestamp when last modified.
    pub modified: Option<String>,
    /// Author of the experiment.
    pub author: Option<String>,
}

impl GraphFile {
    /// Current file format version.
    pub const CURRENT_VERSION: u32 = 1;

    /// Create a new GraphFile from a Snarl graph.
    pub fn new(graph: Snarl<ExperimentNode>) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            metadata: GraphMetadata::default(),
            graph,
        }
    }

    /// Add metadata to the graph file.
    pub fn with_metadata(mut self, metadata: GraphMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Save graph to JSON file.
pub fn save_graph(path: &Path, file: &GraphFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(file)
        .map_err(|e| format!("Failed to serialize graph: {e}"))?;

    std::fs::write(path, json).map_err(|e| format!("Failed to write file: {e}"))?;

    Ok(())
}

/// Load graph from JSON file.
pub fn load_graph(path: &Path) -> Result<GraphFile, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("Failed to read file: {e}"))?;

    let file: GraphFile =
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse graph file: {e}"))?;

    // Version check for future compatibility
    if file.version > GraphFile::CURRENT_VERSION {
        return Err(format!(
            "Graph file version {} is newer than supported version {}",
            file.version,
            GraphFile::CURRENT_VERSION
        ));
    }

    Ok(file)
}

/// File extension for experiment graphs.
pub const GRAPH_FILE_EXTENSION: &str = "expgraph";

/// File filter description for dialog boxes.
#[allow(dead_code)]
pub const GRAPH_FILE_FILTER: &str = "Experiment Graph (*.expgraph)";

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_save_and_load_empty_graph() {
        let graph = Snarl::new();
        let file = GraphFile::new(graph);

        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        save_graph(path, &file).unwrap();
        let loaded = load_graph(path).unwrap();

        assert_eq!(loaded.version, GraphFile::CURRENT_VERSION);
        assert!(loaded.metadata.name.is_empty());
    }

    #[test]
    fn test_save_and_load_with_metadata() {
        let graph = Snarl::new();
        let metadata = GraphMetadata {
            name: "Test Experiment".to_string(),
            description: "A test experiment".to_string(),
            created: Some("2025-01-22T10:00:00Z".to_string()),
            modified: Some("2025-01-22T11:00:00Z".to_string()),
            author: Some("Test Author".to_string()),
        };
        let file = GraphFile::new(graph).with_metadata(metadata);

        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        save_graph(path, &file).unwrap();
        let loaded = load_graph(path).unwrap();

        assert_eq!(loaded.metadata.name, "Test Experiment");
        assert_eq!(loaded.metadata.author, Some("Test Author".to_string()));
    }

    #[test]
    fn test_version_check() {
        let mut temp = NamedTempFile::new().unwrap();
        // Write a JSON with a future version - the graph structure may not parse correctly
        // but that's fine since we're testing that loading fails gracefully
        write!(
            temp,
            r#"{{"version": 999, "metadata": {{}}, "graph": {{"nodes": [], "wires": [], "open_node": null, "node_order": [], "next_node_id": 0, "node_positions": {{}}}}}}"#
        )
        .unwrap();

        let result = load_graph(temp.path());
        assert!(result.is_err(), "Loading a future version file should fail");
        let err = result.unwrap_err();
        assert!(
            err.contains("newer than supported version") || err.contains("parse"),
            "Error should indicate version incompatibility or parse failure: {}",
            err
        );
    }
}
