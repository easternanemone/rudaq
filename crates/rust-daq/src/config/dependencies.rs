//! Dependency tracking for instrument-to-module assignments.
//!
//! This module provides a dependency graph to track which modules depend on which
//! instruments. This is critical for:
//! - Preventing removal of instruments that are in use
//! - Understanding impact when reconfiguring instruments
//! - Validating module configurations
//!
//! # Architecture
//!
//! The dependency graph maintains a mapping from instrument IDs to the set of
//! (module_id, role) pairs that depend on them. This allows:
//! - Quick lookups of which modules use an instrument
//! - Validation before removing an instrument
//! - Cleanup when modules are removed
//!
//! # Example
//!
//! ```rust
//! use rust_daq::config::dependencies::DependencyGraph;
//!
//! let mut graph = DependencyGraph::new();
//!
//! // Module "polarimetry" uses instrument "camera" as "detector"
//! graph.add_assignment("polarimetry", "detector", "camera");
//! graph.add_assignment("polarimetry", "rotation_stage", "rotation_mount");
//!
//! // Check if camera can be removed
//! match graph.can_remove("camera") {
//!     Ok(()) => println!("Can safely remove camera"),
//!     Err(modules) => println!("Camera used by modules: {:?}", modules),
//! }
//! ```

use std::collections::{HashMap, HashSet};

/// Dependency graph tracking module-to-instrument assignments.
///
/// Tracks which modules are using which instruments and in what roles.
/// Prevents removal of instruments that are still assigned to active modules.
///
/// # Architecture
///
/// - Maintains a map: `instrument_id` → set of `(module_id, role)` pairs
/// - Supports adding, removing, and querying assignments
/// - Provides safety checks before instrument removal
///
/// # Example
///
/// ```rust
/// use rust_daq::config::dependencies::DependencyGraph;
///
/// let mut graph = DependencyGraph::new();
///
/// // Module "polarimetry" uses "camera1" in "detector" role
/// graph.add_assignment("polarimetry", "detector", "camera1");
/// graph.add_assignment("polarimetry", "illumination", "laser1");
///
/// // Check if instrument can be removed
/// match graph.can_remove("camera1") {
///     Ok(_) => println!("Safe to remove"),
///     Err(modules) => println!("Used by modules: {:?}", modules),
/// }
///
/// // Remove assignment
/// graph.remove_assignment("polarimetry", "camera1");
/// ```
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
    /// Creates a new empty dependency graph.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_daq::config::dependencies::DependencyGraph;
    ///
    /// let graph = DependencyGraph::new();
    /// ```
    pub fn new() -> Self {
        Self {
            instrument_to_modules: HashMap::new(),
        }
    }

    /// Records that a module uses an instrument in a specific role.
    ///
    /// If the assignment already exists, this is a no-op (sets are idempotent).
    ///
    /// # Arguments
    ///
    /// * `module_id` - Identifier of the module making the assignment
    /// * `role` - Role name for this instrument within the module (e.g., "detector", "source", "stage_x")
    /// * `instrument_id` - Identifier of the instrument being assigned
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rust_daq::config::dependencies::DependencyGraph;
    /// let mut graph = DependencyGraph::new();
    ///
    /// // Polarimetry module uses camera1 as detector
    /// graph.add_assignment("polarimetry", "detector", "camera1");
    ///
    /// // Power calibration module also uses camera1 (different role)
    /// graph.add_assignment("power_cal", "reference_detector", "camera1");
    /// ```
    pub fn add_assignment(&mut self, module_id: &str, role: &str, instrument_id: &str) {
        self.instrument_to_modules
            .entry(instrument_id.to_string())
            .or_default()
            .insert((module_id.to_string(), role.to_string()));
    }

    /// Removes all assignments from a specific module to an instrument.
    ///
    /// Removes all roles that the module assigned to this instrument.
    /// If the instrument has no remaining assignments, it can be safely removed.
    ///
    /// # Arguments
    ///
    /// * `module_id` - Identifier of the module to remove assignments from
    /// * `instrument_id` - Identifier of the instrument to unassign
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rust_daq::config::dependencies::DependencyGraph;
    /// let mut graph = DependencyGraph::new();
    /// graph.add_assignment("module1", "detector", "camera1");
    /// graph.add_assignment("module1", "reference", "camera1");
    ///
    /// // Removes both "detector" and "reference" assignments
    /// graph.remove_assignment("module1", "camera1");
    /// ```
    pub fn remove_assignment(&mut self, module_id: &str, instrument_id: &str) {
        if let Some(modules) = self.instrument_to_modules.get_mut(instrument_id) {
            modules.retain(|(mid, _)| mid != module_id);
        }
    }

    /// Returns all modules currently using the specified instrument.
    ///
    /// # Arguments
    ///
    /// * `instrument_id` - Identifier of the instrument to query
    ///
    /// # Returns
    ///
    /// Returns a vector of `(module_id, role)` tuples. If the instrument has no
    /// dependents, returns an empty vector.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rust_daq::config::dependencies::DependencyGraph;
    /// let mut graph = DependencyGraph::new();
    /// graph.add_assignment("polarimetry", "detector", "camera1");
    /// graph.add_assignment("imaging", "sensor", "camera1");
    ///
    /// let dependents = graph.get_dependents("camera1");
    /// assert_eq!(dependents.len(), 2);
    /// // dependents contains ("polarimetry", "detector") and ("imaging", "sensor")
    /// ```
    pub fn get_dependents(&self, instrument_id: &str) -> Vec<(String, String)> {
        self.instrument_to_modules
            .get(instrument_id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Checks if an instrument can be safely removed.
    ///
    /// An instrument can be removed only if no modules currently depend on it.
    ///
    /// # Arguments
    ///
    /// * `instrument_id` - Identifier of the instrument to check
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the instrument has no dependents and can be removed
    /// - `Err(module_ids)` if the instrument is in use, with a list of module IDs that depend on it
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rust_daq::config::dependencies::DependencyGraph;
    /// let mut graph = DependencyGraph::new();
    /// graph.add_assignment("module1", "detector", "camera1");
    ///
    /// match graph.can_remove("camera1") {
    ///     Ok(_) => {
    ///         // Safe to remove
    ///         graph.remove_all("camera1");
    ///     }
    ///     Err(modules) => {
    ///         println!("Cannot remove camera1: used by {:?}", modules);
    ///     }
    /// }
    /// ```
    pub fn can_remove(&self, instrument_id: &str) -> Result<(), Vec<String>> {
        let dependents = self.get_dependents(instrument_id);
        if dependents.is_empty() {
            Ok(())
        } else {
            let module_ids: Vec<String> = dependents.into_iter().map(|(mid, _)| mid).collect();
            Err(module_ids)
        }
    }

    /// Removes all dependency information for an instrument.
    ///
    /// Clears all module assignments to this instrument. Use after verifying
    /// with [`can_remove`] or when forcibly removing an instrument.
    ///
    /// # Arguments
    ///
    /// * `instrument_id` - Identifier of the instrument to remove
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rust_daq::config::dependencies::DependencyGraph;
    /// let mut graph = DependencyGraph::new();
    /// graph.add_assignment("module1", "detector", "camera1");
    ///
    /// // Force removal regardless of dependencies
    /// graph.remove_all("camera1");
    /// assert_eq!(graph.get_dependents("camera1").len(), 0);
    /// ```
    pub fn remove_all(&mut self, instrument_id: &str) {
        self.instrument_to_modules.remove(instrument_id);
    }
}
