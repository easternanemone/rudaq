// Script for testing validation (syntax check)
// This script is syntactically correct but doesn't execute anything meaningful
// Run with: cargo run --bin script_runner -- --validate examples/scripts/validation_test.rhai

fn calculate_something(x, y) {
    return x * y + x - y;
}

let value = calculate_something(10, 5);
print(`Calculated value: ${value}`);

value
