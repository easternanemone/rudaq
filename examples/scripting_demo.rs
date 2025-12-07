// Standalone demo of scripting safety features
// This can be run independently to verify scripting works correctly

use rust_daq::scripting::{RhaiEngine, ScriptEngine};

#[tokio::main]
async fn main() {
    println!("=== Scripting Safety Demo (V5 RhaiEngine) ===\n");

    let mut engine = RhaiEngine::new().expect("Failed to create engine");

    // Test 1: Simple arithmetic
    println!("Test 1: Simple arithmetic (5 + 5)");
    match engine.execute_script("5 + 5").await {
        Ok(result) => {
            let val: i64 = result.downcast().unwrap_or(0);
            println!("  ✓ Result: {}\n", val);
        }
        Err(e) => println!("  ✗ Error: {}\n", e),
    }

    // Test 2: Script validation (valid)
    println!("Test 2: Valid script validation");
    match engine.validate_script("let x = 10;").await {
        Ok(_) => println!("  ✓ Script is valid\n"),
        Err(e) => println!("  ✗ Validation error: {}\n", e),
    }

    // Test 3: Script validation (invalid syntax)
    println!("Test 3: Invalid script validation");
    match engine.validate_script("let x = ;").await {
        Ok(_) => println!("  ✗ Should have failed\n"),
        Err(e) => println!("  ✓ Correctly caught syntax error: {}\n", e),
    }

    // Test 4: Safety limit (infinite loop)
    println!("Test 4: Safety limit (infinite loop)");
    match engine.execute_script("loop { }").await {
        Ok(_) => println!("  ✗ Infinite loop should have been stopped\n"),
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("Safety limit exceeded") {
                println!("  ✓ Safety limit triggered: {}\n", e);
            } else {
                println!("  ? Stopped but with different error: {}\n", e);
            }
        }
    }

    // Test 5: Large but finite loop (should complete)
    println!("Test 5: Large but finite loop (9000 iterations)");
    match engine
        .execute_script("let x = 0; for i in 0..9000 { x += 1; } x")
        .await
    {
        Ok(result) => {
            let val: i64 = result.downcast().unwrap_or(0);
            println!("  ✓ Completed: {}\n", val);
        }
        Err(e) => println!("  ✗ Error: {}\n", e),
    }

    // Test 6: Exceeding safety limit with calculations
    println!("Test 6: Exceeding limit (15000 operations)");
    match engine
        .execute_script("let x = 0; for i in 0..15000 { x += 1; } x")
        .await
    {
        Ok(result) => println!("  ✗ Should have hit safety limit but got: {:?}\n", result),
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("Safety limit exceeded") {
                println!("  ✓ Safety limit triggered: {}\n", e);
            } else {
                println!("  ? Different error: {}\n", e);
            }
        }
    }

    println!("=== Demo Complete ===");
}
