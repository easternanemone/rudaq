use std::collections::{HashMap, HashSet};

pub struct DependencyGraph {
    // Maps instrument_id -> Set of (module_id, role)
    instrument_to_modules: HashMap<String, HashSet<(String, String)>>,
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            instrument_to_modules: HashMap::new(),
        }
    }

    pub fn add_assignment(&mut self, module_id: &str, role: &str, instrument_id: &str) {
        self.instrument_to_modules
            .entry(instrument_id.to_string())
            .or_default()
            .insert((module_id.to_string(), role.to_string()));
    }

    pub fn remove_assignment(&mut self, module_id: &str, instrument_id: &str) {
        if let Some(modules) = self.instrument_to_modules.get_mut(instrument_id) {
            modules.retain(|(mid, _)| mid != module_id);
        }
    }

    pub fn get_dependents(&self, instrument_id: &str) -> Vec<(String, String)> {
        self.instrument_to_modules
            .get(instrument_id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn can_remove(&self, instrument_id: &str) -> Result<(), Vec<String>> {
        let dependents = self.get_dependents(instrument_id);
        if dependents.is_empty() {
            Ok(())
        } else {
            let module_ids: Vec<String> = dependents.into_iter().map(|(mid, _)| mid).collect();
            Err(module_ids)
        }
    }

    pub fn remove_all(&mut self, instrument_id: &str) {
        self.instrument_to_modules.remove(instrument_id);
    }
}
