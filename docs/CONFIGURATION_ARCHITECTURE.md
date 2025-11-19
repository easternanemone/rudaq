🏗️ Figment & Serde Integration Plan

Status: Proposed

Target Architecture: V5 (Headless-First)

Related Modules: src/config.rs, tools/discovery, src/main.rs

1. Conceptual Analysis: The Power of Layering

The core philosophy of this migration is to move from "loading a file" to "building a configuration state."

The Role of Serde

Serde (SERializing/DEserializing) provides the type system bridge. It allows us to define our configuration as strongly-typed Rust structs (Settings, ApplicationConfig).

Deserialization: Converts the final, messy dictionary of values from files/env-vars into a strict Rust struct. If a value is missing or the wrong type, Serde rejects it immediately.

Serialization: Used by Figment to turn our Default struct back into a data map to serve as the "Base Layer."

The Role of Figment

Figment acts as a funnel that merges multiple data sources (Providers) into a single dictionary before handing it to Serde.

The Layering Strategy:

Base Layer (Code): Hardcoded defaults defined in Rust. Guarantees that every field has a valid value initially.

File Layer (User): config.v4.toml. Overrides specific fields (e.g., port number, log level).

Environment Layer (Ops): RUST_DAQ_* environment variables. Perfect for Docker/CI overrides.

CLI Layer (Runtime): Flags passed to the binary (e.g., --config path/to/custom.toml).

Architecture Diagram

graph TD
    A[Default Struct] -->|Serialize| B(Figment Data Map)
    C[config.v4.toml] -->|Toml Parser| B
    D[Env Vars RUST_DAQ_*] -->|Env Parser| B
    E[CLI Arguments] -->|Clap Parser| B
    
    B -->|Merge & Overlay| F{Combined Dictionary}
    F -->|Deserialize| G[Final Settings Struct]
    G -->|Validate| H[Validated Config]


2. Hierarchical Beads Roadmap

This roadmap is structured to allow incremental migration. The "Bead Groups" represent major milestones.

🔴 Bead Group 1: Foundation & Typing

Goal: Prepare the data structures without changing the loading logic yet.

Task 1.1: Dependency Update

Add figment = { version = "0.10", features = ["toml", "env"] } to Cargo.toml.

Ensure serde has derive feature.

Task 1.2: Struct Refactoring

Modify Settings, ApplicationSettings, TimeoutSettings in src/config.rs.

Derive Serialize (for Defaults) and Deserialize (for Loading) on all of them.

Remove legacy config crate annotations if they conflict.

Task 1.3: Default Implementation

Implement impl Default for the top-level Settings struct.

Move all "hardcoded" values (like 1024 capacity, 5000ms timeouts) into these Default implementations.

🟠 Bead Group 2: The Figment Provider

Goal: Implement the "Base Layer" logic.

Task 2.1: The Provider Trait

Implement figment::Provider for Settings.

Use Serialized::defaults(Settings::default()) to return the data map.

Task 2.2: The Constructor

Create Settings::load_v5() (temporary name to coexist with new).

Chain: Figment::from(Settings::default()).

Task 2.3: File Integration

Add .merge(Toml::file("config/config.v4.toml")).

