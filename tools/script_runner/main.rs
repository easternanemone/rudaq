//! Script Runner CLI Tool
//!
//! A command-line tool for executing scripts using the ScriptEngine trait.
//! Supports multiple scripting backends (currently Rhai) with proper error
//! reporting and logging.
//!
//! # Usage
//!
//! ```bash
//! # Run a script file with default (Rhai) engine
//! cargo run --bin script_runner -- script.rhai
//!
//! # Run with explicit engine type
//! cargo run --bin script_runner -- --engine rhai script.rhai
//!
//! # Set global variables before execution
//! cargo run --bin script_runner -- --global max_iterations=1000 script.rhai
//!
//! # Enable verbose logging
//! cargo run --bin script_runner -- --verbose script.rhai
//!
//! # Validate script syntax without execution
//! cargo run --bin script_runner -- --validate script.rhai
//! ```
//!
//! # Features
//!
//! - Multiple scripting backend support (currently Rhai)
//! - Script validation without execution
//! - Global variable injection
//! - Detailed error reporting with line numbers
//! - Configurable logging levels
//! - Operation safety limits

use clap::{Parser, ValueEnum};
use rust_daq::scripting::{RhaiEngine, ScriptEngine, ScriptError, ScriptValue};
use std::fs;
use std::path::PathBuf;
use std::process;

// =============================================================================
// CLI Argument Structure
// =============================================================================

/// Script Runner - Execute scripts using ScriptEngine
#[derive(Parser, Debug)]
#[command(name = "script_runner")]
#[command(author = "rust-daq team")]
#[command(version = "0.1.0")]
#[command(about = "Execute scripts using various scripting backends", long_about = None)]
struct Args {
    /// Path to the script file to execute
    #[arg(value_name = "SCRIPT_FILE")]
    script_path: PathBuf,

    /// Scripting engine to use
    #[arg(short, long, value_enum, default_value_t = EngineType::Rhai)]
    engine: EngineType,

    /// Set global variables (format: key=value)
    #[arg(short, long = "global", value_name = "KEY=VALUE")]
    globals: Vec<String>,

    /// Validate script syntax without execution
    #[arg(short, long)]
    validate: bool,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Maximum number of operations (for safety limits)
    #[arg(long, default_value_t = 10000)]
    max_operations: u64,
}

/// Available scripting engine types
#[derive(Debug, Clone, ValueEnum)]
enum EngineType {
    /// Rhai scripting engine (Rust-like syntax)
    Rhai,
}

// =============================================================================
// Main Entry Point
// =============================================================================

#[tokio::main]
async fn main() {
    // Parse command-line arguments
    let args = Args::parse();

    // Initialize logging
    init_logging(args.verbose);

    // Run the script and handle errors
    if let Err(e) = run_script(&args).await {
        eprintln!("\n{}", format_error(&e));
        process::exit(1);
    }
}

// =============================================================================
// Core Logic
// =============================================================================

/// Execute or validate a script based on command-line arguments
async fn run_script(args: &Args) -> Result<(), ScriptError> {
    // Read script file
    let script_content = read_script_file(&args.script_path)?;

    log::info!(
        "Using {} engine with max {} operations",
        match args.engine {
            EngineType::Rhai => "Rhai",
        },
        args.max_operations
    );

    // Create script engine based on selected type
    let mut engine = create_engine(args.engine, args.max_operations);

    // Set global variables if provided
    set_globals(&mut engine, &args.globals)?;

    // Either validate or execute
    if args.validate {
        validate_script(&engine, &script_content, &args.script_path).await
    } else {
        execute_script(&mut engine, &script_content, &args.script_path).await
    }
}

/// Create a script engine instance based on the specified type
fn create_engine(engine_type: EngineType, max_operations: u64) -> Box<dyn ScriptEngine> {
    match engine_type {
        EngineType::Rhai => {
            log::debug!("Creating Rhai engine with limit {}", max_operations);
            Box::new(RhaiEngine::with_limit(max_operations))
        }
    }
}

/// Read script content from file
fn read_script_file(path: &PathBuf) -> Result<String, ScriptError> {
    log::debug!("Reading script from: {}", path.display());

    fs::read_to_string(path).map_err(|e| ScriptError::BackendError {
        backend: "FileSystem".to_string(),
        message: format!("Failed to read script file '{}': {}", path.display(), e),
    })
}

/// Set global variables from command-line arguments
fn set_globals(
    engine: &mut Box<dyn ScriptEngine>,
    globals: &[String],
) -> Result<(), ScriptError> {
    for global in globals {
        let parts: Vec<&str> = global.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err(ScriptError::BackendError {
                backend: "CLI".to_string(),
                message: format!(
                    "Invalid global variable format '{}'. Expected: key=value",
                    global
                ),
            });
        }

        let (key, value) = (parts[0], parts[1]);
        log::debug!("Setting global: {} = {}", key, value);

        // Parse value (try integer, then float, then string)
        let script_value = parse_value(value);
        engine.set_global(key, script_value)?;
    }

    Ok(())
}

