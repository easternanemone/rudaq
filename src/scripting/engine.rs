use rhai::{Dynamic, Engine, EvalAltResult, Scope};
use tokio::runtime::Handle;

use crate::scripting::bindings;

pub struct ScriptHost {
    engine: Engine,
    #[allow(dead_code)]
    runtime: Handle,
}

impl ScriptHost {
    pub fn new(runtime: Handle) -> Self {
        let mut engine = Engine::new();

        // Safety: Limit operations to prevent infinite loops
        engine.on_progress(|count| {
            if count > 10000 {
                Some("Safety limit exceeded: maximum 10000 operations".into())
            } else {
                None
            }
        });

        Self { engine, runtime }
    }

    /// Create ScriptHost with hardware bindings registered
    ///
    /// This enables scripts to control hardware devices through
    /// StageHandle and CameraHandle types.
    ///
    /// # Example
    /// ```rust,ignore
    /// let host = ScriptHost::with_hardware(Handle::current());
    /// ```
    pub fn with_hardware(runtime: Handle) -> Self {
        let mut engine = Engine::new();

        // Safety limit
        engine.on_progress(|count| {
            if count > 10000 {
                Some("Safety limit exceeded: maximum 10000 operations".into())
            } else {
                None
            }
        });

        // Register hardware bindings
        bindings::register_hardware(&mut engine);

        Self { engine, runtime }
    }

    pub fn run_script(&self, script: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        let mut scope = Scope::new();
        self.engine.eval_with_scope(&mut scope, script)
    }

    pub fn validate_script(&self, script: &str) -> Result<(), Box<EvalAltResult>> {
        self.engine.compile(script)?;
        Ok(())
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
}
