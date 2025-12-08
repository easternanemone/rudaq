// Global variables demonstration
// This script uses global variables set from the command line
// Run with: cargo run --bin script_runner -- --global max_iterations=50 examples/scripts/globals_demo.rhai

print(`Max iterations: ${max_iterations}`);

let result = 0;

for i in 0..max_iterations {
    result += i;
}

print(`Result after ${max_iterations} iterations: ${result}`);

result