/// Parse a string value into the appropriate type
fn parse_value(value: &str) -> ScriptValue {
    use rhai::Dynamic;

    // Try parsing as integer
    if let Ok(i) = value.parse::<i64>() {
        return ScriptValue::new(Dynamic::from(i));
    }

    // Try parsing as float
    if let Ok(f) = value.parse::<f64>() {
        return ScriptValue::new(Dynamic::from(f));
    }

    // Try parsing as boolean
    if let Ok(b) = value.parse::<bool>() {
        return ScriptValue::new(Dynamic::from(b));
    }

    // Default to string
    ScriptValue::new(Dynamic::from(value.to_string()))
}

/// Validate script syntax without execution
async fn validate_script(
    engine: &Box<dyn ScriptEngine>,
    script: &str,
    path: &PathBuf,
) -> Result<(), ScriptError> {
    log::info!("Validating script: {}", path.display());

    engine.validate_script(script).await?;

    println!("✓ Script validation successful: {}", path.display());
    Ok(())
}

/// Execute script and display result
async fn execute_script(
    engine: &mut Box<dyn ScriptEngine>,
    script: &str,
    path: &PathBuf,
) -> Result<(), ScriptError> {
    log::info!("Executing script: {}", path.display());

    let result = engine.execute_script(script).await?;

    // Display result
    println!("\n=== Script Execution Complete ===");
    println!("Script: {}", path.display());
    println!("Engine: {}", engine.backend_name());

    // Try to display the result value
    display_result(&result);

    Ok(())
}

/// Display the script execution result
fn display_result(result: &ScriptValue) {
    use rhai::Dynamic;

    if let Ok(dynamic) = result.downcast_ref::<Dynamic>() {
        println!("Result: {}", dynamic);
    } else {
        println!("Result: (non-displayable type)");
    }
}

// =============================================================================
// Error Formatting
// =============================================================================

/// Format ScriptError for human-readable display
fn format_error(error: &ScriptError) -> String {
    match error {
        ScriptError::CompilationError {
            message,
            line,
            column,
        } => {
            let mut output = format!("Compilation Error: {}", message);
            if let Some(line_num) = line {
                output.push_str(&format!("\n  at line {}", line_num));
            }
            if let Some(col_num) = column {
                output.push_str(&format!(", column {}", col_num));
            }
            output
        }
        ScriptError::RuntimeError { message, backtrace } => {
            let mut output = format!("Runtime Error: {}", message);
            if let Some(bt) = backtrace {
                output.push_str(&format!("\nBacktrace:\n{}", bt));
            }
            output
        }
        ScriptError::TypeConversionError { expected, found } => {
            format!(
                "Type Conversion Error: expected {}, found {}",
                expected, found
            )
        }
        ScriptError::VariableNotFound { name } => {
            format!("Variable Not Found: {}", name)
        }
        ScriptError::BackendError { backend, message } => {
            format!("{} Error: {}", backend, message)
        }
        _ => format!("Script Error: {}", error),
    }
}

// =============================================================================
// Logging Setup
// =============================================================================

/// Initialize logging based on verbosity level
fn init_logging(verbose: bool) {
    let log_level = if verbose { "debug" } else { "info" };

    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Off)
        .filter_module("script_runner", log_level.parse().unwrap())
        .filter_module("rust_daq", log_level.parse().unwrap())
        .format_timestamp(None)
        .format_module_path(false)
        .init();
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_value_integer() {
        use rhai::Dynamic;
        let value = parse_value("42");
        let dynamic: Dynamic = value.downcast().unwrap();
        assert_eq!(dynamic.as_int().unwrap(), 42);
    }

    #[test]
    fn test_parse_value_float() {
        use rhai::Dynamic;
        let value = parse_value("3.14");
        let dynamic: Dynamic = value.downcast().unwrap();
        assert_eq!(dynamic.as_float().unwrap(), 3.14);
    }

    #[test]
    fn test_parse_value_bool() {
        use rhai::Dynamic;
        let value = parse_value("true");
        let dynamic: Dynamic = value.downcast().unwrap();
        assert_eq!(dynamic.as_bool().unwrap(), true);
    }

    #[test]
    fn test_parse_value_string() {
        use rhai::Dynamic;
        let value = parse_value("hello world");
        let dynamic: Dynamic = value.downcast().unwrap();
        assert_eq!(dynamic.cast::<String>(), "hello world");
    }

    #[test]
    fn test_format_compilation_error() {
        let error = ScriptError::CompilationError {
            message: "unexpected token".to_string(),
            line: Some(10),
            column: Some(5),
        };
        let formatted = format_error(&error);
        assert!(formatted.contains("line 10"));
        assert!(formatted.contains("column 5"));
    }

    #[tokio::test]
    async fn test_validate_script() {
        let engine: Box<dyn ScriptEngine> = Box::new(RhaiEngine::new());
        let script = "let x = 10; x * 2";
        let path = PathBuf::from("test.rhai");

        let result = validate_script(&engine, script, &path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_simple_script() {
        let mut engine: Box<dyn ScriptEngine> = Box::new(RhaiEngine::new());
        let script = "5 + 5";
        let path = PathBuf::from("test.rhai");

        let result = execute_script(&mut engine, script, &path).await;
        assert!(result.is_ok());
    }
}
