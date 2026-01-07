#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::unwrap_err_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    missing_docs
)]
//! Standalone test to verify scripting engine works correctly.
//! This test is independent of the rest of the codebase.

use daq_scripting::rhai::{Dynamic, Engine, EvalAltResult, Scope};

struct TestScriptHost {
    engine: Engine,
}

impl TestScriptHost {
    fn new() -> Self {
        let mut engine = Engine::new();

        // Safety: Limit operations to prevent infinite loops
        engine.on_progress(|count| {
            if count > 10000 {
                Some("Safety limit exceeded: maximum 10000 operations".into())
            } else {
                None
            }
        });

        Self { engine }
    }

    fn run_script(&self, script: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        let mut scope = Scope::new();
        self.engine.eval_with_scope(&mut scope, script)
    }

    fn validate_script(&self, script: &str) -> Result<(), Box<EvalAltResult>> {
        self.engine.compile(script)?;
        Ok(())
    }
}

#[test]
fn test_simple_script() {
    let host = TestScriptHost::new();
    let result = host.run_script("5 + 5").unwrap();
    assert_eq!(result.as_int().unwrap(), 10);
}

#[test]
fn test_safety_limit() {
    let host = TestScriptHost::new();
    let infinite_loop = "loop { }";
    let result = host.run_script(infinite_loop);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    // The error contains "Script terminated" with our safety message in the debug output
    assert!(err_msg.contains("Script terminated") || err_msg.contains("Safety limit exceeded"));
}

#[test]
fn test_script_validation() {
    let host = TestScriptHost::new();

    // Valid script
    assert!(host.validate_script("let x = 10;").is_ok());

    // Invalid syntax
    assert!(host.validate_script("let x = ;").is_err());
}

#[test]
fn test_large_but_valid_loop() {
    let host = TestScriptHost::new();
    // 9000 operations should be within the 10000 limit
    let result = host.run_script("let x = 0; for i in 0..9000 { x += 1; } x");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_int().unwrap(), 9000);
}

#[test]
fn test_exceeding_safety_limit() {
    let host = TestScriptHost::new();
    // 15000 operations should exceed the 10000 limit
    let result = host.run_script("let x = 0; for i in 0..15000 { x += 1; } x");

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Script terminated") || err_msg.contains("Safety limit exceeded"));
}
