#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
use daq_scripting::{RhaiEngine, ScriptEngine};

#[tokio::test]
async fn test_simple_script() {
    let mut engine = RhaiEngine::new().unwrap();
    let result = engine.execute_script("5 + 5").await.unwrap();
    // ScriptValue downcast to i64
    assert_eq!(result.downcast::<i64>().unwrap(), 10);
}

#[tokio::test]
async fn test_safety_limit() {
    let mut engine = RhaiEngine::new().unwrap();
    let infinite_loop = "loop { }";
    let result = engine.execute_script(infinite_loop).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    // The error contains "Script terminated" with our safety message in the debug output
    assert!(err_msg.contains("Script terminated") || err_msg.contains("Safety limit exceeded"));
}

#[tokio::test]
async fn test_script_validation() {
    let engine = RhaiEngine::new().unwrap();

    // Valid script
    assert!(engine.validate_script("let x = 10;").await.is_ok());

    // Invalid syntax
    assert!(engine.validate_script("let x = ;").await.is_err());
}
