//! Metadata editor widget for capturing experiment metadata.
//!
//! Provides a form for entering common metadata fields (sample ID, operator, purpose, notes)
//! and extensible custom key-value pairs. This metadata enriches StartDoc and persists to HDF5.

use std::collections::HashMap;

/// Widget for editing experiment metadata before and during execution.
///
/// # Example
///
/// ```ignore
/// use daq_egui::widgets::MetadataEditor;
///
/// let mut editor = MetadataEditor::new();
///
/// // In your UI code (inside an egui frame):
/// editor.ui(ui);
///
/// // When queuing a plan:
/// let metadata = editor.to_metadata_map();
/// ```
#[derive(Debug, Clone)]
pub struct MetadataEditor {
    /// Sample identifier (e.g., "Sample-2026-01-22-001")
    sample_id: String,
    /// Operator name or initials
    operator: String,
    /// Purpose or goal of this experiment
    purpose: String,
    /// Freeform notes about the experiment
    notes: String,
    /// Comma-separated tags for categorization
    tags: String,
    /// Extensible custom metadata fields (key, value pairs)
    custom_fields: Vec<(String, String)>,
}

impl Default for MetadataEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataEditor {
    /// Create a new metadata editor with empty fields.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sample_id: String::new(),
            operator: String::new(),
            purpose: String::new(),
            notes: String::new(),
            tags: String::new(),
            custom_fields: Vec::new(),
        }
    }

    /// Render the metadata editor UI.
    ///
    /// Shows common fields, notes area, tags, and extensible custom fields.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Experiment Metadata");
        ui.add_space(4.0);

        // Common fields section
        ui.label("Common Fields:");
        ui.horizontal(|ui| {
            ui.label("Sample ID:");
            ui.text_edit_singleline(&mut self.sample_id)
                .on_hover_text("Identifier for the sample being measured");
        });

        ui.horizontal(|ui| {
            ui.label("Operator:");
            ui.text_edit_singleline(&mut self.operator)
                .on_hover_text("Name or initials of person running the experiment");
        });

        ui.horizontal(|ui| {
            ui.label("Purpose:");
            ui.text_edit_singleline(&mut self.purpose)
                .on_hover_text("Goal or objective of this experiment");
        });

        ui.add_space(4.0);

        // Notes field (multiline)
        ui.label("Notes:");
        ui.text_edit_multiline(&mut self.notes)
            .on_hover_text("Freeform notes about conditions, observations, etc.");

        ui.add_space(4.0);

        // Tags field
        ui.horizontal(|ui| {
            ui.label("Tags:");
            ui.text_edit_singleline(&mut self.tags)
                .on_hover_text("Comma-separated tags (e.g., calibration, test, baseline)");
        });

        ui.add_space(8.0);

        // Custom fields section
        ui.separator();
        ui.label("Custom Fields:");

        let mut to_remove: Option<usize> = None;

        for (idx, (key, value)) in self.custom_fields.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("Key:");
                ui.text_edit_singleline(key);
                ui.label("Value:");
                ui.text_edit_singleline(value);
                if ui.button("✖").on_hover_text("Remove this field").clicked() {
                    to_remove = Some(idx);
                }
            });
        }

        // Remove field if delete button clicked
        if let Some(idx) = to_remove {
            self.custom_fields.remove(idx);
        }

        // Add field button
        if ui.button("+ Add Custom Field").clicked() {
            self.custom_fields.push((String::new(), String::new()));
        }
    }

    /// Convert the metadata to a HashMap for use in StartDoc.metadata.
    ///
    /// All common fields are included (even if empty, for consistency).
    /// Tags are converted to a JSON array string.
    /// Custom fields with non-empty keys are included.
    #[must_use]
    pub fn to_metadata_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();

        // Always include common fields (even if empty)
        map.insert("sample_id".to_string(), self.sample_id.clone());
        map.insert("operator".to_string(), self.operator.clone());
        map.insert("purpose".to_string(), self.purpose.clone());
        map.insert("notes".to_string(), self.notes.clone());

        // Parse tags into JSON array
        if !self.tags.is_empty() {
            let tags_vec: Vec<&str> = self.tags.split(',').map(str::trim).collect();
            // Use unwrap_or_default since to_string shouldn't fail for Vec<&str>
            let tags_json = serde_json::to_string(&tags_vec).unwrap_or_default();
            map.insert("tags".to_string(), tags_json);
        } else {
            map.insert("tags".to_string(), "[]".to_string());
        }

        // Include custom fields with non-empty keys
        for (key, value) in &self.custom_fields {
            if !key.is_empty() {
                map.insert(key.clone(), value.clone());
            }
        }

        map
    }

    /// Check if any metadata has been entered.
    ///
    /// Returns true if all fields are empty.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.sample_id.is_empty()
            && self.operator.is_empty()
            && self.purpose.is_empty()
            && self.notes.is_empty()
            && self.tags.is_empty()
            && self.custom_fields.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_editor() {
        let editor = MetadataEditor::new();
        assert!(editor.is_empty());

        let map = editor.to_metadata_map();
        assert_eq!(map.get("sample_id"), Some(&String::new()));
        assert_eq!(map.get("tags"), Some(&"[]".to_string()));
    }

    #[test]
    fn test_common_fields() {
        let mut editor = MetadataEditor::new();
        editor.sample_id = "SAMPLE-001".to_string();
        editor.operator = "JD".to_string();
        editor.purpose = "Calibration".to_string();

        let map = editor.to_metadata_map();
        assert_eq!(map.get("sample_id"), Some(&"SAMPLE-001".to_string()));
        assert_eq!(map.get("operator"), Some(&"JD".to_string()));
        assert_eq!(map.get("purpose"), Some(&"Calibration".to_string()));
    }

    #[test]
    fn test_tags_parsing() {
        let mut editor = MetadataEditor::new();
        editor.tags = "calibration, test, baseline".to_string();

        let map = editor.to_metadata_map();
        let tags_json = map.get("tags").unwrap();
        let parsed: Vec<String> = serde_json::from_str(tags_json).unwrap();
        assert_eq!(parsed, vec!["calibration", "test", "baseline"]);
    }

    #[test]
    fn test_custom_fields() {
        let mut editor = MetadataEditor::new();
        editor
            .custom_fields
            .push(("temperature".to_string(), "20C".to_string()));
        editor
            .custom_fields
            .push(("humidity".to_string(), "45%".to_string()));

        let map = editor.to_metadata_map();
        assert_eq!(map.get("temperature"), Some(&"20C".to_string()));
        assert_eq!(map.get("humidity"), Some(&"45%".to_string()));
    }

    #[test]
    fn test_empty_key_filtering() {
        let mut editor = MetadataEditor::new();
        editor
            .custom_fields
            .push(("valid_key".to_string(), "value".to_string()));
        editor
            .custom_fields
            .push((String::new(), "ignored".to_string()));

        let map = editor.to_metadata_map();
        assert_eq!(map.get("valid_key"), Some(&"value".to_string()));
        assert!(!map.contains_key(""));
    }
}