Add logic to handle the optional nature of the file (don't panic if missing, just warn).

🟡 Bead Group 3: Environment & Validation

Goal: Feature parity with the old system.

Task 3.1: Environment Mapping

Add .merge(Env::prefixed("RUST_DAQ_").split("__")).

Verify that RUST_DAQ_APPLICATION__TIMEOUTS__SERIAL_READ_TIMEOUT_MS maps correctly to nested structs.

Task 3.2: Validation Migration

Port the existing validate() method.

Ensure it runs after figment.extract()?.

Decouple validation from the loading process (it should take &self).

🟢 Bead Group 4: Integration & Cleanup

Goal: Switch the application to the new system.

Task 4.1: CLI Switchover

Update src/main.rs to use Settings::load_v5().

Update tools/discovery/main.rs to read config via Figment (for verification pass).

Task 4.2: Dynamic Patching Support

Ensure tools/discovery still uses toml_edit for writing (Figment is read-only).

Task 4.3: Legacy Removal

Remove the config crate dependency.

Rename load_v5 to new.

Delete src/config_v4.rs.

3. Detailed Implementation Guide

Step 1: The Structs (Serde)

The most important change here is deriving both Serialize and Deserialize. We also use impl Default to define our "Code Layer".

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TimeoutSettings {
    pub serial_read_timeout_ms: u64,
    // ... other fields
}

// This serves as the Source of Truth for defaults
impl Default for TimeoutSettings {
    fn default() -> Self {
        Self {
            serial_read_timeout_ms: 1000,
            serial_write_timeout_ms: 1000,
            // ...
        }
    }
}


Step 2: The Provider (Figment)

This is the "Magic" step. By implementing Provider, our struct becomes a data source itself.

use figment::{Provider, Error, metadata::Metadata, profile::Profile, value::{Map, Dict}};
use figment::providers::Serialized;

impl Provider for Settings {
    fn metadata(&self) -> Metadata {
        Metadata::named("Library Defaults")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        // Serialize 'self' (which contains defaults) into a Figment Map
        Serialized::defaults(Settings::default()).data()
    }
}


Step 3: The Loader (Layering)

This replaces the old Config::builder() logic. Note the explicit handling of nesting using double underscores (__) for environment variables.

use figment::{Figment, providers::{Env, Format, Toml}};

impl Settings {
    pub fn new(config_path: Option<std::path::PathBuf>) -> anyhow::Result<Self> {
        let mut builder = Figment::from(Settings::default()); // Layer 1: Defaults

        // Layer 2: Config File
        // Priority: Explicit CLI path > config.v4.toml > default.toml
        if let Some(path) = config_path {
            builder = builder.merge(Toml::file(path));
        } else {
            builder = builder.merge(Toml::file("config/config.v4.toml"));
        }

        // Layer 3: Environment Variables
        // Example: RUST_DAQ_APPLICATION__TIMEOUTS__SERIAL_READ_TIMEOUT_MS=500
        builder = builder.merge(Env::prefixed("RUST_DAQ_").split("__"));

        // Extract (Deserialize)
        let settings: Settings = builder.extract()?;

        // Validate
        settings.validate()?;

        Ok(settings)
    }
}


Step 4: Advanced - Discovery Tool Integration

The Discovery Tool has a unique requirement: it needs to Read utilizing the complex layering logic (to know where instruments should be), but Write using AST-preserving logic (to update the file without destroying comments).

Pattern:

Read Phase (Figment): Load Settings to get the "Effective Configuration." Use this to skip ports that are already configured.

Scan Phase: Probe hardware.

Write Phase (toml_edit): Open config/config.v4.toml as raw text, parse with toml_edit, patch the specific keys, and save.

Why this separation?

Figment flattens the config. It doesn't know that port = 5000 came from line 10 of the file or from an Env var. You cannot "save" a Figment object back to a file cleanly.

toml_edit preserves the structure but is bad at "effective config" resolution.

Conclusion: Use Figment for Reading (Logic), toml_edit for Writing (Persistence).

// in tools/discovery/main.rs

// 1. Load Effective Config
let settings = rust_daq::config::Settings::new(None)?;

// 2. Check if port is already claimed
if let Some(inst) = settings.instruments.get("my_laser") {
    // logic to skip scanning this port
}

// ... Discovery Loop ...

// 3. Patch File
let raw_config = fs::read_to_string("config/config.v4.toml")?;
let mut doc = raw_config.parse::<DocumentMut>()?;
doc["instruments"][0]["config"]["port"] = value("/dev/ttyUSB1");
fs::write("config/config.v4.toml", doc.to_string())?;

